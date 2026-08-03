//! iced 桌面端（M1）：视频链路（scale/frames/video/status/app）。
//!
//! 迁移设计文档：docs/superpowers/specs/2026-08-03-iced-migration-design.md。
//! M1 已收编视频链路；菜单/输入在 M2/M3 继续从 spike crate
//! （ipkvm-desktop-iced-spike）收编。
//!
//! 可测试性要求（每阶段强制）：lib/bin 拆分，UI 逻辑可 headless 测试
//! （iced_test Simulator），窗口元数据走常量/函数并可断言。

use iced::Size;
use rust_i18n::t;

pub mod app;
pub mod clipboard;
pub mod connect;
pub mod diag;
pub mod dialog;
pub mod fonts;
pub mod frames;
pub mod input;
pub mod keymap;
pub mod locale;
pub mod menu;
pub mod modal;
pub mod perf;
pub mod platform;
pub mod preloaded;
pub mod profile;
pub mod relative;
pub mod scale;
pub mod status;
pub mod theme;
pub mod video;
pub mod video_area;

/// 窗口标题（M5 将嵌入 GIT_COMMIT）。
pub const WINDOW_TITLE: &str = "my_ipkvm iced (M0)";
/// 默认窗口尺寸。
pub const WINDOW_SIZE: Size = Size::new(1280.0, 800.0);

pub use app::run;
pub use app::{App, MockApp};
pub use perf::FrameStats;

rust_i18n::i18n!("locales", fallback = "en");

