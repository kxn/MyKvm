//! 视频帧 → RGBA8888 转换：按 `PixelFormat` 分派的统一入口。
//!
//! 支持：
//! - `Rgb888`：逐像素补 alpha=255
//! - `Bgra8888`：通道交换 B/G ↔ R，保留 alpha
//! - `Mjpeg`：原始 JPEG 字节，用 zune-jpeg 解码（MJPEG 硬件直通帧，见 #35 FU-1）
//!
//! 捕获侧在 commit `3493014` 之后只产出 `Mjpeg` 或 `Rgb888`；`Bgra8888` 分支保留
//! 是为了兼容旧路径与单元测试。

use std::io::Cursor;

use ipkvm_video::{PixelFormat, VideoFrame};
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RgbaFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// 按 `pixel_format` 把任意支持的视频帧转成 RGBA8888。
///
/// 这是本地预览 / 截图路径的统一入口。不支持的格式返回 `Err`，调用方据此降级。
pub fn frame_to_rgba(frame: &VideoFrame) -> Result<RgbaFrame, String> {
    match frame.pixel_format {
        PixelFormat::Rgb888 => rgb888_to_rgba(frame).map(|pixels| RgbaFrame {
            width: frame.width,
            height: frame.height,
            pixels,
        }),
        PixelFormat::Bgra8888 => bgra_to_rgba(frame),
        PixelFormat::Mjpeg => mjpeg_to_rgba(frame),
        other => Err(format!("unsupported pixel format: {:?}", other)),
    }
}

/// RGB888 → RGBA8888（补 alpha=255），处理行首 stride 填充。
fn rgb888_to_rgba(frame: &VideoFrame) -> Result<Vec<u8>, String> {
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

/// 把 BGRA8888 帧转成 RGBA8888，处理行首尾的 stride 填充字节。
pub fn bgra_to_rgba(frame: &VideoFrame) -> Result<RgbaFrame, String> {
    if frame.pixel_format != PixelFormat::Bgra8888 {
        return Err(format!(
            "unsupported preview pixel format: {:?}",
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
            "frame data too short: need {required} bytes, got {}",
            frame.data.len()
        ));
    }
    let mut pixels = vec![0; width * height * 4];
    for y in 0..height {
        let src = &frame.data[y * stride..y * stride + width * 4];
        let dst = &mut pixels[y * width * 4..(y + 1) * width * 4];
        for (rgba, bgra) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            rgba.copy_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
        }
    }
    Ok(RgbaFrame {
        width: frame.width,
        height: frame.height,
        pixels,
    })
}

