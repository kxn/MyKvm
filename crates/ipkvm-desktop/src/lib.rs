pub mod clipboard;
pub mod config;
pub mod frame;
pub mod probe;
pub mod render;
mod session;
pub mod state;

// 桌面会话控制器（及其依赖类型）对 iced 前端复用，同一套
// SessionSupervisor/输入泵只替换前端重绘唤醒机制。
pub use session::{
    ConnectRequest, DesktopSessionController, DesktopSessionError, DesktopSessionFactory,
    ProductionDesktopSessionController, ProductionSessionFactory, SessionParts,
};
// send_pointer 的参数类型必须是可命名的公开类型（#102 绝对指针发送）。
pub use render::FrameSize;
