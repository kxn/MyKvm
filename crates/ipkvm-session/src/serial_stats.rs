//! 串口统计：从 `ipkvm_core::CommandQueue::stats()` 映射。

use ipkvm_core::QueueStats;

/// 串口队列统计快照（批次/帧接受计数）。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SerialStats {
    /// 已接受的命令批次总数。
    pub batches_accepted: u64,
    /// 已接受的 CH9329 帧总数。
    pub frames_accepted: u64,
}

impl From<QueueStats> for SerialStats {
    fn from(stats: QueueStats) -> Self {
        Self {
            batches_accepted: stats.batches_accepted,
            frames_accepted: stats.frames_accepted,
        }
    }
}
