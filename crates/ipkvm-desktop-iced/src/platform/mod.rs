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
        use ::windows::Win32::UI::Shell::ShellExecuteW;
        use ::windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        use ::windows::core::PCWSTR;

        let operation = ::windows::core::w!("open");
        let encoded_url = windows_url_argument(url);
        unsafe {
            let _ = ShellExecuteW(
                None,
                operation,
                PCWSTR(encoded_url.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
        }
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

#[cfg(target_os = "windows")]
fn windows_url_argument(url: &str) -> Vec<u16> {
    url.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 平台默认相对鼠标源工厂。
pub struct PlatformRelativeSourceFactory;

impl crate::relative::RelativeSourceFactory for PlatformRelativeSourceFactory {
    fn create(&self) -> Result<Box<dyn crate::relative::RelativePointerSource>, String> {
        create()
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_url_argument_is_null_terminated_for_shell_execute() {
        let url = "https://github.com/kxn/MyKvm?from=menu";
        let encoded = super::windows_url_argument(url);

        assert_eq!(encoded.last(), Some(&0));
        assert_eq!(
            String::from_utf16(&encoded[..encoded.len() - 1]).unwrap(),
            url
        );
    }
}
