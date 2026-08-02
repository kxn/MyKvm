//! 控制台会话组装：把帧源、输入 sink、连接闸门和输入泵组装成可运行的会话。

use std::sync::{Arc, Mutex};

use ipkvm_core::{InputSink, QueueStats};
use ipkvm_video::FrameSource;
use thiserror::Error;
use tokio::sync::mpsc;

use crate::rfb_connection::{RfbConnectionGate, RfbServerEvent};
use crate::rfb_input::{RfbInputNotice, RfbInputPump, RfbInputRunError};

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
    /// 上次观察的帧 seq（丢帧检测内部状态）。
    last_seq: Option<u64>,
}

impl SessionStats {
    /// 记录一次输入事件（键盘/指针）。
    pub fn observe_input(&mut self) {
        self.input_events = self.input_events.saturating_add(1);
        self.last_input_ns = Some(crate::now_ns());
    }

    /// 观察一帧：seq 跳跃即丢帧（首帧初始化基准，不计数；seq 回退/重复
    /// 视为重置，不计数）。
    pub fn observe_frame_seq(&mut self, seq: u64) {
        match self.last_seq {
            Some(last) if seq > last + 1 => {
                self.dropped_frames = self.dropped_frames.saturating_add(seq - last - 1);
            }
            _ => {}
        }
        self.last_seq = Some(seq);
    }
}

/// 会话级错误。
#[derive(Debug, Error)]
pub enum SessionError {
    #[error("session is already running")]
    AlreadyRunning,
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
    /// 会话统计：泵任务（输入计数/最后输入时间）与会话侧共享，Mutex 内部可变。
    stats: Arc<Mutex<SessionStats>>,
}

impl<S: InputSink + Clone + Send + 'static> ConsoleSession<S> {
    /// 组装会话。事件通道由 `start()` 在启动时重建，因此这里不接收发送端。
    pub fn new(frame_source: Arc<dyn FrameSource>, sink: S, gate: RfbConnectionGate) -> Self {
        Self {
            frame_source,
            sink,
            gate,
            // 占位发送端（无接收端）：与 stop() 停泵后的事件出口语义一致。
            event_tx: mpsc::channel(1).0,
            pump_task: None,
            stats: Arc::new(Mutex::new(SessionStats::default())),
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
    pub fn observe_frame(&mut self) {
        if let Some(frame) = self.frame_source.latest_frame() {
            self.stats.lock().unwrap().observe_frame_seq(frame.seq);
        }
    }

    /// 记录串口统计快照：调用方（持有 `CommandQueue` 的组装层）从队列
    /// `stats()` 读取后填入，供 `/api/status` 与桌面状态栏消费。
    pub fn record_serial_stats(&mut self, stats: QueueStats) {
        self.stats.lock().unwrap().serial = Some(stats.into());
    }

    /// 会话是否已启动（输入泵任务在运行）。
    pub fn is_running(&self) -> bool {
        self.pump_task.is_some()
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
        let (event_tx, mut event_rx) = mpsc::channel(64);
        self.event_tx = event_tx;
        let mut pump = RfbInputPump::new(self.sink.clone());
        let stats = Arc::clone(&self.stats);
        let task = tokio::spawn(async move {
            pump.run(&mut event_rx, |notice: &RfbInputNotice| {
                if matches!(
                    notice,
                    RfbInputNotice::Keyboard { .. } | RfbInputNotice::Pointer { .. }
                ) {
                    stats.lock().unwrap().observe_input();
                }
            })
            .await
        });
        self.pump_task = Some(task);
        Ok(SessionHandle)
    }

    /// 停止会话：用新 channel 的发送端覆盖旧发送端，若没有其他 clone（传输层），
    /// 旧 channel 关闭 → pump 的 `receiver.recv()` 返回 None → 自然退出并
    /// `release_all`。
    ///
    /// 不 abort：abort 会在 release_all 执行前终止任务。停泵依赖 channel 关闭
    /// 自然退出；本方法返回旧泵任务的 join handle（`#[must_use]`），供调用方
    /// 在 async 上下文中 join，构成「释放完成」屏障——方法返回时 pump 可能
    /// 仍在收尾，join（await handle）之后才保证 release_all 已执行。不需要
    /// 屏障的调用方需显式丢弃（`drop(handle)`）。
    pub fn stop(
        &mut self,
    ) -> Result<tokio::task::JoinHandle<Result<(), RfbInputRunError>>, SessionError> {
        let task = self.pump_task.take().ok_or(SessionError::NotRunning)?;
        self.event_tx = mpsc::channel(1).0;
        Ok(task)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use ipkvm_core::{InputResult, KeyEvent, MouseMode, PointerEvent, QueueStats};
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

    /// 会话测试 fixture：MockFrameSource（ipkvm-video mock）+ 记录型 sink +
    /// 新建连接闸门；返回会话与 sink 记录句柄。
    fn console_session_fixture() -> (ConsoleSession<RecordingSink>, RecordingSink) {
        let frame_source: Arc<dyn FrameSource> = Arc::new(MockFrameSource::new());
        let sink = RecordingSink::default();
        let session = ConsoleSession::new(frame_source, sink.clone(), RfbConnectionGate::new());
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

    #[test]
    fn stop_without_start_reports_not_running() {
        let (mut session, _sink) = console_session_fixture();

        assert!(matches!(session.stop(), Err(SessionError::NotRunning)));
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

        // 释放是异步完成的：pump 自身 release_all 一次，其文本键入服务在收到
        // 取消命令后对 sink 克隆再 release_all 一次，共享计数应为 2。
        assert!(
            yield_until(|| sink.recorded.lock().unwrap().release_count == 2).await,
            "stop 后 release_all 未被执行（异步释放未完成）"
        );
        let _ = handle;
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

    // ---- T8 会话状态统计 ----

    /// 帧 seq 跳跃（1→3，缺 2）→ dropped_frames 计数 1；首帧初始化基准
    /// 不计数；seq 回退（3→2）与重复（2→2）视为重置，不计数。
    #[test]
    fn frame_seq_jump_counts_dropped_frame() {
        let mut stats = SessionStats::default();
        stats.observe_frame_seq(1);
        stats.observe_frame_seq(3);
        assert_eq!(stats.dropped_frames, 1);
        stats.observe_frame_seq(2);
        assert_eq!(stats.dropped_frames, 1, "seq 回退不计数");
        stats.observe_frame_seq(2);
        assert_eq!(stats.dropped_frames, 1, "seq 重复不计数");
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

    /// observe_frame 经帧源 latest_frame().seq 检测丢帧：1→3 缺 2 计 1 帧；
    /// 空帧源（无最新帧）不计数也不初始化基准。
    #[test]
    fn observe_frame_counts_dropped_frames() {
        let mock = Arc::new(MockFrameSource::new());
        let frame_source: Arc<dyn FrameSource> = mock.clone();
        let mut session = ConsoleSession::new(
            frame_source,
            RecordingSink::default(),
            RfbConnectionGate::new(),
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
        assert_eq!(session.stats().dropped_frames, 1);
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
}
