use ipkvm_core::{FramebufferSize, InputSink, PointerButton, PointerEvent};

use super::{RfbPointerError, RfbPointerOutcome};

const PERSISTENT_BUTTONS: [(u8, PointerButton); 3] = [
    (1 << 0, PointerButton::Left),
    (1 << 1, PointerButton::Middle),
    (1 << 2, PointerButton::Right),
];
const WHEEL_UP_MASK: u8 = 1 << 3;
const WHEEL_DOWN_MASK: u8 = 1 << 4;
const UNSUPPORTED_BUTTON_MASK: u8 = 0b1110_0000;

#[derive(Debug, Default)]
pub struct RfbPointerMapper {
    committed_button_mask: u8,
}

impl RfbPointerMapper {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle_pointer(
        &mut self,
        sink: &mut impl InputSink,
        button_mask: u8,
        x: u16,
        y: u16,
        framebuffer_size: FramebufferSize,
    ) -> Result<RfbPointerOutcome, RfbPointerError> {
        let mut events = vec![PointerEvent::AbsoluteMove {
            x: u32::from(x),
            y: u32::from(y),
            framebuffer_size,
        }];
        events.extend(button_events(self.committed_button_mask, button_mask));
        let pressed_edges = button_mask & !self.committed_button_mask;
        if pressed_edges & WHEEL_UP_MASK != 0 {
            events.push(PointerEvent::Wheel { delta: 1 });
        }
        if pressed_edges & WHEEL_DOWN_MASK != 0 {
            events.push(PointerEvent::Wheel { delta: -1 });
        }

        sink.handle_pointer_batch(&events)?;
        self.committed_button_mask = button_mask;
        let ignored = button_mask & UNSUPPORTED_BUTTON_MASK;
        if ignored == 0 {
            Ok(RfbPointerOutcome::Applied)
        } else {
            Ok(RfbPointerOutcome::AppliedIgnoringButtons {
                button_mask: ignored,
            })
        }
    }

    pub fn handle_relative_pointer(
        &mut self,
        sink: &mut impl InputSink,
        button_mask: u8,
        dx: i16,
        dy: i16,
        wheel: i8,
    ) -> Result<RfbPointerOutcome, RfbPointerError> {
        let mut events = Vec::new();
        events.extend(button_events(self.committed_button_mask, button_mask));
        if dx != 0 || dy != 0 {
            events.push(PointerEvent::RelativeMove { dx, dy });
        }
        if wheel != 0 {
            events.push(PointerEvent::Wheel {
                delta: i16::from(wheel),
            });
        }

        sink.handle_pointer_batch(&events)?;
        self.committed_button_mask = button_mask;
        let ignored = button_mask & UNSUPPORTED_BUTTON_MASK;
        if ignored == 0 {
            Ok(RfbPointerOutcome::Applied)
        } else {
            Ok(RfbPointerOutcome::AppliedIgnoringButtons {
                button_mask: ignored,
            })
        }
    }
}

