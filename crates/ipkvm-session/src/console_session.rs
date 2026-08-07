//! 控制台会话组装：把帧源、输入 sink、连接闸门和输入泵组装成可运行的会话。

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use ipkvm_core::{InputSink, QueueStats};
use ipkvm_video::FrameSource;
use thiserror::Error;
use tokio::sync::{mpsc, watch};

use crate::rfb_connection::{RfbConnectionGate, RfbServerEvent};
use crate::rfb_input::{RfbInputNotice, RfbInputPump, RfbInputRunError};

/// 输入离线信息：泵因错误退出后的原因与时间。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputOfflineInfo {
    pub reason: String,
    pub since_ns: u64,
}

/// 会话统计：输入事件计数、最后输入时间、丢帧计数、串口统计。
///
/// 输入事件计数与最后输入时间由输入泵 notice 回调（`start()` 的 observe
/// 闭包）写入，`ConsoleSession` 通过 `Arc<Mutex<SessionStats>>` 与泵任务
/// 共享（会话侧 `stats()` 持锁读取）；丢帧与串口统计由会话侧显式调用
/// `observe_frame` / `record_serial_stats` 更新。
#[derive(Clone, Debug, Default)]
pub struct SessionStats {
    /// 累计输入事件数（键盘 + 指针）。
    pub input_events: u64,
    /// 最后输入事件的单调时间（纳秒，`crate::now_ns()`）。
    pub last_input_ns: Option<u64>,
    /// 累计丢帧数（帧 seq 不连续，来自 `observe_frame`）。
    pub dropped_frames: u64,
    /// 串口批次/帧统计（`record_serial_stats` 从 `CommandQueue::stats` 映射）。
    pub serial: Option<crate::serial_stats::SerialStats>,
    /// 最后观察到帧的时间（`observe_frame` 写入；None 表示从未出帧）。
    /// 语义为 observe（观察）时间，与 `capture_ns` 区分。
    pub last_frame_ns: Option<u64>,
    /// 最后观察到帧的采集时间（来自 `frame.timestamp`，统一时钟）。
    /// 与 `last_frame_ns`（observe 时间）同源可比，差值即端到端延迟。
    pub capture_ns: Option<u64>,
    /// RFB 编码统计快照（encode 耗时与字节累计，由装配层从连接 core 快照填入）。
    pub encode: Option<ipkvm_rfb::RfbEncodeStatsSnapshot>,
    /// RFB 成功发送的 FramebufferUpdate 累计次数（updates/sec 的分子）。
    pub updates_sent: u64,
    /// 输入泵失败离线信息（串口写失败等）；恢复/重启后清空。
    pub input_offline: Option<InputOfflineInfo>,
    /// 上次观察的帧 seq（丢帧检测内部状态）。
    last_seq: Option<u64>,
}

impl SessionStats {
    /// 记录一次输入事件（键盘/指针）。
    pub fn observe_input(&mut self) {
        self.input_events = self.input_events.saturating_add(1);
        self.last_input_ns = Some(crate::now_ns());
    }

    /// 观察一帧：记录 seq 用于序列跟踪，但不计算 dropped_frames。
    /// dropped_frames 仅在真正丢帧时计数（RFB 编码/发送失败），
    /// 因为 latest_frame() 轮询机制会导致 seq 跳跃（中间帧被覆盖），
    /// 这不是真正的丢帧。
    pub fn observe_frame_seq(&mut self, seq: u64) {
        self.last_seq = Some(seq);
    }

    /// 记录 RFB 编码统计快照（由装配层从 `RfbConnectionCore::encode_stats_snapshot` 填入）。
    pub fn record_encode_stats(&mut self, snapshot: ipkvm_rfb::RfbEncodeStatsSnapshot) {
        self.encode = Some(snapshot);
    }

    /// 记录一次成功的 FramebufferUpdate 发送（updates/sec 的分子）。
    pub fn record_update_sent(&mut self) {
        self.updates_sent = self.updates_sent.saturating_add(1);
    }
}

