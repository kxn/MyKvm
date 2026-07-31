use crate::geometry::map_framebuffer_axis;
use crate::input::{
    FramebufferSize, InputError, InputResult, InputSink, KeyEvent, MouseMode, PointerButton,
    PointerEvent,
};
use crate::serial::{CommandBatch, CommandQueue};

use super::{AbsoluteMouseReport, Ch9329Command, KeyboardReport, RelativeMouseReport};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MouseState {
    mode: MouseMode,
    buttons: u8,
    last_absolute: Option<(u16, u16)>,
}

impl MouseState {
    fn new(mode: MouseMode) -> Self {
        Self {
            mode,
            buttons: 0,
            last_absolute: None,
        }
    }

    fn release_command(&self) -> InputResult<Ch9329Command> {
        match (self.mode, self.last_absolute) {
            (MouseMode::Absolute, Some((x, y))) => Ok(Ch9329Command::MouseAbsolute(
                AbsoluteMouseReport::new(0, x, y, 0)?,
            )),
            _ => Ok(Ch9329Command::MouseRelative(RelativeMouseReport::new(
                0, 0, 0, 0,
            )?)),
        }
    }
}

#[derive(Debug)]
pub struct Ch9329InputSink<Q> {
    queue: Q,
    address: u8,
    keyboard: KeyboardState,
    mouse: MouseState,
}

impl<Q: CommandQueue> Ch9329InputSink<Q> {
    pub fn new(queue: Q, address: u8, mouse_mode: MouseMode) -> Self {
        Self {
            queue,
            address,
            keyboard: KeyboardState::default(),
            mouse: MouseState::new(mouse_mode),
        }
    }

    pub fn set_mouse_mode(&mut self, mode: MouseMode) -> InputResult<()> {
        if self.mouse.mode == mode {
            return Ok(());
        }

        let mut next = self.mouse;
        next.mode = mode;
        if self.mouse.buttons != 0 {
            self.enqueue_commands(vec![self.mouse.release_command()?])?;
            next.buttons = 0;
        }
        self.mouse = next;
        Ok(())
    }

    pub fn handle_key(&mut self, event: KeyEvent) -> InputResult<()> {
        let Some((next, report)) = self.keyboard.apply_key(event)? else {
            return Ok(());
        };
        self.enqueue_commands(vec![Ch9329Command::Keyboard(report)])?;
        self.keyboard = next;
        Ok(())
    }

    pub fn handle_pointer(&mut self, event: PointerEvent) -> InputResult<()> {
        match event {
            PointerEvent::AbsoluteMove {
                x,
                y,
                framebuffer_size,
            } => self.handle_absolute_move(x, y, framebuffer_size),
            PointerEvent::RelativeMove { dx, dy } => self.handle_relative_move(dx, dy),
            PointerEvent::Button { button, down } => self.handle_button(button, down),
            PointerEvent::Wheel { delta } => self.handle_wheel(delta),
        }
    }

    pub fn release_all(&mut self) -> InputResult<()> {
        let commands = vec![
            Ch9329Command::Keyboard(KeyboardState::default().report()),
            self.mouse.release_command()?,
        ];
        self.enqueue_commands(commands)?;
        self.keyboard = KeyboardState::default();
        self.mouse.buttons = 0;
        Ok(())
    }

    fn handle_absolute_move(
        &mut self,
        x: u32,
        y: u32,
        framebuffer_size: FramebufferSize,
    ) -> InputResult<()> {
        self.require_mode(MouseMode::Absolute, "absolute move")?;
        let (x, y) = map_absolute_position(x, y, framebuffer_size)?;
        let report = AbsoluteMouseReport::new(self.mouse.buttons, x, y, 0)?;
        self.enqueue_commands(vec![Ch9329Command::MouseAbsolute(report)])?;
        self.mouse.last_absolute = Some((x, y));
        Ok(())
    }

    fn handle_relative_move(&mut self, dx: i16, dy: i16) -> InputResult<()> {
        self.require_mode(MouseMode::Relative, "relative move")?;
        let chunks = split_relative(dx, dy, 0);
        if chunks.is_empty() {
            return Ok(());
        }
        let commands = relative_commands(self.mouse.buttons, chunks)?;
        self.enqueue_commands(commands)
    }

