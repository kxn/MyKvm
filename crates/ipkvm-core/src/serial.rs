use thiserror::Error;

use crate::Ch9329Frame;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CommandBatchError {
    #[error("a command batch must contain at least one frame")]
    Empty,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandBatch {
    frames: Vec<Ch9329Frame>,
}

impl CommandBatch {
    pub fn new(frames: Vec<Ch9329Frame>) -> Result<Self, CommandBatchError> {
        if frames.is_empty() {
            return Err(CommandBatchError::Empty);
        }
        Ok(Self { frames })
    }

    pub fn frames(&self) -> &[Ch9329Frame] {
        &self.frames
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CommandQueueError {
    #[error("command queue is closed")]
    Closed,
    #[error("command queue is full")]
    Full,
}

pub type CommandQueueResult<T> = Result<T, CommandQueueError>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct QueueStats {
    pub batches_accepted: u64,
    pub frames_accepted: u64,
}

pub trait CommandQueue: Send + Sync {
    fn enqueue_batch(&self, batch: CommandBatch) -> CommandQueueResult<()>;
    fn stats(&self) -> QueueStats;
}

#[cfg(test)]
mod command_batch_tests {
    use super::*;

    #[test]
    fn command_batch_rejects_empty_frame_list() {
        assert_eq!(CommandBatch::new(Vec::new()), Err(CommandBatchError::Empty));
    }
}
