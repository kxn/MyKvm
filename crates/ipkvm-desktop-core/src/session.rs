use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ipkvm_core::{InputSink, MouseMode, MouseProfile};
use ipkvm_session::rfb_connection::{
    RfbClientId, RfbConnectionGate, RfbDisconnectReason, RfbServerEvent,
};
use ipkvm_session::rfb_input::RfbInputNotice;
use ipkvm_session::supervisor::{RecoveryPolicy, SessionSupervisor};
use ipkvm_video::FrameSource;
use thiserror::Error;
use tokio::sync::mpsc;

use crate::render::FrameSize;

/// 桌面连接请求：把设备选择与高级设置固化为一次会话启动的组件参数。
#[derive(Clone, Debug)]
pub struct ConnectRequest {
    pub video_device_id: String,
    pub control_device_id: String,
    pub baud_rate: u32,
    pub mouse_profile: MouseProfile,
    pub mouse_mode: MouseMode,
    pub preview_fps: u64,
}

impl ConnectRequest {
    pub fn resolved_mouse_mode(&self) -> MouseMode {
        self.mouse_profile.resolve_mode()
    }
}

#[derive(Debug, Error)]
pub enum DesktopSessionError {
    #[error("session component build failed: {0}")]
    Build(String),
    #[error("session start failed: {0}")]
    Session(String),
    #[error("no active desktop controller")]
    NoEventSender,
    #[error("input event rejected: {0}")]
    Input(String),
}

/// 会话组件三元组：帧源、输入 sink、连接闸门。
pub type SessionParts<S> = (Arc<dyn FrameSource>, S, RfbConnectionGate);

/// 桌面会话组件工厂。
///
/// `build_video()` 与 `build_control()` 是独立恢复契约：实现方必须分别打开
/// 视频和控制设备，不能通过一次性 `build()` 隐式构造两遍完整会话。
pub trait DesktopSessionFactory<S: InputSink + Clone + Send + 'static> {
    fn build_video(
        &mut self,
        request: &ConnectRequest,
    ) -> Result<Arc<dyn FrameSource>, DesktopSessionError>;

    fn build_control(&mut self, request: &ConnectRequest) -> Result<S, DesktopSessionError>;

    fn build_gate(
        &mut self,
        _request: &ConnectRequest,
    ) -> Result<RfbConnectionGate, DesktopSessionError> {
        Ok(RfbConnectionGate::new())
    }

    fn build(&mut self, request: &ConnectRequest) -> Result<SessionParts<S>, DesktopSessionError> {
        Ok((
            self.build_video(request)?,
            self.build_control(request)?,
            self.build_gate(request)?,
        ))
    }
}

/// 桌面本地会话控制器：内存事件 → 共享 supervisor → 输入泵 → 真实 sink。
///
/// 自身持有 tokio runtime，GUI 线程直接调用；`connect`/`stop` 在 runtime 内
/// block_on 会话生命周期操作，避免 `tokio::spawn` 找不到运行时上下文。
pub struct DesktopSessionController<S, F>
where
    S: InputSink + Clone + Send + 'static,
    F: DesktopSessionFactory<S>,
{
    runtime: tokio::runtime::Runtime,
    supervisor: SessionSupervisor<S>,
    factory: F,
    current_request: Option<ConnectRequest>,
    notice_rx: mpsc::UnboundedReceiver<RfbInputNotice>,
    event_tx: Option<mpsc::Sender<RfbServerEvent>>,
    /// 未送出的事件（通道满时暂存），保证桌面输入不丢事件。
    pending_events: std::sync::Mutex<VecDeque<RfbServerEvent>>,
}