    fn handle_button(&mut self, button: PointerButton, down: bool) -> InputResult<()> {
        let mask = button_mask(button);
        let already_down = self.mouse.buttons & mask != 0;
        if already_down == down {
            return Ok(());
        }

        let mut next = self.mouse;
        if down {
            next.buttons |= mask;
        } else {
            next.buttons &= !mask;
        }

        let command = match self.mouse.mode {
            MouseMode::Absolute => {
                let (x, y) = self
                    .mouse
                    .last_absolute
                    .ok_or(InputError::PointerPositionUnknown)?;
                Ch9329Command::MouseAbsolute(AbsoluteMouseReport::new(next.buttons, x, y, 0)?)
            }
            MouseMode::Relative => {
                Ch9329Command::MouseRelative(RelativeMouseReport::new(next.buttons, 0, 0, 0)?)
            }
        };
        self.enqueue_commands(vec![command])?;
        self.mouse = next;
        Ok(())
    }

    fn handle_wheel(&mut self, delta: i16) -> InputResult<()> {
        if delta == 0 {
            return Ok(());
        }

        let chunks = split_relative(0, 0, delta);
        let commands = match self.mouse.mode {
            MouseMode::Absolute => {
                let (x, y) = self
                    .mouse
                    .last_absolute
                    .ok_or(InputError::PointerPositionUnknown)?;
                chunks
                    .into_iter()
                    .map(|(_, _, wheel)| {
                        AbsoluteMouseReport::new(self.mouse.buttons, x, y, wheel)
                            .map(Ch9329Command::MouseAbsolute)
                    })
                    .collect::<Result<Vec<_>, _>>()?
            }
            MouseMode::Relative => relative_commands(self.mouse.buttons, chunks)?,
        };
        self.enqueue_commands(commands)
    }

    fn require_mode(&self, expected: MouseMode, event: &'static str) -> InputResult<()> {
        if self.mouse.mode != expected {
            return Err(InputError::PointerModeMismatch {
                mode: self.mouse.mode,
                event,
            });
        }
        Ok(())
    }

    fn enqueue_commands(&self, commands: Vec<Ch9329Command>) -> InputResult<()> {
        let frames = commands
            .iter()
            .map(|command| command.to_frame(self.address))
            .collect::<Result<Vec<_>, _>>()?;
        let batch = CommandBatch::new(frames)
            .expect("input events that enqueue commands always produce a non-empty batch");
        self.queue.enqueue_batch(batch)?;
        Ok(())
    }
}

impl<Q: CommandQueue> InputSink for Ch9329InputSink<Q> {
    fn set_mouse_mode(&mut self, mode: MouseMode) -> InputResult<()> {
        Ch9329InputSink::set_mouse_mode(self, mode)
    }

    fn handle_key(&mut self, event: KeyEvent) -> InputResult<()> {
        Ch9329InputSink::handle_key(self, event)
    }

    fn handle_pointer(&mut self, event: PointerEvent) -> InputResult<()> {
        Ch9329InputSink::handle_pointer(self, event)
    }

    fn release_all(&mut self) -> InputResult<()> {
        Ch9329InputSink::release_all(self)
    }
}

fn map_absolute_position(
    x: u32,
    y: u32,
    framebuffer_size: FramebufferSize,
) -> InputResult<(u16, u16)> {
    if framebuffer_size.width == 0 || framebuffer_size.height == 0 {
        return Err(InputError::InvalidFramebufferSize {
            width: framebuffer_size.width,
            height: framebuffer_size.height,
        });
    }
    Ok((
        map_framebuffer_axis(x, framebuffer_size.width)?,
        map_framebuffer_axis(y, framebuffer_size.height)?,
    ))
}

fn button_mask(button: PointerButton) -> u8 {
    match button {
        PointerButton::Left => 0x01,
        PointerButton::Right => 0x02,
        PointerButton::Middle => 0x04,
    }
}

fn relative_commands(buttons: u8, chunks: Vec<(i8, i8, i8)>) -> InputResult<Vec<Ch9329Command>> {
    chunks
        .into_iter()
        .map(|(dx, dy, wheel)| {
            RelativeMouseReport::new(buttons, dx, dy, wheel)
                .map(Ch9329Command::MouseRelative)
                .map_err(InputError::from)
        })
        .collect()
}

