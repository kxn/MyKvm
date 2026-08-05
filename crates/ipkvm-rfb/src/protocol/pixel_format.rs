use super::wire::write_u16;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RfbColorChannel {
    Red,
    Green,
    Blue,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbPixelFormatError {
    #[error("unsupported bits-per-pixel value {0}")]
    UnsupportedBitsPerPixel(u8),
    #[error("depth {depth} is invalid for {bits_per_pixel} bits per pixel")]
    InvalidDepth { depth: u8, bits_per_pixel: u8 },
    #[error("color-map pixel formats are not supported")]
    ColorMapUnsupported,
    #[error("invalid {channel:?} channel maximum {value}")]
    InvalidChannelMax {
        channel: RfbColorChannel,
        value: u16,
    },
    #[error("{channel:?} channel exceeds the pixel bit width")]
    ChannelOutOfRange { channel: RfbColorChannel },
    #[error("RGB channel masks overlap")]
    OverlappingChannels,
    #[error("{channel_bits} RGB channel bits exceed declared depth {depth}")]
    ChannelsExceedDepth { channel_bits: u8, depth: u8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RfbPixelFormat {
    bits_per_pixel: u8,
    depth: u8,
    big_endian: bool,
    red_max: u16,
    green_max: u16,
    blue_max: u16,
    red_shift: u8,
    green_shift: u8,
    blue_shift: u8,
}

impl RfbPixelFormat {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        bits_per_pixel: u8,
        depth: u8,
        big_endian: bool,
        red_max: u16,
        green_max: u16,
        blue_max: u16,
        red_shift: u8,
        green_shift: u8,
        blue_shift: u8,
    ) -> Result<Self, RfbPixelFormatError> {
        if !matches!(bits_per_pixel, 8 | 16 | 32) {
            return Err(RfbPixelFormatError::UnsupportedBitsPerPixel(bits_per_pixel));
        }
        if depth == 0 || depth > bits_per_pixel {
            return Err(RfbPixelFormatError::InvalidDepth {
                depth,
                bits_per_pixel,
            });
        }

        let (red_bits, red_mask) =
            channel_bits_and_mask(RfbColorChannel::Red, red_max, red_shift, bits_per_pixel)?;
        let (green_bits, green_mask) = channel_bits_and_mask(
            RfbColorChannel::Green,
            green_max,
            green_shift,
            bits_per_pixel,
        )?;
        let (blue_bits, blue_mask) =
            channel_bits_and_mask(RfbColorChannel::Blue, blue_max, blue_shift, bits_per_pixel)?;

        if red_mask & green_mask != 0 || red_mask & blue_mask != 0 || green_mask & blue_mask != 0 {
            return Err(RfbPixelFormatError::OverlappingChannels);
        }

        let channel_bits = red_bits + green_bits + blue_bits;
        if channel_bits > depth {
            return Err(RfbPixelFormatError::ChannelsExceedDepth {
                channel_bits,
                depth,
            });
        }

        Ok(Self {
            bits_per_pixel,
            depth,
            big_endian,
            red_max,
            green_max,
            blue_max,
            red_shift,
            green_shift,
            blue_shift,
        })
    }

    pub fn default_bgrx8888() -> Self {
        Self {
            bits_per_pixel: 32,
            depth: 24,
            big_endian: false,
            red_max: 255,
            green_max: 255,
            blue_max: 255,
            red_shift: 16,
            green_shift: 8,
            blue_shift: 0,
        }
    }

    pub fn bits_per_pixel(self) -> u8 {
        self.bits_per_pixel
    }

    pub fn depth(self) -> u8 {
        self.depth
    }

    pub fn big_endian(self) -> bool {
        self.big_endian
    }

    pub fn red_max(self) -> u16 {
        self.red_max
    }

    pub fn green_max(self) -> u16 {
        self.green_max
    }

    pub fn blue_max(self) -> u16 {
        self.blue_max
    }

    pub fn red_shift(self) -> u8 {
        self.red_shift
    }

    pub fn green_shift(self) -> u8 {
        self.green_shift
    }

    pub fn blue_shift(self) -> u8 {
        self.blue_shift
    }

    pub(crate) fn bytes_per_pixel(self) -> usize {
        usize::from(self.bits_per_pixel / 8)
    }

    /// 是否为源 BGRA8888（小端 BGRX8888）的恒等编码格式。
    ///
    /// 当目标格式与 `default_bgrx8888()` 完全一致时（32bpp、24bit depth、小端、
    /// 三通道 max=255、blue_shift=0/green_shift=8/red_shift=16），`write_bgr` 的
    /// `scale_channel` 是恒等的（`(v*255+127)/255 == v`），可直接走 fast path
    /// 跳过逐像素乘法，让编译器 autovectorize。见调研阶段 1.1（issue #18）。
    pub(crate) fn is_bgrx8888_le_identity(self) -> bool {
        self.bits_per_pixel == 32
            && self.depth == 24
            && !self.big_endian
            && self.red_max == 255
            && self.green_max == 255
            && self.blue_max == 255
            && self.blue_shift == 0
            && self.green_shift == 8
            && self.red_shift == 16
    }

    pub(crate) fn from_wire(bytes: &[u8; 16]) -> Result<Self, RfbPixelFormatError> {
        if bytes[3] == 0 {
            return Err(RfbPixelFormatError::ColorMapUnsupported);
        }

        Self::new(
            bytes[0],
            bytes[1],
            bytes[2] != 0,
            u16::from_be_bytes([bytes[4], bytes[5]]),
            u16::from_be_bytes([bytes[6], bytes[7]]),
            u16::from_be_bytes([bytes[8], bytes[9]]),
            bytes[10],
            bytes[11],
            bytes[12],
        )
    }

    #[cfg(test)]
    pub(crate) fn to_wire(self) -> [u8; 16] {
        let red_max = self.red_max.to_be_bytes();
        let green_max = self.green_max.to_be_bytes();
        let blue_max = self.blue_max.to_be_bytes();
        [
            self.bits_per_pixel,
            self.depth,
            u8::from(self.big_endian),
            1,
            red_max[0],
            red_max[1],
            green_max[0],
            green_max[1],
            blue_max[0],
            blue_max[1],
            self.red_shift,
            self.green_shift,
            self.blue_shift,
            0,
            0,
            0,
        ]
    }

    pub(crate) fn write_wire(self, output: &mut Vec<u8>) {
        output.extend_from_slice(&[
            self.bits_per_pixel,
            self.depth,
            u8::from(self.big_endian),
            1,
        ]);
        write_u16(output, self.red_max);
        write_u16(output, self.green_max);
        write_u16(output, self.blue_max);
        output.extend_from_slice(&[self.red_shift, self.green_shift, self.blue_shift, 0, 0, 0]);
    }

    pub(crate) fn write_bgr(self, output: &mut Vec<u8>, blue: u8, green: u8, red: u8) {
        let red = scale_channel(red, self.red_max) << self.red_shift;
        let green = scale_channel(green, self.green_max) << self.green_shift;
        let blue = scale_channel(blue, self.blue_max) << self.blue_shift;
        let pixel = red | green | blue;
        let bytes = if self.big_endian {
            pixel.to_be_bytes()
        } else {
            pixel.to_le_bytes()
        };
        let count = usize::from(self.bits_per_pixel / 8);
        if self.big_endian {
            output.extend_from_slice(&bytes[4 - count..]);
        } else {
            output.extend_from_slice(&bytes[..count]);
        }
    }
}

fn channel_bits_and_mask(
    channel: RfbColorChannel,
    maximum: u16,
    shift: u8,
    bits_per_pixel: u8,
) -> Result<(u8, u32), RfbPixelFormatError> {
    let range = u32::from(maximum) + 1;
    if maximum == 0 || !range.is_power_of_two() {
        return Err(RfbPixelFormatError::InvalidChannelMax {
            channel,
            value: maximum,
        });
    }

    let channel_bits =
        u8::try_from(range.ilog2()).map_err(|_| RfbPixelFormatError::InvalidChannelMax {
            channel,
            value: maximum,
        })?;
    if u16::from(shift) + u16::from(channel_bits) > u16::from(bits_per_pixel) {
        return Err(RfbPixelFormatError::ChannelOutOfRange { channel });
    }

    Ok((channel_bits, u32::from(maximum) << shift))
}

fn scale_channel(value: u8, maximum: u16) -> u32 {
    (u32::from(value) * u32::from(maximum) + 127) / 255
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn supported_test_formats() -> [RfbPixelFormat; 4] {
        [
            RfbPixelFormat::default_bgrx8888(),
            RfbPixelFormat::new(32, 24, true, 255, 255, 255, 16, 8, 0).unwrap(),
            RfbPixelFormat::new(16, 16, false, 31, 63, 31, 11, 5, 0).unwrap(),
            RfbPixelFormat::new(8, 8, false, 7, 7, 3, 5, 2, 0).unwrap(),
        ]
    }

    #[test]
    fn default_format_matches_bgrx8888_wire_layout() {
        let format = RfbPixelFormat::default_bgrx8888();
        let mut wire = Vec::new();
        format.write_wire(&mut wire);

        assert_eq!(
            wire,
            [32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0,]
        );
    }

    #[test]
    fn default_format_writes_bgr_zero_bytes() {
        let mut output = Vec::new();
        RfbPixelFormat::default_bgrx8888().write_bgr(&mut output, 0x12, 0x34, 0x56);
        assert_eq!(output, [0x12, 0x34, 0x56, 0]);
    }

    #[test]
    fn exposes_default_format_fields() {
        let format = RfbPixelFormat::default_bgrx8888();
        assert_eq!(format.bits_per_pixel(), 32);
        assert_eq!(format.depth(), 24);
        assert!(!format.big_endian());
        assert_eq!(format.red_max(), 255);
        assert_eq!(format.green_max(), 255);
        assert_eq!(format.blue_max(), 255);
        assert_eq!(format.red_shift(), 16);
        assert_eq!(format.green_shift(), 8);
        assert_eq!(format.blue_shift(), 0);
        assert_eq!(format.bytes_per_pixel(), 4);
        assert_eq!(format.to_wire().as_slice(), {
            let mut bytes = Vec::new();
            format.write_wire(&mut bytes);
            bytes
        });
    }

    #[test]
    fn writes_rgb565_little_endian() {
        let format = RfbPixelFormat::new(16, 16, false, 31, 63, 31, 11, 5, 0).unwrap();
        let mut output = Vec::new();
        format.write_bgr(&mut output, 0, 0, 255);
        assert_eq!(output, [0x00, 0xf8]);
    }

    #[test]
    fn writes_rgb332_and_scales_channels() {
        let format = RfbPixelFormat::new(8, 8, false, 7, 7, 3, 5, 2, 0).unwrap();
        let mut output = Vec::new();
        format.write_bgr(&mut output, 255, 128, 0);
        assert_eq!(output, [0x13]);
    }

    #[test]
    fn writes_32_bit_big_endian() {
        let format = RfbPixelFormat::new(32, 24, true, 255, 255, 255, 16, 8, 0).unwrap();
        let mut output = Vec::new();
        format.write_bgr(&mut output, 0x12, 0x34, 0x56);
        assert_eq!(output, [0, 0x56, 0x34, 0x12]);
    }

    #[test]
    fn rejects_color_map_and_invalid_masks() {
        let mut color_map_wire = RfbPixelFormat::default_bgrx8888().to_wire();
        color_map_wire[3] = 0;
        assert_eq!(
            RfbPixelFormat::from_wire(&color_map_wire),
            Err(RfbPixelFormatError::ColorMapUnsupported)
        );

        assert!(matches!(
            RfbPixelFormat::new(16, 16, false, 30, 63, 31, 11, 5, 0),
            Err(RfbPixelFormatError::InvalidChannelMax {
                channel: RfbColorChannel::Red,
                value: 30
            })
        ));
        assert!(matches!(
            RfbPixelFormat::new(16, 16, false, 31, 63, 31, 10, 5, 0),
            Err(RfbPixelFormatError::OverlappingChannels)
        ));
    }

    #[test]
    fn rejects_unsupported_bits_depth_and_channel_ranges() {
        assert_eq!(
            RfbPixelFormat::new(24, 24, false, 255, 255, 255, 16, 8, 0),
            Err(RfbPixelFormatError::UnsupportedBitsPerPixel(24))
        );
        assert!(matches!(
            RfbPixelFormat::new(16, 0, false, 31, 63, 31, 11, 5, 0),
            Err(RfbPixelFormatError::InvalidDepth {
                depth: 0,
                bits_per_pixel: 16
            })
        ));
        assert!(matches!(
            RfbPixelFormat::new(16, 16, false, 31, 63, 31, 12, 5, 0),
            Err(RfbPixelFormatError::ChannelOutOfRange {
                channel: RfbColorChannel::Red
            })
        ));
        assert!(matches!(
            RfbPixelFormat::new(16, 8, false, 31, 63, 31, 11, 5, 0),
            Err(RfbPixelFormatError::ChannelsExceedDepth {
                channel_bits: 16,
                depth: 8
            })
        ));
    }

    #[test]
    fn honors_endianness_and_rounds_channel_scaling() {
        let little = RfbPixelFormat::new(16, 16, false, 31, 63, 31, 11, 5, 0).unwrap();
        let big = RfbPixelFormat::new(16, 16, true, 31, 63, 31, 11, 5, 0).unwrap();
        let mut little_bytes = Vec::new();
        let mut big_bytes = Vec::new();
        little.write_bgr(&mut little_bytes, 0, 0, 128);
        big.write_bgr(&mut big_bytes, 0, 0, 128);

        assert_eq!(little_bytes, [0x00, 0x80]);
        assert_eq!(big_bytes, [0x80, 0x00]);
    }

    proptest! {
        #[test]
        fn encoded_pixel_length_matches_bits_per_pixel(
            blue in any::<u8>(),
            green in any::<u8>(),
            red in any::<u8>(),
        ) {
            for format in supported_test_formats() {
                let mut output = Vec::new();
                format.write_bgr(&mut output, blue, green, red);
                prop_assert_eq!(output.len(), format.bytes_per_pixel());
            }
        }
    }
}
