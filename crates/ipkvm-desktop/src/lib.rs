mod app;
mod clipboard;
pub mod config;
mod fonts;
mod frame;
mod input;
mod locale;
mod menus;
pub mod probe;
mod render;
mod session;
pub mod state;

rust_i18n::i18n!("locales", fallback = "en");

// 桌面会话控制器（及其依赖类型）对非 egui 前端复用：iced spike 等通过它驱动
// 同一套 SessionManager/输入泵，仅替换前端重绘唤醒机制（见 subscribe_frames）。
pub use session::{
    ConnectRequest, DesktopSessionController, DesktopSessionError, ProductionDesktopSessionController,
    ProductionSessionFactory, SessionParts,
};

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
