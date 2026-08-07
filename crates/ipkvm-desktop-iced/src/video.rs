//! 视频帧字节转换：RGB888 → RGBA8888，处理 stride 填充，并包装成 iced Handle。

use iced::widget::image::Handle;
use ipkvm_video::{PixelFormat, VideoFrame};

/// RGB888 → RGBA8888（补 alpha=255）
pub fn rgb_to_rgba(frame: &VideoFrame) -> Result<Vec<u8>, String> {
    if frame.pixel_format != PixelFormat::Rgb888 {
        return Err(format!(
            "unsupported pixel format: {:?}",
            frame.pixel_format
        ));
    }
    let width = frame.width as usize;
    let height = frame.height as usize;
    let stride = frame.stride as usize;
    let expected_rgb = width * height * 3;
    if frame.data.len() < expected_rgb {
        return Err(format!(
            "frame data too short: need {expected_rgb}, got {}",
            frame.data.len()
        ));
    }
    let mut pixels = vec![0u8; width * height * 4];
    for y in 0..height {
        let src = &frame.data[y * stride..y * stride + width * 3];
        let dst = &mut pixels[y * width * 4..(y + 1) * width * 4];
        for (rgba, rgb) in dst.chunks_exact_mut(4).zip(src.chunks_exact(3)) {
            rgba[0] = rgb[0]; // R
            rgba[1] = rgb[1]; // G
            rgba[2] = rgb[2]; // B
            rgba[3] = 255; // A
        }
    }
    Ok(pixels)
}

pub fn handle_from_frame(frame: &VideoFrame) -> Handle {
    match rgb_to_rgba(frame) {
        Ok(pixels) => Handle::from_rgba(frame.width, frame.height, pixels),
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

    #[test]
    fn rgb_to_rgba_adds_alpha() {
        let out = rgb_to_rgba(&rgb_frame(vec![10, 20, 30], 1, 1, 3)).unwrap();
        assert_eq!(out, vec![10, 20, 30, 255]);
    }

    #[test]
    fn rgb_to_rgba_rejects_short_data() {
        assert!(rgb_to_rgba(&rgb_frame(vec![0, 0], 1, 1, 3)).is_err());
    }

    #[test]
    fn handle_from_frame_rgb_works() {
        let frame = rgb_frame(vec![10, 20, 30], 1, 1, 3);
        let handle = handle_from_frame(&frame);
        // Handle 应该成功创建
        assert_ne!(
            handle.id(),
            iced::widget::image::Handle::from_rgba(1, 1, vec![0, 0, 0, 0]).id()
        );
    }
}
