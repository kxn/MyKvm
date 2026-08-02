//! headless 会话自动恢复：输入泵失败/视频从未出帧时按指数退避重建会话。
//!
//! 策略要点（避免目标机反复上下电时抢串口）：
//! - 输入泵因串口写失败退出（`input_offline` 存在）→ 退避重建；
//! - 视频**从未出帧**超过阈值（设备可能未工作）→ 退避重建；
//! - 视频曾出帧后停滞（目标机重启场景）→ 只报告不重启，恢复后自动继续。

use std::sync::Arc;
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
    use super::*;

    #[test]
    fn next_delay_backs_off_exponentially_and_caps() {
        let policy = RecoveryPolicy::default();
        assert_eq!(policy.next_delay(0), Duration::from_secs(1));
        assert_eq!(policy.next_delay(1), Duration::from_secs(2));
        assert_eq!(policy.next_delay(4), Duration::from_secs(16));
        assert_eq!(policy.next_delay(30), policy.max_delay);
        assert_eq!(policy.next_delay(100), policy.max_delay);
    }
}