/// 把 MJPEG 帧（原始 JPEG 字节）解码成 RGBA8888。
///
/// 优先信任解码器解析出的真实尺寸（JPEG 头里的 SOF），fallback 到帧元数据尺寸。
/// 这样即便捕获侧把 width/height 填错（MJPEG 帧的 stride 无意义），也能正确解码。
fn mjpeg_to_rgba(frame: &VideoFrame) -> Result<RgbaFrame, String> {
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    // JpegDecoder 接受任何 Read+Seek；用 std Cursor 包装 JPEG 字节即可。
    let mut decoder = JpegDecoder::new_with_options(Cursor::new(&frame.data[..]), options);
    decoder
        .decode_headers()
        .map_err(|e| format!("mjpeg header decode failed: {e:?}"))?;
    let info = decoder
        .info()
        .ok_or_else(|| "mjpeg decoder reported no image info".to_string())?;
    let width = u32::from(info.width);
    let height = u32::from(info.height);
    let expected = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| "mjpeg decoded size overflow".to_string())?;
    let mut pixels = vec![0u8; expected as usize];
    decoder
        .decode_into(&mut pixels)
        .map_err(|e| format!("mjpeg decode failed: {e:?}"))?;
    Ok(RgbaFrame {
        width,
        height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ipkvm_video::{MonotonicTimestamp, PixelFormat, VideoFrame};

    use super::*;

    fn frame(
        width: u32,
        height: u32,
        stride: u32,
        pixel_format: PixelFormat,
        data: Vec<u8>,
    ) -> VideoFrame {
        VideoFrame::new(
            1,
            MonotonicTimestamp::from_nanos(0),
            width,
            height,
            stride,
            pixel_format,
            Arc::from(data.into_boxed_slice()),
        )
    }

    // ---- frame_to_rgba 分派 ----

    #[test]
    fn frame_to_rgba_dispatches_rgb888() {
        let f = frame(1, 1, 3, PixelFormat::Rgb888, vec![10, 20, 30]);
        let rgba = frame_to_rgba(&f).unwrap();
        assert_eq!(rgba.width, 1);
        assert_eq!(rgba.height, 1);
        assert_eq!(rgba.pixels, vec![10, 20, 30, 255]);
    }

    #[test]
    fn frame_to_rgba_dispatches_bgra8888() {
        let f = frame(1, 1, 4, PixelFormat::Bgra8888, vec![0, 1, 2, 255]);
        let rgba = frame_to_rgba(&f).unwrap();
        assert_eq!(rgba.pixels, vec![2, 1, 0, 255]);
    }

    #[test]
    fn frame_to_rgba_dispatches_mjpeg() {
        let jpeg = encode_test_jpeg(2, 2, &[255, 0, 0]); // 红色 2x2
        let f = frame(2, 2, 0, PixelFormat::Mjpeg, jpeg);
        let rgba = frame_to_rgba(&f).unwrap();
        assert_eq!(rgba.width, 2);
        assert_eq!(rgba.height, 2);
        assert_eq!(rgba.pixels.len(), 2 * 2 * 4);
        // JPEG 有损，不精确比较像素值，验证整体为红色（R 高、G/B 低）。
        for chunk in rgba.pixels.chunks_exact(4) {
            assert!(chunk[0] > 200, "R channel too low: {}", chunk[0]);
            assert!(chunk[1] < 60, "G channel too high: {}", chunk[1]);
            assert!(chunk[2] < 60, "B channel too high: {}", chunk[2]);
            assert_eq!(chunk[3], 255, "alpha should be opaque");
        }
    }

    #[test]
    fn frame_to_rgba_rejects_unsupported_format() {
        let f = frame(1, 1, 1, PixelFormat::Yuy2, vec![0, 0, 0]);
        assert!(frame_to_rgba(&f).is_err());
    }

    #[test]
    fn frame_to_rgba_rejects_truncated_rgb() {
        let f = frame(1, 1, 3, PixelFormat::Rgb888, vec![0, 0]);
        assert!(frame_to_rgba(&f).is_err());
    }

    #[test]
    fn frame_to_rgba_rejects_invalid_mjpeg_bytes() {
        let f = frame(1, 1, 0, PixelFormat::Mjpeg, vec![0, 1, 2, 3]);
        assert!(frame_to_rgba(&f).is_err());
    }

    // ---- rgb888_to_rgba stride 填充 ----

    #[test]
    fn rgb888_to_rgba_honors_stride_padding() {
        // 1 像素宽 × 2 行高，stride=6（每行 3 字节有效 + 3 字节填充）
        let data = vec![10, 20, 30, 99, 99, 99, 40, 50, 60, 99, 99, 99];
        let f = frame(1, 2, 6, PixelFormat::Rgb888, data);
        let rgba = rgb888_to_rgba(&f).unwrap();
        assert_eq!(rgba, vec![10, 20, 30, 255, 40, 50, 60, 255]);
    }

    // ---- bgra_to_rgba（保留的旧入口，供 frame_to_rgba 内部调用）----

    #[test]
    fn bgra_to_rgba_swaps_channels() {
        let frame = bgra_frame(1, 1, 4, vec![0, 1, 2, 255]);

        let rgba = bgra_to_rgba(&frame).unwrap();

        assert_eq!(
            rgba,
            RgbaFrame {
                width: 1,
                height: 1,
                pixels: vec![2, 1, 0, 255],
            }
        );
    }

    #[test]
    fn bgra_to_rgba_honors_stride_padding() {
        let frame = bgra_frame(
            1,
            2,
            8,
            vec![0, 1, 2, 255, 9, 9, 9, 9, 3, 4, 5, 255, 8, 8, 8, 8],
        );

        let rgba = bgra_to_rgba(&frame).unwrap();

        assert_eq!(rgba.pixels, vec![2, 1, 0, 255, 5, 4, 3, 255]);
    }

    #[test]
    fn bgra_to_rgba_rejects_unsupported_pixel_format() {
        let frame = VideoFrame::new(
            1,
            MonotonicTimestamp::from_nanos(0),
            1,
            1,
            3,
            PixelFormat::Yuy2,
            Arc::from(vec![0, 1, 2].into_boxed_slice()),
        );

        assert!(bgra_to_rgba(&frame).is_err());
    }

    #[test]
    fn bgra_to_rgba_rejects_truncated_data() {
        let frame = bgra_frame(2, 2, 8, vec![0; 4]);

        assert!(bgra_to_rgba(&frame).is_err());
    }

    fn bgra_frame(width: u32, height: u32, stride: u32, data: Vec<u8>) -> VideoFrame {
        VideoFrame::new(
            1,
            MonotonicTimestamp::from_nanos(0),
            width,
            height,
            stride,
            PixelFormat::Bgra8888,
            Arc::from(data.into_boxed_slice()),
        )
    }

    /// 用 jpeg-encoder 现场编码一张纯色图，作为 MJPEG 帧的测试数据（无外部 fixture）。
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
}
