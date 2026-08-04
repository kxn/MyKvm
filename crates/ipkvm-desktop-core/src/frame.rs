#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RgbaFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// 把 BGRA8888 帧转成 RGBA8888，处理行首尾的 stride 填充字节。
pub fn bgra_to_rgba(frame: &ipkvm_video::VideoFrame) -> Result<RgbaFrame, String> {
    if frame.pixel_format != ipkvm_video::PixelFormat::Bgra8888 {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ipkvm_video::{MonotonicTimestamp, PixelFormat, VideoFrame};

    use super::*;

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
}
