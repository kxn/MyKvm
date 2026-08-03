//! 菜单交互验证（iced_aw，vendored 补丁版）：4 顶层菜单、子菜单深度 ≥3、
//! 业务动作发布并关闭菜单、点外部关闭且不穿透背景。
//!
//! 已知限制：#95 记录——iced_aw 0.14 不支持 Esc 关闭菜单，故原 Esc 用例移除。
mod common;

use common::{MenuHarness, RECENT};
use iced_test::Simulator;
use ipkvm_desktop_iced::menu::MenuAction;

fn hover_item(ui: &mut Simulator<'static, MenuAction>, label: &str) {
    let item = ui
        .find(label)
        .unwrap_or_else(|_| panic!("hover 目标 {label} 必须可定位"));
    MenuHarness::hover(ui, MenuHarness::center(item.bounds()));
}

#[test]
fn file_menu_can_open_and_show_items() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut ui = MenuHarness::ui();
    assert!(ui.click("File").is_ok(), "点击 File 顶层必须成功");
    assert!(ui.find("Recent").is_ok(), "File 展开后 Recent 必须可见");
    assert!(ui.find("Disconnect").is_ok(), "File 菜单项必须显示译文");
    assert!(ui.find("Reselect device…").is_err(), "旧项必须移除");
    assert!(ui.find("Exit").is_ok(), "File 菜单必须含 Exit");
}

#[test]
fn edit_menu_can_open() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut ui = MenuHarness::ui();
    assert!(ui.click("Edit").is_ok(), "点击 Edit 顶层必须成功");
    assert!(ui.find("Language").is_ok(), "Edit 展开后 Language 必须可见");
    assert!(ui.find("Settings…").is_ok(), "Edit 菜单必须含 Settings");
    #[cfg(windows)]
    {
        assert!(
            ui.find("Save screenshot as JPEG…").is_ok(),
            "Edit 菜单必须含保存截图"
        );
    }
}

#[test]
fn paste_busy_disables_paste_item() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut ui = MenuHarness::ui_with(
        &RECENT,
        true,
        ipkvm_desktop_iced::locale::AppLanguage::System,
        true,
        true,
    );
    assert!(ui.click("Send").is_ok(), "点击 Send 顶层必须成功");
    ui.click("Paste text")
        .expect("Paste text 必须可定位（禁用态仍渲染）");
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(
        !messages.contains(&MenuAction::Simple("paste")),
        "paste_busy 时 Paste text 不得发布动作，实际: {messages:?}"
    );
}

#[test]
fn language_menu_shows_selected_marker() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("zh-CN");

    let mut ui = MenuHarness::ui_with(
        &RECENT,
        false,
        ipkvm_desktop_iced::locale::AppLanguage::Chinese,
        true,
        true,
    );
    assert!(ui.click("编辑").is_ok(), "点击「编辑」必须成功");
    hover_item(&mut ui, "Language");
    assert!(ui.find("✓ 中文").is_ok(), "选中项必须带选中标记");
    assert!(
        ui.find("中文").is_err(),
        "选中项必须带标记（无标记文案不得出现）"
    );
}

#[test]
fn recent_empty_state_uses_i18n() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut ui = MenuHarness::ui_with(
        &[],
        false,
        ipkvm_desktop_iced::locale::AppLanguage::System,
        true,
        true,
    );
    assert!(ui.click("File").is_ok(), "点击 File 顶层必须成功");
    hover_item(&mut ui, "Recent");
    assert!(ui.find("None yet").is_ok(), "空态必须显示 i18n 文案");
    assert!(ui.find("(none)").is_err(), "空态不得显示硬编码英文");
}

#[test]
fn send_menu_can_open_and_show_special_keys() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut ui = MenuHarness::ui();
    assert!(ui.click("Send").is_ok(), "点击 Send 顶层必须成功");
    assert!(
        ui.find("Paste text").is_ok(),
        "Send 菜单必须显示 Paste text"
    );

    hover_item(&mut ui, "Send special keys");
    assert!(
        ui.find("Ctrl+Alt+Del").is_ok(),
        "特殊键子菜单必须展开并显示 Ctrl+Alt+Del"
    );

    ui.click("Ctrl+Alt+Del").expect("点击特殊键叶子项");
    assert!(ui.find("Paste text").is_err(), "点击业务项后菜单必须关闭");
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(
        messages.contains(&MenuAction::SpecialKey("CtrlAltDel".into())),
        "必须发布 SpecialKey(CtrlAltDel)，实际: {messages:?}"
    );
}

#[test]
fn about_menu_can_open() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut ui = MenuHarness::ui();
    assert!(ui.click("About").is_ok(), "点击 About 顶层必须成功");
    assert!(ui.find("Project home").is_ok(), "About 菜单必须可见");
}

#[test]
fn submenu_depth_at_least_3() {
    // File → Recent → More… → p4/p5：深度 = 3。
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut ui = MenuHarness::ui();
    assert!(ui.click("File").is_ok(), "点击 File 顶层");

    hover_item(&mut ui, "Recent");
    assert!(ui.find("p1").is_ok(), "二级菜单必须展开");

    hover_item(&mut ui, "More…");
    assert!(ui.find("p4").is_ok(), "More… 展开后 p4 必须可见（深度 3）");
    assert!(ui.find("p5").is_ok(), "More… 展开后 p5 必须可见");
}

