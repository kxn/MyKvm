use std::collections::{BTreeMap, BTreeSet};

use ipkvm_core::{InputSink, KeyEvent, KeyboardUsage};

use super::keymap::{MappedKey, ShiftRequirement, map_keysym};
use super::{RfbKeyboardError, RfbKeyboardOutcome};

#[derive(Debug, Default)]
pub struct RfbKeyboardMapper {
    active_keys: BTreeMap<u32, MappedKey>,
    committed_usages: BTreeSet<KeyboardUsage>,
}

impl RfbKeyboardMapper {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle_key(
        &mut self,
        sink: &mut impl InputSink,
        down: bool,
        keysym: u32,
    ) -> Result<RfbKeyboardOutcome, RfbKeyboardError> {
        if down && self.active_keys.contains_key(&keysym) {
            return Ok(RfbKeyboardOutcome::DuplicateDown);
        }
        if !down && !self.active_keys.contains_key(&keysym) {
            return Ok(RfbKeyboardOutcome::UnknownRelease);
        }

        let mut next_active = self.active_keys.clone();
        if down {
            next_active.insert(keysym, map_keysym(keysym)?);
        } else {
            next_active.remove(&keysym);
        }

        let target = target_usages(&next_active)?;
        let events = diff_usages(&self.committed_usages, &target);
        if !events.is_empty() {
            sink.handle_key_batch(&events)?;
        }

        self.active_keys = next_active;
        self.committed_usages = target;
        Ok(RfbKeyboardOutcome::Applied)
    }
}

fn target_usages(
    active: &BTreeMap<u32, MappedKey>,
) -> Result<BTreeSet<KeyboardUsage>, RfbKeyboardError> {
    let mut target = BTreeSet::new();
    let mut requires_shift = false;
    let mut forbids_shift = false;

    for mapped in active.values() {
        match *mapped {
            MappedKey::Direct(usage) => {
                target.insert(usage);
            }
            MappedKey::Character { usage, shift } => {
                target.insert(usage);
                match shift {
                    ShiftRequirement::Required => requires_shift = true,
                    ShiftRequirement::NotRequired => forbids_shift = true,
                }
            }
        }
    }

    if requires_shift && forbids_shift {
        return Err(RfbKeyboardError::ConflictingShiftRequirements);
    }
    let left_shift = usage(0xe1);
    let right_shift = usage(0xe5);
    if requires_shift && !target.contains(&left_shift) && !target.contains(&right_shift) {
        target.insert(left_shift);
    } else if forbids_shift {
        target.remove(&left_shift);
        target.remove(&right_shift);
    }

    Ok(target)
}

fn diff_usages(
    current: &BTreeSet<KeyboardUsage>,
    target: &BTreeSet<KeyboardUsage>,
) -> Vec<KeyEvent> {
    let released = current.difference(target).copied();
    let pressed = target.difference(current).copied();
    let mut events = Vec::with_capacity(released.clone().count() + pressed.clone().count());

    events.extend(
        released
            .clone()
            .filter(|usage| !is_modifier(*usage))
            .map(|usage| KeyEvent::Up { usage }),
    );
    events.extend(
        released
            .filter(|usage| is_modifier(*usage))
            .map(|usage| KeyEvent::Up { usage }),
    );
    events.extend(
        pressed
            .clone()
            .filter(|usage| is_modifier(*usage))
            .map(|usage| KeyEvent::Down { usage }),
    );
    events.extend(
        pressed
            .filter(|usage| !is_modifier(*usage))
            .map(|usage| KeyEvent::Down { usage }),
    );
    events
}

fn is_modifier(usage: KeyboardUsage) -> bool {
    (0xe0..=0xe7).contains(&usage.get())
}

fn usage(value: u8) -> KeyboardUsage {
    KeyboardUsage::new(value).expect("keyboard mapper contains valid HID usages")
}

#[cfg(test)]
mod tests {
    use ipkvm_core::{
        Ch9329InputSink, InputError, InputResult, InputSink, KeyEvent, KeyboardUsage, MouseMode,
        PointerEvent, fake_serial::FakeCommandQueue,
    };