impl<S, F> DesktopSessionController<S, F>
where
    S: InputSink + Clone + Send + 'static,
    F: DesktopSessionFactory<S>,
{
    /// 用组件工厂构造控制器（测试注入 fake sink/帧源，生产用 split factory）。
    pub fn with_factory(factory: F) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("desktop tokio runtime");
        let supervisor =
            SessionSupervisor::new(RfbConnectionGate::new(), RecoveryPolicy::default());
        let (_notice_tx, notice_rx) = mpsc::unbounded_channel();
        Self {
            runtime,
            supervisor,
            factory,
            current_request: None,
            notice_rx,
            event_tx: None,
            pending_events: std::sync::Mutex::new(VecDeque::new()),
        }
    }

    /// 连接：创建新的共享 supervisor，并把视频/控制构建结果交给底层状态机。
    /// 构建失败不会直接退回连接页；supervisor 会进入对应恢复状态。
    pub fn connect(&mut self, request: ConnectRequest) -> Result<(), DesktopSessionError> {
        // 先停旧会话并销毁，释放相机/串口句柄，再构建新组件：Windows 串口
        // 独占，旧句柄不释放时重连/重建必然失败（此前 factory 先于停旧执行，
        // 重连打不开串口，还会造成 UI 模式与 sink 模式分叉）。
        let _ = self.runtime.block_on(self.supervisor.stop_manual());
        self.current_request = None;
        self.event_tx = None;
        self.pending_events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();

        let gate = self.factory.build_gate(&request)?;
        let video_result = self.factory.build_video(&request);
        let control_result = self.factory.build_control(&request);
        let mut supervisor = SessionSupervisor::new(gate, RecoveryPolicy::default());
        // 先建 notice 镜像再启动，避免错过泵的早期 notice；unbounded 通道缓冲
        // 到本控制器持有 receiver 为止。
        let (notice_tx, notice_rx) = mpsc::unbounded_channel();
        supervisor.set_notice_mirror(Some(notice_tx));
        let mut video_result = Some(video_result);
        let mut control_result = Some(control_result);
        self.runtime.block_on(supervisor.start_at(
            move || {
                match video_result
                    .take()
                    .expect("video opener called once during start")
                {
                    Ok(source) => Ok(source),
                    Err(error) => Err(error.to_string()),
                }
            },
            move || {
                match control_result
                    .take()
                    .expect("control opener called once during start")
                {
                    Ok(sink) => Ok(sink),
                    Err(error) => Err(error.to_string()),
                }
            },
            Instant::now(),
        ));
        self.supervisor = supervisor;
        self.notice_rx = notice_rx;
        self.current_request = Some(request);

        self.refresh_event_sender()?;
        Ok(())
    }

    /// 回滚到未连接状态：销毁已组装会话（释放相机/串口）、清空事件出口与
    /// 帧源，并换新 notice 通道，保证失败后状态一致、可再次连接。
    #[cfg(test)]
    fn rollback(&mut self) {
        self.event_tx = None;
        self.current_request = None;
        self.pending_events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        let _ = self.runtime.block_on(self.supervisor.stop_manual());
        let (_notice_tx, notice_rx) = mpsc::unbounded_channel();
        self.notice_rx = notice_rx;
    }

    /// 停止连接并销毁会话组件（释放相机/串口），之后可重新连接。
    pub fn stop(&mut self) -> Result<(), DesktopSessionError> {
        self.event_tx = None;
        self.current_request = None;
        self.pending_events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        self.runtime
            .block_on(self.supervisor.stop_manual())
            .map_err(|error| DesktopSessionError::Session(error.to_string()))
    }

    /// 推进共享恢复状态机。UI 在固定 tick 调用；视频/控制会按底层策略独立重试。
    pub fn tick_recovery(&mut self) -> Result<(), DesktopSessionError> {
        let Some(request) = self.current_request.clone() else {
            return Ok(());
        };
        let factory = std::cell::RefCell::new(&mut self.factory);
        self.runtime.block_on(self.supervisor.tick(
            || {
                factory
                    .borrow_mut()
                    .build_video(&request)
                    .map_err(|error| error.to_string())
            },
            || {
                factory
                    .borrow_mut()
                    .build_control(&request)
                    .map_err(|error| error.to_string())
            },
        ));
        self.refresh_event_sender()
    }

    /// 当前会话帧源的最新帧（渲染与控制台无信号检测用）。
    pub fn latest_frame(&self) -> Option<ipkvm_video::SharedVideoFrame> {
        self.supervisor.latest_frame()
    }

    /// 订阅当前会话帧源：新帧到达时通过返回的 watch receiver 通知。
    ///
    /// 前端可把帧通知转换为自己的重绘信号（例如 iced 的 `Subscription`）。
    /// 连接前帧源为空，返回 `None`。
    pub fn subscribe_frames(&self) -> Option<ipkvm_video::FrameReceiver> {
        self.current_request
            .as_ref()
            .map(|_| self.supervisor.subscribe_frames())
    }

    /// 会话输入泵是否仍在运行。
    pub fn is_running(&self) -> bool {
        self.supervisor.status().should_show_work_view()
    }

    /// 当前是否应显示视频/工作页。控制链路失败或恢复中不应回到连接页。
    pub fn should_show_work_view(&self) -> bool {
        self.supervisor.status().should_show_work_view()
    }

    /// 本地控制器是否在线：已连接且输入泵运行（串口写失败等导致泵退出时为 false）。
    pub fn is_control_online(&self) -> bool {
        self.event_tx.as_ref().is_some_and(|tx| !tx.is_closed())
            && self.supervisor.is_control_ready()
    }

    /// 输入泵离线原因（串口写失败等；离线时用于状态栏诊断）。
    pub fn input_offline_reason(&self) -> Option<String> {
        self.supervisor.input_offline_reason()
    }

    /// 发送键盘事件（内存直喂输入泵，不经网络）。
    pub fn send_key(&self, down: bool, keysym: u32) -> Result<(), DesktopSessionError> {
        self.send_event(RfbServerEvent::Key {
            client_id: RfbClientId::local_desktop(),
            down,
            keysym,
        })
    }

    /// 发送指针事件（framebuffer 坐标 + 当前帧尺寸）。
    pub fn send_pointer(
        &self,
        button_mask: u8,
        x: u16,
        y: u16,
        size: FrameSize,
    ) -> Result<(), DesktopSessionError> {
        let framebuffer_size = ipkvm_rfb::RfbSize::new(size.width as u16, size.height as u16)
            .map_err(|error| DesktopSessionError::Input(error.to_string()))?;
        self.send_event(RfbServerEvent::Pointer {
            client_id: RfbClientId::local_desktop(),
            button_mask,
            x,
            y,
            framebuffer_size,
        })
    }

    /// 发送相对指针事件（桌面相对鼠标模式；dx/dy 为帧像素增量）。
    pub fn send_pointer_relative(
        &self,
        button_mask: u8,
        dx: i16,
        dy: i16,
        wheel: i8,
    ) -> Result<(), DesktopSessionError> {
        self.send_event(RfbServerEvent::PointerRelative {
            client_id: RfbClientId::local_desktop(),
            button_mask,
            dx,
            dy,
            wheel,
        })
    }

    /// 在线切换鼠标模式（不经重连，等待输入泵确认后才返回成功）。
    pub fn set_mouse_mode(&mut self, mode: MouseMode) -> Result<(), DesktopSessionError> {
        let mut mode_rx = self
            .supervisor
            .manager()
            .session()
            .ok_or(DesktopSessionError::NoEventSender)?
            .mouse_mode();
        self.send_event(RfbServerEvent::SetMouseMode {
            client_id: RfbClientId::local_desktop(),
            mode,
        })?;

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            self.flush_pending()?;
            if *mode_rx.borrow() == Some(mode) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(DesktopSessionError::Input(
                    "input sink did not confirm the requested mouse mode".into(),
                ));
            }
            let changed = self.runtime.block_on(async {
                tokio::time::timeout(Duration::from_millis(20), mode_rx.changed()).await
            });
            if matches!(changed, Ok(Err(_))) {
                return Err(DesktopSessionError::Input(
                    "mouse mode confirmation channel closed".into(),
                ));
            }
        }
    }

    /// 粘贴文本：以 CutText 进入文本键入服务。
    pub fn paste_text(&self, text: String) -> Result<(), DesktopSessionError> {
        self.send_event(RfbServerEvent::CutText {
            client_id: RfbClientId::local_desktop(),
            bytes: text.into_bytes(),
        })
    }

    /// 释放所有按键/按钮：断开本地控制器（泵执行 release_all），再重新连接，
    /// 保证后续键鼠仍有活动控制器。
    pub fn release_all(&self) -> Result<(), DesktopSessionError> {
        self.send_event(RfbServerEvent::Disconnected {
            client_id: RfbClientId::local_desktop(),
            peer_addr: LOCAL_PEER,
            reason: RfbDisconnectReason::ClientClosed,
        })?;
        self.send_event(RfbServerEvent::Connected {
            client_id: RfbClientId::local_desktop(),
            peer_addr: LOCAL_PEER,
            shared: true,
        })
    }

    /// 取走已排队 notice（app 每帧调用并更新状态栏）。
    pub fn drain_notices(&mut self) -> Vec<RfbInputNotice> {
        let mut notices = Vec::new();
        while let Ok(notice) = self.notice_rx.try_recv() {
            notices.push(notice);
        }
        notices
    }

    /// 补送暂存事件（事件通道满时残留的输入）。
    ///
    /// 根因：`send_event` 只在「下一次发送」时补送 pending，突发填满通道后，
    /// 若无后续输入，残余事件（可能包含最后一次 key-up）会无限期滞留。
    /// 前端应在每帧或固定间隔调用本方法，保证补送不依赖下一次输入。
    pub fn flush_pending(&mut self) -> Result<(), DesktopSessionError> {
        let mut pending = self
            .pending_events
            .lock()
            .map_err(|_| DesktopSessionError::Input("pending event queue poisoned".into()))?;
        if pending.is_empty() {
            return Ok(());
        }
        let Some(tx) = &self.event_tx else {
            pending.clear();
            return Err(DesktopSessionError::NoEventSender);
        };
        flush_pending_events(&mut pending, |next| tx.try_send(next))
    }

    fn send_event(&self, event: RfbServerEvent) -> Result<(), DesktopSessionError> {
        let mut pending = self
            .pending_events
            .lock()
            .map_err(|_| DesktopSessionError::Input("pending event queue poisoned".into()))?;
        pending.push_back(event);
        let Some(tx) = &self.event_tx else {
            pending.clear();
            return Err(DesktopSessionError::NoEventSender);
        };
        flush_pending_events(&mut pending, |next| tx.try_send(next))
    }

    fn refresh_event_sender(&mut self) -> Result<(), DesktopSessionError> {
        let sender = self.supervisor.event_publisher().borrow().clone();
        let Some(sender) = sender else {
            self.event_tx = None;
            self.pending_events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clear();
            return Ok(());
        };
        if self.event_tx.as_ref().is_some_and(|tx| !tx.is_closed()) {
            return Ok(());
        }
        sender
            .try_send(RfbServerEvent::Connected {
                client_id: RfbClientId::local_desktop(),
                peer_addr: LOCAL_PEER,
                shared: true,
            })
            .map_err(|error| DesktopSessionError::Input(error.to_string()))?;
        self.event_tx = Some(sender);
        Ok(())
    }
}

