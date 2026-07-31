#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyEvent {
    Down { hid_usage: u8 },
    Up { hid_usage: u8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerEvent {
    Move { x: u16, y: u16 },
    Button { button: PointerButton, down: bool },
    Wheel { delta: i16 },
}

pub trait InputSink {
    fn key_down(&mut self, event: KeyEvent);
    fn key_up(&mut self, event: KeyEvent);
    fn pointer_move(&mut self, event: PointerEvent);
    fn pointer_button(&mut self, event: PointerEvent);
    fn wheel(&mut self, event: PointerEvent);
    fn release_all(&mut self);
}
