//! Spike 2 走廊 hover 验证（自绘菜单）：父项 → 子菜单连续穿越 100 次，误关闭 = 0。
//!
//! 测试真实自绘 overlay：先打开 File → Recent 子菜单，再拿父项/子项的真实
//! visible_bounds 计算走廊中点，逐点注入 CursorMoved 并断言没有任何
//! CloseSubmenus / CloseMenus 消息。

mod common;

use common::MenuHarness;
use iced::mouse;
use iced::{Event, Point};
use ipkvm_desktop_iced::menu::MenuAction;

#[test]
fn corridor_hover_100_crossings_zero_misclose() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut h = MenuHarness::new();

    // 打开 File。
    let mut ui = h.ui();
    assert!(ui.click("File").is_ok());
    h.drive(ui);
    assert_eq!(h.state.open_root, Some(0));

    // 打开 Recent 子菜单（File 菜单下标 3）。
    let mut ui = h.ui();
    assert!(ui.click("Recent").is_ok(), "Recent 必须可点击");
    h.drive(ui);
    assert_eq!(h.state.open_path, vec![4], "Recent 子菜单必须已展开");

    // 取父项与子项的真实 bounds（自绘菜单 text 元素 bounds == 命中矩形）。
    let mut ui = h.ui();
    let parent = ui
        .find("Recent")
        .expect("父项 Recent 必须可见")
        .visible_bounds()
        .expect("父项必须有 bounds");
    let child = ui
        .find("p1")
        .expect("子项 p1 必须可见")
        .visible_bounds()
        .expect("子项必须有 bounds");
    drop(ui);

    let corridor_mid = Point::new(
        (parent.x + parent.width + child.x) / 2.0,
        (parent.y + parent.height + child.y) / 2.0,
    );
    let points = [parent.center(), corridor_mid, child.center()];

    let mut misclose = 0;
    for i in 0..100 {
        let pos = points[i % points.len()];
        let mut ui = h.ui();
        ui.simulate([Event::Mouse(mouse::Event::CursorMoved { position: pos })]);
        let actions = h.drive(ui);
        if actions
            .iter()
            .any(|a| matches!(a, MenuAction::CloseSubmenus | MenuAction::CloseMenus))
        {
            misclose += 1;
        }
        assert!(
            !h.state.open_path.is_empty(),
            "第 {i} 次穿越后子菜单必须保持打开"
        );
    }

    assert_eq!(
        misclose, 0,
        "100 次走廊穿越后误关闭次数必须为 0（实际 {misclose}）"
    );
}
