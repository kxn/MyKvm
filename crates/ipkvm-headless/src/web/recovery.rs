//! headless 会话自动恢复：输入泵失败/视频从未出帧时按指数退避重建会话。
//!
//! 策略要点（避免目标机反复上下电时抢串口）：
//! - 输入泵因串口写失败退出（`input_offline` 存在）→ 退避重建；
//! - 视频**从未出帧**超过阈值（设备可能未工作）→ 退避重建；
//! - 视频曾出帧后停滞（目标机重启场景）→ 只报告不重启，恢复后自动继续。

use std::sync::{Arc, atomic::Ordering};
use std::time::Duration;

use ipkvm_core::InputSink;
use tokio::sync::watch;

use crate::frame_source::EmptyFrameSource;

use super::service::{ApiState, create_and_start_session, session_state_name};

/// 自动恢复策略：指数退避 + 上限；视频只对“从未出帧”重启。
#[derive(Clone, Debug)]
pub struct RecoveryPolicy {
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub video_start_timeout: Duration,
    pub tick: Duration,
}

impl Default for RecoveryPolicy {
    fn default() -> Self {
        Self {
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            video_start_timeout: Duration::from_secs(5),
            tick: Duration::from_millis(500),
        }
    }
}

impl RecoveryPolicy {
    /// 第 `consecutive_failures` 次失败后的重试间隔（1s,2s,4s,…，上限 30s）。
    pub fn next_delay(&self, consecutive_failures: u32) -> Duration {
        let delay = self
            .base_delay
            .saturating_mul(2u32.saturating_pow(consecutive_failures.min(30)));
        delay.min(self.max_delay)
    }
}

