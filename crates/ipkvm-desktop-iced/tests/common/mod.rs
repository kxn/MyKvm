//! 菜单测试共享 harness：构建 iced_aw 菜单 + 背景按钮。
//!
//! iced_aw 的菜单打开/关闭状态在 widget 树内管理，因此测试必须在**同一个**
//! `Simulator` 上连续驱动（click/hover/find）；`into_messages` 会移动
//! Simulator，只能作为测试最后一步取消息。

use iced::Element;
use iced::widget::{button, stack, text};
use iced_test::simulator::{self, Simulator};
use ipkvm_desktop_iced::menu::{self, MenuAction};

pub const RECENT: [&str; 5] = ["p1", "p2", "p3", "p4", "p5"];

pub struct MenuHarness;

impl MenuHarness {
    pub fn view() -> Element<'static, MenuAction> {
        let bar = menu::menu_bar(&RECENT);
        let background: Element<'static, MenuAction, iced::Theme, iced::Renderer> =
            button(text("Hit me"))
                .on_press(MenuAction::Simple("bg"))
                .into();
        stack![background, bar].into()
    }

    pub fn ui() -> Simulator<'static, MenuAction> {
        simulator::simulator(Self::view())
    }

    /// 把光标移到指定位置并注入 CursorMoved。
    ///
    /// 注意：`Simulator::simulate` 的命中测试用的是内部 cursor（`point_at`
    /// 设置），仅注入 CursorMoved 事件不会更新它，菜单不会响应 hover。
    pub fn hover(ui: &mut Simulator<'static, MenuAction>, position: iced::Point) {
        ui.point_at(position);
        ui.simulate([iced::Event::Mouse(iced::mouse::Event::CursorMoved {
            position,
        })]);
    }

    pub fn center(bounds: iced::Rectangle) -> iced::Point {
        iced::Point::new(
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        )
    }
}