#[test]
fn item_action_publishes_and_closes_menus() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut ui = MenuHarness::ui();
    assert!(ui.click("File").is_ok(), "打开 File");
    hover_item(&mut ui, "Recent");
    assert!(ui.find("p1").is_ok(), "Recent 子菜单必须展开");

    ui.click("p1").expect("点击最近使用项 p1");
    assert!(
        ui.find("Disconnect").is_err(),
        "点击业务项后菜单必须全部关闭"
    );

    let messages: Vec<_> = ui.into_messages().collect();
    assert!(
        messages.contains(&MenuAction::LoadRecent("p1".into())),
        "点击 p1 必须发布 LoadRecent，实际: {messages:?}"
    );
}

#[test]
fn outside_click_closes_menu_without_reaching_background() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut ui = MenuHarness::ui();
    assert!(ui.click("File").is_ok(), "打开 File");
    assert!(ui.find("Disconnect").is_ok(), "菜单必须已打开");

    // 菜单打开时点背景按钮：只关菜单，不触发背景（手动模拟点击，保留 ui）。
    let bg = ui.find("Hit me").expect("背景按钮必须可定位");
    let pos = bg.visible_bounds().expect("背景 bounds").center();
    ui.simulate([
        iced::Event::Mouse(iced::mouse::Event::CursorMoved { position: pos }),
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)),
        iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
            iced::mouse::Button::Left,
        )),
    ]);

    assert!(ui.find("Disconnect").is_err(), "点外部必须关闭菜单");

    let messages: Vec<_> = ui.into_messages().collect();
    assert!(
        !messages.contains(&MenuAction::Simple("bg")),
        "菜单打开时背景不得收到点击，实际: {messages:?}"
    );
}

#[test]
fn disconnect_item_disabled_when_offline() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut ui = MenuHarness::ui_with(
        &RECENT,
        false,
        ipkvm_desktop_iced::locale::AppLanguage::System,
        false,
        true,
    );
    assert!(ui.click("File").is_ok(), "打开 File");
    let disconnect = ui
        .find("Disconnect")
        .expect("Disconnect 必须可定位（禁用态仍渲染）");
    MenuHarness::click_at(&mut ui, MenuHarness::center(disconnect.bounds()));
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(
        !messages.contains(&MenuAction::Disconnect),
        "离线时点击断开连接不得发布动作，实际: {messages:?}"
    );
}

#[test]
fn disconnect_item_enabled_when_online() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut ui = MenuHarness::ui_with(
        &RECENT,
        false,
        ipkvm_desktop_iced::locale::AppLanguage::System,
        true,
        true,
    );
    assert!(ui.click("File").is_ok(), "打开 File");
    let disconnect = ui.find("Disconnect").expect("Disconnect 必须可定位");
    MenuHarness::click_at(&mut ui, MenuHarness::center(disconnect.bounds()));
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(
        messages.contains(&MenuAction::Disconnect),
        "在线时点击断开连接必须发布动作，实际: {messages:?}"
    );
}

#[test]
fn screenshot_items_disabled_without_frame() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut ui = MenuHarness::ui_with(
        &RECENT,
        false,
        ipkvm_desktop_iced::locale::AppLanguage::System,
        true,
        false,
    );
    assert!(ui.click("Edit").is_ok(), "打开 Edit");
    let copy = ui
        .find("Copy screenshot")
        .expect("Copy screenshot 必须可定位（禁用态仍渲染）");
    MenuHarness::click_at(&mut ui, MenuHarness::center(copy.bounds()));
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(
        !messages.contains(&MenuAction::Simple("copy_screenshot")),
        "无帧时 Copy screenshot 不得发布动作，实际: {messages:?}"
    );

    #[cfg(windows)]
    {
        let mut ui = MenuHarness::ui_with(
            &RECENT,
            false,
            ipkvm_desktop_iced::locale::AppLanguage::System,
            true,
            false,
        );
        assert!(ui.click("Edit").is_ok(), "打开 Edit");
        let save = ui
            .find("Save screenshot as JPEG…")
            .expect("保存截图项必须可定位（禁用态仍渲染）");
        MenuHarness::click_at(&mut ui, MenuHarness::center(save.bounds()));
        let messages: Vec<_> = ui.into_messages().collect();
        assert!(
            !messages.contains(&MenuAction::Simple("save_screenshot")),
            "无帧时保存截图不得发布动作，实际: {messages:?}"
        );
    }
}

#[test]
fn screenshot_items_enabled_with_frame() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut ui = MenuHarness::ui_with(
        &RECENT,
        false,
        ipkvm_desktop_iced::locale::AppLanguage::System,
        true,
        true,
    );
    assert!(ui.click("Edit").is_ok(), "打开 Edit");
    let copy = ui
        .find("Copy screenshot")
        .expect("Copy screenshot 必须可定位");
    MenuHarness::click_at(&mut ui, MenuHarness::center(copy.bounds()));
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(
        messages.contains(&MenuAction::Simple("copy_screenshot")),
        "有帧时点击 Copy screenshot 必须发布动作，实际: {messages:?}"
    );
}
