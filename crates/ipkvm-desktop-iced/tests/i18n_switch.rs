//! i18n 切换验证（iced_aw）：zh↔en 菜单文案切换、显示译文而非 key 原文。
//!
//! iced_aw 菜单状态在 widget 树内，语言切换 = 重建 view；每个用例在设置
//! locale 后重新构建 Simulator。
mod common;

use common::MenuHarness;
use iced_test::Simulator;
use ipkvm_desktop_iced::menu::MenuAction;

fn hover_item(ui: &mut Simulator<'static, MenuAction>, label: &str) {
    let item = ui
        .find(label)
        .unwrap_or_else(|_| panic!("hover 目标 {label} 必须可定位"));
    MenuHarness::hover(ui, MenuHarness::center(item.bounds()));
}

#[test]
fn english_menu_shows_translated_labels_not_keys() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut ui = MenuHarness::ui();
    assert!(ui.click("File").is_ok(), "点击 File 必须成功");
    assert!(
        ui.find("Disconnect").is_ok(),
        "File 菜单项必须显示译文（而非 key 原文）"
    );
    assert!(
        ui.find("menu.disconnect").is_err(),
        "File 菜单项不得显示 i18n key 原文"
    );
    assert!(ui.find("Recent").is_ok(), "Recent 子菜单触发项必须可见");
}

#[test]
fn chinese_menu_switches_all_labels() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("zh-CN");

    let mut ui = MenuHarness::ui();
    assert!(ui.click("文件").is_ok(), "点击「文件」必须成功");
    assert!(ui.find("断开连接").is_ok(), "中文模式下必须显示中文菜单项");
    assert!(ui.find("最近使用").is_ok(), "「最近使用」必须可见");
    assert!(ui.find("File").is_err(), "中文模式下不得残留英文顶层菜单");
}

#[test]
fn edit_menu_language_submenu_translates() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("zh-CN");

    let mut ui = MenuHarness::ui();
    assert!(ui.click("编辑").is_ok(), "点击「编辑」必须成功");
    assert!(ui.find("复制截图").is_ok(), "编辑菜单项必须显示中文译文");
    assert!(
        ui.find("edit.copy_screenshot").is_err(),
        "编辑菜单项不得显示 i18n key 原文"
    );

    // zh-CN 下 edit.language 的译文为 "Language"（保留英文），点击展开子菜单。
    hover_item(&mut ui, "Language");
    assert!(ui.find("✓ 跟随系统").is_ok(), "默认 System 选中必须带标记");
    assert!(ui.find("中文").is_ok(), "语言子菜单必须显示「中文」");
}

#[test]
fn labels_are_single_line_no_newline() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    for locale in ["en", "zh-CN"] {
        rust_i18n::set_locale(locale);
        for key in [
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
            "language.system",
            "language.chinese",
            "language.english",
            "about.project_home",
            "dialog.jpeg_filter",
            "modal.settings_title",
            "modal.close",
            "modal.about_title",
            "modal.save_title",
            "modal.connection_title",
            "message.no_frame_screenshot",
            "message.screenshot_copied",
            "message.screenshot_copy_failed",
            "message.screenshot_saved",
            "message.screenshot_save_failed",
        ] {
            let label = ipkvm_desktop_iced::translate_key(key);
            assert!(
                !label.contains('\n'),
                "[{locale}] {key} 的译文不得包含换行符"
            );
            assert!(
                !label.is_empty() && label != key,
                "[{locale}] {key} 译文不得为空或等于 key 原文"
            );
        }
    }
}
