use ipkvm_rfb::{BgraFrameView, RfbSize};
use ipkvm_video::{PixelFormat, VideoFrame};

use super::RfbFrameError;

pub(super) fn frame_view(frame: &VideoFrame) -> Result<BgraFrameView<'_>, RfbFrameError> {
    if frame.pixel_format != PixelFormat::Bgra8888 {
        return Err(RfbFrameError::UnsupportedPixelFormat(frame.pixel_format));
    }

    let width =
        u16::try_from(frame.width).map_err(|_| RfbFrameError::WidthOutOfRange(frame.width))?;
    let height =
        u16::try_from(frame.height).map_err(|_| RfbFrameError::HeightOutOfRange(frame.height))?;
    let stride =
        usize::try_from(frame.stride).map_err(|_| RfbFrameError::StrideOutOfRange(frame.stride))?;
    let size = RfbSize::new(width, height)?;

    Ok(BgraFrameView::new(size, stride, &frame.data)?)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ipkvm_rfb::{RfbFramebufferError, RfbSize};
    use ipkvm_video::{MonotonicTimestamp, PixelFormat, VideoFrame};

    use super::*;

    fn video_frame(
        seq: u64,
        width: u32,
        height: u32,
        stride: u32,
        pixel_format: PixelFormat,
        data: Vec<u8>,
    ) -> VideoFrame {
        VideoFrame::new(
            seq,
            MonotonicTimestamp::from_nanos(seq),
            width,
            height,
            stride,
            pixel_format,
            Arc::from(data.into_boxed_slice()),
        )
    }

    #[test]
    fn frame_adapter_accepts_padded_bgra() {
        let frame = video_frame(1, 2, 2, 12, PixelFormat::Bgra8888, vec![0; 20]);
        let view = frame_view(&frame).unwrap();

        assert_eq!(view.size(), RfbSize::new(2, 2).unwrap());
        assert_eq!(view.stride(), 12);
    }

    #[test]
    fn frame_adapter_rejects_unsupported_format_and_width() {
        let wrong = video_frame(1, 1, 1, 4, PixelFormat::Mjpeg, vec![0; 4]);
        assert!(matches!(
            frame_view(&wrong),
            Err(RfbFrameError::UnsupportedPixelFormat(PixelFormat::Mjpeg))
        ));

        let wide = video_frame(
            2,
            u32::from(u16::MAX) + 1,
            1,
            4,
            PixelFormat::Bgra8888,
            vec![0; 4],
        );
        assert!(matches!(
            frame_view(&wide),
            Err(RfbFrameError::WidthOutOfRange(_))
        ));
    }

    #[test]
    fn frame_adapter_rejects_invalid_stride_and_data_length() {
        let short_stride = video_frame(3, 2, 1, 7, PixelFormat::Bgra8888, vec![0; 8]);
        assert!(matches!(
            frame_view(&short_stride),
            Err(RfbFrameError::InvalidBgraFrame(
                RfbFramebufferError::StrideTooSmall { .. }
            ))
        ));

        let short_data = video_frame(4, 2, 1, 8, PixelFormat::Bgra8888, vec![0; 7]);
        assert!(matches!(
            frame_view(&short_data),
            Err(RfbFrameError::InvalidBgraFrame(
                RfbFramebufferError::PixelDataTooShort { .. }
            ))
        ));
    }
}
