use std::net::SocketAddr;
use std::sync::Arc;

use ipkvm_core::{Ch9329InputSink, InputSink, MouseMode, SerialCommandQueue};
use ipkvm_session::rfb_connection::{
    RfbClientId, RfbConnectionGate, RfbDisconnectReason, RfbServerEvent,
};
use ipkvm_session::rfb_input::RfbInputNotice;
use ipkvm_session::session_manager::SessionManager;
use ipkvm_video::FrameSource;
use thiserror::Error;
use tokio::sync::mpsc;

use crate::render::FrameSize;

/// 桌面连接请求：把设备选择与高级设置固化为一次会话启动的组件参数。
pub struct ConnectRequest {
    pub video_device_id: String,
    pub control_device_id: String,
    pub baud_rate: u32,
    pub mouse_mode: MouseMode,
    pub preview_fps: u64,
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

/// 桌面本地会话控制器：内存事件 → 共享 SessionManager → 输入泵 → 真实 sink。
///
/// 自身持有 tokio runtime，GUI 线程直接调用；`connect`/`stop` 在 runtime 内
/// block_on 会话生命周期操作，避免 `tokio::spawn` 找不到运行时上下文。
pub struct DesktopSessionController<S, F>
where
    S: InputSink + Clone + Send + 'static,
    F: FnMut(&ConnectRequest) -> Result<SessionParts<S>, DesktopSessionError>,
{
    runtime: tokio::runtime::Runtime,
    manager: SessionManager<S>,
    factory: F,
    notice_rx: mpsc::UnboundedReceiver<RfbInputNotice>,
    event_tx: Option<mpsc::Sender<RfbServerEvent>>,
    frame_source: Option<Arc<dyn FrameSource>>,
}

impl<S, F> DesktopSessionController<S, F>
where
    S: InputSink + Clone + Send + 'static,
    F: FnMut(&ConnectRequest) -> Result<SessionParts<S>, DesktopSessionError>,
{
    /// 用组件工厂构造控制器（测试注入 fake sink/帧源，生产用 [`production_parts`]）。
    pub fn with_factory(factory: F) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("desktop tokio runtime");
        let manager = SessionManager::empty();
        let (_notice_tx, notice_rx) = mpsc::unbounded_channel();
        Self {
            runtime,
            manager,
            factory,
            notice_rx,
            event_tx: None,
            frame_source: None,
        }
    }

    /// 连接：会话级停旧启新（`replace_and_start`），然后以本地桌面控制器身份
    /// 发送 `Connected` 成为当前唯一活动控制器。
    pub fn connect(&mut self, request: ConnectRequest) -> Result<(), DesktopSessionError> {
        let (frame_source, sink, gate) = (self.factory)(&request)?;
        self.frame_source = Some(Arc::clone(&frame_source));
        // 先建 notice 镜像再启动，避免错过泵的早期 notice；unbounded 通道缓冲
        // 到本控制器持有 receiver 为止。
        let (notice_tx, notice_rx) = mpsc::unbounded_channel();
        self.manager.set_notice_mirror(Some(notice_tx));
        self.runtime
            .block_on(self.manager.replace_and_start(frame_source, sink, gate))
            .map_err(|error| DesktopSessionError::Session(error.to_string()))?;
        self.notice_rx = notice_rx;

        let sender = self
            .manager
            .event_publisher()
            .borrow()
            .clone()
            .ok_or(DesktopSessionError::NoEventSender)?;
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

    /// 停止连接并销毁会话组件（释放相机/串口），之后可重新连接。
    pub fn stop(&mut self) -> Result<(), DesktopSessionError> {
        self.event_tx = None;
        self.frame_source = None;
        self.runtime
            .block_on(self.manager.stop_and_destroy())
            .map_err(|error| DesktopSessionError::Session(error.to_string()))
    }

    /// 当前会话帧源的最新帧（渲染与控制台无信号检测用）。
    pub fn latest_frame(&self) -> Option<ipkvm_video::SharedVideoFrame> {
        self.frame_source
            .as_ref()
            .and_then(|source| source.latest_frame())
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
        let Some(tx) = &self.event_tx else {
            return Err(DesktopSessionError::NoEventSender);
        };
        tx.try_send(RfbServerEvent::Disconnected {
            client_id: RfbClientId::local_desktop(),
            peer_addr: LOCAL_PEER,
            reason: RfbDisconnectReason::ClientClosed,
        })
        .map_err(|error| DesktopSessionError::Input(error.to_string()))?;
        tx.try_send(RfbServerEvent::Connected {
            client_id: RfbClientId::local_desktop(),
            peer_addr: LOCAL_PEER,
            shared: true,
        })
        .map_err(|error| DesktopSessionError::Input(error.to_string()))?;
        Ok(())
    }

    /// 取走已排队 notice（app 每帧调用并更新状态栏）。
    pub fn drain_notices(&mut self) -> Vec<RfbInputNotice> {
        let mut notices = Vec::new();
        while let Ok(notice) = self.notice_rx.try_recv() {
            notices.push(notice);
        }
        notices
    }

    fn send_event(&self, event: RfbServerEvent) -> Result<(), DesktopSessionError> {
        let Some(tx) = &self.event_tx else {
            return Err(DesktopSessionError::NoEventSender);
        };
        tx.try_send(event)
            .map_err(|error| DesktopSessionError::Input(error.to_string()))
    }
}

const LOCAL_PEER: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0);

