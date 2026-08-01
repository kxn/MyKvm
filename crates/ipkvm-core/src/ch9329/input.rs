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
    fn apply_keys(
        &self,
        events: &[KeyEvent],
    ) -> InputResult<Option<(KeyboardState, KeyboardReport)>> {
        let mut next = *self;
        for event in events {
            match *event {
                KeyEvent::Down { usage } => {
                    next.press(usage.get())?;
                }
                KeyEvent::Up { usage } => {
                    next.release(usage.get());
                }
            }
        }
        if next.has_same_pressed_state(self) {
            return Ok(None);
        }
        Ok(Some((next, next.report())))
    }

    /// 键盘报告语义是“修饰键位 + 普通键集合”，槽位排列顺序不参与比较。
    fn has_same_pressed_state(&self, other: &KeyboardState) -> bool {
        if self.modifiers != other.modifiers {
            return false;
        }
        let mut self_keys = self.keys;
        let mut other_keys = other.keys;
        self_keys.sort_unstable();
        other_keys.sort_unstable();
        self_keys == other_keys
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

    fn apply_events(
        &self,
        events: &[PointerEvent],
    ) -> InputResult<(MouseState, Vec<Ch9329Command>)> {
        let mut next = *self;
        let mut commands = Vec::new();
        for event in events {
            next.apply_event(*event, &mut commands)?;
        }
        Ok((next, commands))
    }

    fn apply_event(
        &mut self,
        event: PointerEvent,
        commands: &mut Vec<Ch9329Command>,
    ) -> InputResult<()> {
        match event {
            PointerEvent::AbsoluteMove {
                x,
                y,
                framebuffer_size,
            } => self.apply_absolute_move(x, y, framebuffer_size, commands),
            PointerEvent::RelativeMove { dx, dy } => self.apply_relative_move(dx, dy, commands),
            PointerEvent::Button { button, down } => self.apply_button(button, down, commands),
            PointerEvent::Wheel { delta } => self.apply_wheel(delta, commands),
        }
    }

    fn apply_absolute_move(
        &mut self,
        x: u32,
        y: u32,
        framebuffer_size: FramebufferSize,
        commands: &mut Vec<Ch9329Command>,
    ) -> InputResult<()> {
        self.require_mode(MouseMode::Absolute, "absolute move")?;
        let (x, y) = map_absolute_position(x, y, framebuffer_size)?;
        let report = AbsoluteMouseReport::new(self.buttons, x, y, 0)?;
        commands.push(Ch9329Command::MouseAbsolute(report));
        self.last_absolute = Some((x, y));
        Ok(())
    }

    fn apply_relative_move(
        &self,
        dx: i16,
        dy: i16,
        commands: &mut Vec<Ch9329Command>,
    ) -> InputResult<()> {
        self.require_mode(MouseMode::Relative, "relative move")?;
        commands.extend(relative_commands(self.buttons, split_relative(dx, dy, 0))?);
        Ok(())
    }

    fn apply_button(
        &mut self,
        button: PointerButton,
        down: bool,
        commands: &mut Vec<Ch9329Command>,
    ) -> InputResult<()> {
        let mask = button_mask(button);
        let already_down = self.buttons & mask != 0;
        if already_down == down {
            return Ok(());
        }

        if down {
            self.buttons |= mask;
        } else {
            self.buttons &= !mask;
        }
        let command = match self.mode {
            MouseMode::Absolute => {
                let (x, y) = self
                    .last_absolute
                    .ok_or(InputError::PointerPositionUnknown)?;
                Ch9329Command::MouseAbsolute(AbsoluteMouseReport::new(self.buttons, x, y, 0)?)
            }
            MouseMode::Relative => {
                Ch9329Command::MouseRelative(RelativeMouseReport::new(self.buttons, 0, 0, 0)?)
            }
        };
        commands.push(command);
        Ok(())
    }

    fn apply_wheel(&self, delta: i16, commands: &mut Vec<Ch9329Command>) -> InputResult<()> {
        let chunks = split_relative(0, 0, delta);
        if chunks.is_empty() {
            return Ok(());
        }

        match self.mode {
            MouseMode::Absolute => {
                let (x, y) = self
                    .last_absolute
                    .ok_or(InputError::PointerPositionUnknown)?;
                for (_, _, wheel) in chunks {
                    commands.push(Ch9329Command::MouseAbsolute(AbsoluteMouseReport::new(
                        self.buttons,
                        x,
                        y,
                        wheel,
                    )?));
                }
            }
            MouseMode::Relative => {
                commands.extend(relative_commands(self.buttons, chunks)?);
            }
        }
        Ok(())
    }

    fn require_mode(&self, expected: MouseMode, event: &'static str) -> InputResult<()> {
        if self.mode != expected {
            return Err(InputError::PointerModeMismatch {
                mode: self.mode,
                event,
            });
        }
        Ok(())
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
        self.handle_key_batch(std::slice::from_ref(&event))
    }

    pub fn handle_key_batch(&mut self, events: &[KeyEvent]) -> InputResult<()> {
        let Some((next, report)) = self.keyboard.apply_keys(events)? else {
            return Ok(());
        };
        self.enqueue_commands(vec![Ch9329Command::Keyboard(report)])?;
        self.keyboard = next;
        Ok(())
    }

    pub fn handle_pointer(&mut self, event: PointerEvent) -> InputResult<()> {
        self.handle_pointer_batch(std::slice::from_ref(&event))
    }

    pub fn handle_pointer_batch(&mut self, events: &[PointerEvent]) -> InputResult<()> {
        let (next, commands) = self.mouse.apply_events(events)?;
        if commands.is_empty() {
            return Ok(());
        }
        self.enqueue_commands(commands)?;
        self.mouse = next;
        Ok(())
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

    fn handle_key_batch(&mut self, events: &[KeyEvent]) -> InputResult<()> {
        Ch9329InputSink::handle_key_batch(self, events)
    }

    fn handle_pointer(&mut self, event: PointerEvent) -> InputResult<()> {
        Ch9329InputSink::handle_pointer(self, event)
    }

    fn handle_pointer_batch(&mut self, events: &[PointerEvent]) -> InputResult<()> {
        Ch9329InputSink::handle_pointer_batch(self, events)
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
    use std::collections::BTreeSet;

    use super::*;
    use crate::fake_serial::FakeCommandQueue;
    use crate::{
        Ch9329Frame, CommandQueueError, FramebufferSize, InputError, InputSink, KeyEvent,
        KeyboardUsage, MouseMode, PointerButton, PointerEvent,
    };
    use proptest::prelude::*;

    fn expected_frame(command: u8, data: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0x57, 0xab, 0x00, command, data.len() as u8];
        bytes.extend_from_slice(data);
        let checksum = bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        bytes.push(checksum);
        bytes
    }

    fn new_batches(queue: &FakeCommandQueue, before: usize) -> Vec<Vec<u8>> {
        queue.accepted_batches()[before..]
            .iter()
            .flat_map(|batch| batch.frames())
            .map(|frame| frame.as_bytes().to_vec())
            .collect()
    }

    fn test_modifier_mask(usage: u8) -> Option<u8> {
        (0xe0..=0xe7)
            .contains(&usage)
            .then(|| 1u8 << (usage - 0xe0))
    }

    /// 独立参考模型：只按协议语义维护“修饰键位 + 普通键集合”，
    /// 不依赖生产 `KeyboardState` 的槽位实现。
    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    struct KeyboardModel {
        modifiers: u8,
        regular: BTreeSet<u8>,
    }

    impl KeyboardModel {
        fn apply(&mut self, events: &[KeyEvent]) -> Result<bool, ()> {
            let mut next = self.clone();
            for event in events {
                match *event {
                    KeyEvent::Down { usage } => {
                        let value = usage.get();
                        if let Some(mask) = test_modifier_mask(value) {
                            next.modifiers |= mask;
                        } else if next.regular.len() >= 6 && !next.regular.contains(&value) {
                            return Err(());
                        } else {
                            next.regular.insert(value);
                        }
                    }
                    KeyEvent::Up { usage } => {
                        let value = usage.get();
                        if let Some(mask) = test_modifier_mask(value) {
                            next.modifiers &= !mask;
                        } else {
                            next.regular.remove(&value);
                        }
                    }
                }
            }
            let changed = next != *self;
            *self = next;
            Ok(changed)
        }
    }

    /// 键盘报告的 6 个普通键槽位是无序集合：按集合和修饰键比较，
    /// 不约束生产实现的槽位排列顺序。
    fn assert_keyboard_frame_matches_model(frame_bytes: &[u8], model: &KeyboardModel) -> bool {
        let Ok(frame) = Ch9329Frame::parse(frame_bytes) else {
            return false;
        };
        let data = frame.data();
        if data.len() != 8 || data[0] != model.modifiers || data[1] != 0 {
            return false;
        }
        let keys: BTreeSet<u8> = data[2..].iter().copied().filter(|key| *key != 0).collect();
        keys == model.regular
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum ExpectedPointerFrame {
        Absolute {
            buttons: u8,
            x: u16,
            y: u16,
            wheel: i8,
        },
        Relative {
            buttons: u8,
            dx: i8,
            dy: i8,
            wheel: i8,
        },
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct PointerModel {
        mode: MouseMode,
        buttons: u8,
        last_absolute: Option<(u16, u16)>,
    }

    impl PointerModel {
        fn new(mode: MouseMode) -> Self {
            Self {
                mode,
                buttons: 0,
                last_absolute: None,
            }
        }

        fn apply(&mut self, events: &[PointerEvent]) -> Result<Vec<ExpectedPointerFrame>, ()> {
            let mut next = *self;
            let mut frames = Vec::new();
            for event in events {
                match *event {
                    PointerEvent::AbsoluteMove {
                        x,
                        y,
                        framebuffer_size,
                    } => {
                        if next.mode != MouseMode::Absolute
                            || framebuffer_size.width == 0
                            || framebuffer_size.height == 0
                            || x >= framebuffer_size.width
                            || y >= framebuffer_size.height
                        {
                            return Err(());
                        }
                        let mapped_x =
                            (4096u64 * u64::from(x) / u64::from(framebuffer_size.width)) as u16;
                        let mapped_y =
                            (4096u64 * u64::from(y) / u64::from(framebuffer_size.height)) as u16;
                        frames.push(ExpectedPointerFrame::Absolute {
                            buttons: next.buttons,
                            x: mapped_x,
                            y: mapped_y,
                            wheel: 0,
                        });
                        next.last_absolute = Some((mapped_x, mapped_y));
                    }
                    PointerEvent::RelativeMove { dx, dy } => {
                        if next.mode != MouseMode::Relative {
                            return Err(());
                        }
                        for (part_dx, part_dy, _) in test_split_delta(dx, dy, 0) {
                            frames.push(ExpectedPointerFrame::Relative {
                                buttons: next.buttons,
                                dx: part_dx,
                                dy: part_dy,
                                wheel: 0,
                            });
                        }
                    }
                    PointerEvent::Button { button, down } => {
                        let mask = button_mask(button);
                        if (next.buttons & mask != 0) == down {
                            continue;
                        }
                        if down {
                            next.buttons |= mask;
                        } else {
                            next.buttons &= !mask;
                        }
                        match next.mode {
                            MouseMode::Absolute => {
                                let (x, y) = next.last_absolute.ok_or(())?;
                                frames.push(ExpectedPointerFrame::Absolute {
                                    buttons: next.buttons,
                                    x,
                                    y,
                                    wheel: 0,
                                });
                            }
                            MouseMode::Relative => {
                                frames.push(ExpectedPointerFrame::Relative {
                                    buttons: next.buttons,
                                    dx: 0,
                                    dy: 0,
                                    wheel: 0,
                                });
                            }
                        }
                    }
                    PointerEvent::Wheel { delta } => {
                        let chunks = test_split_delta(0, 0, delta);
                        if chunks.is_empty() {
                            continue;
                        }
                        match next.mode {
                            MouseMode::Absolute => {
                                let (x, y) = next.last_absolute.ok_or(())?;
                                for (_, _, wheel) in chunks {
                                    frames.push(ExpectedPointerFrame::Absolute {
                                        buttons: next.buttons,
                                        x,
                                        y,
                                        wheel,
                                    });
                                }
                            }
                            MouseMode::Relative => {
                                for (dx, dy, wheel) in chunks {
                                    frames.push(ExpectedPointerFrame::Relative {
                                        buttons: next.buttons,
                                        dx,
                                        dy,
                                        wheel,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            *self = next;
            Ok(frames)
        }
    }

    fn test_split_delta(mut dx: i16, mut dy: i16, mut wheel: i16) -> Vec<(i8, i8, i8)> {
        let mut chunks = Vec::new();
        while dx != 0 || dy != 0 || wheel != 0 {
            let part_dx = dx.clamp(-127, 127) as i8;
            let part_dy = dy.clamp(-127, 127) as i8;
            let part_wheel = wheel.clamp(-127, 127) as i8;
            chunks.push((part_dx, part_dy, part_wheel));
            dx -= i16::from(part_dx);
            dy -= i16::from(part_dy);
            wheel -= i16::from(part_wheel);
        }
        chunks
    }

    fn parse_pointer_frame(frame: &Ch9329Frame) -> ExpectedPointerFrame {
        match frame.command() {
            0x04 => ExpectedPointerFrame::Absolute {
                buttons: frame.data()[1],
                x: u16::from_le_bytes([frame.data()[2], frame.data()[3]]),
                y: u16::from_le_bytes([frame.data()[4], frame.data()[5]]),
                wheel: frame.data()[6] as i8,
            },
            0x05 => ExpectedPointerFrame::Relative {
                buttons: frame.data()[1],
                dx: frame.data()[2] as i8,
                dy: frame.data()[3] as i8,
                wheel: frame.data()[4] as i8,
            },
            other => panic!("unexpected pointer command: {other:#04x}"),
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum RawPointerEvent {
        Move(i32, i32),
        Button(u8, bool),
        Wheel(i16),
    }

    fn raw_pointer_event() -> impl Strategy<Value = RawPointerEvent> {
        prop_oneof![
            (-1000i32..=1000, -1000i32..=1000).prop_map(|(dx, dy)| RawPointerEvent::Move(dx, dy)),
            (0u8..=2, any::<bool>())
                .prop_map(|(button, down)| RawPointerEvent::Button(button, down)),
            (-64i16..=64).prop_map(RawPointerEvent::Wheel),
        ]
    }

    fn pointer_button(index: u8) -> PointerButton {
        match index {
            0 => PointerButton::Left,
            1 => PointerButton::Middle,
            _ => PointerButton::Right,
        }
    }

    fn decode_pointer_event(
        event: RawPointerEvent,
        absolute: bool,
        width: u32,
        height: u32,
    ) -> PointerEvent {
        match event {
            RawPointerEvent::Move(dx, dy) if absolute => PointerEvent::AbsoluteMove {
                x: dx.rem_euclid(i32::try_from(width).unwrap()) as u32,
                y: dy.rem_euclid(i32::try_from(height).unwrap()) as u32,
                framebuffer_size: FramebufferSize { width, height },
            },
            RawPointerEvent::Move(dx, dy) => PointerEvent::RelativeMove {
                dx: dx as i16,
                dy: dy as i16,
            },
            RawPointerEvent::Button(index, down) => PointerEvent::Button {
                button: pointer_button(index),
                down,
            },
            RawPointerEvent::Wheel(delta) => PointerEvent::Wheel { delta },
        }
    }

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
    fn key_batch_enqueues_one_final_report() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let shift = KeyboardUsage::new(0xe1).unwrap();
        let a = KeyboardUsage::new(0x04).unwrap();

        sink.handle_key_batch(&[KeyEvent::Down { usage: shift }, KeyEvent::Down { usage: a }])
            .unwrap();

        let batches = queue.accepted_batches();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].frames().len(), 1);
        assert_eq!(
            batches[0].frames()[0].data(),
            &[0x02, 0, 0x04, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn failed_key_batch_does_not_commit_partial_state() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let keys = (0x04..=0x0a)
            .map(|usage| KeyEvent::Down {
                usage: KeyboardUsage::new(usage).unwrap(),
            })
            .collect::<Vec<_>>();

        assert_eq!(
            sink.handle_key_batch(&keys),
            Err(InputError::RolloverLimitExceeded)
        );
        assert!(queue.accepted_batches().is_empty());

        sink.handle_key(KeyEvent::Down {
            usage: KeyboardUsage::new(0x0a).unwrap(),
        })
        .unwrap();
        assert_eq!(queue.accepted_batches().len(), 1);
    }

    #[test]
    fn empty_or_net_unchanged_key_batch_does_not_enqueue() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let a = KeyboardUsage::new(0x04).unwrap();

        sink.handle_key_batch(&[]).unwrap();
        sink.handle_key_batch(&[KeyEvent::Down { usage: a }, KeyEvent::Up { usage: a }])
            .unwrap();

        assert!(queue.accepted_batches().is_empty());
    }

    #[test]
    fn batch_that_only_reorders_slots_does_not_enqueue() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let a = KeyboardUsage::new(0x04).unwrap();
        let other = KeyboardUsage::new(0x1b).unwrap();

        sink.handle_key(KeyEvent::Down { usage: other }).unwrap();
        sink.handle_key(KeyEvent::Down { usage: a }).unwrap();
        let before = queue.accepted_batches().len();
        assert_eq!(before, 2);

        // 释放后在同一批内重按：按键集合不变，只是槽位顺序变化。
        sink.handle_key_batch(&[
            KeyEvent::Up { usage: other },
            KeyEvent::Down { usage: other },
        ])
        .unwrap();

        assert_eq!(queue.accepted_batches().len(), before);
        let keys: BTreeSet<u8> = sink
            .keyboard
            .keys
            .iter()
            .copied()
            .filter(|key| *key != 0)
            .collect();
        assert_eq!(keys, BTreeSet::from([0x04, 0x1b]));
    }

    #[test]
    fn rejected_key_batch_can_be_retried_without_state_drift() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let events = [
            KeyEvent::Down {
                usage: KeyboardUsage::new(0xe1).unwrap(),
            },
            KeyEvent::Down {
                usage: KeyboardUsage::new(0x04).unwrap(),
            },
        ];
        queue.fail_next(CommandQueueError::Closed);

        assert_eq!(
            sink.handle_key_batch(&events),
            Err(InputError::CommandQueue(CommandQueueError::Closed))
        );
        sink.handle_key_batch(&events).unwrap();

        let batches = queue.accepted_batches();
        assert_eq!(batches.len(), 1);
        assert_eq!(
            batches[0].frames()[0].data(),
            &[0x02, 0, 0x04, 0, 0, 0, 0, 0]
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
    fn pointer_batch_enqueues_all_reports_atomically() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let size = FramebufferSize {
            width: 100,
            height: 100,
        };

        sink.handle_pointer_batch(&[
            PointerEvent::AbsoluteMove {
                x: 10,
                y: 20,
                framebuffer_size: size,
            },
            PointerEvent::Button {
                button: PointerButton::Left,
                down: true,
            },
            PointerEvent::Wheel { delta: 1 },
        ])
        .unwrap();

        let batches = queue.accepted_batches();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].frames().len(), 3);
        assert_eq!(batches[0].frames()[0].data()[1], 0);
        assert_eq!(batches[0].frames()[1].data()[1], 1);
        assert_eq!(batches[0].frames()[2].data()[1], 1);
        assert_eq!(batches[0].frames()[2].data()[6], 1);
    }

    #[test]
    fn pointer_batch_rejects_later_invalid_event_without_partial_state() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let size = FramebufferSize {
            width: 100,
            height: 100,
        };

        assert_eq!(
            sink.handle_pointer_batch(&[
                PointerEvent::AbsoluteMove {
                    x: 10,
                    y: 20,
                    framebuffer_size: size,
                },
                PointerEvent::AbsoluteMove {
                    x: 100,
                    y: 20,
                    framebuffer_size: size,
                },
            ]),
            Err(InputError::PointerOutOfBounds {
                coordinate: 100,
                extent: 100,
            })
        );
        assert!(queue.accepted_batches().is_empty());
        assert_eq!(
            sink.handle_pointer(PointerEvent::Button {
                button: PointerButton::Left,
                down: true,
            }),
            Err(InputError::PointerPositionUnknown)
        );
    }

    #[test]
    fn rejected_pointer_batch_can_be_retried_without_state_drift() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let events = [
            PointerEvent::AbsoluteMove {
                x: 10,
                y: 20,
                framebuffer_size: FramebufferSize {
                    width: 100,
                    height: 100,
                },
            },
            PointerEvent::Button {
                button: PointerButton::Left,
                down: true,
            },
        ];
        queue.fail_next(CommandQueueError::Closed);

        assert_eq!(
            sink.handle_pointer_batch(&events),
            Err(InputError::CommandQueue(CommandQueueError::Closed))
        );
        sink.handle_pointer_batch(&events).unwrap();

        let batches = queue.accepted_batches();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].frames().len(), 2);
        assert_eq!(batches[0].frames()[0].data()[1], 0);
        assert_eq!(batches[0].frames()[1].data()[1], 1);
    }

    #[test]
    fn pointer_batch_preserves_transient_click_and_skips_noops() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Relative);

        sink.handle_pointer_batch(&[]).unwrap();
        sink.handle_pointer_batch(&[
            PointerEvent::Button {
                button: PointerButton::Left,
                down: false,
            },
            PointerEvent::RelativeMove { dx: 0, dy: 0 },
            PointerEvent::Wheel { delta: 0 },
        ])
        .unwrap();
        assert!(queue.accepted_batches().is_empty());

        sink.handle_pointer_batch(&[
            PointerEvent::Button {
                button: PointerButton::Left,
                down: true,
            },
            PointerEvent::Button {
                button: PointerButton::Left,
                down: false,
            },
        ])
        .unwrap();

        let batches = queue.accepted_batches();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].frames().len(), 2);
        assert_eq!(batches[0].frames()[0].data()[1], 1);
        assert_eq!(batches[0].frames()[1].data()[1], 0);
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
                match state.apply_keys(std::slice::from_ref(&event)) {
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

    #[test]
    fn mixed_input_transcript_matches_protocol_golden_frames() {
        let queue = FakeCommandQueue::new();
        let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
        let size = FramebufferSize {
            width: 1280,
            height: 720,
        };
        let a = KeyboardUsage::new(0x04).unwrap();
        let shift = KeyboardUsage::new(0xe1).unwrap();

        sink.handle_key(KeyEvent::Down { usage: a }).unwrap();
        sink.handle_key(KeyEvent::Down { usage: shift }).unwrap();
        sink.handle_key(KeyEvent::Up { usage: a }).unwrap();
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
        sink.handle_pointer(PointerEvent::Button {
            button: PointerButton::Left,
            down: false,
        })
        .unwrap();
        sink.release_all().unwrap();

        let frames: Vec<Vec<u8>> = queue
            .accepted_batches()
            .iter()
            .flat_map(|batch| batch.frames())
            .map(|frame| frame.as_bytes().to_vec())
            .collect();
        let expected = vec![
            expected_frame(0x02, &[0x00, 0x00, 0x04, 0, 0, 0, 0, 0]),
            expected_frame(0x02, &[0x02, 0x00, 0x04, 0, 0, 0, 0, 0]),
            expected_frame(0x02, &[0x02, 0x00, 0x00, 0, 0, 0, 0, 0]),
            expected_frame(0x04, &[0x02, 0x00, 0x40, 0x01, 0x38, 0x02, 0x00]),
            expected_frame(0x04, &[0x02, 0x01, 0x40, 0x01, 0x38, 0x02, 0x00]),
            expected_frame(0x04, &[0x02, 0x00, 0x40, 0x01, 0x38, 0x02, 0x00]),
            expected_frame(0x02, &[0x00, 0x00, 0x00, 0, 0, 0, 0, 0]),
            expected_frame(0x04, &[0x02, 0x00, 0x40, 0x01, 0x38, 0x02, 0x00]),
        ];

        assert_eq!(frames, expected);
        assert_eq!(queue.stats().batches_accepted, 7);
        assert_eq!(queue.stats().frames_accepted, 8);
    }

    proptest! {
        #[test]
        fn keyboard_reports_match_independent_reference_model(
            batches in proptest::collection::vec(
                proptest::collection::vec((0x04u8..=0x20, any::<bool>()), 0..8),
                0..32,
            ),
        ) {
            let queue = FakeCommandQueue::new();
            let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
            let mut model = KeyboardModel::default();

            for raw_events in batches {
                let events: Vec<KeyEvent> = raw_events
                    .into_iter()
                    .map(|(value, down)| {
                        let usage = KeyboardUsage::new(value).unwrap();
                        if down {
                            KeyEvent::Down { usage }
                        } else {
                            KeyEvent::Up { usage }
                        }
                    })
                    .collect();
                let mut preview = model.clone();
                let preview_result = preview.apply(&events);
                let before = queue.accepted_batches().len();
                let sink_result = sink.handle_key_batch(&events);

                match preview_result {
                    Ok(changed) => {
                        prop_assert!(sink_result.is_ok());
                        if changed {
                            let frames = new_batches(&queue, before);
                            prop_assert_eq!(frames.len(), 1);
                            prop_assert!(assert_keyboard_frame_matches_model(
                                &frames[0],
                                &preview
                            ));
                        } else {
                            prop_assert_eq!(queue.accepted_batches().len(), before);
                        }
                        model = preview;
                    }
                    Err(()) => {
                        prop_assert!(sink_result.is_err());
                        prop_assert_eq!(queue.accepted_batches().len(), before);
                    }
                }

                prop_assert_eq!(sink.keyboard.modifiers, model.modifiers);
                let occupied: BTreeSet<u8> = sink
                    .keyboard
                    .keys
                    .iter()
                    .copied()
                    .filter(|key| *key != 0)
                    .collect();
                prop_assert!(occupied == model.regular);
            }
        }

        #[test]
        fn keyboard_failure_rolls_back_to_reference_model(
            batches in proptest::collection::vec(
                (
                    proptest::collection::vec((0x04u8..=0x20, any::<bool>()), 0..8),
                    any::<bool>(),
                ),
                0..24,
            ),
        ) {
            let queue = FakeCommandQueue::new();
            let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
            let mut model = KeyboardModel::default();

            for (raw_events, fail_batch) in batches {
                let events: Vec<KeyEvent> = raw_events
                    .into_iter()
                    .map(|(value, down)| {
                        let usage = KeyboardUsage::new(value).unwrap();
                        if down {
                            KeyEvent::Down { usage }
                        } else {
                            KeyEvent::Up { usage }
                        }
                    })
                    .collect();
                let mut preview = model.clone();
                let preview_result = preview.apply(&events);
                let changed = preview_result.unwrap_or(false);
                let before = queue.accepted_batches().len();

                if fail_batch && changed {
                    queue.fail_next(CommandQueueError::Closed);
                }
                let sink_result = sink.handle_key_batch(&events);

                if fail_batch && changed {
                    prop_assert!(sink_result.is_err());
                    prop_assert_eq!(queue.accepted_batches().len(), before);
                } else {
                    match preview_result {
                        Ok(true) => {
                            prop_assert!(sink_result.is_ok());
                            let frames = new_batches(&queue, before);
                            prop_assert_eq!(frames.len(), 1);
                            prop_assert!(assert_keyboard_frame_matches_model(
                                &frames[0],
                                &preview
                            ));
                            model = preview;
                        }
                        Ok(false) => {
                            prop_assert!(sink_result.is_ok());
                            prop_assert_eq!(queue.accepted_batches().len(), before);
                        }
                        Err(()) => {
                            prop_assert!(sink_result.is_err());
                            prop_assert_eq!(queue.accepted_batches().len(), before);
                        }
                    }
                }

                prop_assert_eq!(sink.keyboard.modifiers, model.modifiers);
                let occupied: BTreeSet<u8> = sink
                    .keyboard
                    .keys
                    .iter()
                    .copied()
                    .filter(|key| *key != 0)
                    .collect();
                prop_assert!(occupied == model.regular);
            }
        }

        #[test]
        fn pointer_reports_match_independent_reference_model(
            absolute in any::<bool>(),
            width in 1u32..=64,
            height in 1u32..=64,
            events in proptest::collection::vec(raw_pointer_event(), 0..32),
        ) {
            let queue = FakeCommandQueue::new();
            let mode = if absolute {
                MouseMode::Absolute
            } else {
                MouseMode::Relative
            };
            let mut sink = Ch9329InputSink::new(queue.clone(), 0, mode);
            let mut model = PointerModel::new(mode);

            for event in events {
                let pointer_event = decode_pointer_event(event, absolute, width, height);
                let mut preview = model;
                let preview_frames = preview.apply(std::slice::from_ref(&pointer_event));
                let before = queue.accepted_batches().len();
                let sink_result = sink.handle_pointer(pointer_event);

                match preview_frames {
                    Ok(expected_frames) => {
                        prop_assert!(sink_result.is_ok());
                        let frames = new_batches(&queue, before);
                        prop_assert_eq!(frames.len(), expected_frames.len());
                        for (actual, expected) in frames.iter().zip(expected_frames.iter()) {
                            let parsed = parse_pointer_frame(&Ch9329Frame::parse(actual).unwrap());
                            prop_assert_eq!(&parsed, expected);
                        }
                        model = preview;
                    }
                    Err(()) => {
                        prop_assert!(sink_result.is_err());
                        prop_assert_eq!(queue.accepted_batches().len(), before);
                    }
                }

                prop_assert_eq!(sink.mouse.buttons, model.buttons);
                prop_assert_eq!(sink.mouse.last_absolute, model.last_absolute);
            }
        }

        #[test]
        fn pointer_failure_rolls_back_to_reference_model(
            absolute in any::<bool>(),
            width in 1u32..=64,
            height in 1u32..=64,
            events in proptest::collection::vec(
                (raw_pointer_event(), any::<bool>()),
                0..24,
            ),
        ) {
            let queue = FakeCommandQueue::new();
            let mode = if absolute {
                MouseMode::Absolute
            } else {
                MouseMode::Relative
            };
            let mut sink = Ch9329InputSink::new(queue.clone(), 0, mode);
            let mut model = PointerModel::new(mode);

            for (event, fail_batch) in events {
                let pointer_event = decode_pointer_event(event, absolute, width, height);
                let mut preview = model;
                let preview_frames = preview.apply(std::slice::from_ref(&pointer_event));
                let should_enqueue = preview_frames
                    .as_ref()
                    .is_ok_and(|frames| !frames.is_empty());
                let before = queue.accepted_batches().len();

                if fail_batch && should_enqueue {
                    queue.fail_next(CommandQueueError::Closed);
                }
                let sink_result = sink.handle_pointer(pointer_event);

                if fail_batch && should_enqueue {
                    prop_assert!(sink_result.is_err());
                    prop_assert_eq!(queue.accepted_batches().len(), before);
                } else {
                    match preview_frames {
                        Ok(expected_frames) => {
                            prop_assert!(sink_result.is_ok());
                            let frames = new_batches(&queue, before);
                            prop_assert_eq!(frames.len(), expected_frames.len());
                            for (actual, expected) in frames.iter().zip(expected_frames.iter()) {
                                let parsed =
                                    parse_pointer_frame(&Ch9329Frame::parse(actual).unwrap());
                                prop_assert_eq!(&parsed, expected);
                            }
                            model = preview;
                        }
                        Err(()) => {
                            prop_assert!(sink_result.is_err());
                            prop_assert_eq!(queue.accepted_batches().len(), before);
                        }
                    }
                }

                prop_assert_eq!(sink.mouse.buttons, model.buttons);
                prop_assert_eq!(sink.mouse.last_absolute, model.last_absolute);
            }
        }
    }
}
