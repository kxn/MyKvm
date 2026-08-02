mod app;
mod clipboard;
mod fonts;
mod frame;
mod input;
mod locale;
mod probe;
mod render;
mod session;
mod state;

rust_i18n::i18n!("locales", fallback = "en");

use thiserror::Error;

/// i18n 全局 locale 是进程级状态，涉及它的测试串行执行，避免并行断言互踩。
#[cfg(test)]
pub(crate) static I18N_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Debug, Error)]
pub enum DesktopError {
    #[error("desktop gui failed: {0}")]
    Gui(String),
}

pub fn run() -> Result<(), DesktopError> {
    app::run().map_err(|error| DesktopError::Gui(error.to_string()))
}
