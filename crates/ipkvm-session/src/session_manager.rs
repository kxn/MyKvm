//! 会话管理：创建、重启、停止控制台会话的生命周期。
//!
//! 委托 `ConsoleSession` 真实组装，不复制状态——state 从实际会话推断。

use std::sync::Arc;

use ipkvm_core::InputSink;
use ipkvm_video::FrameSource;
use tokio::sync::watch;

use crate::console_session::{ConsoleSession, SessionError, SessionHandle};
use crate::rfb_connection::{RfbConnectionGate, RfbServerEvent};
use crate::rfb_input::{RfbInputNotice, RfbInputRunError};

/// 会话状态：从实际会话的 `is_running()` 推断，不复制维护。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionState {
    /// 未创建会话（`empty()` 启动后、或会话已销毁）。
    Absent,
    Stopped,
    Running,
}

/// 会话管理器：委托 `ConsoleSession` 管理单个会话的生命周期。
///
/// 不复制状态——`state()` 从实际会话推断；`stop()` 委托返回的旧泵任务
/// 句柄保存在 `pending_stop`，`wait_stopped()` await 后构成「释放完成」
/// 屏障，供组装层在重启前等待旧泵收尾。
///
/// `event_publisher` 由本管理器拥有，会话持有其 clone：传输层经
/// `event_publisher()` 拿到的订阅端在会话重建后仍能读到新事件发送端。
pub struct SessionManager<S: InputSink + Clone + Send + 'static> {
    session: Option<ConsoleSession<S>>,
    /// 上次 `stop()` 委托返回的旧泵任务句柄（await 前存在）。
    pending_stop: Option<tokio::task::JoinHandle<Result<(), RfbInputRunError>>>,
    /// 当前事件出口发布端：`Some` 表示会话已启动、传输层可发事件；`None`
    /// 表示无活动出口（会话未创建/已停止/已销毁）。
    event_publisher: watch::Sender<Option<tokio::sync::mpsc::Sender<RfbServerEvent>>>,
    /// 会话 notice 镜像：组装/重建会话时注入，供桌面本地控制器观察输入状态。
    notice_mirror: Option<tokio::sync::mpsc::UnboundedSender<RfbInputNotice>>,
}

