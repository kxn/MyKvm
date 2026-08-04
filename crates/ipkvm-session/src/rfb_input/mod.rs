mod keyboard;
mod keymap;
mod pointer;
mod pump;
mod text;

use ipkvm_core::InputError;
use thiserror::Error;

pub use keyboard::RfbKeyboardMapper;
pub use pointer::RfbPointerMapper;
pub use pump::{
    RfbControllerReleaseReason, RfbInputError, RfbInputEventError, RfbInputEventKind,
    RfbInputLifecycleError, RfbInputNotice, RfbInputOperation, RfbInputPump, RfbInputRunError,
    RfbKeyboardRejection,
};
pub use text::{TextInputConfig, TextInputNotice, TextInputService};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RfbPointerOutcome {
    Applied,
    AppliedIgnoringButtons { button_mask: u8 },
    IgnoredForMouseMode { mode: ipkvm_core::MouseMode },
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbPointerError {
    #[error("input sink rejected RFB pointer state")]
    Input(#[from] InputError),
}
