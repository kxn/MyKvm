use crate::input::{InputResult, KeyEvent, MouseMode};
use crate::serial::{CommandBatch, CommandQueue};

use super::{Ch9329Command, KeyboardReport};

const FIRST_MODIFIER_USAGE: u8 = 0xe0;
const LAST_MODIFIER_USAGE: u8 = 0xe7;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct KeyboardState {
    modifiers: u8,
    keys: [u8; 6],
}

impl KeyboardState {
    fn apply_key(&self, event: KeyEvent) -> InputResult<Option<(KeyboardState, KeyboardReport)>> {
        let mut next = *self;
        let changed = match event {
            KeyEvent::Down { usage } => next.press(usage.get())?,
            KeyEvent::Up { usage } => next.release(usage.get()),
        };
        if !changed {
            return Ok(None);
        }
        Ok(Some((next, next.report())))
    }

    fn press(&mut self, usage: u8) -> InputResult<bool> {
        if let Some(mask) = modifier_mask(usage) {
            let changed = self.modifiers & mask == 0;
            self.modifiers |= mask;
            return Ok(changed);
        }

        if self.keys.contains(&usage) {
            return Ok(false);
        }
        let Some(slot) = self.keys.iter_mut().find(|key| **key == 0) else {
            return Err(crate::InputError::RolloverLimitExceeded);
        };
        *slot = usage;
        Ok(true)
    }

    fn release(&mut self, usage: u8) -> bool {
        if let Some(mask) = modifier_mask(usage) {
            let changed = self.modifiers & mask != 0;
            self.modifiers &= !mask;
            return changed;
        }

        let Some(position) = self.keys.iter().position(|key| *key == usage) else {
            return false;
        };
        self.keys.copy_within(position + 1.., position);
        self.keys[5] = 0;
        true
    }

    fn report(&self) -> KeyboardReport {
        KeyboardReport::new(self.modifiers, self.keys)
    }
}

fn modifier_mask(usage: u8) -> Option<u8> {
    (FIRST_MODIFIER_USAGE..=LAST_MODIFIER_USAGE)
        .contains(&usage)
        .then(|| 1u8 << (usage - FIRST_MODIFIER_USAGE))
}

#[derive(Debug)]
pub struct Ch9329InputSink<Q> {
    queue: Q,
    address: u8,
    keyboard: KeyboardState,
}

impl<Q: CommandQueue> Ch9329InputSink<Q> {
    pub fn new(queue: Q, address: u8, _mouse_mode: MouseMode) -> Self {
        Self {
            queue,
            address,
            keyboard: KeyboardState::default(),
        }
    }

    pub fn handle_key(&mut self, event: KeyEvent) -> InputResult<()> {
        let Some((next, report)) = self.keyboard.apply_key(event)? else {
            return Ok(());
        };
        self.enqueue_keyboard(report)?;
        self.keyboard = next;
        Ok(())
    }

    pub fn release_all(&mut self) -> InputResult<()> {
        self.enqueue_keyboard(KeyboardState::default().report())?;
        self.keyboard = KeyboardState::default();
        Ok(())
    }

