//! 光标隐藏/锁定收口：Windows 用 ShowCursor + ClipCursor，其余平台 no-op。
//!
//! iced 0.14 无窗口级 grab API（已核实 iced_runtime-0.14 window.rs），
//! ClipCursor 裁剪到前台窗口矩形作为 Locked 的等价实现。
//!
//! 实现细节以 windows crate 0.61 实际签名为准：
//! - `ShowCursor(bshow: bool) -> i32`（非 BOOL 参数）；
//! - `GetForegroundWindow() -> HWND`（直接返回，无 Result）；
//! - `ClipCursor(lprect: Option<*const RECT>) -> Result<()>`。
//! 光标控制尽力而为：Win32 调用失败静默，不 panic、不阻塞。

/// 光标控制器（平台差异收口，供测试注入）。
pub trait CursorController: Send + Sync {
    fn set_visible(&self, visible: bool);
    fn set_clipped(&self, clipped: bool);
}

/// 只在状态变化时放行（ShowCursor 是计数 API，必须幂等同步）。
/// 0=未知（首调用必须放行），1=可见，2=隐藏。
#[derive(Default)]
pub struct VisibilityGate {
    state: std::sync::atomic::AtomicU8,
}

impl VisibilityGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// 返回 true 表示状态发生变化（应真正调用 ShowCursor）。
    pub fn update(&self, visible: bool) -> bool {
        use std::sync::atomic::Ordering;
        let next = if visible { 1u8 } else { 2u8 };
        let previous = self.state.swap(next, Ordering::Relaxed);
        previous == 0 || previous != next
    }
}

/// 生产实现（Windows）。
#[cfg(target_os = "windows")]
#[derive(Default)]
pub struct WindowsCursorController {
    gate: VisibilityGate,
}

#[cfg(target_os = "windows")]
impl CursorController for WindowsCursorController {
    fn set_visible(&self, visible: bool) {
        if !self.gate.update(visible) {
            return;
        }
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::ShowCursor;
            let _ = ShowCursor(visible);
        }
    }

    fn set_clipped(&self, clipped: bool) {
        unsafe {
            use windows::Win32::Foundation::RECT;
            use windows::Win32::UI::WindowsAndMessaging::{
                ClipCursor, GetForegroundWindow, GetWindowRect,
            };
            if clipped {
                let hwnd = GetForegroundWindow();
                if !hwnd.is_invalid() {
                    let mut rect = RECT::default();
                    if GetWindowRect(hwnd, &mut rect).is_ok() {
                        let _ = ClipCursor(Some(&rect as *const RECT));
                    }
                }
            } else {
                let _ = ClipCursor(None);
            }
        }
    }
}

/// 非 Windows：占位实现（迁移留口）。
#[cfg(not(target_os = "windows"))]
#[derive(Default)]
pub struct NoopCursorController;

#[cfg(not(target_os = "windows"))]
impl CursorController for NoopCursorController {
    fn set_visible(&self, _visible: bool) {}
    fn set_clipped(&self, _clipped: bool) {}
}

#[cfg(target_os = "windows")]
pub type ProductionCursorController = WindowsCursorController;
#[cfg(not(target_os = "windows"))]
pub type ProductionCursorController = NoopCursorController;

#[cfg(test)]
mod tests {
    use super::VisibilityGate;

    #[test]
    fn visibility_gate_only_passes_on_change() {
        let gate = VisibilityGate::new();
        assert!(
            gate.update(false),
            "首次 update(false) 必须放行（未知状态不得吞掉首次隐藏）"
        );
        let gate = VisibilityGate::new();
        assert!(
            gate.update(true),
            "首次 update(true) 必须放行（未知状态不得吞掉首次显示）"
        );
        let gate = VisibilityGate::new();
        // 首 true 也算变化：true → false → false → true 共 3 次放行。
        assert!(gate.update(true), "首次 true 必须放行");
        assert!(!gate.update(true), "重复 true 不得放行");
        assert!(gate.update(false), "true→false 必须放行");
        assert!(!gate.update(false), "重复 false 不得放行");
        assert!(gate.update(true), "false→true 必须放行");
    }
}
