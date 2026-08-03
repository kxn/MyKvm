//! iced 桌面端迁移壳（M0）：空窗口 + 双端共存入口。
//!
//! 迁移设计文档：docs/superpowers/specs/2026-08-03-iced-migration-design.md。
//! M0 只建立依赖基线与可运行壳；视频/菜单/输入分别在 M1/M2/M3 收编
//! spike crate（ipkvm-desktop-iced-spike）已验证的算法。

use iced::widget::{center, text};
use iced::{Element, Size, Task};

#[derive(Debug, Clone, Copy)]
enum Message {}

#[derive(Debug, Default)]
struct App;

impl App {
    fn update(&mut self, _message: Message) -> Task<Message> {
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        center(text("my_ipkvm · iced 迁移 M0")).into()
    }
}

fn main() -> iced::Result {
    iced::application(|| (App::default(), Task::none()), App::update, App::view)
        .title("my_ipkvm iced (M0)")
        .window_size(Size::new(1280.0, 800.0))
        .run()
}
