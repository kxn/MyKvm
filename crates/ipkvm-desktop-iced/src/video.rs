//! 视频帧字节转换：BGRA8888/RGB888 → RGBA8888，处理 stride 填充，并包装成 iced Handle。

use iced::widget::image::Handle;
use ipkvm_video::{PixelFormat, VideoFrame};

/// BGRA8888 → RGBA8888
pub fn bgra_to_rgba(frame: &VideoFrame) -> Result<Vec<u8>, String> {
    if frame.pixel_format != PixelFormat::Bgra8888 {
        return Err(format!(
            "unsupported pixel format: {:?}",
            frame.pixel_format
        ));
    }
    let width = frame.width as usize;
    let height = frame.height as usize;
    let stride = frame.stride as usize;
    let Some(required) = stride
        .checked_mul(height.saturating_sub(1))
        .and_then(|rows| rows.checked_add(width * 4))
    else {
        return Err("frame stride or size overflow".into());
    };
    if frame.data.len() < required {
        return Err(format!(
            "frame data too short: need {required}, got {}",
            frame.data.len()
        ));
    }
    let mut pixels = vec![0u8; width * height * 4];
    for y in 0..height {
        let src = &frame.data[y * stride..y * stride + width * 4];
        let dst = &mut pixels[y * width * 4..(y + 1) * width * 4];
        for (rgba, bgra) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            rgba.copy_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
        }
    }
    Ok(pixels)
}

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
    let result = match frame.pixel_format {
        PixelFormat::Rgb888 => rgb_to_rgba(frame),
        PixelFormat::Bgra8888 => bgra_to_rgba(frame),
        _ => Err(format!(
            "unsupported pixel format: {:?}",
            frame.pixel_format
        )),
    };
    match result {
        Ok(pixels) => Handle::from_rgba(frame.width, frame.height, pixels),
        Err(_) => Handle::from_rgba(1, 1, vec![0, 0, 0, 0]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipkvm_video::{MonotonicTimestamp, PixelFormat, VideoFrame};
    use std::sync::Arc;

    fn bgra_frame(data: Vec<u8>, width: u32, height: u32, stride: u32) -> VideoFrame {
        VideoFrame::new(
            1,
            MonotonicTimestamp::from_nanos(1),
            width,
            height,
            stride,
            PixelFormat::Bgra8888,
            Arc::from(data.into_boxed_slice()),
        )
    }

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
    fn bgra_to_rgba_swaps_channels_and_keeps_alpha() {
        let out = bgra_to_rgba(&bgra_frame(vec![10, 20, 30, 255], 1, 1, 4)).unwrap();
        assert_eq!(out, vec![30, 20, 10, 255]);
    }

    #[test]
    fn bgra_to_rgba_honors_stride_padding() {
        let out = bgra_to_rgba(&bgra_frame(
            vec![0, 1, 2, 255, 9, 9, 9, 9, 3, 4, 5, 255, 8, 8, 8, 8],
            1,
            2,
            8,
        ))
        .unwrap();
        assert_eq!(out, vec![2, 1, 0, 255, 5, 4, 3, 255]);
    }

    #[test]
    fn bgra_to_rgba_rejects_short_data() {
        assert!(bgra_to_rgba(&bgra_frame(vec![0, 0, 0], 1, 1, 4)).is_err());
    }

    #[test]
    fn handle_from_frame_never_panics_on_bad_frame() {
        let bad = bgra_frame(vec![0; 3], 1, 1, 4);
        let _ = handle_from_frame(&bad);
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
