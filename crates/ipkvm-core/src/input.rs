use thiserror::Error;

use crate::ch9329::{Ch9329FrameError, Ch9329ReportError};
use crate::serial::{CommandQueueError, QueueStats};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
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

/// 目标端鼠标兼容预设或显式原始模式。
///
/// Profile 身份与解析后的 `MouseMode` 分开保存：即使两个预设当前使用相同
/// 模式（例如 Windows 与 BIOS），后续仍可独立调整映射。
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MouseProfile {
    Windows,
    Linux,
    Bios,
    Android,
    MacOs,
    #[default]
    RawAbsolute,
    RawRelative,
}

impl MouseProfile {
    pub const ALL: [Self; 7] = [
        Self::Windows,
        Self::Linux,
        Self::Bios,
        Self::Android,
        Self::MacOs,
        Self::RawAbsolute,
        Self::RawRelative,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::Bios => "bios",
            Self::Android => "android",
            Self::MacOs => "macos",
            Self::RawAbsolute => "raw_absolute",
            Self::RawRelative => "raw_relative",
        }
    }

    pub fn parse(value: &str) -> Result<Self, MouseProfileParseError> {
        match value {
            "windows" => Ok(Self::Windows),
            "linux" => Ok(Self::Linux),
            "bios" => Ok(Self::Bios),
            "android" => Ok(Self::Android),
            "macos" => Ok(Self::MacOs),
            "raw_absolute" => Ok(Self::RawAbsolute),
            "raw_relative" => Ok(Self::RawRelative),
            other => Err(MouseProfileParseError::Unknown(other.to_owned())),
        }
    }

    pub const fn resolve_mode(self) -> MouseMode {
        match self {
            Self::Linux | Self::RawRelative => MouseMode::Relative,
            Self::Windows | Self::Bios | Self::Android | Self::MacOs | Self::RawAbsolute => {
                MouseMode::Absolute
            }
        }
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum MouseProfileParseError {
    #[error("unknown mouse profile: {0}")]
    Unknown(String),
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
    #[error("pointer event {event} does not match mouse mode {mode:?}")]
    PointerModeMismatch {
        mode: MouseMode,
        event: &'static str,
    },
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
    /// 返回 sink 创建时已经生效的鼠标模式；未知时返回 `None`，由上层在首个
    /// 指针事件到达时完成模式收敛。
    fn initial_mouse_mode(&self) -> Option<MouseMode> {
        None
    }

    /// 切换鼠标报告模式。
    ///
    /// 实现必须在旧模式下释放当前按住的鼠标按钮，再提交新模式；成功返回后，上层
    /// 可以认为 sink 的鼠标按钮状态已清零并重置 pointer mapper。失败时不得提交新
    /// 模式，也不得改变已提交的键盘或鼠标软件状态。
    fn set_mouse_mode(&mut self, mode: MouseMode) -> InputResult<()>;

    fn handle_key(&mut self, event: KeyEvent) -> InputResult<()> {
        self.handle_key_batch(std::slice::from_ref(&event))
    }

    fn handle_key_batch(&mut self, events: &[KeyEvent]) -> InputResult<()>;

    fn handle_pointer(&mut self, event: PointerEvent) -> InputResult<()> {
        self.handle_pointer_batch(std::slice::from_ref(&event))
    }

    fn handle_pointer_batch(&mut self, events: &[PointerEvent]) -> InputResult<()>;
    fn release_all(&mut self) -> InputResult<()>;

    fn queue_stats(&self) -> Option<QueueStats> {
        None
    }
}
