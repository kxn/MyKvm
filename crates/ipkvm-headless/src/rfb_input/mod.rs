mod keyboard;
mod keymap;

use ipkvm_core::InputError;
use thiserror::Error;

pub use keyboard::RfbKeyboardMapper;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RfbKeyboardOutcome {
    Applied,
    DuplicateDown,
    UnknownRelease,
    IgnoredLock,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbKeyboardError {
    #[error("unsupported RFB keysym: {0:#010x}")]
    UnsupportedKeysym(u32),
    #[error("active RFB characters require conflicting Shift states")]
    ConflictingShiftRequirements,
    #[error("input sink rejected RFB keyboard state")]
    Input(#[from] InputError),
}