/// i18n 全局 locale 是进程级状态，涉及它的测试串行执行。
pub static I18N_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 运行时翻译（集成测试无法直接调 t! 宏，收口成函数）。
pub fn translate_key(key: &str) -> String {
    match key {
        "menu.file" => t!("menu.file").to_string(),
        "menu.edit" => t!("menu.edit").to_string(),
        "menu.send" => t!("menu.send").to_string(),
        "menu.about" => t!("menu.about").to_string(),
        "menu.disconnect" => t!("menu.disconnect").to_string(),
        "file.load_profile" => t!("file.load_profile").to_string(),
        "file.recent" => t!("file.recent").to_string(),
        "file.recent_more" => t!("file.recent_more").to_string(),
        "file.exit" => t!("file.exit").to_string(),
        "edit.copy_screenshot" => t!("edit.copy_screenshot").to_string(),
        "edit.save_screenshot" => t!("edit.save_screenshot").to_string(),
        "edit.save_screenshot_unsupported" => t!("edit.save_screenshot_unsupported").to_string(),
        "edit.language" => t!("edit.language").to_string(),
        "edit.settings" => t!("edit.settings").to_string(),
        "send.paste_text" => t!("send.paste_text").to_string(),
        "send.release_all" => t!("send.release_all").to_string(),
        "send.special_keys" => t!("send.special_keys").to_string(),
        "special_keys.ctrl_alt_del" => t!("special_keys.ctrl_alt_del").to_string(),
        "special_keys.win" => t!("special_keys.win").to_string(),
        "special_keys.print_screen" => t!("special_keys.print_screen").to_string(),
        "special_keys.alt_tab" => t!("special_keys.alt_tab").to_string(),
        "language.system" => t!("language.system").to_string(),
        "language.chinese" => t!("language.chinese").to_string(),
        "language.english" => t!("language.english").to_string(),
        "about.project_home" => t!("about.project_home").to_string(),
        "about.title" => t!("about.title").to_string(),
        "about.version" => t!("about.version", commit = "x").to_string(),
        "about.license" => t!("about.license").to_string(),
        "about.project_url" => t!("about.project_url", url = "x").to_string(),
        "dialog.jpeg_filter" => t!("dialog.jpeg_filter").to_string(),
        "modal.settings_title" => t!("modal.settings_title").to_string(),
        "modal.connection_title" => t!("modal.connection_title").to_string(),
        "modal.close" => t!("modal.close").to_string(),
        "modal.about_title" => t!("modal.about_title").to_string(),
        "modal.save_title" => t!("modal.save_title").to_string(),
        "modal.name_label" => t!("modal.name_label").to_string(),
        "modal.save" => t!("modal.save").to_string(),
        "device.title" => t!("device.title").to_string(),
        "device.video" => t!("device.video").to_string(),
        "device.control" => t!("device.control").to_string(),
        "device.refresh" => t!("device.refresh").to_string(),
        "device.connect" => t!("device.connect").to_string(),
        "device.preview" => t!("device.preview").to_string(),
        "device.no_preview" => t!("device.no_preview").to_string(),
        "preview.no_signal" => t!("preview.no_signal").to_string(),
        "preview.open_failed" => t!("preview.open_failed").to_string(),
        "profile.save" => t!("profile.save").to_string(),
        "profile.saved" => t!("profile.saved", name = "x").to_string(),
        "profile.save_failed" => t!("profile.save_failed", error = "x").to_string(),
        "profile.load_failed" => t!("profile.load_failed", error = "x").to_string(),
        "profile.device_missing" => t!("profile.device_missing").to_string(),
        "profile.control_missing" => t!("profile.control_missing").to_string(),
        "profile.no_recent" => t!("profile.no_recent").to_string(),
        "profile.overwrite_body" => t!("profile.overwrite_body", name = "x").to_string(),
        "profile.overwrite_confirm" => t!("profile.overwrite_confirm").to_string(),
        "profile.restore_defaults" => t!("profile.restore_defaults").to_string(),
        "connection_settings.title" => t!("connection_settings.title").to_string(),
        "settings.title" => t!("settings.title").to_string(),
        "settings.baud_rate" => t!("settings.baud_rate").to_string(),
        "settings.auto_baud" => t!("settings.auto_baud").to_string(),
        "settings.preview_fps" => t!("settings.preview_fps").to_string(),
        "settings.mouse_mode" => t!("settings.mouse_mode").to_string(),
        "settings.relative_sensitivity" => t!("settings.relative_sensitivity").to_string(),
        "settings.scale_mode" => t!("settings.scale_mode").to_string(),
        "mouse_mode.absolute" => t!("mouse_mode.absolute").to_string(),
        "mouse_mode.relative" => t!("mouse_mode.relative").to_string(),
        "scale_mode.fit_window" => t!("scale_mode.fit_window").to_string(),
        "scale_mode.actual_size" => t!("scale_mode.actual_size").to_string(),
        "scale_mode.resize_to_video" => t!("scale_mode.resize_to_video").to_string(),
        "status.control_device" => t!("status.control_device", value = "x").to_string(),
        "status.keyboard" => t!("status.keyboard", value = "x").to_string(),
        "status.pointer" => t!("status.pointer", value = "x").to_string(),
        "status.video" => t!("status.video", value = "x").to_string(),
        "status.message" => t!("status.message", message = "x").to_string(),
        "status.offline" => t!("status.offline").to_string(),
        "status.pasting" => t!("status.pasting").to_string(),
        "status.remote_input" => t!("status.remote_input").to_string(),
        "status.ready" => t!("status.ready").to_string(),
        "status.relative_mode" => t!("status.relative_mode").to_string(),
        "status.video_no_signal" => t!("status.video_no_signal").to_string(),
        "status.video_stalled" => t!("status.video_stalled").to_string(),
        "control_status.not_selected" => t!("control_status.not_selected").to_string(),
        "control_status.checking" => t!("control_status.checking").to_string(),
        "control_status.ready" => t!("control_status.ready", port = "x").to_string(),
        "control_status.not_ch9329" => t!("control_status.not_ch9329", reason = "x").to_string(),
        "control_status.no_response" => t!("control_status.no_response").to_string(),
        "control_status.open_failed" => t!("control_status.open_failed", error = "x").to_string(),
        "control_status.offline" => t!("control_status.offline").to_string(),
        "video_status.not_selected" => t!("video_status.not_selected").to_string(),
        "video_status.checking" => t!("video_status.checking").to_string(),
        "video_status.ready" => {
            t!("video_status.ready", width = 1, height = 1, label = "x").to_string()
        }
        "video_status.no_signal" => t!("video_status.no_signal").to_string(),
        "video_status.open_failed" => t!("video_status.open_failed", error = "x").to_string(),
        "video_status.disconnected" => t!("video_status.disconnected").to_string(),
        "control_status_label.not_selected" => t!("control_status_label.not_selected").to_string(),
        "control_status_label.checking" => t!("control_status_label.checking").to_string(),
        "control_status_label.ready" => t!("control_status_label.ready", port = "x").to_string(),
        "control_status_label.not_ch9329" => {
            t!("control_status_label.not_ch9329", reason = "x").to_string()
        }
        "control_status_label.no_response" => t!("control_status_label.no_response").to_string(),
        "control_status_label.open_failed" => {
            t!("control_status_label.open_failed", error = "x").to_string()
        }
        "control_status_label.disconnected" => t!("control_status_label.disconnected").to_string(),
        "message.enumeration_failed" => t!("message.enumeration_failed", error = "x").to_string(),
        "message.connect_failed" => t!("message.connect_failed", error = "x").to_string(),
        "message.baud_selected" => t!("message.baud_selected", baud = 9600).to_string(),
        "message.offline_with_reason" => {
            t!("message.offline_with_reason", reason = "x").to_string()
        }
        "message.offline_reconnect" => t!("message.offline_reconnect").to_string(),
        "message.input_rejected" => t!("message.input_rejected").to_string(),
        "message.unsupported_key" => t!("message.unsupported_key").to_string(),
        "message.clipboard_empty" => t!("message.clipboard_empty").to_string(),
        "message.clipboard_read_failed" => {
            t!("message.clipboard_read_failed", error = "x").to_string()
        }
        "message.no_frame_screenshot" => t!("message.no_frame_screenshot").to_string(),
        "message.screenshot_copied" => t!("message.screenshot_copied").to_string(),
        "message.screenshot_copy_failed" => {
            t!("message.screenshot_copy_failed", error = "x").to_string()
        }
        "message.screenshot_saved" => t!("message.screenshot_saved", path = "x").to_string(),
        "message.screenshot_save_failed" => {
            t!("message.screenshot_save_failed", error = "x").to_string()
        }
        "message.keyboard_send_failed" => {
            t!("message.keyboard_send_failed", error = "x").to_string()
        }
        "message.pointer_send_failed" => t!("message.pointer_send_failed", error = "x").to_string(),
        "common.cancel" => t!("common.cancel").to_string(),
        "common.not_selected" => t!("common.not_selected").to_string(),
        _ => key.to_string(),
    }
}