fn split_relative(mut dx: i16, mut dy: i16, mut wheel: i16) -> Vec<(i8, i8, i8)> {
    let mut chunks = Vec::new();
    while dx != 0 || dy != 0 || wheel != 0 {
        let x = dx.clamp(-127, 127) as i8;
        let y = dy.clamp(-127, 127) as i8;
        let wheel_part = wheel.clamp(-127, 127) as i8;
        chunks.push((x, y, wheel_part));
        dx -= i16::from(x);
        dy -= i16::from(y);
        wheel -= i16::from(wheel_part);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_serial::FakeCommandQueue;
    use crate::{
        CommandQueueError, FramebufferSize, InputError, InputSink, KeyEvent, KeyboardUsage,
        MouseMode, PointerButton, PointerEvent,
    };
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

    #[test]
    fn ch9329_sink_implements_input_sink() {
        fn assert_input_sink<T: InputSink>() {}
        assert_input_sink::<Ch9329InputSink<FakeCommandQueue>>();
    }

    #[test]
    fn pointer_event_must_match_configured_mode() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue, 0, MouseMode::Absolute);
        assert_eq!(
            sink.handle_pointer(PointerEvent::RelativeMove { dx: 1, dy: 0 }),
            Err(InputError::PointerModeMismatch {
                mode: MouseMode::Absolute,
                event: "relative move",
            })
        );
    }

    #[test]
    fn absolute_button_requires_known_position() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        assert_eq!(
            sink.handle_pointer(PointerEvent::Button {
                button: PointerButton::Left,
                down: true,
            }),
            Err(InputError::PointerPositionUnknown)
        );
        assert!(queue.accepted_batches().is_empty());
    }

    #[test]
    fn absolute_move_carries_held_buttons() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let size = FramebufferSize {
            width: 1280,
            height: 720,
        };
        sink.handle_pointer(PointerEvent::AbsoluteMove {
            x: 100,
            y: 100,
            framebuffer_size: size,
        })
        .unwrap();
        sink.handle_pointer(PointerEvent::Button {
            button: PointerButton::Left,
            down: true,
        })
        .unwrap();
        sink.handle_pointer(PointerEvent::AbsoluteMove {
            x: 200,
            y: 100,
            framebuffer_size: size,
        })
        .unwrap();
        assert_eq!(
            queue.accepted_batches().last().unwrap().frames()[0].data()[1],
            0x01
        );
    }

    #[test]
    fn invalid_framebuffer_preserves_full_dimensions_in_error() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue, 0, MouseMode::Absolute);
        assert_eq!(
            sink.handle_pointer(PointerEvent::AbsoluteMove {
                x: 0,
                y: 0,
                framebuffer_size: FramebufferSize {
                    width: 1280,
                    height: 0,
                },
            }),
            Err(InputError::InvalidFramebufferSize {
                width: 1280,
                height: 0,
            })
        );
    }

    #[test]
    fn releasing_one_button_preserves_other_buttons() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Relative);
        for button in [PointerButton::Left, PointerButton::Right] {
            sink.handle_pointer(PointerEvent::Button { button, down: true })
                .unwrap();
        }
        sink.handle_pointer(PointerEvent::Button {
            button: PointerButton::Left,
            down: false,
        })
        .unwrap();
        assert_eq!(
            queue.accepted_batches().last().unwrap().frames()[0].data()[1],
            0x02
        );
    }

    #[test]
    fn duplicate_button_and_zero_motion_do_not_enqueue_batches() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Relative);
        let event = PointerEvent::Button {
            button: PointerButton::Middle,
            down: true,
        };
        sink.handle_pointer(event).unwrap();
        sink.handle_pointer(event).unwrap();
        sink.handle_pointer(PointerEvent::RelativeMove { dx: 0, dy: 0 })
            .unwrap();
        sink.handle_pointer(PointerEvent::Wheel { delta: 0 })
            .unwrap();
        assert_eq!(queue.accepted_batches().len(), 1);
    }

    #[test]
    fn relative_motion_is_split_without_losing_distance() {
        assert_eq!(
            split_relative(200, -200, 0),
            vec![(127, -127, 0), (73, -73, 0)]
        );
    }

    #[test]
    fn relative_move_is_one_atomic_multi_frame_batch() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Relative);
        sink.handle_pointer(PointerEvent::RelativeMove { dx: 200, dy: -200 })
            .unwrap();
        let batches = queue.accepted_batches();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].frames().len(), 2);
        assert_eq!(batches[0].frames()[0].data(), &[1, 0, 127, 129, 0]);
        assert_eq!(batches[0].frames()[1].data(), &[1, 0, 73, 183, 0]);
    }

    #[test]
    fn absolute_wheel_is_split_at_last_position() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        sink.handle_pointer(PointerEvent::AbsoluteMove {
            x: 100,
            y: 100,
            framebuffer_size: FramebufferSize {
                width: 1280,
                height: 720,
            },
        })
        .unwrap();
        sink.handle_pointer(PointerEvent::Wheel { delta: 200 })
            .unwrap();
        let batches = queue.accepted_batches();
        let wheel_frames = batches.last().unwrap().frames();
        assert_eq!(wheel_frames.len(), 2);
        assert_eq!(wheel_frames[0].data()[6], 127);
        assert_eq!(wheel_frames[1].data()[6], 73);
        assert_eq!(&wheel_frames[0].data()[2..6], &wheel_frames[1].data()[2..6]);
    }

    #[test]
    fn mode_switch_releases_buttons_in_old_mode() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Relative);
        sink.handle_pointer(PointerEvent::Button {
            button: PointerButton::Left,
            down: true,
        })
        .unwrap();
        sink.set_mouse_mode(MouseMode::Absolute).unwrap();
        let batches = queue.accepted_batches();
        let release = &batches.last().unwrap().frames()[0];
        assert_eq!(release.command(), 0x05);
        assert_eq!(release.data(), &[1, 0, 0, 0, 0]);
    }

    #[test]
    fn failed_button_batch_does_not_commit_button_state() {
        let queue = FakeCommandQueue::new();
        queue.fail_next(CommandQueueError::Closed);
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Relative);
        let event = PointerEvent::Button {
            button: PointerButton::Left,
            down: true,
        };
        assert!(sink.handle_pointer(event).is_err());
        sink.handle_pointer(event).unwrap();
        assert_eq!(
            queue.accepted_batches().last().unwrap().frames()[0].data()[1],
            0x01
        );
    }

    #[test]
    fn failed_mode_switch_keeps_old_mode_and_buttons() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Relative);
        sink.handle_pointer(PointerEvent::Button {
            button: PointerButton::Left,
            down: true,
        })
        .unwrap();
        queue.fail_next(CommandQueueError::Closed);
        assert!(sink.set_mouse_mode(MouseMode::Absolute).is_err());
        sink.handle_pointer(PointerEvent::RelativeMove { dx: 1, dy: 0 })
            .unwrap();
        let batches = queue.accepted_batches();
        let frame = &batches.last().unwrap().frames()[0];
        assert_eq!(frame.command(), 0x05);
        assert_eq!(frame.data()[1], 0x01);
    }

    #[test]
    fn release_all_always_contains_keyboard_and_mouse_release() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        sink.release_all().unwrap();
        let batches = queue.accepted_batches();
        let frames = batches.last().unwrap().frames();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].command(), 0x02);
        assert_eq!(frames[0].data(), &[0; 8]);
        assert_eq!(frames[1].command(), 0x05);
        assert_eq!(frames[1].data(), &[1, 0, 0, 0, 0]);
    }

    #[test]
    fn release_all_uses_last_absolute_position_when_available() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        sink.handle_pointer(PointerEvent::AbsoluteMove {
            x: 100,
            y: 100,
            framebuffer_size: FramebufferSize {
                width: 1280,
                height: 720,
            },
        })
        .unwrap();
        sink.release_all().unwrap();
        let batches = queue.accepted_batches();
        let release = &batches.last().unwrap().frames()[1];
        assert_eq!(release.command(), 0x04);
        assert_eq!(release.data()[1], 0);
        assert_eq!(&release.data()[2..6], &[0x40, 0x01, 0x38, 0x02]);
    }

    #[test]
    fn failed_release_all_preserves_keyboard_and_mouse_state() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Relative);
        let key = KeyboardUsage::new(4).unwrap();
        sink.handle_key(KeyEvent::Down { usage: key }).unwrap();
        let button_down = PointerEvent::Button {
            button: PointerButton::Left,
            down: true,
        };
        sink.handle_pointer(button_down).unwrap();

        queue.fail_next(CommandQueueError::Closed);
        assert!(sink.release_all().is_err());
        sink.handle_key(KeyEvent::Up { usage: key }).unwrap();
        sink.handle_pointer(button_down).unwrap();

        let batches = queue.accepted_batches();
        assert_eq!(batches.len(), 3);
        assert_eq!(batches.last().unwrap().frames()[0].command(), 0x02);
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

        #[test]
        fn relative_chunks_preserve_totals(
            dx in any::<i16>(),
            dy in any::<i16>(),
            wheel in any::<i16>()
        ) {
            let chunks = split_relative(dx, dy, wheel);
            prop_assert!(
                chunks
                    .iter()
                    .all(|(x, y, w)| *x != -128 && *y != -128 && *w != -128)
            );
            let totals = chunks.iter().fold((0i32, 0i32, 0i32), |sum, part| {
                (
                    sum.0 + i32::from(part.0),
                    sum.1 + i32::from(part.1),
                    sum.2 + i32::from(part.2),
                )
            });
            prop_assert_eq!(
                totals,
                (i32::from(dx), i32::from(dy), i32::from(wheel))
            );
        }
    }
}