/// 会话级错误。
#[derive(Debug, Error)]
pub enum SessionError {
    #[error("session is already running")]
    AlreadyRunning,
    #[error("session is already created")]
    AlreadyCreated,
    #[error("session is not running")]
    NotRunning,
    #[error("input pump failed: {0}")]
    Input(#[from] RfbInputRunError),
}

/// 预留句柄标记，供未来句柄式控制（#31/#32）。
#[derive(Clone, Debug)]
pub struct SessionHandle;

/// 控制台会话：帧源 + 输入 sink + 连接闸门 + 输入泵的组装。
///
/// `S: Clone` 是 `RfbInputPump::new` 的要求（内部以 sink 克隆启动独立文本
/// 键入服务）；会话保留一份 sink，调用方（如 SessionManager）保留另一份供
/// 统计与后续复用。
pub struct ConsoleSession<S: InputSink + Clone + Send + 'static> {
    /// 帧源。帧 seq 检测（`observe_frame`）消费。
    frame_source: Arc<dyn FrameSource>,
    sink: S,
    gate: RfbConnectionGate,
    event_tx: mpsc::Sender<RfbServerEvent>,
    pump_task: Option<tokio::task::JoinHandle<Result<(), RfbInputRunError>>>,
    /// 停止信号：`stop()` 置 true，泵任务 `run_until_stopped` 收到后自然退出。
    stop_tx: watch::Sender<bool>,
    /// 泵任务真实存活标志：任务自行退出（错误/异常）时由任务包装置 false，
    /// 避免 `pump_task` 残留导致 `is_running()` 与实际情况漂移。
    running: Arc<AtomicBool>,
    /// 会话统计：泵任务（输入计数/最后输入时间）与会话侧共享，Mutex 内部可变。
    stats: Arc<Mutex<SessionStats>>,
    /// 当前事件出口发布端：`start()` 写入新 channel 的发送端，`stop()` 写入
    /// `None`。由 `SessionManager` 持有原始 sender，会话持有 clone——两者共享
    /// 同一 watch channel，使传输层订阅端在会话重建后仍能拿到新发送端。
    event_publisher: watch::Sender<Option<mpsc::Sender<RfbServerEvent>>>,
    /// 可选 notice 镜像：把输入泵每条 notice 转发给桌面本地控制器等观察者。
    notice_mirror: Option<mpsc::UnboundedSender<RfbInputNotice>>,
    /// 输入泵最近一次成功应用的鼠标模式；服务端控制面用它确认异步切换。
    mouse_mode: watch::Sender<Option<ipkvm_core::MouseMode>>,
}

