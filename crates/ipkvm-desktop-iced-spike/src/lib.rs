//! iced 迁移可行性验证 spike（#73）。
//!
//! 本 crate 不替代 egui 桌面端，仅用于验证 iced 能否承载同样的会话/输入/渲染链路。
//! 复用 [`ipkvm_desktop::DesktopSessionController`]，仅把前端唤醒机制从 egui
//! 的 `request_repaint` 换成 iced 的 `Subscription`。

pub mod app;
pub mod frames;
pub mod keymap;
pub mod menu;
pub mod modal;
pub mod platform;
pub mod relative;
pub mod scale;

pub use app::{FrameStats, Message, RecordingSink, SpikeApp, handle_from_frame};

// i18n：编译期加载本 crate 的 locales/，fallback en。
// spike 用精简 locales（仅菜单/模态键），与 desktop 的 locales 独立。
rust_i18n::i18n!("locales", fallback = "en");
use rust_i18n::t;

/// i18n 全局 locale 是进程级状态，涉及它的测试串行执行，避免并行断言互踩。
/// （沿用 desktop crate 的 I18N_TEST_LOCK 模式。集成测试也用，故去掉 cfg(test)。）
pub static I18N_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 运行时翻译（供集成测试使用）。
///
/// rust-i18n 的 `t!` 宏只接受字面量 key，且展开后引用调用方 crate 根的
/// `_rust_i18n_t`；集成测试是独立 crate，无法直接调用。这里把 spike 用到的
/// 全部 key 收口成一个可运行时调用的函数。
pub fn translate_key(key: &str) -> String {
    match key {
        "menu.file" => t!("menu.file").to_string(),
        "menu.edit" => t!("menu.edit").to_string(),
        "menu.send" => t!("menu.send").to_string(),
        "menu.about" => t!("menu.about").to_string(),
        "menu.reselect_device" => t!("menu.reselect_device").to_string(),
        "menu.stop_connection" => t!("menu.stop_connection").to_string(),
        "file.load_profile" => t!("file.load_profile").to_string(),
        "file.recent" => t!("file.recent").to_string(),
        "file.recent_more" => t!("file.recent_more").to_string(),
        "file.exit" => t!("file.exit").to_string(),
        "edit.copy_screenshot" => t!("edit.copy_screenshot").to_string(),
        "edit.language" => t!("edit.language").to_string(),
        "edit.settings" => t!("edit.settings").to_string(),
        "send.paste_text" => t!("send.paste_text").to_string(),
        "send.release_all" => t!("send.release_all").to_string(),
        "send.special_keys" => t!("send.special_keys").to_string(),
        "special_keys.ctrl_alt_del" => t!("special_keys.ctrl_alt_del").to_string(),
        "language.system" => t!("language.system").to_string(),
        "language.chinese" => t!("language.chinese").to_string(),
        "language.english" => t!("language.english").to_string(),
        "modal.settings_title" => t!("modal.settings_title").to_string(),
        "modal.connection_title" => t!("modal.connection_title").to_string(),
        "modal.close" => t!("modal.close").to_string(),
        "modal.about_title" => t!("modal.about_title").to_string(),
        "modal.save_title" => t!("modal.save_title").to_string(),
        _ => key.to_string(),
    }
}
