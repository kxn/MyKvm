use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RfbSize {
    width: u16,
    height: u16,
}

impl RfbSize {
    pub fn new(width: u16, height: u16) -> Result<Self, RfbFramebufferError> {
        if width == 0 || height == 0 {
            return Err(RfbFramebufferError::ZeroSize { width, height });
        }
        Ok(Self { width, height })
    }

    pub fn width(self) -> u16 {
        self.width
    }

    pub fn height(self) -> u16 {
        self.height
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RfbRectangle {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl RfbRectangle {
    pub fn intersection(self, size: RfbSize) -> Option<Self> {
        if self.width == 0 || self.height == 0 {
            return None;
        }

        let left = u32::from(self.x);
        let top = u32::from(self.y);
        let frame_right = u32::from(size.width);
        let frame_bottom = u32::from(size.height);
        if left >= frame_right || top >= frame_bottom {
            return None;
        }

        let right = left.checked_add(u32::from(self.width))?.min(frame_right);
        let bottom = top.checked_add(u32::from(self.height))?.min(frame_bottom);
        let width = u16::try_from(right - left).ok()?;
        let height = u16::try_from(bottom - top).ok()?;
        (width != 0 && height != 0).then_some(Self {
            x: self.x,
            y: self.y,
            width,
            height,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BgraFrameView<'a> {
    size: RfbSize,
    stride: usize,
    pixels: &'a [u8],
    byte_span: usize,
}

impl<'a> BgraFrameView<'a> {
    pub fn new(
        size: RfbSize,
        stride: usize,
        pixels: &'a [u8],
    ) -> Result<Self, RfbFramebufferError> {
        let line_bytes = usize::from(size.width)
            .checked_mul(4)
            .ok_or(RfbFramebufferError::SizeOverflow)?;
        if stride < line_bytes {
            return Err(RfbFramebufferError::StrideTooSmall {
                minimum: line_bytes,
                actual: stride,
            });
        }

        let byte_span = usize::from(size.height - 1)
            .checked_mul(stride)
            .and_then(|prefix| prefix.checked_add(line_bytes))
            .ok_or(RfbFramebufferError::SizeOverflow)?;
        if pixels.len() < byte_span {
            return Err(RfbFramebufferError::PixelDataTooShort {
                required: byte_span,
                actual: pixels.len(),
            });
        }

        Ok(Self {
            size,
            stride,
            pixels,
            byte_span,
        })
    }

    pub fn size(self) -> RfbSize {
        self.size
    }

    pub fn stride(self) -> usize {
        self.stride
    }

    pub fn byte_span(self) -> usize {
        self.byte_span
    }

    pub(crate) fn row(self, y: u16) -> &'a [u8] {
        debug_assert!(y < self.size.height);
        let start = usize::from(y) * self.stride;
        let end = start + usize::from(self.size.width) * 4;
        &self.pixels[start..end]
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbFramebufferError {
    #[error("framebuffer size must be non-zero, got {width}x{height}")]
    ZeroSize { width: u16, height: u16 },
    #[error("framebuffer stride {actual} is smaller than required row size {minimum}")]
    StrideTooSmall { minimum: usize, actual: usize },
    #[error("pixel data has {actual} bytes but {required} are required")]
    PixelDataTooShort { required: usize, actual: usize },
    #[error("framebuffer byte size overflow")]
    SizeOverflow,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn size_rejects_zero_dimensions() {
        assert!(matches!(
            RfbSize::new(0, 1080),
            Err(RfbFramebufferError::ZeroSize {
                width: 0,
                height: 1080
            })
        ));
        assert!(RfbSize::new(1920, 0).is_err());
    }

    #[test]
    fn rectangle_intersection_clips_without_u16_overflow() {
        let frame = RfbSize::new(100, 80).unwrap();
        let requested = RfbRectangle {
            x: 90,
            y: 70,
            width: u16::MAX,
            height: u16::MAX,
        };

        assert_eq!(
            requested.intersection(frame),
            Some(RfbRectangle {
                x: 90,
                y: 70,
                width: 10,
                height: 10,
            })
        );
    }

    #[test]
    fn frame_view_accepts_padding_without_requiring_a_final_padding_tail() {
        let size = RfbSize::new(2, 2).unwrap();
        let pixels = [0_u8; 20];
        let frame = BgraFrameView::new(size, 12, &pixels).unwrap();

        assert_eq!(frame.byte_span(), 20);
        assert_eq!(frame.row(1), &[0; 8]);
    }

    #[test]
    fn frame_view_rejects_short_stride_and_short_pixels() {
        let size = RfbSize::new(2, 2).unwrap();
        assert!(matches!(
            BgraFrameView::new(size, 7, &[0; 16]),
            Err(RfbFramebufferError::StrideTooSmall {
                minimum: 8,
                actual: 7
            })
        ));
        assert!(matches!(
            BgraFrameView::new(size, 8, &[0; 15]),
            Err(RfbFramebufferError::PixelDataTooShort {
                required: 16,
                actual: 15
            })
        ));
    }

    #[test]
    fn frame_view_reports_span_overflow() {
        let size = RfbSize::new(2, 2).unwrap();
        assert!(matches!(
            BgraFrameView::new(size, usize::MAX, &[]),
            Err(RfbFramebufferError::SizeOverflow)
        ));
    }

    proptest! {
        #[test]
        fn rectangle_intersection_stays_inside_frame(
            frame_width in 1_u16..=u16::MAX,
            frame_height in 1_u16..=u16::MAX,
            x in any::<u16>(),
            y in any::<u16>(),
            width in any::<u16>(),
            height in any::<u16>(),
        ) {
            let frame = RfbSize::new(frame_width, frame_height).unwrap();
            let rectangle = RfbRectangle { x, y, width, height };
            if let Some(intersection) = rectangle.intersection(frame) {
                prop_assert!(
                    u32::from(intersection.x) + u32::from(intersection.width)
                        <= u32::from(frame_width)
                );
                prop_assert!(
                    u32::from(intersection.y) + u32::from(intersection.height)
                        <= u32::from(frame_height)
                );
            }
        }
    }
}
