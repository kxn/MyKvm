//! 菜单/模态测试共享 harness：持有 MenuState，驱动消息 → 状态 → 重建 view。

use iced::Element;
use iced::widget::{button, stack, text};
use iced_test::simulator::{self, Simulator};
use ipkvm_desktop_iced::menu::{self, MenuAction, MenuState};

pub const RECENT: [&str; 5] = ["p1", "p2", "p3", "p4", "p5"];

#[derive(Default)]
pub struct MenuHarness {
    pub state: MenuState,
    pub actions: Vec<MenuAction>,
    pub bg_hits: u32,
}

impl MenuHarness {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn view(&self) -> Element<'static, MenuAction> {
        let bar = menu::menu_bar(&self.state, &RECENT);
        let background: Element<'static, MenuAction, iced::Theme, iced::Renderer> =
            button(text("Hit me"))
                .on_press(MenuAction::Simple("bg"))
                .into();
        stack![background, bar].into()
    }

    pub fn apply(&mut self, action: MenuAction) {
        if let Some(business) = self.state.apply(action) {
            if business == MenuAction::Simple("bg") {
                self.bg_hits += 1;
            } else {
                self.actions.push(business);
            }
        }
    }

    /// 消费 simulator 产生的全部消息并应用到状态；返回业务动作（不含内部状态消息）。
    pub fn drive(&mut self, ui: Simulator<'_, MenuAction>) -> Vec<MenuAction> {
        let before = self.actions.len();
        for msg in ui.into_messages() {
            self.apply(msg);
        }
        self.actions[before..].to_vec()
    }

    pub fn ui(&self) -> Simulator<'static, MenuAction> {
        simulator::simulator(self.view())
    }
}

