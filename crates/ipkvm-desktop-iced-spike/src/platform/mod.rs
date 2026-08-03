//! 平台差异模块（相对鼠标 raw input 等）。
//!
//! Windows：Raw Input（`windows`）；macOS/Linux：stub（迁移时补实现，不堵口子）。

#[cfg(not(windows))]
pub mod stub;
#[cfg(windows)]
pub mod windows;

#[cfg(not(windows))]
pub use stub::StubRawInput;
#[cfg(windows)]
pub use windows::WindowsRawInput;

use crate::relative::RelativePointerSource;

/// 平台默认相对鼠标源工厂。
#[cfg(windows)]
pub fn create() -> Result<Box<dyn RelativePointerSource>, String> {
    Ok(Box::new(WindowsRawInput::new()?))
}

#[cfg(not(windows))]
pub fn create() -> Result<Box<dyn RelativePointerSource>, String> {
    Ok(Box::new(StubRawInput::new()))
}
