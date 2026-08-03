//! 走廊 hover 验证（iced_aw，vendored 补丁版）：父项 ↔ 子菜单连续穿越
//! 100 次，菜单不得误关。iced_aw 用 safe bounds + safe triangle 保持
//! 父项到子菜单之间的连通区域。
mod common;

use common::MenuHarness;
use iced::Point;
use iced_test::Simulator;
use ipkvm_desktop_iced::menu::MenuAction;

fn hover_item(ui: &mut Simulator<'static, MenuAction>, label: &str) {
    let item = ui
        .find(label)
        .unwrap_or_else(|_| panic!("hover 目标 {label} 必须可定位"));
    MenuHarness::hover(ui, MenuHarness::center(item.bounds()));
}

#[test]
fn corridor_hover_100_crossings_zero_misclose() {
    let _lock = ipkvm_desktop_iced::I18N_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    rust_i18n::set_locale("en");

    let mut ui = MenuHarness::ui();

    // 打开 File → Recent → More…（深度 3）。
    assert!(ui.click("File").is_ok(), "打开 File");
    hover_item(&mut ui, "Recent");
    assert!(ui.find("p1").is_ok(), "二级菜单必须展开");
    hover_item(&mut ui, "More…");
    assert!(ui.find("p4").is_ok(), "三级菜单必须展开");

    // 取父项与子项的真实 bounds，逐点穿越父项 ↔ 子菜单。
    let parent = ui.find("More…").expect("父项 More… 必须可见");
    let child = ui.find("p4").expect("子项 p4 必须可见");
    let p = MenuHarness::center(parent.bounds());
    let c = MenuHarness::center(child.bounds());

    for i in 0..100 {
        let t = if i % 2 == 0 { 0.0 } else { 1.0 };
        MenuHarness::hover(
            &mut ui,
            Point::new(p.x + (c.x - p.x) * t, p.y + (c.y - p.y) * t),
        );
        assert!(ui.find("p4").is_ok(), "第 {i} 次穿越后三级菜单必须保持打开");
        assert!(ui.find("p1").is_ok(), "第 {i} 次穿越后二级菜单必须保持打开");
    }
}
