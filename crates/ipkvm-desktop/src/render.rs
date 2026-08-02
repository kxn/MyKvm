#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameSize {
    pub width: u32,
    pub height: u32,
}

/// 视频视口布局与坐标换算：帧到容器、指针到帧缓冲。
pub struct VideoViewport;

impl VideoViewport {
    /// 计算视频帧在容器内的绘制矩形（保比例、居中）。
    pub fn frame_rect(
        container: eframe::egui::Rect,
        frame: FrameSize,
        mode: crate::state::VideoScaleMode,
    ) -> eframe::egui::Rect {
        if frame.width == 0 || frame.height == 0 {
            return container;
        }
        let size = match mode {
            crate::state::VideoScaleMode::FitWindow
            | crate::state::VideoScaleMode::ResizeWindowToVideo => {
                let frame_aspect = frame.width as f32 / frame.height as f32;
                let container_aspect = container.width() / container.height();
                if container_aspect > frame_aspect {
                    eframe::egui::vec2(container.height() * frame_aspect, container.height())
                } else {
                    eframe::egui::vec2(container.width(), container.width() / frame_aspect)
                }
            }
            crate::state::VideoScaleMode::ActualSize => {
                eframe::egui::vec2(frame.width as f32, frame.height as f32)
            }
        };
        // 兜底：任何模式下视频矩形都不允许超出容器（ActualSize 大于容器时
        // 等比缩小，避免底部/右侧被面板裁剪遮挡）。
        let scale = (container.width() / size.x)
            .min(container.height() / size.y)
            .min(1.0);
        let size = eframe::egui::vec2(size.x * scale, size.y * scale);
        eframe::egui::Rect::from_center_size(container.center(), size)
    }

    /// 把窗口指针坐标映射回帧缓冲坐标；窗口外或帧尺寸非法时返回 None。
    pub fn map_pointer(
        point: eframe::egui::Pos2,
        rect: eframe::egui::Rect,
        frame: FrameSize,
    ) -> Option<(u16, u16)> {
        if !rect.contains(point) || frame.width == 0 || frame.height == 0 {
            return None;
        }
        let x = ((point.x - rect.left()) / rect.width() * frame.width as f32)
            .floor()
            .clamp(0.0, frame.width.saturating_sub(1) as f32) as u16;
        let y = ((point.y - rect.top()) / rect.height() * frame.height as f32)
            .floor()
            .clamp(0.0, frame.height.saturating_sub(1) as f32) as u16;
        Some((x, y))
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Rect, pos2};

    use super::*;
    use crate::state::VideoScaleMode;

    #[test]
    fn fit_window_preserves_aspect_ratio() {
        let container = Rect::from_min_size(pos2(0.0, 0.0), eframe::egui::vec2(1000.0, 500.0));
        let rect = VideoViewport::frame_rect(
            container,
            FrameSize {
                width: 1920,
                height: 1080,
            },
            VideoScaleMode::FitWindow,
        );
        assert!((rect.width() - 888.8889).abs() < 0.01);
        assert_eq!(rect.height(), 500.0);
    }

    #[test]
    fn pointer_maps_back_to_framebuffer_coordinates() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), eframe::egui::vec2(200.0, 100.0));
        let frame = FrameSize {
            width: 400,
            height: 200,
        };

        assert_eq!(
            VideoViewport::map_pointer(pos2(110.0, 70.0), rect, frame),
            Some((200, 100))
        );
        assert_eq!(
            VideoViewport::map_pointer(pos2(9.0, 70.0), rect, frame),
            None
        );
    }

    #[test]
    fn actual_size_larger_than_container_is_scaled_down_to_fit() {
        let container = Rect::from_min_size(pos2(0.0, 0.0), eframe::egui::vec2(200.0, 100.0));
        let frame = FrameSize {
            width: 1920,
            height: 1080,
        };

        let rect =
            VideoViewport::frame_rect(container, frame, crate::state::VideoScaleMode::ActualSize);

        assert!(rect.width() <= container.width() + 0.01);
        assert!(rect.height() <= container.height() + 0.01);
        assert!((rect.width() / rect.height() - 16.0 / 9.0).abs() < 0.01);
    }
}
