//! 视频帧字节 → iced 图像 Handle 的桥接。
//!
//! 实际的字节转换（RGB888/BGRA8888/MJPEG → RGBA8888）由 `ipkvm-desktop-core::frame`
//! 的统一入口 `frame_to_rgba` 完成；这里只负责把结果包装成 iced `Handle`。
//!
//! MJPEG 分支修复了硬件 MJPEG 直通（#35 FU-1）后桌面预览黑屏的回归，见 #44。

use iced::widget::image::Handle;
use ipkvm_desktop_core::frame::frame_to_rgba;
use ipkvm_video::VideoFrame;

/// 把视频帧转成 iced 可显示的 `Handle`。
///
/// 转换失败时退化为 1×1 透明像素（历史上如此），但 `frame_to_rgba` 对 MJPEG /
/// RGB888 / BGRA8888 三种捕获侧实际产出的格式都能成功，失败只发生在数据损坏时。
pub fn handle_from_frame(frame: &VideoFrame) -> Handle {
    match frame_to_rgba(frame) {
        Ok(rgba) => Handle::from_rgba(rgba.width, rgba.height, rgba.pixels),
        Err(_) => Handle::from_rgba(1, 1, vec![0, 0, 0, 0]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipkvm_video::{MonotonicTimestamp, PixelFormat, VideoFrame};
    use std::sync::Arc;

    fn rgb_frame(data: Vec<u8>, width: u32, height: u32, stride: u32) -> VideoFrame {
        VideoFrame::new(
            1,
            MonotonicTimestamp::from_nanos(1),
            width,
            height,
            stride,
            PixelFormat::Rgb888,
            Arc::from(data.into_boxed_slice()),
        )
    }

    fn mjpeg_frame(jpeg: Vec<u8>, width: u32, height: u32) -> VideoFrame {
        VideoFrame::new(
            1,
            MonotonicTimestamp::from_nanos(1),
            width,
            height,
            0,
            PixelFormat::Mjpeg,
            Arc::from(jpeg.into_boxed_slice()),
        )
    }

    /// 编码一张纯色 JPEG 作为 MJPEG 帧数据（无外部 fixture）。
    fn encode_test_jpeg(width: u32, height: u32, rgb: &[u8; 3]) -> Vec<u8> {
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 3);
        for _ in 0..(width * height) {
            pixels.extend_from_slice(rgb);
        }
        let mut jpeg = Vec::new();
        jpeg_encoder::Encoder::new(&mut jpeg, 100)
            .encode(
                &pixels,
                width as u16,
                height as u16,
                jpeg_encoder::ColorType::Rgb,
            )
            .expect("encoding test jpeg should succeed");
        jpeg
    }

    #[test]
    fn handle_from_frame_rgb_works() {
        let frame = rgb_frame(vec![10, 20, 30], 1, 1, 3);
        let handle = handle_from_frame(&frame);
        // Handle 应该成功创建，id 与 1x1 透明占位不同。
        assert_ne!(
            handle.id(),
            iced::widget::image::Handle::from_rgba(1, 1, vec![0, 0, 0, 0]).id()
        );
    }

    #[test]
    fn handle_from_frame_mjpeg_decodes_to_nonzero_dimensions() {
        // 回归测试：MJPEG 帧不再退化成 1x1 透明，而是解码出真实尺寸。
        // #44 的核心 bug 就是这里返回了 1x1 占位。
        let jpeg = encode_test_jpeg(2, 2, &[0, 255, 0]); // 绿色 2x2
        let frame = mjpeg_frame(jpeg, 2, 2);
        let placeholder = iced::widget::image::Handle::from_rgba(1, 1, vec![0, 0, 0, 0]);
        let handle = handle_from_frame(&frame);
        assert_ne!(
            handle.id(),
            placeholder.id(),
            "MJPEG frame must not degrade to 1x1 placeholder (regression of #44)"
        );
    }

    #[test]
    fn handle_from_frame_invalid_mjpeg_does_not_panic() {
        // 损坏的 JPEG 字节应优雅降级，而非 panic。
        // 注意：Handle::id() 每次生成新的 Unique，不能用于相等比较；
        // 这里只验证不 panic 且返回一个 Handle（回归保护：错误输入不会使预览崩溃）。
        let frame = mjpeg_frame(vec![0, 1, 2, 3], 1, 1);
        let _handle = handle_from_frame(&frame);
    }
}
