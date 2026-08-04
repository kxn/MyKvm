//! 平台差异模块（相对鼠标 raw input 等）。
//!
//! Windows：Raw Input（`windows`）；macOS/Linux：stub（迁移时补实现，不堵口子）。

pub mod cursor;
#[cfg(not(windows))]
pub mod stub;
#[cfg(windows)]
pub mod windows;

pub use cursor::{CursorController, ProductionCursorController};
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

/// 用系统默认浏览器打开 URL。
pub fn open_url(url: &str) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
}

/// 平台默认相对鼠标源工厂。
pub struct PlatformRelativeSourceFactory;

impl crate::relative::RelativeSourceFactory for PlatformRelativeSourceFactory {
    fn create(&self) -> Result<Box<dyn crate::relative::RelativePointerSource>, String> {
        create()
    }
}
