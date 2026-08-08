//! 与 UI 和真实硬件无关的桌面状态、探测抽象和会话控制器。

pub mod config;
pub mod frame;
pub mod probe;
pub mod render;
pub mod session;
pub mod state;

pub use render::FrameSize;
pub use session::{
    ConnectRequest, DesktopSessionController, DesktopSessionError, DesktopSessionFactory,
    SessionParts,
};
