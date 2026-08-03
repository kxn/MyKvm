//! iced 桌面端迁移壳（M0）：空窗口 + 双端共存入口。
//!
//! 迁移设计文档：docs/superpowers/specs/2026-08-03-iced-migration-design.md。
//! M0 只建立依赖基线与可运行壳；视频/菜单/输入分别在 M1/M2/M3 收编
//! spike crate（ipkvm-desktop-iced-spike）已验证的算法。
//!
//! 可测试性要求（每阶段强制）：lib/bin 拆分，UI 逻辑可 headless 测试
//! （iced_test Simulator），窗口元数据走常量/函数并可断言。

use iced::widget::{center, text};
use iced::{Element, Size, Task};

pub mod scale;
pub mod frames;
pub mod video;

/// 占位文案（M0 渲染断言用）。
pub const PLACEHOLDER: &str = "my_ipkvm · iced 迁移 M0";
/// 窗口标题（M5 将嵌入 GIT_COMMIT）。
pub const WINDOW_TITLE: &str = "my_ipkvm iced (M0)";
/// 默认窗口尺寸。
pub const WINDOW_SIZE: Size = Size::new(1280.0, 800.0);

#[derive(Debug, Clone, Copy)]
pub enum Message {}

#[derive(Debug, Default)]
pub struct App;

impl App {
    pub fn update(&mut self, _message: Message) -> Task<Message> {
        Task::none()
    }

    pub fn view(&self) -> Element<'_, Message> {
        center(text(PLACEHOLDER)).into()
    }
}

/// 启动 iced 应用（bin 入口调用；测试不启动真实窗口）。
pub fn run() -> iced::Result {
    iced::application(|| (App::default(), Task::none()), App::update, App::view)
        .title(WINDOW_TITLE)
        .window_size(WINDOW_SIZE)
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced_test::simulator;

    #[test]
    fn placeholder_text_renders_in_headless_ui() {
        // 空窗口壳的 view 必须能 headless 渲染并找到占位文案。
        let app = App::default();
        let mut ui = simulator::simulator(app.view());
        assert!(
            ui.find(PLACEHOLDER).is_ok(),
            "占位文案必须渲染（find 失败）"
        );
    }

    #[test]
    fn window_title_and_size_are_stable() {
        // 窗口元数据走常量，M5 改标题时此处会强制更新。
        assert_eq!(WINDOW_TITLE, "my_ipkvm iced (M0)");
        assert_eq!(WINDOW_SIZE, Size::new(1280.0, 800.0));
    }
}
