use thiserror::Error;

use crate::ch9329::{Ch9329FrameError, Ch9329ReportError};
use crate::serial::CommandQueueError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyboardUsage(u8);

impl KeyboardUsage {
    pub fn new(value: u8) -> InputResult<Self> {
        if value <= 0x03 {
            return Err(InputError::InvalidKeyUsage(value));
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyEvent {
    Down { usage: KeyboardUsage },
    Up { usage: KeyboardUsage },
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
    #[error("invalid keyboard HID usage: {0:#04x}")]
    InvalidKeyUsage(u8),
    #[error("keyboard rollover limit exceeded")]
    RolloverLimitExceeded,
    #[error("invalid framebuffer size: {width}x{height}")]
    InvalidFramebufferSize { width: u32, height: u32 },
    #[error("pointer coordinate {coordinate} is outside extent {extent}")]
    PointerOutOfBounds { coordinate: u32, extent: u32 },
    #[error("absolute pointer position is not known")]
    PointerPositionUnknown,
    #[error("command queue rejected a batch")]
    CommandQueue(#[from] CommandQueueError),
    #[error("failed to build a CH9329 frame")]
    Frame(#[from] Ch9329FrameError),
    #[error("failed to build a CH9329 input report")]
    Report(#[from] Ch9329ReportError),
}

pub type InputResult<T> = Result<T, InputError>;

pub trait InputSink {
    fn set_mouse_mode(&mut self, mode: MouseMode) -> InputResult<()>;
    fn handle_key(&mut self, event: KeyEvent) -> InputResult<()>;
    fn handle_pointer(&mut self, event: PointerEvent) -> InputResult<()>;
    fn release_all(&mut self) -> InputResult<()>;
}
