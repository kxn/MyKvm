//! 视频帧缩放纯函数。
//!
//! 来自迁移前桌面端 `VideoViewport`，逻辑等价但核心数学与 GUI 类型解耦，
//! 便于在 250% DPI 三模式下做纯函数断言。iced 端用 [`frame_rect`] 做薄包装。
//!
//! 三模式语义（与迁移前桌面端一致）：
//! - `FitWindow`：保比例 aspect-fit 居中，不越界。
//! - `ActualSize`：用帧物理像素尺寸，但超出容器时等比缩小（防底部/右侧被裁剪）。
//! - `ResizeWindowToVideo`：布局同 `FitWindow`（窗口尺寸调整在外层另做）。

/// 视频缩放模式。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScaleMode {
    /// 保比例适应窗口（aspect-fit 居中）。
    FitWindow,
    /// 1:1 物理像素，超出容器时等比缩小。
    ActualSize,
    /// 窗口尺寸跟随帧（布局同 FitWindow，窗口调整在外层）。
    ResizeWindowToVideo,
}

/// 矩形（原点 + 尺寸），GUI 无关。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn from_min_size(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// 帧尺寸（像素）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameSize {
    pub width: u32,
    pub height: u32,
}

/// 计算视频帧在容器内的绘制矩形（保比例、居中、不越界）。
///
/// 返回 (x, y, w, h)。帧尺寸为 0 时返回整个容器。
pub fn frame_rect(container: Rect, frame: FrameSize, mode: ScaleMode) -> Rect {
    if frame.width == 0 || frame.height == 0 {
        return container;
    }
    let size = match mode {
        ScaleMode::FitWindow | ScaleMode::ResizeWindowToVideo => {
            let frame_aspect = frame.width as f32 / frame.height as f32;
            let container_aspect = container.w / container.h;
            if container_aspect > frame_aspect {
                (container.h * frame_aspect, container.h)
            } else {
                (container.w, container.w / frame_aspect)
            }
        }
        ScaleMode::ActualSize => (frame.width as f32, frame.height as f32),
    };
    // 兜底：任何模式下视频矩形都不允许超出容器（ActualSize 大于容器时
    // 等比缩小，避免底部/右侧被面板裁剪遮挡）。
    let scale = (container.w / size.0).min(container.h / size.1).min(1.0);
    let (w, h) = (size.0 * scale, size.1 * scale);
    let (cx, cy) = container.center();
    Rect::from_min_size(cx - w / 2.0, cy - h / 2.0, w, h)
}