/// 生产组件工厂：相机帧源 + CH9329 串口 sink + 新连接闸门。
pub fn production_parts(
    request: &ConnectRequest,
) -> Result<SessionParts<Ch9329InputSink<SerialCommandQueue>>, DesktopSessionError> {
    let frame_source: Arc<dyn FrameSource> = Arc::new(
        ipkvm_video::camera::CameraSource::open(&request.video_device_id, request.preview_fps)
            .map_err(|error| DesktopSessionError::Build(error.to_string()))?,
    );
    let queue = SerialCommandQueue::open(&request.control_device_id, request.baud_rate)
        .map_err(|error| DesktopSessionError::Build(error.to_string()))?;
    let sink = Ch9329InputSink::new(queue, 0, request.mouse_mode);
    Ok((frame_source, sink, RfbConnectionGate::new()))
}

pub type ProductionSessionFactory =
    fn(
        &ConnectRequest,
    ) -> Result<SessionParts<Ch9329InputSink<SerialCommandQueue>>, DesktopSessionError>;

pub type ProductionDesktopSessionController =
    DesktopSessionController<Ch9329InputSink<SerialCommandQueue>, ProductionSessionFactory>;

impl ProductionDesktopSessionController {
    /// 生产控制器：相机 + CH9329 串口。
    pub fn production() -> Self {
        Self::with_factory(production_parts)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use ipkvm_core::{InputResult, KeyEvent, MouseMode, PointerEvent};
    use ipkvm_session::rfb_connection::RfbConnectionGate;
    use ipkvm_video::mock::MockFrameSource;

    use super::*;

    /// 记录型输入 sink：观察泵写入的键/指针批次与 release_all 次数。
    #[derive(Clone, Debug, Default)]
    struct RecordingSink {
        recorded: Arc<Mutex<Recorded>>,
    }

    #[derive(Clone, Debug, Default)]
    struct Recorded {
        key_batches: usize,
        pointer_batches: usize,
        release_count: usize,
    }

    impl InputSink for RecordingSink {
        fn set_mouse_mode(&mut self, _mode: MouseMode) -> InputResult<()> {
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
            mouse_mode: MouseMode::Absolute,
            preview_fps: 30,
        }
    }

    fn controller_with_sink() -> (
        DesktopSessionController<
            RecordingSink,
            impl FnMut(&ConnectRequest) -> Result<SessionParts<RecordingSink>, DesktopSessionError>,
        >,
        RecordingSink,
    ) {
        let sink = RecordingSink::default();
        let sink_for_factory = sink.clone();
        let controller = DesktopSessionController::with_factory(move |_request| {
            let frame_source: Arc<dyn FrameSource> = Arc::new(MockFrameSource::new());
            Ok((
                frame_source,
                sink_for_factory.clone(),
                RfbConnectionGate::new(),
            ))
        });
        (controller, sink)
    }

    fn wait_until(mut condition: impl FnMut() -> bool, what: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !condition() {
            assert!(std::time::Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
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
}
