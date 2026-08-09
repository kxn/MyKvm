use std::sync::{Arc, Mutex};

use crate::{CommandBatch, CommandQueue, CommandQueueError, CommandQueueResult, QueueStats};

#[derive(Debug, Default)]
struct FakeCommandQueueState {
    accepted_batches: Vec<CommandBatch>,
    fail_next: Option<CommandQueueError>,
    stats: QueueStats,
    recovery_generation: u64,
}

#[derive(Clone, Debug, Default)]
pub struct FakeCommandQueue {
    state: Arc<Mutex<FakeCommandQueueState>>,
}

impl FakeCommandQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn accepted_batches(&self) -> Vec<CommandBatch> {
        self.state
            .lock()
            .expect("fake command queue lock poisoned")
            .accepted_batches
            .clone()
    }

    pub fn fail_next(&self, error: CommandQueueError) {
        self.state
            .lock()
            .expect("fake command queue lock poisoned")
            .fail_next = Some(error);
    }

    pub fn bump_recovery_generation(&self) {
        let mut state = self.state.lock().expect("fake command queue lock poisoned");
        state.recovery_generation = state.recovery_generation.saturating_add(1);
    }
}

impl CommandQueue for FakeCommandQueue {
    fn enqueue_batch(&self, batch: CommandBatch) -> CommandQueueResult<()> {
        let mut state = self.state.lock().expect("fake command queue lock poisoned");
        if let Some(error) = state.fail_next.take() {
            return Err(error);
        }

        state.stats.batches_accepted = state.stats.batches_accepted.saturating_add(1);
        state.stats.frames_accepted = state
            .stats
            .frames_accepted
            .saturating_add(batch.frames().len() as u64);
        state.accepted_batches.push(batch);
        Ok(())
    }

    fn stats(&self) -> QueueStats {
        self.state
            .lock()
            .expect("fake command queue lock poisoned")
            .stats
    }

    fn recovery_generation(&self) -> u64 {
        self.state
            .lock()
            .expect("fake command queue lock poisoned")
            .recovery_generation
    }
}

#[cfg(test)]
mod command_queue_tests {
    use super::*;
    use crate::Ch9329Frame;

    fn frame(command: u8) -> Ch9329Frame {
        Ch9329Frame::new(0, command, &[]).unwrap()
    }

    #[test]
    fn fake_queue_preserves_batch_boundaries_and_order() {
        let queue = FakeCommandQueue::new();
        let first = CommandBatch::new(vec![frame(2), frame(4)]).unwrap();
        let second = CommandBatch::new(vec![frame(5)]).unwrap();
        queue.enqueue_batch(first.clone()).unwrap();
        queue.enqueue_batch(second.clone()).unwrap();
        assert_eq!(queue.accepted_batches(), vec![first, second]);
        assert_eq!(queue.stats().batches_accepted, 2);
        assert_eq!(queue.stats().frames_accepted, 3);
    }

    #[test]
    fn fake_queue_rejects_configured_batch_without_recording_it() {
        let queue = FakeCommandQueue::new();
        queue.fail_next(CommandQueueError::Closed);
        assert_eq!(
            queue.enqueue_batch(CommandBatch::new(vec![frame(2)]).unwrap()),
            Err(CommandQueueError::Closed)
        );
        assert!(queue.accepted_batches().is_empty());
        assert_eq!(queue.stats().batches_accepted, 0);
    }

    #[test]
    fn fake_queue_clones_share_state() {
        let queue = FakeCommandQueue::new();
        let clone = queue.clone();
        clone
            .enqueue_batch(CommandBatch::new(vec![frame(2)]).unwrap())
            .unwrap();
        assert_eq!(queue.accepted_batches().len(), 1);
    }
}
