#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SupervisorStatus {
    pub intent: SessionIntent,
    pub video: VideoRuntimeStatus,
    pub control: ControlRuntimeStatus,
}

impl SupervisorStatus {
    pub fn new(
        intent: SessionIntent,
        video: VideoRuntimeStatus,
        control: ControlRuntimeStatus,
    ) -> Self {
        Self {
            intent,
            video,
            control,
        }
    }

    /// 上层页面路由只由用户意图决定；底层单路失败不应把用户踢回连接页。
    pub fn should_show_work_view(&self) -> bool {
        matches!(
            self.intent,
            SessionIntent::Running | SessionIntent::Recovering | SessionIntent::Failed
        )
    }
}

impl Default for SupervisorStatus {
    fn default() -> Self {
        Self {
            intent: SessionIntent::NoSelection,
            video: VideoRuntimeStatus::Idle,
            control: ControlRuntimeStatus::Idle,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionIntent {
    NoSelection,
    ManualStopped,
    Running,
    Recovering,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VideoRuntimeStatus {
    Idle,
    Starting { attempt: u32 },
    Streaming,
    Stalled { reason: String },
    Recovering { reason: String, attempt: u32 },
    Failed { reason: String, attempts: u32 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlRuntimeStatus {
    Idle,
    Starting { attempt: u32 },
    Ready,
    Recovering { reason: String, attempt: u32 },
    Failed { reason: String, attempts: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimePhase {
    Idle,
    Starting,
    Ready,
    Recovering,
    Failed,
}

impl VideoRuntimeStatus {
    pub fn phase(&self) -> RuntimePhase {
        match self {
            Self::Idle => RuntimePhase::Idle,
            Self::Starting { .. } => RuntimePhase::Starting,
            Self::Streaming => RuntimePhase::Ready,
            Self::Stalled { .. } | Self::Recovering { .. } => RuntimePhase::Recovering,
            Self::Failed { .. } => RuntimePhase::Failed,
        }
    }
}

impl ControlRuntimeStatus {
    pub fn phase(&self) -> RuntimePhase {
        match self {
            Self::Idle => RuntimePhase::Idle,
            Self::Starting { .. } => RuntimePhase::Starting,
            Self::Ready => RuntimePhase::Ready,
            Self::Recovering { .. } => RuntimePhase::Recovering,
            Self::Failed { .. } => RuntimePhase::Failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ControlRuntimeStatus, SessionIntent, SupervisorStatus, VideoRuntimeStatus};

    #[test]
    fn control_failed_still_prefers_work_view() {
        let status = SupervisorStatus {
            intent: SessionIntent::Running,
            video: VideoRuntimeStatus::Streaming,
            control: ControlRuntimeStatus::Failed {
                reason: "serial device disappeared".to_string(),
                attempts: 5,
            },
        };

        assert!(status.should_show_work_view());
    }
}