impl<S: InputSink + Clone + Send + 'static> ConsoleSession<S> {
    /// 组装会话。事件通道由 `start()` 在启动时重建，因此这里不接收发送端。
    ///
    /// `event_publisher` 由 `SessionManager` 持有原始 sender 后传入 clone：会话
    /// 在 `start()`/`stop()` 时把当前事件出口写入，供传输层订阅端读取最新
    /// 发送端（会话重建后订阅端自动看到新 channel）。
    pub fn new(
        frame_source: Arc<dyn FrameSource>,
        sink: S,
        gate: RfbConnectionGate,
        event_publisher: watch::Sender<Option<mpsc::Sender<RfbServerEvent>>>,
    ) -> Self {
        let (stop_tx, _) = watch::channel(false);
        let (mouse_mode, _) = watch::channel(None);
        Self {
            frame_source,
            sink,
            gate,
            // 占位发送端（无接收端）：与 stop() 停泵后的事件出口语义一致。
            event_tx: mpsc::channel(1).0,
            pump_task: None,
            stop_tx,
            running: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(Mutex::new(SessionStats::default())),
            event_publisher,
            notice_mirror: None,
            mouse_mode,
        }
    }

    /// 连接闸门引用：传输层与会话共享同一仲裁。
    pub fn gate(&self) -> &RfbConnectionGate {
        &self.gate
    }

    /// 事件发送端引用；`start()` 之后才有效（start() 重建 channel 使事件流向输入泵）。
    pub fn event_tx(&self) -> &mpsc::Sender<RfbServerEvent> {
        &self.event_tx
    }

    /// 事件出口订阅端：返回的 `watch::Receiver` 反映当前活动事件发送端。
    ///
    /// `start()` 后为 `Some(sender)`，`stop()` 或会话未启动时为 `None`。传输层
    /// 应在每次建立连接前读取最新值，而非缓存启动时的发送端——这样会话重启
    /// 后能拿到新 channel。
    pub fn event_publisher(&self) -> watch::Receiver<Option<mpsc::Sender<RfbServerEvent>>> {
        self.event_publisher.subscribe()
    }

    /// 订阅输入泵已确认的鼠标模式；`None` 表示当前没有活动控制器。
    pub fn mouse_mode(&self) -> watch::Receiver<Option<ipkvm_core::MouseMode>> {
        self.mouse_mode.subscribe()
    }

    /// 设置 notice 镜像发送端；`None` 关闭镜像。
    pub fn set_notice_mirror(
        &mut self,
        notice_mirror: Option<mpsc::UnboundedSender<RfbInputNotice>>,
    ) {
        self.notice_mirror = notice_mirror;
    }

    /// 会话统计访问：返回互斥锁守卫——泵任务并发写入输入计数/最后输入时间，
    /// 读方短暂持锁读取（守卫可解引用为 `&SessionStats`）。
    ///
    /// 警告：`SessionStats` 由泵线程与调用方经内部互斥共享，std Mutex 非
    /// 重入——勿同线程同时持多个守卫，勿跨 await 持有守卫。
    pub fn stats(&self) -> std::sync::MutexGuard<'_, SessionStats> {
        self.stats.lock().unwrap()
    }

    /// 观察帧源最新帧：seq 跳跃计入丢帧（真实丢帧检测，非延时/采样）。
    ///
    /// 由调用方显式调用（桌面渲染循环或 headless 快照前），不在内部自动轮询。
    /// 同时记录采集时间（`capture_ns`，来自 frame.timestamp）与观察时间
    /// （`last_frame_ns`），两者同源可比。
    pub fn observe_frame(&mut self) {
        if let Some(frame) = self.frame_source.latest_frame() {
            let mut stats = self.stats.lock().unwrap();
            stats.observe_frame_seq(frame.seq);
            stats.capture_ns = Some(frame.timestamp.nanos);
            stats.last_frame_ns = Some(crate::now_ns());
        }
    }

    /// 记录串口统计快照：调用方（持有 `CommandQueue` 的组装层）从队列
    /// `stats()` 读取后填入，供 `/api/status` 与桌面状态栏消费。
    pub fn record_serial_stats(&mut self, stats: QueueStats) {
        self.stats.lock().unwrap().serial = Some(stats.into());
    }

    /// 刷新调用方可见的会话状态快照。
    pub fn refresh_stats(&mut self) {
        self.observe_frame();
        if let Some(stats) = self.sink.queue_stats() {
            self.record_serial_stats(stats);
        }
    }

    /// 会话是否已启动（输入泵任务在运行）。
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// 启动输入泵：重建事件 channel，spawn 泵任务消费事件并驱动 sink。
    ///
    /// 调用方必须运行在 tokio runtime 上下文中（`RfbInputPump::new` 内部
    /// `tokio::spawn` 文本键入服务）。
    ///
    /// observe 闭包随泵任务在独立任务中执行，无法借用 `self`（`tokio::spawn`
    /// 要求 `'static`），因此经共享 `Arc<Mutex<SessionStats>>` 更新统计：
    /// 键盘/指针 notice 各计一次输入事件。文本键入（CutText）走独立 notice
    /// （`TextTyped` 等），不计入键盘/指针统计。
    pub fn start(&mut self) -> Result<SessionHandle, SessionError> {
        if self.is_running() {
            return Err(SessionError::AlreadyRunning);
        }
        // 复位上一次 stop 留下的停止信号，再订阅新接收端。
        let _ = self.stop_tx.send(false);
        let stop_rx = self.stop_tx.subscribe();
        let (event_tx, mut event_rx) = mpsc::channel(64);
        self.event_tx = event_tx;
        // 发布新事件出口：传输层订阅端据此拿到当前 channel 的发送端。
        self.event_publisher
            .send_replace(Some(self.event_tx.clone()));
        let mut pump = RfbInputPump::with_mouse_mode_observer(
            self.sink.clone(),
            Some(self.mouse_mode.clone()),
        );
        let stats = Arc::clone(&self.stats);
        let running = Arc::clone(&self.running);
        let notice_mirror = self.notice_mirror.clone();
        self.running.store(true, Ordering::SeqCst);
        stats.lock().unwrap().input_offline = None;
        let task = tokio::spawn(async move {
            let result = pump
                .run_until_stopped(&mut event_rx, stop_rx, |notice: &RfbInputNotice| {
                    match notice {
                        RfbInputNotice::Keyboard { .. } | RfbInputNotice::Pointer { .. } => {
                            stats.lock().unwrap().observe_input();
                        }
                        RfbInputNotice::FrameUpdateStatsObserved { encode, .. } => {
                            let mut s = stats.lock().unwrap();
                            s.record_update_sent();
                            s.record_encode_stats(*encode);
                        }
                        _ => {}
                    }
                    if let Some(tx) = &notice_mirror {
                        let _ = tx.send(notice.clone());
                    }
                })
                .await;
            if let Err(error) = &result {
                stats.lock().unwrap().input_offline = Some(InputOfflineInfo {
                    reason: error.to_string(),
                    since_ns: crate::now_ns(),
                });
            }
            // 无论正常退出还是错误退出，都让会话状态回到未运行。
            running.store(false, Ordering::SeqCst);
            result
        });
        self.pump_task = Some(task);
        Ok(SessionHandle)
    }

    /// 停止会话：发送停止信号，泵任务收到后自然退出并执行 `release_all`。
    ///
    /// 不 abort、不依赖事件发送端全部释放——传输层可能长期持有 `event_tx`
    /// 克隆，channel 关闭无法作为停泵依据。本方法返回旧泵任务的 join handle
    /// （`#[must_use]`），供调用方在 async 上下文中 join，构成「释放完成」
    /// 屏障——方法返回时 pump 可能仍在收尾，join（await handle）之后才保证
    /// `release_all` 已执行。不需要屏障的调用方需显式丢弃（`drop(handle)`）。
    ///
    /// 泵已因错误自行退出时返回 `NotRunning`（旧任务句柄被丢弃，仅用于观测）。
    pub fn stop(
        &mut self,
    ) -> Result<tokio::task::JoinHandle<Result<(), RfbInputRunError>>, SessionError> {
        let task = self.pump_task.take().ok_or(SessionError::NotRunning)?;
        if !self.is_running() {
            return Err(SessionError::NotRunning);
        }
        self.running.store(false, Ordering::SeqCst);
        let _ = self.stop_tx.send(true);
        // 覆盖旧发送端，使 start() 重建事件出口前的 event_tx() 保持「无接收端」语义。
        self.event_tx = mpsc::channel(1).0;
        // 发布无活动事件出口：传输层据此拒绝新连接，直到下一次 start()。
        self.event_publisher.send_replace(None);
        Ok(task)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use ipkvm_core::{
        Ch9329Frame, Ch9329InputSink, CommandBatch, CommandQueue, InputError, InputResult,
        KeyEvent, MouseMode, PointerEvent, QueueStats, fake_serial::FakeCommandQueue,
    };
    use ipkvm_video::mock::MockFrameSource;
    use ipkvm_video::{MonotonicTimestamp, PixelFormat, VideoFrame};

    use super::*;
    use crate::rfb_connection::{RfbClientId, RfbTransportKind};
    use crate::serial_stats::SerialStats;

    /// 记录型输入 sink：内部共享 `Arc<Mutex<Recorded>>`，供测试观察泵（及
    /// 其文本键入服务克隆）写入的批次与 release 行为。
    #[derive(Clone, Debug, Default)]
    struct RecordingSink {
        recorded: Arc<Mutex<Recorded>>,
    }

    /// 泵行为记录：键/指针批次次数与 release_all 次数。
    #[derive(Clone, Debug, Default)]
    struct Recorded {
        key_batches: usize,
        pointer_batches: usize,
        release_count: usize,
        fail_next_key: bool,
    }

    impl InputSink for RecordingSink {
        fn set_mouse_mode(&mut self, _mode: MouseMode) -> InputResult<()> {
            Ok(())
        }

        fn handle_key_batch(&mut self, _events: &[KeyEvent]) -> InputResult<()> {
            let mut recorded = self.recorded.lock().unwrap();
            if std::mem::take(&mut recorded.fail_next_key) {
                return Err(InputError::RolloverLimitExceeded);
            }
            recorded.key_batches += 1;
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

    /// 会话测试 fixture：MockFrameSource（ipkvm-video mock）+ 记录型 sink +
    /// 新建连接闸门；返回会话与 sink 记录句柄。
    fn console_session_fixture() -> (ConsoleSession<RecordingSink>, RecordingSink) {
        let frame_source: Arc<dyn FrameSource> = Arc::new(MockFrameSource::new());
        let sink = RecordingSink::default();
        let (event_publisher, _) = watch::channel(None);
        let session = ConsoleSession::new(
            frame_source,
            sink.clone(),
            RfbConnectionGate::new(),
            event_publisher,
        );
        (session, sink)
    }

    /// 让出执行权直到条件成立（每次让出给泵/文本键入服务任务运行机会），
    /// 最多 2000 次；超时返回条件最后一次求值结果。
    async fn yield_until(mut condition: impl FnMut() -> bool) -> bool {
        for _ in 0..2_000 {
            if condition() {
                return true;
            }
            tokio::task::yield_now().await;
        }
        condition()
    }

    /// 带真实时间上限的轮询（用于依赖节流/文本服务的异步收尾）。
    async fn wait_for(mut condition: impl FnMut() -> bool, what: &str) {
        assert!(
            tokio::time::timeout(Duration::from_secs(1), async {
                while !condition() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .is_ok(),
            "{what}"
        );
    }

    #[test]
    fn stop_without_start_reports_not_running() {
        let (mut session, _sink) = console_session_fixture();

        assert!(matches!(session.stop(), Err(SessionError::NotRunning)));
    }

    #[tokio::test]
    async fn notice_mirror_receives_input_and_text_notices() {
        let (mut session, _sink) = console_session_fixture();
        let (notice_tx, mut notice_rx) = tokio::sync::mpsc::unbounded_channel();
        session.set_notice_mirror(Some(notice_tx));
        session.start().unwrap();

        let event_tx = session.event_tx().clone();
        let client_id = RfbClientId::for_test(1);
        let peer_addr = "127.0.0.1:5900".parse().unwrap();
        event_tx
            .send(RfbServerEvent::Connected {
                client_id,
                peer_addr,
                shared: true,
            })
            .await
            .unwrap();
        event_tx
            .send(RfbServerEvent::CutText {
                client_id,
                bytes: b"a".to_vec(),
            })
            .await
            .unwrap();

        let mut seen_text_typed = false;
        for _ in 0..8 {
            let notice = tokio::time::timeout(std::time::Duration::from_secs(1), notice_rx.recv())
                .await
                .unwrap()
                .unwrap();
            if matches!(notice, crate::rfb_input::RfbInputNotice::TextTyped { .. }) {
                seen_text_typed = true;
                break;
            }
        }
        assert!(seen_text_typed);

        drop(event_tx);
        drop(session.stop().unwrap());
    }

    #[tokio::test]
    async fn second_start_without_stop_reports_already_running() {
        let (mut session, _sink) = console_session_fixture();

        session.start().unwrap();
        assert!(matches!(session.start(), Err(SessionError::AlreadyRunning)));

        // stop() 自 T7 起返回 `#[must_use]` 的 join handle，此测试不关心屏障。
        drop(session.stop().unwrap());
    }

    #[tokio::test]
    async fn start_runs_the_pump_and_stop_releases_asynchronously() {
        let (mut session, sink) = console_session_fixture();

        let handle = session.start().unwrap();
        assert!(session.is_running());

        // 事件走 start() 重建的 channel 到达输入泵；client_id 需与 Connected 一致。
        let event_tx = session.event_tx().clone();
        let client_id = RfbClientId::for_test(1);
        let peer_addr = "127.0.0.1:5900".parse().unwrap();
        event_tx
            .send(RfbServerEvent::Connected {
                client_id,
                peer_addr,
                shared: true,
            })
            .await
            .unwrap();
        event_tx
            .send(RfbServerEvent::Key {
                client_id,
                down: true,
                keysym: 0x61,
            })
            .await
            .unwrap();
        assert!(
            yield_until(|| sink.recorded.lock().unwrap().key_batches == 1).await,
            "键盘事件未到达输入泵"
        );

        event_tx
            .send(RfbServerEvent::Pointer {
                client_id,
                button_mask: 1,
                x: 100,
                y: 200,
                framebuffer_size: ipkvm_rfb::RfbSize::new(1920, 1080).unwrap(),
            })
            .await
            .unwrap();
        assert!(
            yield_until(|| sink.recorded.lock().unwrap().pointer_batches == 1).await,
            "指针事件未到达输入泵"
        );

        // 关键：测试持有的发送端克隆必须先释放，stop() 覆盖旧 sender 后旧
        // channel 才能关闭（停泵依赖「无其他 clone」），pump 收到 None 后
        // 自然退出并 release_all。
        drop(event_tx);
        // stop() 自 T7 起返回 join handle（#[must_use]）；此处异步释放由
        // 下方 yield_until 观察，句柄显式丢弃。
        drop(session.stop().unwrap());
        assert!(!session.is_running());

        // 释放是异步完成的：首次指针模式收敛释放一次，pump 停止时再释放一次，
        // 文本键入服务收到取消后对 sink 克隆再释放一次，共享计数应为 3。
        assert!(
            yield_until(|| sink.recorded.lock().unwrap().release_count == 3).await,
            "stop 后 release_all 未被执行（异步释放未完成）"
        );
        let _ = handle;
    }

    #[tokio::test]
    async fn stop_finishes_even_while_event_sender_clone_is_held() {
        let (mut session, sink) = console_session_fixture();
        session.start().unwrap();

        let event_tx = session.event_tx().clone();
        let client_id = RfbClientId::for_test(1);
        let peer_addr = "127.0.0.1:5900".parse().unwrap();
        event_tx
            .send(RfbServerEvent::Connected {
                client_id,
                peer_addr,
                shared: true,
            })
            .await
            .unwrap();
        event_tx
            .send(RfbServerEvent::Key {
                client_id,
                down: true,
                keysym: 0x61,
            })
            .await
            .unwrap();
        assert!(
            yield_until(|| sink.recorded.lock().unwrap().key_batches == 1).await,
            "键盘事件未到达输入泵"
        );

        // 关键：模拟传输层仍持有旧 channel 的发送端克隆，stop 不能依赖发送端全部释放。
        let join = session.stop().unwrap();
        let pump_result = tokio::time::timeout(Duration::from_secs(1), join)
            .await
            .expect("stop 后泵任务应在超时内退出")
            .expect("泵任务不应 panic");
        pump_result.expect("泵应正常退出");
        assert!(!session.is_running());
        assert!(
            sink.recorded.lock().unwrap().release_count >= 1,
            "stop 后必须已执行 release_all"
        );

        drop(event_tx);
    }

    #[tokio::test]
    async fn pump_error_marks_session_stopped() {
        let (mut session, sink) = console_session_fixture();
        sink.recorded.lock().unwrap().fail_next_key = true;
        session.start().unwrap();

        let event_tx = session.event_tx().clone();
        let client_id = RfbClientId::for_test(1);
        let peer_addr = "127.0.0.1:5900".parse().unwrap();
        event_tx
            .send(RfbServerEvent::Connected {
                client_id,
                peer_addr,
                shared: true,
            })
            .await
            .unwrap();
        event_tx
            .send(RfbServerEvent::Key {
                client_id,
                down: true,
                keysym: 0x61,
            })
            .await
            .unwrap();
        assert!(
            yield_until(|| !session.is_running()).await,
            "泵失败后会话应自动转为未运行"
        );
        assert!(
            matches!(session.stop(), Err(SessionError::NotRunning)),
            "泵已失败时 stop 应报告 NotRunning"
        );
        assert!(
            session.stats().input_offline.is_some(),
            "泵失败后必须记录 input_offline"
        );
        assert!(
            !session
                .stats()
                .input_offline
                .as_ref()
                .unwrap()
                .reason
                .is_empty()
        );

        drop(event_tx);
    }

    #[test]
    fn gate_is_exposed_for_the_transport_layer() {
        let (session, _sink) = console_session_fixture();

        let reservation = session
            .gate()
            .try_acquire(RfbTransportKind::Tcp, "127.0.0.1:5900".parse().unwrap())
            .unwrap();
        assert_eq!(reservation.client_id().get(), 1);
    }

    // ---- T9 事件出口发布（传输层订阅） ----

    /// event_publisher 反映会话生命周期：未启动 None → start 后 Some → stop 后 None。
    /// 这是传输层「每次连接前读最新 sender」的契约基础。
    #[tokio::test]
    async fn event_publisher_reflects_session_lifecycle() {
        let (mut session, _sink) = console_session_fixture();

        // 未启动：订阅端读到 None。
        let publisher = session.event_publisher();
        assert!(publisher.borrow().is_none());

        // start 后读到 Some（活动出口）。
        session.start().unwrap();
        assert!(publisher.borrow().is_some());

        // stop 后回到 None。
        drop(session.stop().unwrap());
        assert!(publisher.borrow().is_none());
    }

    /// restart 后 event_publisher 反映**新** channel 的发送端——旧的 sender
    /// 已随 stop 失效，传输层必须经订阅端重新获取。若 start 未发布新 sender，
    /// 或发布了旧 sender 的克隆，此断言失败。
    #[tokio::test]
    async fn event_publisher_publishes_new_sender_after_restart() {
        let (mut session, _sink) = console_session_fixture();

        session.start().unwrap();
        let publisher = session.event_publisher();
        let first_sender = publisher.borrow().clone().unwrap();

        drop(session.stop().unwrap());
        // stop 期间 sender 失效，publisher 读到 None。
        assert!(publisher.borrow().is_none());

        session.start().unwrap();
        let second_sender = publisher.borrow().clone().unwrap();
        // 新 sender 与旧 sender 指向不同 channel（restart 后旧 channel 已关闭）。
        assert!(
            !first_sender.same_channel(&second_sender),
            "restart 后必须发布新 channel 的发送端"
        );

        drop(session.stop().unwrap());
    }

    // ---- T8 会话状态统计 ----

    /// 帧 seq 跳跃（1→3，缺 2）→ dropped_frames 计数 1；首帧初始化基准
    /// 不计数；seq 回退（3→2）与重复（2→2）视为重置，不计数。
    #[test]
    fn frame_seq_jump_does_not_count_as_dropped() {
        let mut stats = SessionStats::default();
        stats.observe_frame_seq(1);
        stats.observe_frame_seq(3);
        assert_eq!(
            stats.dropped_frames, 0,
            "seq 跳跃不计为丢帧（latest_frame 轮询机制）"
        );
        stats.observe_frame_seq(2);
        assert_eq!(stats.dropped_frames, 0, "seq 回退不计数");
    }

    /// 大跨度跳跃也不计为丢帧。
    #[test]
    fn large_frame_seq_jump_does_not_count_as_dropped() {
        let mut stats = SessionStats::default();
        stats.observe_frame_seq(1);
        stats.observe_frame_seq(5);
        assert_eq!(stats.dropped_frames, 0);
    }

    /// 输入事件计数与最后时间更新。
    #[test]
    fn input_notice_updates_stats() {
        let mut stats = SessionStats::default();
        stats.observe_input();
        stats.observe_input();
        assert_eq!(stats.input_events, 2);
        assert!(stats.last_input_ns.is_some());
    }

    /// 泵通知回路：Connected 不计入，Key + Pointer 各计一次 → input_events 2，
    /// 最后输入时间已记录（泵线程写、会话线程读）。
    #[tokio::test]
    async fn stats_accumulate_input_events_through_pump() {
        let (mut session, _sink) = console_session_fixture();
        session.start().unwrap();

        let event_tx = session.event_tx().clone();
        let client_id = RfbClientId::for_test(1);
        let peer_addr = "127.0.0.1:5900".parse().unwrap();
        event_tx
            .send(RfbServerEvent::Connected {
                client_id,
                peer_addr,
                shared: true,
            })
            .await
            .unwrap();
        event_tx
            .send(RfbServerEvent::Key {
                client_id,
                down: true,
                keysym: 0x61,
            })
            .await
            .unwrap();
        event_tx
            .send(RfbServerEvent::Pointer {
                client_id,
                button_mask: 1,
                x: 100,
                y: 200,
                framebuffer_size: ipkvm_rfb::RfbSize::new(1920, 1080).unwrap(),
            })
            .await
            .unwrap();
        assert!(
            yield_until(|| session.stats().input_events == 2).await,
            "Key+Pointer 后输入事件统计未累积到 2"
        );
        assert!(
            session.stats().last_input_ns.is_some(),
            "最后输入时间未记录"
        );

        drop(event_tx);
        drop(session.stop().unwrap());
    }

    /// 文本键入（CutText/TextTyped）走独立 notice，不计入键盘/指针输入统计。
    #[tokio::test]
    async fn text_events_do_not_count_as_input_stats() {
        let (mut session, sink) = console_session_fixture();
        session.start().unwrap();

        let event_tx = session.event_tx().clone();
        let client_id = RfbClientId::for_test(1);
        let peer_addr = "127.0.0.1:5900".parse().unwrap();
        event_tx
            .send(RfbServerEvent::Connected {
                client_id,
                peer_addr,
                shared: true,
            })
            .await
            .unwrap();
        event_tx
            .send(RfbServerEvent::CutText {
                client_id,
                bytes: b"ab".to_vec(),
            })
            .await
            .unwrap();
        // 等文本服务实际键入完成（2 字符 → 4 批按键），证明 TextTyped 通知已到达。
        wait_for(
            || sink.recorded.lock().unwrap().key_batches >= 4,
            "文本键入未完成",
        )
        .await;

        assert_eq!(session.stats().input_events, 0, "文本键入不应计入输入统计");
        assert!(session.stats().last_input_ns.is_none());

        drop(event_tx);
        drop(session.stop().unwrap());
    }

    /// observe_frame 经帧源 latest_frame().seq 检测丢帧：1→3 缺 2 计 1 帧；
    /// 空帧源（无最新帧）不计数也不初始化基准。
    #[test]
    fn observe_frame_does_not_count_dropped_frames() {
        let mock = Arc::new(MockFrameSource::new());
        let frame_source: Arc<dyn FrameSource> = mock.clone();
        let (event_publisher, _) = watch::channel(None);
        let mut session = ConsoleSession::new(
            frame_source,
            RecordingSink::default(),
            RfbConnectionGate::new(),
            event_publisher,
        );

        session.observe_frame();
        assert_eq!(session.stats().dropped_frames, 0, "空帧源不计数");

        let frame = |seq| {
            Arc::new(VideoFrame::new(
                seq,
                MonotonicTimestamp::from_nanos(0),
                1920,
                1080,
                0,
                PixelFormat::Mjpeg,
                Arc::new([0u8; 16]),
            ))
        };
        mock.publish_frame(frame(1));
        session.observe_frame();
        mock.publish_frame(frame(3));
        session.observe_frame();
        assert_eq!(session.stats().dropped_frames, 0, "seq 跳跃不计为丢帧");
    }

    /// observe_frame 同时记录 capture_ns（来自 frame.timestamp，统一时钟）与
    /// last_frame_ns（observe 时间），且 capture <= observe（端到端延迟非负）。
    #[test]
    fn observe_frame_records_capture_and_observe_times() {
        let mock = Arc::new(MockFrameSource::new());
        let frame_source: Arc<dyn FrameSource> = mock.clone();
        let (event_publisher, _) = watch::channel(None);
        let mut session = ConsoleSession::new(
            frame_source,
            RecordingSink::default(),
            RfbConnectionGate::new(),
            event_publisher,
        );

        // 用统一时钟填 timestamp，模拟真实采集时间。
        let expected_capture = ipkvm_video::now_ns();
        let frame = Arc::new(VideoFrame::new(
            1,
            MonotonicTimestamp::from_nanos(expected_capture),
            2,
            1,
            8,
            PixelFormat::Bgra8888,
            Arc::new([0u8; 8]),
        ));
        mock.publish_frame(frame);
        session.observe_frame();

        let stats = session.stats();
        let capture = stats.capture_ns.expect("capture_ns 应被记录");
        let observe = stats.last_frame_ns.expect("last_frame_ns 应被记录");
        assert_eq!(
            capture, expected_capture,
            "capture_ns 应等于 frame.timestamp"
        );
        assert!(
            observe >= capture,
            "observe ({observe}) 应 >= capture ({capture})：统一时钟下端到端延迟非负"
        );
    }

    /// 串口统计：record_serial_stats 从 QueueStats 映射填入 SessionStats.serial。
    #[test]
    fn record_serial_stats_maps_queue_stats() {
        let (mut session, _sink) = console_session_fixture();
        assert!(session.stats().serial.is_none());

        let queue_stats = QueueStats {
            batches_accepted: 3,
            frames_accepted: 7,
        };
        session.record_serial_stats(queue_stats);
        let serial: SerialStats = queue_stats.into();
        assert_eq!(session.stats().serial.unwrap(), serial);
    }

    #[test]
    fn refresh_stats_observes_frame_and_sink_queue_stats() {
        let mock = Arc::new(MockFrameSource::new());
        let frame_source: Arc<dyn FrameSource> = mock.clone();
        let queue = FakeCommandQueue::new();
        let sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let (event_publisher, _) = watch::channel(None);
        let mut session = ConsoleSession::new(
            frame_source,
            sink,
            RfbConnectionGate::new(),
            event_publisher,
        );

        let frame = |seq| {
            Arc::new(VideoFrame::new(
                seq,
                MonotonicTimestamp::from_nanos(seq),
                1,
                1,
                4,
                PixelFormat::Bgra8888,
                Arc::new([0u8; 4]),
            ))
        };
        mock.publish_frame(frame(1));
        session.refresh_stats();
        mock.publish_frame(frame(3));

        queue
            .enqueue_batch(CommandBatch::new(vec![Ch9329Frame::new(0, 0, &[]).unwrap()]).unwrap())
            .unwrap();
        session.refresh_stats();

        let stats = session.stats();
        assert_eq!(stats.dropped_frames, 0, "seq 跳跃不计为丢帧");
        assert_eq!(
            stats.serial,
            Some(crate::serial_stats::SerialStats {
                batches_accepted: 1,
                frames_accepted: 1
            })
        );
    }
}