    use super::*;
    use crate::rfb_input::{RfbKeyboardError, RfbKeyboardOutcome};

    #[derive(Default)]
    struct RecordingSink {
        batches: Vec<Vec<KeyEvent>>,
        fail_next: bool,
    }

    impl InputSink for RecordingSink {
        fn set_mouse_mode(&mut self, _mode: MouseMode) -> InputResult<()> {
            Ok(())
        }

        fn handle_key_batch(&mut self, events: &[KeyEvent]) -> InputResult<()> {
            if std::mem::take(&mut self.fail_next) {
                return Err(InputError::RolloverLimitExceeded);
            }
            self.batches.push(events.to_vec());
            Ok(())
        }

        fn handle_pointer_batch(&mut self, _events: &[PointerEvent]) -> InputResult<()> {
            Ok(())
        }

        fn release_all(&mut self) -> InputResult<()> {
            Ok(())
        }
    }

    fn down(usage: u8) -> KeyEvent {
        KeyEvent::Down {
            usage: KeyboardUsage::new(usage).unwrap(),
        }
    }

    fn up(usage: u8) -> KeyEvent {
        KeyEvent::Up {
            usage: KeyboardUsage::new(usage).unwrap(),
        }
    }

    #[test]
    fn uppercase_without_remote_shift_is_one_atomic_batch() {
        let mut mapper = RfbKeyboardMapper::new();
        let mut sink = RecordingSink::default();

        assert_eq!(
            mapper.handle_key(&mut sink, true, 'A' as u32),
            Ok(RfbKeyboardOutcome::Applied)
        );
        assert_eq!(sink.batches, vec![vec![down(0xe1), down(0x04)]]);

        mapper.handle_key(&mut sink, false, 'A' as u32).unwrap();
        assert_eq!(sink.batches[1], vec![up(0x04), up(0xe1)]);
    }

    #[test]
    fn lowercase_temporarily_suppresses_remote_shift() {
        let mut mapper = RfbKeyboardMapper::new();
        let mut sink = RecordingSink::default();

        mapper.handle_key(&mut sink, true, 0xffe1).unwrap();
        mapper.handle_key(&mut sink, true, 'a' as u32).unwrap();
        mapper.handle_key(&mut sink, false, 'a' as u32).unwrap();

        assert_eq!(sink.batches[0], vec![down(0xe1)]);
        assert_eq!(sink.batches[1], vec![up(0xe1), down(0x04)]);
        assert_eq!(sink.batches[2], vec![up(0x04), down(0xe1)]);
    }

    #[test]
    fn opposite_shift_characters_are_rejected_without_state_commit() {
        let mut mapper = RfbKeyboardMapper::new();
        let mut sink = RecordingSink::default();
        mapper.handle_key(&mut sink, true, 'A' as u32).unwrap();

        assert_eq!(
            mapper.handle_key(&mut sink, true, 'b' as u32),
            Err(RfbKeyboardError::ConflictingShiftRequirements)
        );
        assert_eq!(sink.batches.len(), 1);
        assert_eq!(
            mapper.handle_key(&mut sink, false, 'b' as u32),
            Ok(RfbKeyboardOutcome::UnknownRelease)
        );
        mapper.handle_key(&mut sink, false, 'A' as u32).unwrap();
        assert_eq!(sink.batches[1], vec![up(0x04), up(0xe1)]);
    }

    #[test]
    fn rejected_sink_batch_can_be_retried() {
        let mut mapper = RfbKeyboardMapper::new();
        let mut sink = RecordingSink {
            fail_next: true,
            ..RecordingSink::default()
        };

        assert!(matches!(
            mapper.handle_key(&mut sink, true, 'A' as u32),
            Err(RfbKeyboardError::Input(_))
        ));
        assert_eq!(
            mapper.handle_key(&mut sink, true, 'A' as u32),
            Ok(RfbKeyboardOutcome::Applied)
        );
        assert_eq!(sink.batches, vec![vec![down(0xe1), down(0x04)]]);
    }

