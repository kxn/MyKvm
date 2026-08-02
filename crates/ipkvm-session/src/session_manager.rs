//! 会话管理：创建、重启、停止控制台会话的生命周期。
//!
//! 委托 `ConsoleSession` 真实组装，不复制状态——state 从实际会话推断。

use std::sync::Arc;

use ipkvm_core::InputSink;
use ipkvm_video::FrameSource;

use crate::console_session::{ConsoleSession, SessionError, SessionHandle};
use crate::rfb_connection::RfbConnectionGate;
use crate::rfb_input::RfbInputRunError;

/// 会话状态：从实际会话的 `is_running()` 推断，不复制维护。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionState {
    Stopped,
    Running,
}

/// 会话管理器：委托 `ConsoleSession` 管理单个会话的生命周期。
///
/// 不复制状态——`state()` 从实际会话推断；`stop()` 委托返回的旧泵任务
/// 句柄保存在 `pending_stop`，`wait_stopped()` await 后构成「释放完成」
/// 屏障，供组装层在重启前等待旧泵收尾。
pub struct SessionManager<S: InputSink + Clone + Send + 'static> {
    session: Option<ConsoleSession<S>>,
    /// 上次 `stop()` 委托返回的旧泵任务句柄（await 前存在）。
    pending_stop: Option<tokio::task::JoinHandle<Result<(), RfbInputRunError>>>,
}

impl<S: InputSink + Clone + Send + 'static> SessionManager<S> {
    /// 组装会话管理器：调用方构造帧源与 sink（设备选择由调用方/配置层完成）。
    pub fn new(frame_source: Arc<dyn FrameSource>, sink: S, gate: RfbConnectionGate) -> Self {
        Self {
            session: Some(ConsoleSession::new(frame_source, sink, gate)),
            pending_stop: None,
        }
    }

    /// 会话状态：实际会话正在运行 → Running，否则 Stopped。
    pub fn state(&self) -> SessionState {
        if self.session.as_ref().is_some_and(|s| s.is_running()) {
            SessionState::Running
        } else {
            SessionState::Stopped
        }
    }

    /// 底层真实会话引用（传输层事件发送端等经此获取）。
    pub fn session(&self) -> Option<&ConsoleSession<S>> {
        self.session.as_ref()
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
    /// `stop()` 的语义）。帧源/串口设备重建属于 #31 配置层接线，不在此范围。
    pub async fn restart(&mut self) -> Result<(), SessionError> {
        self.stop()?;
        self.wait_stopped().await;
        self.start()?;
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
}