/// 恢复循环：常驻后台任务，按策略监视会话状态并重建会话。
pub async fn run_recovery_loop<I: InputSink + Clone + Send + 'static>(
    api: Arc<ApiState<I>>,
    mut shutdown: watch::Receiver<bool>,
    policy: RecoveryPolicy,
) {
    let mut failures: u32 = 0;
    let mut stopped_since: Option<std::time::Instant> = None;
    let mut last_attempt: Option<std::time::Instant> = None;
    loop {
        if *shutdown.borrow() {
            return;
        }
        tokio::select! {
            _ = tokio::time::sleep(policy.tick) => {}
            _ = shutdown.changed() => return,
        }
        // 手动停止：用户 stop 后不得自动复活，直到 create/restart 清除标记。
        if api.manual_stop.load(Ordering::Relaxed) {
            stopped_since = None;
            failures = 0;
            continue;
        }
        let now = std::time::Instant::now();
        let mut manager = api.manager.lock().await;
        manager.refresh_stats();
        let (state_name, input_offline, last_frame_ns) = match manager.session() {
            Some(session) => {
                let stats = session.stats();
                (
                    session_state_name(manager.state()),
                    stats.input_offline.clone(),
                    stats.last_frame_ns,
                )
            }
            None => ("absent", None, None),
        };
        match state_name {
            "running" => {
                stopped_since = None;
                failures = 0;
            }
            "stopped" => {
                // 持锁后再查一次，闭合「读标记 → 等待 manager 锁 → stop 已置位」
                // 的竞态：stop 持同一把锁置位，恢复循环拿到锁后必然看到新值。
                if api.manual_stop.load(Ordering::Relaxed) {
                    continue;
                }
                let stopped_at = *stopped_since.get_or_insert(now);
                let reason_present = input_offline.is_some();
                let video_never_started = last_frame_ns.is_none()
                    && now.duration_since(stopped_at) >= policy.video_start_timeout;
                if !(reason_present || video_never_started) {
                    continue;
                }
                let delay = policy.next_delay(failures);
                if last_attempt.is_some_and(|t| now.duration_since(t) < delay) {
                    continue;
                }
                let Some(selection) = api.selection.lock().await.clone() else {
                    continue;
                };
                let _ = manager.stop_and_destroy().await;
                api.frame_source
                    .set_current(Arc::new(EmptyFrameSource::new()));
                match api.factory.build(&selection) {
                    Ok((frame_source, sink)) => {
                        match create_and_start_session(
                            &mut manager,
                            &frame_source,
                            sink,
                            api.gate.clone(),
                        ) {
                            Ok(()) => {
                                api.frame_source.set_current(frame_source);
                                *api.selection.lock().await = Some(selection);
                                failures = 0;
                                stopped_since = None;
                                last_attempt = Some(now);
                            }
                            Err(_) => {
                                failures = failures.saturating_add(1);
                                last_attempt = Some(now);
                            }
                        }
                    }
                    Err(_) => {
                        failures = failures.saturating_add(1);
                        last_attempt = Some(now);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use ipkvm_core::{InputResult, InputSink, KeyEvent, MouseMode, PointerEvent};
    use ipkvm_device::StaticDeviceInventoryProvider;
    use ipkvm_session::{
        rfb_connection::RfbConnectionGate,
        session_manager::{SessionManager, SessionState},
    };
    use ipkvm_video::mock::MockFrameSource;

    use super::*;
    use crate::frame_source::SwitchableFrameSource;
    use crate::settings::SettingsStore;
    use crate::web::service::{SessionFactory, SessionSelection};

    /// 无副作用测试 sink。
    #[derive(Clone, Debug, Default)]
    struct NoopSink;

    impl InputSink for NoopSink {
        fn set_mouse_mode(&mut self, _mode: MouseMode) -> InputResult<()> {
            Ok(())
        }

        fn handle_key_batch(&mut self, _events: &[KeyEvent]) -> InputResult<()> {
            Ok(())
        }

        fn handle_pointer_batch(&mut self, _events: &[PointerEvent]) -> InputResult<()> {
            Ok(())
        }

        fn release_all(&mut self) -> InputResult<()> {
            Ok(())
        }
    }

    /// 计数工厂：每次 build 递增计数并返回可用会话组件。
    struct CountingFactory {
        builds: Arc<AtomicUsize>,
    }

    impl SessionFactory<NoopSink> for CountingFactory {
        fn build(
            &self,
            _selection: &SessionSelection,
        ) -> Result<(Arc<dyn ipkvm_video::FrameSource>, NoopSink), String> {
            self.builds.fetch_add(1, Ordering::SeqCst);
            Ok((Arc::new(MockFrameSource::new()), NoopSink))
        }
    }

    /// 构造「已停止且视频从未出帧」的会话状态：无帧源 + start 后正常 stop，
    /// 满足恢复循环的 `video_never_started` 条件。返回可注入恢复循环的
    /// ApiState 与 build 计数。
    async fn stopped_without_frames_api(builds: Arc<AtomicUsize>) -> Arc<ApiState<NoopSink>> {
        let source: Arc<dyn ipkvm_video::FrameSource> = Arc::new(MockFrameSource::new());
        let gate = RfbConnectionGate::new();
        let mut manager = SessionManager::new(Arc::clone(&source), NoopSink, gate.clone());
        manager.start().unwrap();
        manager.stop().unwrap();
        manager.wait_stopped().await;
        assert_eq!(manager.state(), SessionState::Stopped);
        assert!(
            manager.session().unwrap().stats().last_frame_ns.is_none(),
            "无帧源会话必须满足 video_never_started 条件"
        );

        let switchable = Arc::new(SwitchableFrameSource::new(source));
        let manager = Arc::new(tokio::sync::Mutex::new(manager));
        let settings_dir = std::env::temp_dir().join(format!(
            "ipkvm-headless-recovery-settings-{}",
            std::process::id()
        ));
        let (settings, _) = SettingsStore::load_from(settings_dir);
        Arc::new(ApiState {
            frame_source: switchable,
            gate,
            manager,
            selection: tokio::sync::Mutex::new(Some(SessionSelection::default())),
            factory: Arc::new(CountingFactory { builds }),
            device_provider: Arc::new(StaticDeviceInventoryProvider::new(Vec::new(), Vec::new())),
            settings: Arc::new(settings),
            manual_stop: Arc::new(AtomicBool::new(false)),
        })
    }

    /// 推进虚拟时钟并让恢复循环运行多个 tick。
    async fn advance(duration: Duration) {
        tokio::time::advance(duration).await;
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
    }

    #[test]
    fn next_delay_backs_off_exponentially_and_caps() {
        let policy = RecoveryPolicy::default();
        assert_eq!(policy.next_delay(0), Duration::from_secs(1));
        assert_eq!(policy.next_delay(1), Duration::from_secs(2));
        assert_eq!(policy.next_delay(4), Duration::from_secs(16));
        assert_eq!(policy.next_delay(30), policy.max_delay);
        assert_eq!(policy.next_delay(100), policy.max_delay);
    }

    /// 手动停止标记置位时，即使会话满足自动恢复条件（stopped + 视频从未出帧），
    /// 恢复循环也不得重建；标记清除后恢复自动重建。
    #[tokio::test(start_paused = true)]
    async fn manual_stop_prevents_revival_until_cleared() {
        let builds = Arc::new(AtomicUsize::new(0));
        let api = stopped_without_frames_api(Arc::clone(&builds)).await;
        api.manual_stop.store(true, Ordering::SeqCst);

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let policy = RecoveryPolicy {
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_secs(1),
            // 置 0 使「视频从未出帧」条件在首 tick 即成立（std Instant 不受
            // tokio 虚拟时钟影响，不能靠推进时间满足超时）。
            video_start_timeout: Duration::ZERO,
            tick: Duration::from_millis(10),
        };
        let task = tokio::spawn(run_recovery_loop(
            Arc::clone(&api),
            shutdown_rx,
            policy.clone(),
        ));

        advance(Duration::from_millis(100)).await;
        assert_eq!(builds.load(Ordering::SeqCst), 0, "手动停止期间不得重建会话");

        api.manual_stop.store(false, Ordering::SeqCst);
        advance(Duration::from_millis(100)).await;
        assert_eq!(
            builds.load(Ordering::SeqCst),
            1,
            "清除手动停止后应恢复自动重建"
        );
        assert_eq!(api.manager.lock().await.state(), SessionState::Running);

        // 会话已 running：后续 tick 不再重建。
        advance(Duration::from_millis(100)).await;
        assert_eq!(builds.load(Ordering::SeqCst), 1);

        shutdown_tx.send_replace(true);
        task.await.unwrap();
    }
}
