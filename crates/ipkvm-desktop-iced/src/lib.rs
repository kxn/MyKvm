//! iced 桌面端（M1）：视频链路（scale/frames/video/status/app）。
//!
//! 迁移设计文档：docs/superpowers/specs/2026-08-03-iced-migration-design.md。
//! M1 已收编视频链路；菜单/输入在 M2/M3 继续从 spike crate
//! （ipkvm-desktop-iced-spike）收编。
//!
//! 可测试性要求（每阶段强制）：lib/bin 拆分，UI 逻辑可 headless 测试
//! （iced_test Simulator），窗口元数据走常量/函数并可断言。

use iced::Size;

pub mod app;
pub mod frames;
pub mod scale;
pub mod status;
pub mod video;

/// 窗口标题（M5 将嵌入 GIT_COMMIT）。
pub const WINDOW_TITLE: &str = "my_ipkvm iced (M0)";
/// 默认窗口尺寸。
pub const WINDOW_SIZE: Size = Size::new(1280.0, 800.0);

pub use app::run;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_title_and_size_are_stable() {
        // 窗口元数据走常量，M5 改标题时此处会强制更新。
        assert_eq!(WINDOW_TITLE, "my_ipkvm iced (M0)");
        assert_eq!(WINDOW_SIZE, Size::new(1280.0, 800.0));
    }
}