    fn enqueue_keyboard(&self, report: KeyboardReport) -> InputResult<()> {
        let frame = Ch9329Command::Keyboard(report).to_frame(self.address)?;
        let batch = CommandBatch::new(vec![frame])
            .expect("a single keyboard frame always forms a non-empty batch");
        self.queue.enqueue_batch(batch)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_serial::FakeCommandQueue;
    use crate::{CommandQueueError, InputError, KeyEvent, KeyboardUsage, MouseMode};
    use proptest::prelude::*;

    #[test]
    fn keyboard_usage_rejects_reserved_zero() {
        assert_eq!(KeyboardUsage::new(0), Err(InputError::InvalidKeyUsage(0)));
    }

    #[test]
    fn keyboard_sink_accepts_regular_key_down() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        sink.handle_key(KeyEvent::Down {
            usage: KeyboardUsage::new(0x04).unwrap(),
        })
        .unwrap();
        assert_eq!(queue.accepted_batches().len(), 1);
        assert_eq!(
            queue.accepted_batches()[0].frames()[0].data(),
            &[0, 0, 4, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn modifier_uses_modifier_byte_without_regular_key_slot() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        sink.handle_key(KeyEvent::Down {
            usage: KeyboardUsage::new(0xe1).unwrap(),
        })
        .unwrap();
        assert_eq!(
            queue.accepted_batches().last().unwrap().frames()[0].data(),
            &[0x02, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn seventh_regular_key_is_rejected_without_state_change() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        for value in 0x04..=0x09 {
            sink.handle_key(KeyEvent::Down {
                usage: KeyboardUsage::new(value).unwrap(),
            })
            .unwrap();
        }
        assert_eq!(
            sink.handle_key(KeyEvent::Down {
                usage: KeyboardUsage::new(0x0a).unwrap(),
            }),
            Err(InputError::RolloverLimitExceeded)
        );
        assert_eq!(queue.accepted_batches().len(), 6);

        sink.handle_key(KeyEvent::Up {
            usage: KeyboardUsage::new(0x04).unwrap(),
        })
        .unwrap();
        sink.handle_key(KeyEvent::Down {
            usage: KeyboardUsage::new(0x0a).unwrap(),
        })
        .unwrap();
        assert_eq!(
            queue.accepted_batches().last().unwrap().frames()[0].data(),
            &[0, 0, 5, 6, 7, 8, 9, 10]
        );
    }

    #[test]
    fn duplicate_down_and_ghost_up_do_not_enqueue_batches() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let key = KeyboardUsage::new(0x04).unwrap();
        sink.handle_key(KeyEvent::Down { usage: key }).unwrap();
        sink.handle_key(KeyEvent::Down { usage: key }).unwrap();
        sink.handle_key(KeyEvent::Up {
            usage: KeyboardUsage::new(0x05).unwrap(),
        })
        .unwrap();
        assert_eq!(queue.accepted_batches().len(), 1);
    }

    #[test]
    fn queue_failure_does_not_commit_key_state() {
        let queue = FakeCommandQueue::new();
        queue.fail_next(CommandQueueError::Closed);
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let key = KeyboardUsage::new(4).unwrap();
        assert_eq!(
            sink.handle_key(KeyEvent::Down { usage: key }),
            Err(InputError::CommandQueue(CommandQueueError::Closed))
        );
        sink.handle_key(KeyEvent::Down { usage: key }).unwrap();
        assert_eq!(queue.accepted_batches().len(), 1);
        assert_eq!(
            queue.accepted_batches()[0].frames()[0].data(),
            &[0, 0, 4, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn release_all_enqueues_zero_keyboard_even_when_state_is_empty() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        sink.release_all().unwrap();
        assert_eq!(
            queue.accepted_batches().last().unwrap().frames()[0].data(),
            &[0; 8]
        );
    }

    proptest! {
        #[test]
        fn keyboard_state_never_contains_duplicates_or_more_than_six_keys(
            events in proptest::collection::vec((0x04u8..=0x20, any::<bool>()), 0..128)
        ) {
            let mut state = KeyboardState::default();
            for (value, down) in events {
                let usage = KeyboardUsage::new(value).unwrap();
                let event = if down {
                    KeyEvent::Down { usage }
                } else {
                    KeyEvent::Up { usage }
                };
                match state.apply_key(event) {
                    Ok(Some((next, _))) => state = next,
                    Ok(None) | Err(InputError::RolloverLimitExceeded) => {}
                    Err(error) => panic!("unexpected keyboard error: {error}"),
                }
                let occupied: Vec<_> =
                    state.keys.iter().copied().filter(|key| *key != 0).collect();
                let unique: std::collections::HashSet<_> =
                    occupied.iter().copied().collect();
                prop_assert_eq!(occupied.len(), unique.len());
                prop_assert!(occupied.len() <= 6);
            }
        }
    }
}