impl<S: InputSink + Clone + Send + 'static> SessionManager<S> {
    /// 组装会话管理器：调用方构造帧源与 sink（设备选择由调用方/配置层完成）。
    pub fn new(frame_source: Arc<dyn FrameSource>, sink: S, gate: RfbConnectionGate) -> Self {
        let (event_publisher, _) = watch::channel(None);
        let session = ConsoleSession::new(frame_source, sink, gate, event_publisher.clone());
        Self {
            session: Some(session),
            pending_stop: None,
            event_publisher,
            notice_mirror: None,
        }
    }

    /// 零会话启动：不立即创建会话，待 `create()` 接入帧源/sink。
    ///
    /// `event_publisher` 仍建立，传输层订阅端初始读到 `None`（无活动出口），
    /// `create()` 首启后会读到新 channel 的发送端。
    pub fn empty() -> Self {
        let (event_publisher, _) = watch::channel(None);
        Self {
            session: None,
            pending_stop: None,
            event_publisher,
            notice_mirror: None,
        }
    }

    /// 设置 notice 镜像发送端；`None` 关闭镜像。会应用到当前已组装会话；
    /// 已在运行的泵在下次重启/换设备重建后接入新镜像。
    pub fn set_notice_mirror(
        &mut self,
        notice_mirror: Option<tokio::sync::mpsc::UnboundedSender<RfbInputNotice>>,
    ) {
        self.notice_mirror = notice_mirror;
        if let Some(session) = self.session.as_mut() {
            session.set_notice_mirror(self.notice_mirror.clone());
        }
    }

    /// 首次创建会话（用于 `empty()` 启动后由 API/配置层注入帧源与 sink）。
    ///
    /// 已有会话时返回 `AlreadyCreated`。本方法只组装会话结构，不启动泵——
    /// 调用方随后调用 `start()`。
    pub fn create(
        &mut self,
        frame_source: Arc<dyn FrameSource>,
        sink: S,
        gate: RfbConnectionGate,
    ) -> Result<(), SessionError> {
        if self.session.is_some() {
            return Err(SessionError::AlreadyCreated);
        }
        let mut session = ConsoleSession::new(
            frame_source,
            sink,
            gate,
            self.event_publisher.clone(),
        );
        session.set_notice_mirror(self.notice_mirror.clone());
        self.session = Some(session);
        Ok(())
    }

    /// 会话状态：实际会话正在运行 → Running，已组装未启动 → Stopped，
    /// 无会话 → Absent。
    pub fn state(&self) -> SessionState {
        match &self.session {
            Some(session) if session.is_running() => SessionState::Running,
            Some(_) => SessionState::Stopped,
            None => SessionState::Absent,
        }
    }

    /// 底层真实会话引用（传输层事件发送端等经此获取）。
    pub fn session(&self) -> Option<&ConsoleSession<S>> {
        self.session.as_ref()
    }

    /// 刷新当前会话统计快照（帧丢失、串口队列等）。
    pub fn refresh_stats(&mut self) {
        if let Some(session) = self.session.as_mut() {
            session.refresh_stats();
        }
    }

    /// 事件出口订阅端：传输层经此获取当前活动发送端。
    ///
    /// 会话未创建时订阅端恒读 `None`；`start()` 后读 `Some(sender)`，
    /// `stop()` 后读 `None`，会话重建后自动读到新 channel。
    pub fn event_publisher(
        &self,
    ) -> watch::Receiver<Option<tokio::sync::mpsc::Sender<RfbServerEvent>>> {
        self.event_publisher.subscribe()
    }

    /// 启动会话：委托 `ConsoleSession::start`（重建事件 channel 并 spawn 泵任务）。
    pub fn start(&mut self) -> Result<SessionHandle, SessionError> {
        let Some(session) = self.session.as_mut() else {
            return Err(SessionError::NotRunning);
        };
        session.start()
    }

    /// 停止会话：委托 `ConsoleSession::stop`，返回的泵任务句柄暂存
    /// `pending_stop`，供 `wait_stopped()` await 构成释放屏障。
    pub fn stop(&mut self) -> Result<(), SessionError> {
        let Some(session) = self.session.as_mut() else {
            return Err(SessionError::NotRunning);
        };
        self.pending_stop = Some(session.stop()?);
        Ok(())
    }

    /// 等待上次 `stop()` 的旧泵收尾完成（释放完成屏障）。
    ///
    /// 无 pending（未 stop 或已等待过）时立即返回。旧泵正常退出时内部结果
    /// 必为 Ok；若任务异常终止（join 失败，如泵 panic），任务本身已结束、
    /// 屏障目的（收尾完成）已达成——泵任务内部错误（panic/Err）不在此
    /// 传播；错误面观测留给 #31。
    pub async fn wait_stopped(&mut self) {
        let Some(handle) = self.pending_stop.take() else {
            return;
        };
        let _ = handle.await;
    }

    /// 无竞态重启：`stop()` → 等待旧泵收尾（释放屏障）→ `start()`。
    ///
    /// 返回时旧泵已释放、新泵已启动。对已停止会话返回 `NotRunning`（委托
    /// `stop()` 的语义）。本方法只对**相同**帧源/串口配置做泵重启；运行时
    /// 换设备（重建帧源/串口）走 `replace_and_start()`。
    pub async fn restart(&mut self) -> Result<(), SessionError> {
        self.stop()?;
        self.wait_stopped().await;
        self.start()?;
        Ok(())
    }

    /// 替换会话组件并启动新会话。
    ///
    /// 这是运行时换设备的会话级边界：如旧会话正在运行，先停泵并等待释放；
    /// 然后丢弃旧帧源/sink，组装新 `ConsoleSession` 并启动。已停止或空会话
    /// 直接替换并启动。
    pub async fn replace_and_start(
        &mut self,
        frame_source: Arc<dyn FrameSource>,
        sink: S,
        gate: RfbConnectionGate,
    ) -> Result<(), SessionError> {
        if self
            .session
            .as_ref()
            .is_some_and(|session| session.is_running())
        {
            self.stop()?;
        }
        self.wait_stopped().await;
        let mut session = ConsoleSession::new(
            frame_source,
            sink,
            gate,
            self.event_publisher.clone(),
        );
        session.set_notice_mirror(self.notice_mirror.clone());
        self.session = Some(session);
        self.start()?;
        Ok(())
    }

    /// 停止并销毁当前会话组件。
    ///
    /// 运行时换独占设备前调用此方法：先等旧输入泵 release，再 drop 旧
    /// `ConsoleSession` 持有的帧源和 sink，确保后续工厂可以重新打开同一个
    /// 相机或串口。空会话调用是 no-op。
    pub async fn stop_and_destroy(&mut self) -> Result<(), SessionError> {
        if self
            .session
            .as_ref()
            .is_some_and(|session| session.is_running())
        {
            self.stop()?;
        }
        self.wait_stopped().await;
        self.session = None;
        self.event_publisher.send_replace(None);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use ipkvm_core::{InputResult, KeyEvent, MouseMode, PointerEvent};
    use ipkvm_video::mock::MockFrameSource;

    use super::*;
    use crate::rfb_connection::{RfbClientId, RfbServerEvent};

    /// 记录型输入 sink：内部共享 `Arc<Mutex<Recorded>>`，供测试观察泵写入
    /// 的批次与 release 行为（与 console_session 测试同款；该模块私有，
    /// 不复用，自建）。
    #[derive(Clone, Debug, Default)]
    struct RecordingSink {
        recorded: Arc<Mutex<Recorded>>,
    }

    /// 泵行为记录：键批次次数与 release_all 次数。
    #[derive(Clone, Debug, Default)]
    struct Recorded {
        key_batches: usize,
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
            Ok(())
        }

        fn release_all(&mut self) -> InputResult<()> {
            self.recorded.lock().unwrap().release_count += 1;
            Ok(())
        }
    }

    /// SessionManager 测试 fixture：MockFrameSource（ipkvm-video mock）+
    /// 记录型 sink + 新建连接闸门；返回管理器与 sink 记录句柄。
    fn session_manager_fixture() -> (SessionManager<RecordingSink>, RecordingSink) {
        let frame_source: Arc<dyn FrameSource> = Arc::new(MockFrameSource::new());
        let sink = RecordingSink::default();
        let manager = SessionManager::new(frame_source, sink.clone(), RfbConnectionGate::new());
        (manager, sink)
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

    /// 同步生命周期：state 从真实会话推断，start/restart/stop 委托生效。
    #[tokio::test]
    async fn manager_delegates_start_stop_to_console_session() {
        let (mut manager, _sink) = session_manager_fixture();

        assert_eq!(manager.state(), SessionState::Stopped);
        manager.start().unwrap();
        assert_eq!(manager.state(), SessionState::Running);
        manager.restart().await.unwrap();
        assert_eq!(manager.state(), SessionState::Running);
        manager.stop().unwrap();
        assert_eq!(manager.state(), SessionState::Stopped);
        manager.wait_stopped().await;
    }

    /// 重启必须等旧泵收尾（release 屏障完成）后才启动新泵；返回时 release_all
    /// 已执行。若 restart 退化为「不等屏障直接 start」，该断言会失败。
    #[tokio::test]
    async fn restart_waits_for_old_pump_release_before_starting_new() {
        let (mut manager, sink) = session_manager_fixture();

        manager.start().unwrap();
        let event_tx = manager.session().unwrap().event_tx().clone();
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

        manager.restart().await.unwrap();
        assert_eq!(manager.state(), SessionState::Running);
        assert!(
            sink.recorded.lock().unwrap().release_count >= 1,
            "restart 返回时旧泵必须已释放"
        );

        drop(event_tx);
        manager.stop().unwrap();
        manager.wait_stopped().await;
    }

    /// 对已停止的会话 restart 返回 NotRunning（文档语义）。
    #[tokio::test]
    async fn restart_on_stopped_session_returns_not_running() {
        let (mut manager, _sink) = session_manager_fixture();

        assert!(matches!(
            manager.restart().await,
            Err(SessionError::NotRunning)
        ));
    }

    /// 连续两次 stop：第二次报告 NotRunning。
    #[tokio::test]
    async fn second_stop_returns_not_running() {
        let (mut manager, _sink) = session_manager_fixture();

        manager.start().unwrap();
        manager.stop().unwrap();
        manager.wait_stopped().await;
        assert!(matches!(manager.stop(), Err(SessionError::NotRunning)));
    }

    /// 无 pending stop 时 wait_stopped 立即返回（不阻塞）。
    #[tokio::test]
    async fn wait_stopped_without_pending_returns_immediately() {
        let (mut manager, _sink) = session_manager_fixture();

        assert!(
            tokio::time::timeout(Duration::from_millis(100), manager.wait_stopped())
                .await
                .is_ok(),
            "无 pending stop 时 wait_stopped 应立即返回"
        );
    }

    /// 传输层仍持有事件发送端克隆时，stop + wait_stopped 仍必须构成释放屏障
    /// （停止不能依赖发送端全部释放）。
    #[tokio::test]
    async fn wait_stopped_completes_while_event_sender_clone_is_held() {
        let (mut manager, sink) = session_manager_fixture();

        manager.start().unwrap();
        let event_tx = manager.session().unwrap().event_tx().clone();
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

        manager.stop().unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), manager.wait_stopped())
                .await
                .is_ok(),
            "外部持有发送端克隆时 wait_stopped 仍应在超时内完成"
        );
        assert!(
            sink.recorded.lock().unwrap().release_count >= 1,
            "屏障返回时 pump 自身 release_all 必须已完成"
        );

        drop(event_tx);
    }

    /// 停泵屏障：stop() 委托的泵任务句柄经 wait_stopped() await 后，旧 pump
    /// 已收尾（release_all 执行完成）；同时显式覆盖「无其他 event_tx clone
    /// 时 stop 正常释放」语义。
    #[tokio::test]
    async fn wait_stopped_awaits_release_barrier() {
        let (mut manager, sink) = session_manager_fixture();

        manager.start().unwrap();
        assert_eq!(manager.state(), SessionState::Running);

        // 事件走 start() 重建的 channel 到达输入泵，建立 active controller
        // 使停泵时触发 release_all。
        let event_tx = manager.session().unwrap().event_tx().clone();
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

        // 无其他 event_tx clone：发送端克隆先释放，stop() 覆盖旧 sender 后
        // 旧 channel 关闭，pump 收到 None 后自然退出。
        drop(event_tx);
        manager.stop().unwrap();
        assert_eq!(manager.state(), SessionState::Stopped);

        // 屏障：wait_stopped() 返回时旧泵任务已收尾。join handle 解析时
        // pump 的同步 release_all（第一次释放）已执行，该断言确定性成立；
        // 若 wait_stopped 退化为 no-op，pump 尚未收尾时此处即读到 0 而失败，
        // 保证本测试对屏障具有判别力。
        manager.wait_stopped().await;
        assert!(
            sink.recorded.lock().unwrap().release_count >= 1,
            "wait_stopped 返回时 pump 自身 release_all 必须已完成"
        );

        // 文本键入服务对 sink 克隆的第二次 release 在 pump 收尾后异步到达，
        // 用轮询观察（纯轮询断言不足以判别屏障，仅覆盖异步收尾本身）。
        assert!(
            yield_until(|| sink.recorded.lock().unwrap().release_count == 2).await,
            "wait_stopped 后文本服务 release_all 未执行完成"
        );
    }

    // ---- 零会话启动与 create ----

    /// empty() 启动：无会话，state 为 Absent；start/stop/restart 均报告 NotRunning。
    #[tokio::test]
    async fn empty_manager_has_absent_state() {
        let mut manager: SessionManager<RecordingSink> = SessionManager::empty();

        assert_eq!(manager.state(), SessionState::Absent);
        assert!(matches!(manager.start(), Err(SessionError::NotRunning)));
        assert!(matches!(manager.stop(), Err(SessionError::NotRunning)));
        assert!(matches!(
            manager.restart().await,
            Err(SessionError::NotRunning)
        ));
    }

    /// create() 首次注入帧源/sink 后会话进入 Stopped，start 后 Running；
    /// 重复 create 报告 AlreadyCreated。
    #[tokio::test]
    async fn create_assembles_session_then_start_runs() {
        let mut manager: SessionManager<RecordingSink> = SessionManager::empty();
        let frame_source: Arc<dyn FrameSource> = Arc::new(MockFrameSource::new());
        let sink = RecordingSink::default();

        manager
            .create(frame_source, sink.clone(), RfbConnectionGate::new())
            .unwrap();
        assert_eq!(manager.state(), SessionState::Stopped);

        manager.start().unwrap();
        assert_eq!(manager.state(), SessionState::Running);

        // 已有会话时 create 报告 AlreadyCreated。
        assert!(matches!(
            manager.create(
                Arc::new(MockFrameSource::new()),
                sink.clone(),
                RfbConnectionGate::new(),
            ),
            Err(SessionError::AlreadyCreated)
        ));

        manager.stop().unwrap();
        manager.wait_stopped().await;
    }

    /// empty() 管理器的 event_publisher 订阅端恒读 None；create + start 后
    /// 读到 Some(sender)，传输层据此区分「无活动出口」与「可发事件」。
    #[tokio::test]
    async fn empty_manager_publisher_is_none_until_create_and_start() {
        let mut manager: SessionManager<RecordingSink> = SessionManager::empty();
        let publisher = manager.event_publisher();
        assert!(publisher.borrow().is_none());

        let frame_source: Arc<dyn FrameSource> = Arc::new(MockFrameSource::new());
        manager
            .create(
                frame_source,
                RecordingSink::default(),
                RfbConnectionGate::new(),
            )
            .unwrap();
        // create 只组装，未启动 → 仍 None。
        assert!(publisher.borrow().is_none());

        manager.start().unwrap();
        assert!(publisher.borrow().is_some());

        manager.stop().unwrap();
        assert!(publisher.borrow().is_none());
        manager.wait_stopped().await;
    }

    /// notice mirror 必须跨 create/replace_and_start 保持：重建会话后仍能收到 notice。
    #[tokio::test]
    async fn notice_mirror_survives_create_and_replace() {
        let (mut manager, _sink) = session_manager_fixture();
        let (notice_tx, mut notice_rx) = tokio::sync::mpsc::unbounded_channel();
        manager.set_notice_mirror(Some(notice_tx));
        manager.start().unwrap();

        let client_id = RfbClientId::for_test(7);
        let peer_addr = "127.0.0.1:5900".parse().unwrap();
        let event_tx = manager
            .event_publisher()
            .borrow()
            .clone()
            .expect("started session must publish event sender");
        event_tx
            .send(RfbServerEvent::Connected {
                client_id,
                peer_addr,
                shared: true,
            })
            .await
            .unwrap();

        let mut saw_notice = false;
        for _ in 0..4 {
            let notice = tokio::time::timeout(Duration::from_secs(1), notice_rx.recv())
                .await
                .unwrap()
                .unwrap();
            if matches!(
                notice,
                crate::rfb_input::RfbInputNotice::ControllerAcquired { .. }
            ) {
                saw_notice = true;
                break;
            }
        }
        assert!(saw_notice, "create 后 mirror 未收到 ControllerAcquired");

        manager
            .replace_and_start(
                Arc::new(MockFrameSource::new()),
                RecordingSink::default(),
                RfbConnectionGate::new(),
            )
            .await
            .unwrap();
        let event_tx = manager
            .event_publisher()
            .borrow()
            .clone()
            .expect("replaced session must publish event sender");
        event_tx
            .send(RfbServerEvent::Connected {
                client_id,
                peer_addr,
                shared: true,
            })
            .await
            .unwrap();

        let mut saw_after_replace = false;
        for _ in 0..4 {
            let notice = tokio::time::timeout(Duration::from_secs(1), notice_rx.recv())
                .await
                .unwrap()
                .unwrap();
            if matches!(
                notice,
                crate::rfb_input::RfbInputNotice::ControllerAcquired { .. }
            ) {
                saw_after_replace = true;
                break;
            }
        }
        assert!(
            saw_after_replace,
            "replace_and_start 后 mirror 未收到新会话 notice"
        );

        manager.stop().unwrap();
        manager.wait_stopped().await;
    }

    /// replace_and_start：运行中会话先停旧泵，再替换为新帧源/sink 并启动；
    /// 传输层订阅端应读到新的事件 channel。
    #[tokio::test]
    async fn replace_and_start_replaces_running_session_with_new_sender() {
        let (mut manager, _sink) = session_manager_fixture();
        manager.start().unwrap();
        let publisher = manager.event_publisher();
        let first_sender = publisher.borrow().clone().unwrap();

        manager
            .replace_and_start(
                Arc::new(MockFrameSource::new()),
                RecordingSink::default(),
                RfbConnectionGate::new(),
            )
            .await
            .unwrap();

        assert_eq!(manager.state(), SessionState::Running);
        let second_sender = publisher.borrow().clone().unwrap();
        assert!(
            !first_sender.same_channel(&second_sender),
            "替换会话后必须发布新事件 channel"
        );

        manager.stop().unwrap();
        manager.wait_stopped().await;
    }

    /// replace_and_start 对 stopped/absent 同样是“按新组件启动”，供 API restart
    /// 在停止后再次启动，避免把用户操作暴露成底层 NotRunning 冲突。
    #[tokio::test]
    async fn replace_and_start_starts_from_stopped_or_absent_state() {
        let (mut stopped, _sink) = session_manager_fixture();
        stopped.start().unwrap();
        stopped.stop().unwrap();
        stopped.wait_stopped().await;
        assert_eq!(stopped.state(), SessionState::Stopped);

        stopped
            .replace_and_start(
                Arc::new(MockFrameSource::new()),
                RecordingSink::default(),
                RfbConnectionGate::new(),
            )
            .await
            .unwrap();
        assert_eq!(stopped.state(), SessionState::Running);
        stopped.stop().unwrap();
        stopped.wait_stopped().await;

        let mut absent: SessionManager<RecordingSink> = SessionManager::empty();
        absent
            .replace_and_start(
                Arc::new(MockFrameSource::new()),
                RecordingSink::default(),
                RfbConnectionGate::new(),
            )
            .await
            .unwrap();
        assert_eq!(absent.state(), SessionState::Running);
        absent.stop().unwrap();
        absent.wait_stopped().await;
    }

    #[tokio::test]
    async fn stop_and_destroy_releases_session_and_allows_create_again() {
        let (mut manager, _sink) = session_manager_fixture();
        manager.start().unwrap();

        manager.stop_and_destroy().await.unwrap();

        assert_eq!(manager.state(), SessionState::Absent);
        assert!(manager.event_publisher().borrow().is_none());
        manager
            .create(
                Arc::new(MockFrameSource::new()),
                RecordingSink::default(),
                RfbConnectionGate::new(),
            )
            .unwrap();
        assert_eq!(manager.state(), SessionState::Stopped);
    }
}
