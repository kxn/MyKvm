use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyEvent {
    Down { hid_usage: u8 },
    Up { hid_usage: u8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseMode {
    Absolute,
    Relative,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FramebufferSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerEvent {
    AbsoluteMove {
        x: u32,
        y: u32,
        framebuffer_size: FramebufferSize,
    },
    RelativeMove {
        dx: i16,
        dy: i16,
    },
    Button {
        button: PointerButton,
        down: bool,
    },
    /// 滚轮方向约定：正数表示向上滚动，负数表示向下滚动。
    Wheel {
        delta: i16,
    },
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum InputError {
    #[error("serial transport is disconnected")]
    SerialDisconnected,
    #[error("unsupported key: {0}")]
    UnsupportedKey(String),
    #[error("keyboard rollover limit exceeded")]
    RolloverLimitExceeded,
    #[error("text contains unsupported character: {0:?}")]
    UnsupportedText(char),
}

pub type InputResult<T> = Result<T, InputError>;

pub trait InputSink {
    fn set_mouse_mode(&mut self, mode: MouseMode) -> InputResult<()>;
    fn handle_key(&mut self, event: KeyEvent) -> InputResult<()>;
    fn handle_pointer(&mut self, event: PointerEvent) -> InputResult<()>;
    fn type_text(&mut self, text: &str) -> InputResult<()>;
    fn release_all(&mut self) -> InputResult<()>;
}
