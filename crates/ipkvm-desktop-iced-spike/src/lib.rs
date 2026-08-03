//! iced 迁移可行性验证 spike（#73）。
//!
//! 本 crate 不替代 egui 桌面端，仅用于验证 iced 能否承载同样的会话/输入/渲染链路。
//! 复用 [`ipkvm_desktop::DesktopSessionController`]，仅把前端唤醒机制从 egui
//! 的 `request_repaint` 换成 iced 的 `Subscription`。

pub mod app;
pub mod frames;
pub mod keymap;
pub mod menu;
pub mod modal;
pub mod relative;
pub mod scale;