/// M2 全部 i18n key（labels 测试遍历）。
pub const I18N_KEYS: &[&str] = &[
    "menu.file",
    "menu.edit",
    "menu.send",
    "menu.about",
    "menu.disconnect",
    "file.load_profile",
    "file.recent",
    "file.recent_more",
    "file.exit",
    "edit.copy_screenshot",
    "edit.save_screenshot",
    "edit.save_screenshot_unsupported",
    "edit.language",
    "edit.settings",
    "send.paste_text",
    "send.release_all",
    "send.special_keys",
    "special_keys.ctrl_alt_del",
    "special_keys.win",
    "special_keys.print_screen",
    "special_keys.alt_tab",
    "language.system",
    "language.chinese",
    "language.english",
    "about.title",
    "about.version",
    "about.license",
    "about.project_url",
    "dialog.jpeg_filter",
    "modal.settings_title",
    "modal.connection_title",
    "modal.close",
    "modal.about_title",
    "modal.save_title",
    "modal.name_label",
    "modal.save",
    "device.title",
    "device.video",
    "device.control",
    "device.refresh",
    "device.connect",
    "device.preview",
    "device.no_preview",
    "preview.no_signal",
    "preview.open_failed",
    "profile.save",
    "profile.saved",
    "profile.save_failed",
    "profile.load_failed",
    "profile.device_missing",
    "profile.control_missing",
    "profile.no_recent",
    "profile.overwrite_body",
    "profile.overwrite_confirm",
    "profile.restore_defaults",
    "connection_settings.title",
    "settings.title",
    "settings.baud_rate",
    "settings.auto_baud",
    "settings.preview_fps",
    "settings.mouse_mode",
    "settings.relative_sensitivity",
    "settings.scale_mode",
    "mouse_mode.absolute",
    "mouse_mode.relative",
    "scale_mode.fit_window",
    "scale_mode.actual_size",
    "scale_mode.resize_to_video",
    "status.control_device",
    "status.keyboard",
    "status.pointer",
    "status.video",
    "status.message",
    "status.offline",
    "status.pasting",
    "status.remote_input",
    "status.ready",
    "status.relative_mode",
    "status.video_no_signal",
    "status.video_stalled",
    "control_status.not_selected",
    "control_status.checking",
    "control_status.ready",
    "control_status.not_ch9329",
    "control_status.no_response",
    "control_status.open_failed",
    "control_status.offline",
    "video_status.not_selected",
    "video_status.checking",
    "video_status.ready",
    "video_status.no_signal",
    "video_status.open_failed",
    "video_status.disconnected",
    "control_status_label.not_selected",
    "control_status_label.checking",
    "control_status_label.ready",
    "control_status_label.not_ch9329",
    "control_status_label.no_response",
    "control_status_label.open_failed",
    "control_status_label.disconnected",
    "message.enumeration_failed",
    "message.connect_failed",
    "message.baud_selected",
    "message.offline_with_reason",
    "message.offline_reconnect",
    "message.input_rejected",
    "message.unsupported_key",
    "message.clipboard_empty",
    "message.clipboard_read_failed",
    "message.no_frame_screenshot",
    "message.screenshot_copied",
    "message.screenshot_copy_failed",
    "message.screenshot_saved",
    "message.screenshot_save_failed",
    "message.keyboard_send_failed",
    "message.pointer_send_failed",
    "common.cancel",
    "common.not_selected",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_title_and_size_are_stable() {
        // 窗口元数据走常量，M5 改标题时此处会强制更新。
        assert_eq!(WINDOW_TITLE, "my_ipkvm iced (M0)");
        assert_eq!(WINDOW_SIZE, Size::new(1280.0, 800.0));
    }

    #[test]
    fn labels_are_single_line_nonempty_and_not_keys() {
        let _guard = I18N_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        for locale in ["en", "zh-CN"] {
            rust_i18n::set_locale(locale);
            for key in I18N_KEYS {
                let label = translate_key(key);
                assert!(!label.contains('\n'), "[{locale}] {key} 译文不得含换行");
                assert!(
                    !label.is_empty() && label != *key,
                    "[{locale}] {key} 译文不得为空或等于 key"
                );
            }
        }
    }

    #[test]
    fn fixed_width_en_labels_stay_within_28_chars() {
        let _guard = I18N_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        rust_i18n::set_locale("en");
        // 菜单 240 / 按钮 140 / 下拉 240 固定宽度下不换行越界的英文关键标签。
        for key in [
            "file.load_profile",
            "send.release_all",
            "device.control",
            "device.refresh",
            "edit.save_screenshot",
            "settings.auto_baud",
            "connection_settings.title",
            "profile.save",
            "modal.connection_title",
        ] {
            let label = translate_key(key);
            assert!(
                label.chars().count() <= 28,
                "[en] {key} 英文文案 {label:?} 超过 28 字符，固定宽度控件会换行/越界"
            );
        }
    }

    #[test]
    fn gui_entry_suppresses_console_subsystem_on_windows() {
        let source = include_str!("main.rs");
        assert!(
            source.contains("#![cfg_attr(windows, windows_subsystem = \"windows\")]"),
            "main.rs 缺少 windows_subsystem 属性，Windows release 启动会带黑窗"
        );
    }
}
