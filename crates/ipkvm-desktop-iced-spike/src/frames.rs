//! 帧订阅：把 `DesktopSessionController::subscribe_frames` 的 watch receiver
//! 转成 iced `Subscription`（替代 egui 的 `request_repaint`）。
//!
//! 阶段 1 实现。