    #[test]
    fn aliases_share_one_usage_until_the_last_release() {
        let mut mapper = RfbKeyboardMapper::new();
        let mut sink = RecordingSink::default();

        mapper.handle_key(&mut sink, true, 0xffe7).unwrap();
        mapper.handle_key(&mut sink, true, 0xffeb).unwrap();
        assert_eq!(sink.batches, vec![vec![down(0xe3)]]);

        mapper.handle_key(&mut sink, false, 0xffe7).unwrap();
        assert_eq!(sink.batches.len(), 1);

        mapper.handle_key(&mut sink, false, 0xffeb).unwrap();
        assert_eq!(sink.batches[1], vec![up(0xe3)]);
    }

    #[test]
    fn duplicate_down_and_unknown_up_are_deterministic() {
        let mut mapper = RfbKeyboardMapper::new();
        let mut sink = RecordingSink::default();

        mapper.handle_key(&mut sink, true, 'a' as u32).unwrap();
        assert_eq!(
            mapper.handle_key(&mut sink, true, 'a' as u32),
            Ok(RfbKeyboardOutcome::DuplicateDown)
        );
        assert_eq!(
            mapper.handle_key(&mut sink, false, 0xdead_beef),
            Ok(RfbKeyboardOutcome::UnknownRelease)
        );
        assert_eq!(sink.batches, vec![vec![down(0x04)]]);
    }

    #[test]
    fn lock_keysyms_reach_sink_as_explicit_hid_keys() {
        for (keysym, usage) in [(0xffe5, 0x39), (0xff7f, 0x53), (0xff14, 0x47)] {
            let mut mapper = RfbKeyboardMapper::new();
            let mut sink = RecordingSink::default();

            assert_eq!(
                mapper.handle_key(&mut sink, true, keysym),
                Ok(RfbKeyboardOutcome::Applied),
                "lock keysym {keysym:#x} down 必须应用"
            );
            assert_eq!(
                mapper.handle_key(&mut sink, false, keysym),
                Ok(RfbKeyboardOutcome::Applied),
                "lock keysym {keysym:#x} up 必须应用"
            );
            assert_eq!(sink.batches, vec![vec![down(usage)], vec![up(usage)]]);
        }
    }

    #[test]
    fn required_characters_share_synthesized_shift() {
        let mut mapper = RfbKeyboardMapper::new();
        let mut sink = RecordingSink::default();

        mapper.handle_key(&mut sink, true, 'A' as u32).unwrap();
        mapper.handle_key(&mut sink, true, 'B' as u32).unwrap();
        mapper.handle_key(&mut sink, false, 'A' as u32).unwrap();
        mapper.handle_key(&mut sink, false, 'B' as u32).unwrap();

        assert_eq!(
            sink.batches,
            vec![
                vec![down(0xe1), down(0x04)],
                vec![down(0x05)],
                vec![up(0x04)],
                vec![up(0x05), up(0xe1)],
            ]
        );
    }

    #[test]
    fn real_ch9329_sink_rejects_seventh_key_without_mapper_state_drift() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let mut mapper = RfbKeyboardMapper::new();

        for character in 'a'..='f' {
            assert_eq!(
                mapper.handle_key(&mut sink, true, character as u32),
                Ok(RfbKeyboardOutcome::Applied)
            );
        }
        assert_eq!(
            mapper.handle_key(&mut sink, true, 'g' as u32),
            Err(RfbKeyboardError::Input(InputError::RolloverLimitExceeded))
        );
        assert_eq!(
            mapper.handle_key(&mut sink, false, 'g' as u32),
            Ok(RfbKeyboardOutcome::UnknownRelease)
        );

        for character in 'a'..='f' {
            assert_eq!(
                mapper.handle_key(&mut sink, false, character as u32),
                Ok(RfbKeyboardOutcome::Applied)
            );
        }

        let batches = queue.accepted_batches();
        assert_eq!(batches.len(), 12);
        assert_eq!(batches.last().unwrap().frames()[0].data(), &[0; 8]);
    }
}
