use std::time::Duration;

/// 底层链路恢复策略。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryPolicy {
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub max_attempts: u32,
    pub tick: Duration,
    pub video_start_timeout: Duration,
}

impl RecoveryPolicy {
    pub fn next_delay(&self, attempt: u32) -> Option<Duration> {
        if attempt >= self.max_attempts {
            return None;
        }

        let multiplier = 1u32.checked_shl(attempt).unwrap_or(u32::MAX);
        Some(
            self.base_delay
                .saturating_mul(multiplier)
                .min(self.max_delay),
        )
    }
}

impl Default for RecoveryPolicy {
    fn default() -> Self {
        Self {
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(3),
            max_attempts: 8,
            tick: Duration::from_millis(100),
            video_start_timeout: Duration::from_secs(3),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::RecoveryPolicy;

    #[test]
    fn next_delay_exponential_caps_and_exhausts() {
        let policy = RecoveryPolicy {
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(350),
            max_attempts: 4,
            tick: Duration::from_millis(25),
            video_start_timeout: Duration::from_secs(2),
        };

        assert_eq!(policy.next_delay(0), Some(Duration::from_millis(100)));
        assert_eq!(policy.next_delay(1), Some(Duration::from_millis(200)));
        assert_eq!(policy.next_delay(2), Some(Duration::from_millis(350)));
        assert_eq!(policy.next_delay(3), Some(Duration::from_millis(350)));
        assert_eq!(policy.next_delay(4), None);
    }
}
