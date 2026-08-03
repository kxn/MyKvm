//! Spike 2 i18n 切换验证：zh↔en 菜单文案切换、显示译文而非 key 原文。

mod common;

use common::MenuHarness;

#[test]
fn english_menu_shows_translated_labels_not_keys() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut h = MenuHarness::new();
    let mut ui = h.ui();
    assert!(ui.click("File").is_ok(), "点击 File 必须成功");
    h.drive(ui);

    let mut ui = h.ui();
    assert!(
        ui.find("Reselect device…").is_ok(),
        "File 菜单项必须显示译文（而非 key 原文）"
    );
    assert!(
        ui.find("menu.reselect_device").is_err(),
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

    let mut h = MenuHarness::new();
    let mut ui = h.ui();
    assert!(ui.click("文件").is_ok(), "点击「文件」必须成功");
    h.drive(ui);

    let mut ui = h.ui();
    assert!(
        ui.find("重新选择设备…").is_ok(),
        "中文模式下必须显示中文菜单项"
    );
    assert!(ui.find("最近使用").is_ok(), "「最近使用」必须可见");
    assert!(ui.find("File").is_err(), "中文模式下不得残留英文顶层菜单");
}

#[test]
fn edit_menu_language_submenu_translates() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("zh-CN");

    let mut h = MenuHarness::new();
    let mut ui = h.ui();
    assert!(ui.click("编辑").is_ok(), "点击「编辑」必须成功");
    h.drive(ui);

    let mut ui = h.ui();
    assert!(ui.find("复制截图").is_ok(), "编辑菜单项必须显示中文译文");
    assert!(
        ui.find("edit.copy_screenshot").is_err(),
        "编辑菜单项不得显示 i18n key 原文"
    );
    assert!(ui.click("Language").is_ok(), "语言子菜单必须可打开");
    h.drive(ui);
    assert_eq!(h.state.open_path, vec![2], "Language 应在编辑菜单下标 2");

    let mut ui = h.ui();
    assert!(ui.find("跟随系统").is_ok(), "语言子菜单必须显示中文选项");
    assert!(ui.find("中文").is_ok());
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
            "menu.reselect_device",
            "menu.stop_connection",
            "file.load_profile",
            "file.recent",
            "file.recent_more",
            "file.exit",
            "edit.copy_screenshot",
            "edit.language",
            "edit.settings",
            "send.paste_text",
            "send.release_all",
            "send.special_keys",
            "special_keys.ctrl_alt_del",
            "language.system",
            "language.chinese",
            "language.english",
            "modal.settings_title",
            "modal.close",
            "modal.about_title",
            "modal.save_title",
            "modal.connection_title",
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

