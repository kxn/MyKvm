//! Linux/macOS 相对鼠标 stub。
//!
//! 当前 Linux/macOS 均返回“未实现”错误，保证 trait 形状编译通过。
//! macOS 迁移方向：NSEvent local monitor（`addLocalMonitorForEventsMatchingMask`，
//! 需要 objc2 + block2 运行时注入）或 winit `DeviceEvent::MouseMotion` 集成
//! （iced 0.14 不暴露 winit DeviceEvent，需自建事件通道）。见 issue #42 跟踪。

use crate::relative::{DeltaReceiver, RelativePointerSource};

pub struct StubRawInput;

impl Default for StubRawInput {
    fn default() -> Self {
        Self
    }
}

impl StubRawInput {
    pub fn new() -> Self {
        Self
    }
}

impl RelativePointerSource for StubRawInput {
    fn receiver(&mut self) -> Result<DeltaReceiver, String> {
        Err("relative pointer capture is not implemented on this platform yet".into())
    }

    fn stop(&mut self) {}
}