/// 尽力把暂存队列按 FIFO 顺序送入事件通道；通道满时保留剩余事件等待下次
/// 提交，通道关闭时清空并返回错误。返回 Ok 不保证队列已清空（Full 是
/// 正常暂存而非失败）。
fn flush_pending_events(
    pending: &mut VecDeque<RfbServerEvent>,
    mut try_send: impl FnMut(
        RfbServerEvent,
    ) -> Result<(), tokio::sync::mpsc::error::TrySendError<RfbServerEvent>>,
) -> Result<(), DesktopSessionError> {
    while let Some(next) = pending.front().cloned() {
        match try_send(next) {
            Ok(()) => {
                pending.pop_front();
            }
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => break,
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                pending.clear();
                return Err(DesktopSessionError::Input("event channel closed".into()));
            }
        }
    }
    Ok(())
}

const LOCAL_PEER: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0);

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    use ipkvm_core::{InputResult, KeyEvent, MouseMode, PointerEvent};
    use ipkvm_video::mock::MockFrameSource;

    use super::*;

    #[derive(Clone)]
    struct TestSessionFactory {
        source: Arc<MockFrameSource>,
        sink: RecordingSink,
    }

    impl TestSessionFactory {
        fn new(sink: RecordingSink) -> Self {
            Self {
                source: Arc::new(MockFrameSource::new()),
                sink,
            }
        }
    }

    impl DesktopSessionFactory<RecordingSink> for TestSessionFactory {
        fn build_video(
            &mut self,
            _request: &ConnectRequest,
        ) -> Result<Arc<dyn FrameSource>, DesktopSessionError> {
            Ok(self.source.clone())
        }

        fn build_control(
            &mut self,
            _request: &ConnectRequest,
        ) -> Result<RecordingSink, DesktopSessionError> {
            Ok(self.sink.clone())
        }
    }

    type TestController = DesktopSessionController<RecordingSink, TestSessionFactory>;

    /// 记录型输入 sink：观察泵写入的键/指针批次与 release_all 次数。
    #[derive(Clone, Debug, Default)]
    struct RecordingSink {
        recorded: Arc<Mutex<Recorded>>,
    }

    #[derive(Clone, Debug, Default)]
    struct Recorded {
        key_batches: usize,
        pointer_batches: usize,
        mouse_mode_calls: usize,
        fail_next_mouse_mode: bool,
        release_count: usize,
    }

    impl InputSink for RecordingSink {
        fn set_mouse_mode(&mut self, _mode: MouseMode) -> InputResult<()> {
            let mut recorded = self.recorded.lock().unwrap();
            if std::mem::take(&mut recorded.fail_next_mouse_mode) {
                return Err(ipkvm_core::InputError::PointerPositionUnknown);
            }
            recorded.mouse_mode_calls += 1;
            Ok(())
        }

        fn handle_key_batch(&mut self, _events: &[KeyEvent]) -> InputResult<()> {
            self.recorded.lock().unwrap().key_batches += 1;
            Ok(())
        }

        fn handle_pointer_batch(&mut self, _events: &[PointerEvent]) -> InputResult<()> {
            self.recorded.lock().unwrap().pointer_batches += 1;
            Ok(())
        }

        fn release_all(&mut self) -> InputResult<()> {
            self.recorded.lock().unwrap().release_count += 1;
            Ok(())
        }
    }

    fn request() -> ConnectRequest {
        ConnectRequest {
            video_device_id: "cam0".into(),
            control_device_id: "COM9".into(),
            baud_rate: 9_600,
            mouse_profile: MouseProfile::RawAbsolute,
            mouse_mode: MouseMode::Absolute,
            preview_fps: 30,
        }
    }

    fn controller_with_sink() -> (TestController, RecordingSink) {
        let sink = RecordingSink::default();
        let factory = TestSessionFactory::new(sink.clone());
        let controller = DesktopSessionController::with_factory(factory);
        (controller, sink)
    }

    struct SplitRecoverFactory {
        source: Arc<MockFrameSource>,
        sink: RecordingSink,
        video_failures: Arc<AtomicUsize>,
        control_failures: Arc<AtomicUsize>,
    }

    impl DesktopSessionFactory<RecordingSink> for SplitRecoverFactory {
        fn build_video(
            &mut self,
            _request: &ConnectRequest,
        ) -> Result<Arc<dyn FrameSource>, DesktopSessionError> {
            if self.video_failures.load(Ordering::SeqCst) > 0 {
                self.video_failures.fetch_sub(1, Ordering::SeqCst);
                return Err(DesktopSessionError::Build("camera reset".into()));
            }
            Ok(self.source.clone())
        }

        fn build_control(
            &mut self,
            _request: &ConnectRequest,
        ) -> Result<RecordingSink, DesktopSessionError> {
            if self.control_failures.load(Ordering::SeqCst) > 0 {
                self.control_failures.fetch_sub(1, Ordering::SeqCst);
                return Err(DesktopSessionError::Build("serial reset".into()));
            }
            Ok(self.sink.clone())
        }
    }

    fn wait_until(mut condition: impl FnMut() -> bool, what: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !condition() {
            assert!(std::time::Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// 构造一个 2×2 BGRA 帧（每像素 4 字节，stride=8）用于帧通知测试。
    fn make_frame(seq: u64) -> ipkvm_video::VideoFrame {
        ipkvm_video::VideoFrame::new(
            seq,
            ipkvm_video::MonotonicTimestamp::from_nanos(seq),
            2,
            2,
            8,
            ipkvm_video::PixelFormat::Bgra8888,
            Arc::from(vec![0u8; 16].into_boxed_slice()),
        )
    }

    #[test]
    fn connect_then_keyboard_and_pointer_reach_sink() {
        let (mut controller, sink) = controller_with_sink();

        controller.connect(request()).unwrap();
        controller.send_key(true, 0x61).unwrap();
        controller
            .send_pointer(
                1,
                100,
                50,
                FrameSize {
                    width: 1920,
                    height: 1080,
                },
            )
            .unwrap();

        wait_until(
            || {
                let recorded = sink.recorded.lock().unwrap();
                recorded.key_batches == 1 && recorded.pointer_batches == 1
            },
            "键鼠事件未到达记录型 sink",
        );

        controller.stop().unwrap();
    }

    #[test]
    fn release_all_resets_controller_and_input_continues() {
        let (mut controller, sink) = controller_with_sink();

        controller.connect(request()).unwrap();
        controller.send_key(true, 0x61).unwrap();
        wait_until(
            || sink.recorded.lock().unwrap().key_batches == 1,
            "首个按键未到达 sink",
        );

        controller.release_all().unwrap();
        wait_until(
            || sink.recorded.lock().unwrap().release_count >= 1,
            "release_all 未触发 sink 释放",
        );

        controller.send_key(true, 0x62).unwrap();
        wait_until(
            || sink.recorded.lock().unwrap().key_batches == 2,
            "release_all 后再次输入未到达 sink",
        );

        controller.stop().unwrap();
    }

    #[test]
    fn stop_marks_controller_offline() {
        let (mut controller, _sink) = controller_with_sink();

        controller.connect(request()).unwrap();
        assert!(controller.is_control_online());

        controller.stop().unwrap();
        assert!(!controller.is_control_online());
    }

    #[test]
    fn rollback_clears_state_and_releases_session() {
        let (mut controller, _sink) = controller_with_sink();
        controller.connect(request()).unwrap();
        assert!(controller.is_control_online());

        controller.rollback();

        assert!(!controller.is_control_online());
        assert!(controller.latest_frame().is_none());
        controller.connect(request()).unwrap();
        assert!(controller.is_control_online());
        controller.stop().unwrap();
    }

    #[test]
    fn control_open_failure_keeps_video_active_and_recovers_on_tick() {
        let source = Arc::new(MockFrameSource::new());
        source.publish_frame(Arc::new(make_frame(1)));
        let sink = RecordingSink::default();
        let control_failures = Arc::new(AtomicUsize::new(1));
        let factory = SplitRecoverFactory {
            source,
            sink: sink.clone(),
            video_failures: Arc::new(AtomicUsize::new(0)),
            control_failures,
        };
        let mut controller = DesktopSessionController::with_factory(factory);

        controller.connect(request()).unwrap();

        assert!(controller.should_show_work_view());
        assert!(!controller.is_control_online());
        assert_eq!(
            controller.latest_frame().as_ref().map(|frame| frame.seq),
            Some(1)
        );
        assert_eq!(
            controller.input_offline_reason(),
            Some("session component build failed: serial reset".into())
        );

        std::thread::sleep(Duration::from_millis(300));
        controller.tick_recovery().unwrap();
        assert!(controller.is_control_online());
        controller.send_key(true, 0x61).unwrap();
        wait_until(
            || sink.recorded.lock().unwrap().key_batches == 1,
            "控制链路恢复后键盘事件必须到达 sink",
        );
        controller.stop().unwrap();
    }

    #[test]
    fn video_open_failure_keeps_work_view_and_recovers_on_tick() {
        let source = Arc::new(MockFrameSource::new());
        source.publish_frame(Arc::new(make_frame(1)));
        let sink = RecordingSink::default();
        let video_failures = Arc::new(AtomicUsize::new(1));
        let factory = SplitRecoverFactory {
            source,
            sink,
            video_failures,
            control_failures: Arc::new(AtomicUsize::new(0)),
        };
        let mut controller = DesktopSessionController::with_factory(factory);

        controller.connect(request()).unwrap();
        assert!(controller.should_show_work_view());
        assert!(controller.is_control_online());
        assert!(controller.latest_frame().is_none());

        std::thread::sleep(Duration::from_millis(300));
        controller.tick_recovery().unwrap();
        wait_until(
            || controller.latest_frame().is_some(),
            "视频链路恢复后必须重新发布帧",
        );
        controller.stop().unwrap();
    }

    fn key_event(tag: u8) -> RfbServerEvent {
        RfbServerEvent::Key {
            client_id: RfbClientId::local_desktop(),
            down: true,
            keysym: u32::from(tag),
        }
    }

    fn keysym_of(event: &RfbServerEvent) -> u32 {
        match event {
            RfbServerEvent::Key { keysym, .. } => *keysym,
            _ => 0,
        }
    }

    #[test]
    fn flush_pending_events_drains_in_fifo_order() {
        let mut pending = std::collections::VecDeque::new();
        let mut delivered = Vec::new();
        pending.push_back(key_event(1));
        pending.push_back(key_event(2));

        let result = flush_pending_events(&mut pending, |next| {
            delivered.push(next);
            Ok(())
        });

        assert!(result.is_ok());
        assert!(pending.is_empty());
        assert_eq!(
            delivered.iter().map(keysym_of).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn flush_pending_events_keeps_remainder_when_full_then_resumes_in_order() {
        use tokio::sync::mpsc::error::TrySendError;

        let mut pending = std::collections::VecDeque::new();
        let mut delivered = Vec::new();
        pending.push_back(key_event(1));
        pending.push_back(key_event(2));
        pending.push_back(key_event(3));

        let mut accepts = 0;
        let result = flush_pending_events(&mut pending, |next| {
            if accepts < 2 {
                accepts += 1;
                delivered.push(next);
                Ok(())
            } else {
                Err(TrySendError::Full(next))
            }
        });

        assert!(result.is_ok());
        assert_eq!(pending.len(), 1);
        assert_eq!(
            delivered.iter().map(keysym_of).collect::<Vec<_>>(),
            vec![1, 2]
        );

        let result = flush_pending_events(&mut pending, |next| {
            delivered.push(next);
            Ok(())
        });

        assert!(result.is_ok());
        assert!(pending.is_empty());
        assert_eq!(
            delivered.iter().map(keysym_of).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn flush_pending_events_clears_on_closed_channel() {
        use tokio::sync::mpsc::error::TrySendError;

        let mut pending = std::collections::VecDeque::new();
        pending.push_back(key_event(1));

        let result = flush_pending_events(&mut pending, |next| Err(TrySendError::Closed(next)));

        assert!(result.is_err());
        assert!(pending.is_empty());
    }

    #[test]
    fn connect_then_relative_pointer_reaches_sink() {
        let (mut controller, sink) = controller_with_sink();
        controller.connect(request()).unwrap();

        controller.send_pointer_relative(1, 10, -3, 2).unwrap();

        wait_until(
            || sink.recorded.lock().unwrap().pointer_batches == 1,
            "相对指针事件未到达记录型 sink",
        );
        controller.stop().unwrap();
    }

    #[test]
    fn subscribe_frames_is_none_before_connect() {
        let (controller, _sink) = controller_with_sink();
        assert!(
            controller.subscribe_frames().is_none(),
            "连接前帧源为空，subscribe_frames 必须返回 None"
        );
    }

    #[test]
    fn subscribe_frames_notifies_on_new_frame_after_connect() {
        // 用一个可从外部推送帧的 mock 帧源，factory 把它包成 trait 对象，
        // 外层保留句柄以便 publish_frame 触发 watch 通知。
        let mock = Arc::new(MockFrameSource::new());
        let sink = RecordingSink::default();
        let factory = TestSessionFactory {
            source: Arc::clone(&mock),
            sink,
        };
        let mut controller = DesktopSessionController::with_factory(factory);

        controller.connect(request()).unwrap();

        let receiver = controller
            .subscribe_frames()
            .expect("连接后 subscribe_frames 必须返回 Some");

        // 初始：watch 当前值为 None（MockFrameSource::new 的初始），尚未变化。
        assert!(receiver.borrow().is_none());

        // 推一帧：watch 标记已变化，可读到刚发布的帧 seq。
        mock.publish_frame(Arc::new(make_frame(1)));
        wait_until(
            || {
                receiver
                    .borrow()
                    .as_ref()
                    .is_some_and(|frame| frame.seq == 1)
            },
            "新帧后 shared frame hub 必须转发到 receiver",
        );
        let frame = receiver.borrow().as_ref().unwrap().clone();
        assert_eq!(frame.seq, 1, "watch 必须读到刚发布的帧");

        controller.stop().unwrap();
    }

    #[test]
    fn set_mouse_mode_reaches_sink() {
        let (mut controller, sink) = controller_with_sink();
        controller.connect(request()).unwrap();

        controller.set_mouse_mode(MouseMode::Absolute).unwrap();

        wait_until(
            || sink.recorded.lock().unwrap().mouse_mode_calls == 1,
            "set_mouse_mode 未到达记录型 sink",
        );
        controller.stop().unwrap();
    }

    #[test]
    fn set_mouse_mode_reports_sink_rejection_instead_of_acknowledging_enqueue() {
        let (mut controller, sink) = controller_with_sink();
        controller.connect(request()).unwrap();
        sink.recorded.lock().unwrap().fail_next_mouse_mode = true;

        let error = controller.set_mouse_mode(MouseMode::Relative).unwrap_err();

        assert!(
            error.to_string().contains("mouse mode"),
            "sink 拒绝模式切换时，控制器必须报告确认失败，而不是只报告入队成功"
        );
        assert_eq!(sink.recorded.lock().unwrap().mouse_mode_calls, 0);
        controller.stop().unwrap();
    }

    #[test]
    fn reconnect_stops_old_session_before_building_new() {
        let (mut controller, sink) = controller_with_sink();
        controller.connect(request()).unwrap();
        controller.send_key(true, 0x61).unwrap();
        wait_until(
            || sink.recorded.lock().unwrap().key_batches == 1,
            "首个会话键盘事件未到达",
        );

        // 再次连接：旧会话必须先停止（旧 sink 收到 release），新会话才能起。
        controller.connect(request()).unwrap();
        assert!(
            sink.recorded.lock().unwrap().release_count >= 1,
            "重连前旧会话必须先释放（串口独占语义）"
        );

        controller.send_key(true, 0x62).unwrap();
        wait_until(
            || sink.recorded.lock().unwrap().key_batches == 2,
            "重连后新会话输入未到达",
        );
        controller.stop().unwrap();
    }
}
