use ipkvm_core::{InputResult, InputSink, KeyEvent, KeyboardUsage, MouseMode, PointerEvent};
use ipkvm_headless::rfb_input::{RfbKeyboardError, RfbKeyboardMapper, RfbKeyboardOutcome};

#[derive(Default)]
struct RecordingSink {
    batches: Vec<Vec<KeyEvent>>,
}

impl InputSink for RecordingSink {
    fn set_mouse_mode(&mut self, _mode: MouseMode) -> InputResult<()> {
        Ok(())
    }

    fn handle_key_batch(&mut self, events: &[KeyEvent]) -> InputResult<()> {
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

fn assert_direct_round_trip(keysym: u32, usage: u8) {
    let mut mapper = RfbKeyboardMapper::new();
    let mut sink = RecordingSink::default();

    assert_eq!(
        mapper.handle_key(&mut sink, true, keysym),
        Ok(RfbKeyboardOutcome::Applied)
    );
    assert_eq!(
        mapper.handle_key(&mut sink, false, keysym),
        Ok(RfbKeyboardOutcome::Applied)
    );
    assert_eq!(sink.batches, vec![vec![down(usage)], vec![up(usage)]]);
}

#[test]
fn public_mapper_handles_modifiers_function_keys_and_keypad() {
    for (keysym, usage) in [
        (0xffe1, 0xe1),
        (0xffe2, 0xe5),
        (0xffe3, 0xe0),
        (0xffe4, 0xe4),
        (0xffe7, 0xe3),
        (0xffe8, 0xe7),
        (0xffe9, 0xe2),
        (0xffea, 0xe6),
        (0xffeb, 0xe3),
        (0xffec, 0xe7),
        (0xff8d, 0x58),
        (0xffaa, 0x55),
        (0xffab, 0x57),
        (0xffac, 0x85),
        (0xffad, 0x56),
        (0xffae, 0x63),
        (0xffaf, 0x54),
        (0xffbd, 0x67),
    ] {
        assert_direct_round_trip(keysym, usage);
    }

    for keysym in 0xffbe..=0xffc9 {
        assert_direct_round_trip(keysym, 0x3a + (keysym - 0xffbe) as u8);
    }
    for keysym in 0xffca..=0xffd1 {
        assert_direct_round_trip(keysym, 0x68 + (keysym - 0xffca) as u8);
    }
    for keysym in 0xffb0..=0xffb9 {
        let digit = (keysym - 0xffb0) as u8;
        let usage = if digit == 0 { 0x62 } else { 0x58 + digit };
        assert_direct_round_trip(keysym, usage);
    }
}

#[test]
fn public_mapper_handles_navigation_and_system_keys() {
    for (keysym, usage) in [
        (0xff08, 0x2a),
        (0xff09, 0x2b),
        (0xff0d, 0x28),
        (0xff13, 0x48),
        (0xff14, 0x47),
        (0xff15, 0x46),
        (0xff1b, 0x29),
        (0xff50, 0x4a),
        (0xff51, 0x50),
        (0xff52, 0x52),
        (0xff53, 0x4f),
        (0xff54, 0x51),
        (0xff55, 0x4b),
        (0xff56, 0x4e),
        (0xff57, 0x4d),
        (0xff61, 0x46),
        (0xff63, 0x49),
        (0xff67, 0x65),
        (0xffff, 0x4c),
    ] {
        assert_direct_round_trip(keysym, usage);
    }
}

#[test]
fn keypad_navigation_uses_physical_keypad_usages() {
    for (keysym, usage) in [
        (0xff80, 0x2c),
        (0xff89, 0x2b),
        (0xff91, 0x3a),
        (0xff92, 0x3b),
        (0xff93, 0x3c),
        (0xff94, 0x3d),
        (0xff95, 0x5f),
        (0xff96, 0x5c),
        (0xff97, 0x60),
        (0xff98, 0x5e),
        (0xff99, 0x5a),
        (0xff9a, 0x61),
        (0xff9b, 0x5b),
        (0xff9c, 0x59),
        (0xff9d, 0x5d),
        (0xff9e, 0x62),
        (0xff9f, 0x63),
    ] {
        assert_direct_round_trip(keysym, usage);
    }

    assert_direct_round_trip('0' as u32, 0x27);
    assert_direct_round_trip(0xffb0, 0x62);
}

#[test]
fn iso_left_tab_synthesizes_shift_and_tab_atomically() {
    let mut mapper = RfbKeyboardMapper::new();
    let mut sink = RecordingSink::default();

    mapper.handle_key(&mut sink, true, 0xfe20).unwrap();
    mapper.handle_key(&mut sink, false, 0xfe20).unwrap();

    assert_eq!(
        sink.batches,
        vec![vec![down(0xe1), down(0x2b)], vec![up(0x2b), up(0xe1)]]
    );
}

#[test]
fn lock_keysyms_do_not_reach_the_input_sink() {
    let mut mapper = RfbKeyboardMapper::new();
    let mut sink = RecordingSink::default();

    for keysym in [0xff7f, 0xffe5] {
        assert_eq!(
            mapper.handle_key(&mut sink, true, keysym),
            Ok(RfbKeyboardOutcome::IgnoredLock)
        );
        assert_eq!(
            mapper.handle_key(&mut sink, false, keysym),
            Ok(RfbKeyboardOutcome::UnknownRelease)
        );
    }
    assert!(sink.batches.is_empty());
}

#[test]
fn unsupported_keysyms_return_stable_errors_without_sink_calls() {
    let mut mapper = RfbKeyboardMapper::new();
    let mut sink = RecordingSink::default();

    for keysym in [0x00e9, 0x0101_f642, 0xdead_beef] {
        assert_eq!(
            mapper.handle_key(&mut sink, true, keysym),
            Err(RfbKeyboardError::UnsupportedKeysym(keysym))
        );
    }
    assert!(sink.batches.is_empty());
}
