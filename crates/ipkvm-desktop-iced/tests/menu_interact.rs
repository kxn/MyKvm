//! Spike 2 菜单交互验证（自绘菜单）：4 顶层菜单可打开；子菜单深度 ≥3；
//! 业务动作可触发；Esc / 点击外部可关闭。

mod common;

use common::MenuHarness;
use ipkvm_desktop_iced::menu::MenuAction;

#[test]
fn file_menu_can_open_and_show_items() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut h = MenuHarness::new();
    let mut ui = h.ui();
    assert!(ui.click("File").is_ok());
    h.drive(ui);
    assert_eq!(h.state.open_root, Some(0));

    let mut ui = h.ui();
    assert!(ui.find("Recent").is_ok(), "File 展开后 Recent 必须可见");
    assert!(ui.find("Reselect device…").is_ok(), "菜单项必须显示译文");
}

#[test]
fn edit_menu_can_open() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut h = MenuHarness::new();
    let mut ui = h.ui();
    assert!(ui.click("Edit").is_ok());
    h.drive(ui);
    assert_eq!(h.state.open_root, Some(1));

    let mut ui = h.ui();
    assert!(ui.find("Language").is_ok(), "Edit 展开后 Language 必须可见");
}

#[test]
fn send_menu_can_open_and_show_special_keys() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut h = MenuHarness::new();
    let mut ui = h.ui();
    assert!(ui.click("Send").is_ok(), "Send 按钮必须可定位");
    h.drive(ui);
    assert_eq!(h.state.open_root, Some(2));

    let mut ui = h.ui();
    assert!(
        ui.find("Paste text").is_ok(),
        "Send 菜单必须显示 Paste text"
    );
    assert!(
        ui.click("Send special keys").is_ok(),
        "特殊键子菜单必须可打开"
    );
    h.drive(ui);
    assert_eq!(
        h.state.open_path,
        vec![3],
        "特殊键子菜单应在 Send 菜单下标 3"
    );

    let mut ui = h.ui();
    assert!(
        ui.find("Ctrl+Alt+Del").is_ok(),
        "特殊键子菜单必须显示 Ctrl+Alt+Del"
    );
}

#[test]
fn about_menu_can_open() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut h = MenuHarness::new();
    let mut ui = h.ui();
    assert!(ui.click("About").is_ok());
    h.drive(ui);
    assert_eq!(h.state.open_root, Some(3));
}

#[test]
fn submenu_depth_at_least_3() {
    // File → Recent → More… → p4/p5：深度 = 3。
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut h = MenuHarness::new();
    let mut ui = h.ui();
    assert!(ui.click("File").is_ok());
    h.drive(ui);
    assert_eq!(h.state.open_root, Some(0));

    let mut ui = h.ui();
    assert!(ui.click("Recent").is_ok(), "点击 Recent 打开二级菜单");
    h.drive(ui);
    assert_eq!(
        h.state.open_path,
        vec![4],
        "Recent 在 File 菜单中的下标应为 4"
    );

    let mut ui = h.ui();
    assert!(ui.click("More…").is_ok(), "点击 More… 打开三级菜单");
    h.drive(ui);
    assert_eq!(
        h.state.open_path,
        vec![4, 3],
        "More… 应在 Recent 菜单下标 3"
    );

    let mut ui = h.ui();
    assert!(ui.find("p4").is_ok(), "More… 展开后 p4 必须可见（深度 4）");
}

#[test]
fn item_action_publishes_and_closes_menus() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut h = MenuHarness::new();
    let mut ui = h.ui();
    assert!(ui.click("File").is_ok());
    h.drive(ui);

    let mut ui = h.ui();
    assert!(ui.click("Recent").is_ok(), "先展开 Recent 子菜单");
    h.drive(ui);

    let mut ui = h.ui();
    assert!(ui.click("p1").is_ok(), "最近使用项必须可点击");
    let actions = h.drive(ui);
    assert!(
        actions.contains(&MenuAction::LoadRecent("p1".into())),
        "点击 p1 必须发布 LoadRecent（实际: {actions:?}）"
    );
    assert_eq!(h.state.open_root, None, "点击业务项后菜单必须全部关闭");
}

#[test]
fn escape_closes_menus() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut h = MenuHarness::new();
    let mut ui = h.ui();
    assert!(ui.click("File").is_ok());
    h.drive(ui);
    assert!(h.state.open_root.is_some());

    let mut ui = h.ui();
    ui.tap_key(iced::keyboard::Key::Named(
        iced::keyboard::key::Named::Escape,
    ));
    h.drive(ui);
    assert_eq!(h.state.open_root, None, "Esc 后菜单必须关闭");
}

#[test]
fn outside_click_closes_menu_without_reaching_background() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut h = MenuHarness::new();
    let mut ui = h.ui();
    assert!(ui.click("File").is_ok());
    h.drive(ui);
    assert!(h.state.open_root.is_some());

    // 菜单打开时点背景按钮：只关菜单，不触发背景。
    let mut ui = h.ui();
    assert!(ui.click("Hit me").is_ok(), "背景按钮必须可定位");
    h.drive(ui);
    assert_eq!(h.state.open_root, None, "点外部必须关闭菜单");
    assert_eq!(h.bg_hits, 0, "菜单打开时背景不得收到点击");

    // 菜单关闭后背景恢复响应。
    let mut ui = h.ui();
    assert!(ui.click("Hit me").is_ok());
    h.drive(ui);
    assert_eq!(h.bg_hits, 1, "菜单关闭后背景必须恢复响应");
}