/// 把窗口指针坐标映射回帧缓冲坐标；窗口外或帧尺寸非法时返回 None。
pub fn map_pointer(point: (f32, f32), rect: Rect, frame: FrameSize) -> Option<(u16, u16)> {
    let inside = point.0 >= rect.x
        && point.0 <= rect.x + rect.w
        && point.1 >= rect.y
        && point.1 <= rect.y + rect.h;
    if !inside || frame.width == 0 || frame.height == 0 {
        return None;
    }
    let x = ((point.0 - rect.x) / rect.w * frame.width as f32)
        .floor()
        .clamp(0.0, frame.width.saturating_sub(1) as f32) as u16;
    let y = ((point.1 - rect.y) / rect.h * frame.height as f32)
        .floor()
        .clamp(0.0, frame.height.saturating_sub(1) as f32) as u16;
    Some((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_window_preserves_aspect_ratio() {
        // 迁移前桌面端的同一用例：1000×500 容器装 1920×1080 帧。
        let container = Rect::from_min_size(0.0, 0.0, 1000.0, 500.0);
        let rect = frame_rect(
            container,
            FrameSize {
                width: 1920,
                height: 1080,
            },
            ScaleMode::FitWindow,
        );
        assert!((rect.w - 888.8889).abs() < 0.01);
        assert_eq!(rect.h, 500.0);
    }

    #[test]
    fn fit_window_centered_and_within_container() {
        // 容器比帧更宽：高度占满，宽度按比例，水平居中。
        let container = Rect::from_min_size(0.0, 0.0, 1000.0, 500.0);
        let rect = frame_rect(
            container,
            FrameSize {
                width: 1920,
                height: 1080,
            },
            ScaleMode::FitWindow,
        );
        // 居中：左右留白相等。
        assert!((rect.x - (1000.0 - rect.w) / 2.0).abs() < 0.01);
        assert!(rect.x >= -0.01);
        assert!(rect.x + rect.w <= 1000.0 + 0.01);
        assert!(rect.y >= -0.01);
        assert!(rect.y + rect.h <= 500.0 + 0.01);
    }

    #[test]
    fn actual_size_smaller_than_container_uses_physical_pixels() {
        // 帧小于容器：ActualSize 用物理像素，居中，不放大。
        let container = Rect::from_min_size(0.0, 0.0, 800.0, 600.0);
        let rect = frame_rect(
            container,
            FrameSize {
                width: 320,
                height: 240,
            },
            ScaleMode::ActualSize,
        );
        assert_eq!(rect.w, 320.0);
        assert_eq!(rect.h, 240.0);
    }

    #[test]
    fn actual_size_larger_than_container_is_scaled_down_to_fit() {
        // 帧远大于容器：ActualSize 等比缩小，保持 16:9，不越界。
        let container = Rect::from_min_size(0.0, 0.0, 200.0, 100.0);
        let rect = frame_rect(
            container,
            FrameSize {
                width: 1920,
                height: 1080,
            },
            ScaleMode::ActualSize,
        );
        assert!(rect.w <= 200.0 + 0.01);
        assert!(rect.h <= 100.0 + 0.01);
        assert!((rect.w / rect.h - 16.0 / 9.0).abs() < 0.01);
    }

    #[test]
    fn fit_window_at_250_percent_dpi_three_modes_do_not_overflow_or_clip_bottom() {
        // #73 验收点：250% DPI（=高 DPI）下三模式都不越界、不截底。
        // 250% DPI 时逻辑像素与物理像素分离，这里用容器 384×216（逻辑）模拟，
        // 帧仍为 1920×1080（物理）。关键是不管哪种模式，rect 不得超出容器。
        let container = Rect::from_min_size(0.0, 0.0, 384.0, 216.0);
        let frame = FrameSize {
            width: 1920,
            height: 1080,
        };
        for mode in [
            ScaleMode::FitWindow,
            ScaleMode::ActualSize,
            ScaleMode::ResizeWindowToVideo,
        ] {
            let rect = frame_rect(container, frame, mode);
            assert!(rect.x >= -0.01, "{mode:?}: 左边界越界 x={}", rect.x);
            assert!(rect.y >= -0.01, "{mode:?}: 顶部越界 y={}", rect.y);
            assert!(
                rect.x + rect.w <= container.w + 0.01,
                "{mode:?}: 右边界越界 x+w={}",
                rect.x + rect.w
            );
            // 关键：底部不得被状态栏/面板截断。
            assert!(
                rect.y + rect.h <= container.h + 0.01,
                "{mode:?}: 底部截断 y+h={}",
                rect.y + rect.h
            );
        }
    }

    #[test]
    fn zero_frame_returns_container() {
        let container = Rect::from_min_size(5.0, 6.0, 100.0, 100.0);
        let rect = frame_rect(
            container,
            FrameSize {
                width: 0,
                height: 0,
            },
            ScaleMode::FitWindow,
        );
        assert_eq!(rect, container);
    }

    #[test]
    fn pointer_maps_back_to_framebuffer_coordinates() {
        let rect = Rect::from_min_size(10.0, 20.0, 200.0, 100.0);
        let frame = FrameSize {
            width: 400,
            height: 200,
        };
        assert_eq!(map_pointer((110.0, 70.0), rect, frame), Some((200, 100)));
        assert_eq!(map_pointer((9.0, 70.0), rect, frame), None);
    }

    #[test]
    fn pointer_outside_rect_returns_none() {
        let rect = Rect::from_min_size(0.0, 0.0, 100.0, 100.0);
        let frame = FrameSize {
            width: 100,
            height: 100,
        };
        assert_eq!(map_pointer((150.0, 50.0), rect, frame), None);
        assert_eq!(map_pointer((50.0, 150.0), rect, frame), None);
    }
}