fn button_events(committed: u8, new_mask: u8) -> Vec<PointerEvent> {
    let mut events = Vec::new();
    for (mask, button) in PERSISTENT_BUTTONS {
        if committed & mask != 0 && new_mask & mask == 0 {
            events.push(PointerEvent::Button {
                button,
                down: false,
            });
        }
    }
    for (mask, button) in PERSISTENT_BUTTONS {
        if committed & mask == 0 && new_mask & mask != 0 {
            events.push(PointerEvent::Button { button, down: true });
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use ipkvm_core::{
        FramebufferSize, InputError, InputResult, InputSink, KeyEvent, MouseMode, PointerButton,
        PointerEvent,
    };

    use super::*;
    use crate::rfb_input::{RfbPointerError, RfbPointerOutcome};

    #[derive(Default)]
    struct RecordingSink {
        batches: Vec<Vec<PointerEvent>>,
        fail_next: bool,
    }

    impl InputSink for RecordingSink {
        fn set_mouse_mode(&mut self, _mode: MouseMode) -> InputResult<()> {
            Ok(())
        }

        fn handle_key_batch(&mut self, _events: &[KeyEvent]) -> InputResult<()> {
            Ok(())
        }

        fn handle_pointer_batch(&mut self, events: &[PointerEvent]) -> InputResult<()> {
            if std::mem::take(&mut self.fail_next) {
                return Err(InputError::PointerPositionUnknown);
            }
            self.batches.push(events.to_vec());
            Ok(())
        }

        fn release_all(&mut self) -> InputResult<()> {
            Ok(())
        }
    }

    fn size() -> FramebufferSize {
        FramebufferSize {
            width: 1920,
            height: 1080,
        }
    }

    fn absolute_move(x: u16, y: u16) -> PointerEvent {
        PointerEvent::AbsoluteMove {
            x: u32::from(x),
            y: u32::from(y),
            framebuffer_size: size(),
        }
    }

    fn button(button: PointerButton, down: bool) -> PointerEvent {
        PointerEvent::Button { button, down }
    }

    #[test]
    fn first_left_press_moves_before_pressing_button() {
        let mut mapper = RfbPointerMapper::new();
        let mut sink = RecordingSink::default();

        assert_eq!(
            mapper.handle_pointer(&mut sink, 0x01, 100, 200, size()),
            Ok(RfbPointerOutcome::Applied)
        );
        assert_eq!(
            sink.batches,
            vec![vec![
                absolute_move(100, 200),
                button(PointerButton::Left, true),
            ]]
        );
    }

    #[test]
    fn changing_left_to_right_releases_before_pressing() {
        let mut mapper = RfbPointerMapper::new();
        let mut sink = RecordingSink::default();
        mapper
            .handle_pointer(&mut sink, 0x01, 100, 200, size())
            .unwrap();

        mapper
            .handle_pointer(&mut sink, 0x04, 101, 201, size())
            .unwrap();

        assert_eq!(
            sink.batches[1],
            vec![
                absolute_move(101, 201),
                button(PointerButton::Left, false),
                button(PointerButton::Right, true),
            ]
        );
    }

    #[test]
    fn repeated_button_state_only_updates_position() {
        let mut mapper = RfbPointerMapper::new();
        let mut sink = RecordingSink::default();
        mapper
            .handle_pointer(&mut sink, 0x02, 100, 200, size())
            .unwrap();

        mapper
            .handle_pointer(&mut sink, 0x02, 300, 400, size())
            .unwrap();

        assert_eq!(sink.batches[1], vec![absolute_move(300, 400)]);
    }

    #[test]
    fn multiple_button_changes_follow_left_middle_right_order() {
        let mut mapper = RfbPointerMapper::new();
        let mut sink = RecordingSink::default();

        mapper
            .handle_pointer(&mut sink, 0x07, 100, 200, size())
            .unwrap();
        assert_eq!(
            sink.batches[0],
            vec![
                absolute_move(100, 200),
                button(PointerButton::Left, true),
                button(PointerButton::Middle, true),
                button(PointerButton::Right, true),
            ]
        );

        mapper
            .handle_pointer(&mut sink, 0x00, 100, 200, size())
            .unwrap();
        assert_eq!(
            sink.batches[1],
            vec![
                absolute_move(100, 200),
                button(PointerButton::Left, false),
                button(PointerButton::Middle, false),
                button(PointerButton::Right, false),
            ]
        );
    }

    #[test]
    fn wheel_up_is_emitted_once_on_the_rising_edge() {
        let mut mapper = RfbPointerMapper::new();
        let mut sink = RecordingSink::default();

        mapper
            .handle_pointer(&mut sink, 0x08, 100, 200, size())
            .unwrap();
        mapper
            .handle_pointer(&mut sink, 0x08, 101, 201, size())
            .unwrap();
        mapper
            .handle_pointer(&mut sink, 0x00, 102, 202, size())
            .unwrap();

        assert_eq!(
            sink.batches,
            vec![
                vec![absolute_move(100, 200), PointerEvent::Wheel { delta: 1 }],
                vec![absolute_move(101, 201)],
                vec![absolute_move(102, 202)],
            ]
        );
    }

    #[test]
    fn wheel_down_uses_negative_delta() {
        let mut mapper = RfbPointerMapper::new();
        let mut sink = RecordingSink::default();

        mapper
            .handle_pointer(&mut sink, 0x10, 100, 200, size())
            .unwrap();

        assert_eq!(
            sink.batches[0],
            vec![absolute_move(100, 200), PointerEvent::Wheel { delta: -1 }]
        );
    }

    #[test]
    fn simultaneous_wheel_edges_remain_two_discrete_steps() {
        let mut mapper = RfbPointerMapper::new();
        let mut sink = RecordingSink::default();

        mapper
            .handle_pointer(&mut sink, 0x18, 100, 200, size())
            .unwrap();

        assert_eq!(
            sink.batches[0],
            vec![
                absolute_move(100, 200),
                PointerEvent::Wheel { delta: 1 },
                PointerEvent::Wheel { delta: -1 },
            ]
        );
    }

    #[test]
    fn wheel_while_left_is_held_does_not_repeat_button_down() {
        let mut mapper = RfbPointerMapper::new();
        let mut sink = RecordingSink::default();
        mapper
            .handle_pointer(&mut sink, 0x01, 100, 200, size())
            .unwrap();

        mapper
            .handle_pointer(&mut sink, 0x09, 100, 200, size())
            .unwrap();

        assert_eq!(
            sink.batches[1],
            vec![absolute_move(100, 200), PointerEvent::Wheel { delta: 1 }]
        );
    }

    #[test]
    fn unsupported_buttons_are_ignored_but_reported() {
        let mut mapper = RfbPointerMapper::new();
        let mut sink = RecordingSink::default();

        assert_eq!(
            mapper.handle_pointer(&mut sink, 0xe0, 100, 200, size()),
            Ok(RfbPointerOutcome::AppliedIgnoringButtons { button_mask: 0xe0 })
        );
        assert_eq!(sink.batches, vec![vec![absolute_move(100, 200)]]);
    }

    #[test]
    fn rejected_sink_batch_can_retry_button_and_wheel_edges() {
        let mut mapper = RfbPointerMapper::new();
        let mut sink = RecordingSink {
            fail_next: true,
            ..RecordingSink::default()
        };

        assert_eq!(
            mapper.handle_pointer(&mut sink, 0x09, 100, 200, size()),
            Err(RfbPointerError::Input(InputError::PointerPositionUnknown))
        );
        assert!(sink.batches.is_empty());

        assert_eq!(
            mapper.handle_pointer(&mut sink, 0x09, 100, 200, size()),
            Ok(RfbPointerOutcome::Applied)
        );
        assert_eq!(
            sink.batches[0],
            vec![
                absolute_move(100, 200),
                button(PointerButton::Left, true),
                PointerEvent::Wheel { delta: 1 },
            ]
        );

        mapper
            .handle_pointer(&mut sink, 0x09, 101, 201, size())
            .unwrap();
        assert_eq!(sink.batches[1], vec![absolute_move(101, 201)]);
    }

    #[test]
    fn relative_move_emits_delta_and_preserves_buttons() {
        let mut mapper = RfbPointerMapper::new();
        let mut sink = RecordingSink::default();

        mapper
            .handle_relative_pointer(&mut sink, 0x01, 12, -4, 0)
            .unwrap();

        assert_eq!(
            sink.batches,
            vec![vec![
                button(PointerButton::Left, true),
                PointerEvent::RelativeMove { dx: 12, dy: -4 },
            ]]
        );
    }

    #[test]
    fn relative_wheel_emits_wheel_and_keeps_button_state() {
        let mut mapper = RfbPointerMapper::new();
        let mut sink = RecordingSink::default();
        mapper
            .handle_relative_pointer(&mut sink, 0x01, 0, 0, 0)
            .unwrap();

        mapper
            .handle_relative_pointer(&mut sink, 0x01, 0, 0, 3)
            .unwrap();

        assert_eq!(sink.batches[1], vec![PointerEvent::Wheel { delta: 3 }]);
    }

    #[test]
    fn relative_zero_delta_skips_move_but_still_sends_buttons() {
        let mut mapper = RfbPointerMapper::new();
        let mut sink = RecordingSink::default();

        mapper
            .handle_relative_pointer(&mut sink, 0x01, 0, 0, 0)
            .unwrap();

        assert_eq!(sink.batches[0], vec![button(PointerButton::Left, true)]);
    }
}
