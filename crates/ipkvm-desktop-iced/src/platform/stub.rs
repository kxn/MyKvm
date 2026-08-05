//! 非 Windows 平台相对鼠标 stub。
//!
//! macOS 迁移时在此实现（winit 集成模式或 NSEvent local monitor）；当前只保证
//! trait 形状编译通过（`cargo check --target x86_64-apple-darwin`）。

use crate::relative::{DeltaReceiver, RelativePointerSource};

pub struct StubRawInput;

impl Default for StubRawInput {
    fn default() -> Self {
        Self
    }
}

impl StubRawInput {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RelativePointerSource for StubRawInput {
    fn receiver(&mut self) -> Result<DeltaReceiver, String> {
        Err("relative pointer capture is not implemented on this platform yet".into())
    }

    fn stop(&mut self) {}
}
