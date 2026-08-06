use super::pixel_format::RfbPixelFormat;
use super::wire::{write_i32, write_u16, write_u32};
use crate::{BgraFrameView, RfbConfigError, RfbEncodeError, RfbRectangle, RfbSize};

pub(crate) const PROTOCOL_VERSION: &[u8; 12] = b"RFB 003.008\n";
pub(crate) const NONE_SECURITY_TYPES: [u8; 2] = [1, 1];
pub(crate) const VNC_SECURITY_TYPES: [u8; 2] = [1, 2];
pub(crate) const SECURITY_RESULT_OK: [u8; 4] = [0, 0, 0, 0];
pub(crate) const SECURITY_RESULT_FAILED: [u8; 4] = [0, 0, 0, 1];
pub(crate) const AUTH_FAILED_REASON: &[u8] = b"authentication failed";

/// RFB 编码号（RFC 6143）。
pub(crate) const ENCODING_RAW: i32 = 0;
pub(crate) const ENCODING_TIGHT: i32 = 7;

pub(crate) fn encode_server_init(
    size: RfbSize,
    pixel_format: RfbPixelFormat,
    desktop_name: &str,
) -> Result<Vec<u8>, RfbConfigError> {
    let name_length =
        u32::try_from(desktop_name.len()).map_err(|_| RfbConfigError::LimitOverflow)?;
    let capacity = 24_usize
        .checked_add(desktop_name.len())
        .ok_or(RfbConfigError::LimitOverflow)?;
    let mut output = Vec::with_capacity(capacity);
    write_u16(&mut output, size.width());
    write_u16(&mut output, size.height());
    pixel_format.write_wire(&mut output);
    write_u32(&mut output, name_length);
    output.extend_from_slice(desktop_name.as_bytes());
    Ok(output)
}

#[cfg(test)]
pub(crate) fn checked_raw_message_len(
    width: usize,
    height: usize,
    bytes_per_pixel: usize,
) -> Result<usize, RfbEncodeError> {
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
        .and_then(|body| body.checked_add(16))
        .ok_or(RfbEncodeError::LengthOverflow)
}

pub(crate) fn checked_output_len(
    current: usize,
    additional: usize,
) -> Result<usize, RfbEncodeError> {
    current
        .checked_add(additional)
        .ok_or(RfbEncodeError::LengthOverflow)
}

pub(crate) fn encode_empty_update() -> Vec<u8> {
    vec![0, 0, 0, 0]
}

pub(crate) fn encode_desktop_size_update(size: RfbSize) -> Vec<u8> {
    let mut output = Vec::with_capacity(16);
    output.extend_from_slice(&[0, 0, 0, 1]);
    write_u16(&mut output, 0);
    write_u16(&mut output, 0);
    write_u16(&mut output, size.width());
    write_u16(&mut output, size.height());
    write_i32(&mut output, -223);
    output
}

/// 把一帧 BGRA8888 像素的指定矩形编码为 RFB Raw FramebufferUpdate 消息，
/// 追加写入 `output`。返回追加的字节数。
///
/// 调用方负责在调用前用 `checked_raw_message_len` 的结果做容量检查（见
/// `queue_framebuffer_update`）。本函数不重复容量检查。
#[cfg(test)]
pub(crate) fn encode_raw_update(
    frame: BgraFrameView<'_>,
    rectangle: RfbRectangle,
    pixel_format: RfbPixelFormat,
    output: &mut Vec<u8>,
) -> Result<usize, RfbEncodeError> {
    encode_raw_update_multi(
        frame,
        std::slice::from_ref(&rectangle),
        pixel_format,
        output,
    )
}

/// 把一帧 BGRA8888 像素的多个矩形编码为单条 RFB Raw FramebufferUpdate 消息
///（多矩形，num-rectangles = rectangles.len()），追加写入 `output`。
/// 返回追加的字节数。见调研阶段 2.1（issue #21）。
///
/// 调用方负责容量检查。空 rectangles 切片返回错误。
pub(crate) fn encode_raw_update_multi(
    frame: BgraFrameView<'_>,
    rectangles: &[RfbRectangle],
    pixel_format: RfbPixelFormat,
    output: &mut Vec<u8>,
) -> Result<usize, RfbEncodeError> {
    if rectangles.is_empty() {
        return Err(RfbEncodeError::LengthOverflow);
    }
    let num_rects = u16::try_from(rectangles.len()).map_err(|_| RfbEncodeError::LengthOverflow)?;
    let start = output.len();
    // FramebufferUpdate header: message-type=0, num-rectangles=u16 BE。
    output.extend_from_slice(&[0, 0]);
    output.extend_from_slice(&num_rects.to_be_bytes());

    for rectangle in rectangles {
        // 每矩形 header: x, y, width, height (u16 BE), encoding=0 (Raw, i32 BE)。
        write_u16(output, rectangle.x);
        write_u16(output, rectangle.y);
        write_u16(output, rectangle.width);
        write_u16(output, rectangle.height);
        write_i32(output, ENCODING_RAW);

        let end_y = rectangle
            .y
            .checked_add(rectangle.height)
            .ok_or(RfbEncodeError::LengthOverflow)?;
        let start_x = usize::from(rectangle.x)
            .checked_mul(4)
            .ok_or(RfbEncodeError::LengthOverflow)?;
        let end_x = usize::from(rectangle.width)
            .checked_mul(4)
            .and_then(|width| start_x.checked_add(width))
            .ok_or(RfbEncodeError::LengthOverflow)?;

        if pixel_format.is_bgrx8888_le_identity() {
            for y in rectangle.y..end_y {
                for pixel in frame.row(y)[start_x..end_x].chunks_exact(4) {
                    output.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 0]);
                }
            }
        } else {
            for y in rectangle.y..end_y {
                for pixel in frame.row(y)[start_x..end_x].chunks_exact(4) {
                    pixel_format.write_bgr(output, pixel[0], pixel[1], pixel[2]);
                }
            }
        }
    }
    Ok(output.len() - start)
}

/// 把一帧 BGRA8888 像素的多个矩形编码为单条 Tight+JPEG FramebufferUpdate 消息。
///
/// 每矩形：encoding=7（Tight），1 字节压缩控制（高半字节 0x09=JPEG 子编码），
/// Tight 变长长度前缀（JPEG 字节数），JPEG 字节流。
/// noVNC 的 TightDecoder 原生支持解码。见调研阶段 2.2（issue #22）。
pub(crate) fn encode_tight_jpeg_update(
    frame: BgraFrameView<'_>,
    rectangles: &[RfbRectangle],
    jpeg_quality: u8,
    output: &mut Vec<u8>,
) -> Result<usize, RfbEncodeError> {
    if rectangles.is_empty() {
        return Err(RfbEncodeError::LengthOverflow);
    }
    let num_rects = u16::try_from(rectangles.len()).map_err(|_| RfbEncodeError::LengthOverflow)?;
    let start = output.len();
    output.extend_from_slice(&[0, 0]);
    output.extend_from_slice(&num_rects.to_be_bytes());

    let mut jpeg_buf = Vec::new();
    let mut packed = Vec::new();
    for rectangle in rectangles {
        write_u16(output, rectangle.x);
        write_u16(output, rectangle.y);
        write_u16(output, rectangle.width);
        write_u16(output, rectangle.height);
        write_i32(output, ENCODING_TIGHT);

        let w = usize::from(rectangle.width);
        let h = usize::from(rectangle.height);
        let end_y = rectangle
            .y
            .checked_add(rectangle.height)
            .ok_or(RfbEncodeError::LengthOverflow)?;
        let start_x = usize::from(rectangle.x)
            .checked_mul(4)
            .ok_or(RfbEncodeError::LengthOverflow)?;
        let end_x = usize::from(rectangle.width)
            .checked_mul(4)
            .and_then(|width| start_x.checked_add(width))
            .ok_or(RfbEncodeError::LengthOverflow)?;

        // 紧凑拷贝矩形像素（处理 stride）。
        packed.clear();
        packed.reserve(w * h * 4);
        for y in rectangle.y..end_y {
            packed.extend_from_slice(&frame.row(y)[start_x..end_x]);
        }

        // JPEG 编码（BGRA 输入）。
        jpeg_buf.clear();
        jpeg_encoder::Encoder::new(&mut jpeg_buf, jpeg_quality)
            .encode(&packed, w as u16, h as u16, jpeg_encoder::ColorType::Bgra)
            .map_err(|_| RfbEncodeError::LengthOverflow)?;

        // 压缩控制字节：高半字节 0x09=JPEG 子编码，低半字节 0x00（无 zlib reset）。
        // 0x09 << 4 = 0x90。
        output.push(0x90);
        // Tight 变长长度前缀（7-bit 分段）。
        write_tight_length(output, jpeg_buf.len());
        output.extend_from_slice(&jpeg_buf);
    }
    Ok(output.len() - start)
}

/// Tight 变长长度编码：7-bit 分段，高位 0x80 表示继续，最多 4 字节。
fn write_tight_length(output: &mut Vec<u8>, mut value: usize) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value > 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RfbPixelFormat, RfbSize};

    #[test]
    fn server_init_matches_rfb_wire_layout() {
        let bytes = encode_server_init(
            RfbSize::new(640, 480).unwrap(),
            RfbPixelFormat::default_bgrx8888(),
            "机房 KVM",
        )
        .unwrap();

        let mut expected = vec![0x02, 0x80, 0x01, 0xe0];
        expected.extend_from_slice(&RfbPixelFormat::default_bgrx8888().to_wire());
        expected.extend_from_slice(&[0, 0, 0, 10]);
        expected.extend_from_slice("机房 KVM".as_bytes());
        assert_eq!(bytes, expected);
    }

    #[test]
    fn handshake_constants_match_rfb_38_none_security() {
        assert_eq!(PROTOCOL_VERSION, b"RFB 003.008\n");
        assert_eq!(NONE_SECURITY_TYPES, [1, 1]);
        assert_eq!(SECURITY_RESULT_OK, [0, 0, 0, 0]);
        assert_eq!(VNC_SECURITY_TYPES, [1, 2]);
        assert_eq!(SECURITY_RESULT_FAILED, [0, 0, 0, 1]);
    }

    #[test]
    fn raw_and_queue_length_helpers_report_overflow() {
        assert_eq!(
            checked_raw_message_len(usize::MAX, 2, 4),
            Err(RfbEncodeError::LengthOverflow)
        );
        assert_eq!(
            checked_output_len(usize::MAX, 1),
            Err(RfbEncodeError::LengthOverflow)
        );
    }

    /// Fast path：默认 BGRX8888 LE 格式下，encode 输出每像素为 [B,G,R,0]，
    /// alpha 字节（源帧的 A）被丢弃置 0，与逐像素 write_bgr 输出逐字节一致。
    #[test]
    fn raw_fast_path_drops_alpha_and_matches_write_bgr() {
        use crate::framebuffer::BgraFrameView;
        use crate::framebuffer::RfbRectangle;

        // 2x2 像素，alpha 各异（确保 fast path 把 A 置 0 而非透传）。
        let pixels = [
            11, 22, 33, 255, // B,G,R,A
            44, 55, 66, 128, //
            77, 88, 99, 1, //
            100, 110, 120, 200, //
        ];
        let frame = BgraFrameView::new(RfbSize::new(2, 2).unwrap(), 8, &pixels).unwrap();
        let rectangle = RfbRectangle {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        };
        let mut output = Vec::new();
        let written = encode_raw_update(
            frame,
            rectangle,
            RfbPixelFormat::default_bgrx8888(),
            &mut output,
        )
        .unwrap();
        assert_eq!(written, output.len());
        assert_eq!(written, 16 + 2 * 2 * 4);
        // 16B header + 4 像素 × [B,G,R,0]
        assert_eq!(
            output,
            [
                0, 0, 0, 1, // message-type=0, num-rectangles=1
                0, 0, // x
                0, 0, // y
                0, 2, // width
                0, 2, // height
                0, 0, 0, 0, // encoding=0 (Raw)
                11, 22, 33, 0, // 像素0: B,G,R,0（alpha 255 被丢弃）
                44, 55, 66, 0, // 像素1: alpha 128 被丢弃
                77, 88, 99, 0, // 像素2: alpha 1 被丢弃
                100, 110, 120, 0, // 像素3: alpha 200 被丢弃
            ]
        );
    }

    /// 非恒等格式（rgb565）不走 fast path，仍走逐像素 write_bgr（scale_channel）。
    #[test]
    fn raw_non_identity_format_uses_scale_path() {
        use crate::framebuffer::BgraFrameView;
        use crate::framebuffer::RfbRectangle;

        let pixels = [0, 0, 255, 255, 0, 0, 255, 255]; // 2 个纯红像素
        let frame = BgraFrameView::new(RfbSize::new(2, 1).unwrap(), 8, &pixels).unwrap();
        let rgb565 = RfbPixelFormat::new(16, 16, false, 31, 63, 31, 11, 5, 0).unwrap();
        assert!(!rgb565.is_bgrx8888_le_identity());

        let rectangle = RfbRectangle {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
        };
        let mut output = Vec::new();
        encode_raw_update(frame, rectangle, rgb565, &mut output).unwrap();
        // rgb565 纯红 (R=255→31, G=0, B=0): (31<<11) = 0xF800, LE = [0x00, 0xF8]
        assert_eq!(
            &output[16..],
            &[0x00, 0xF8, 0x00, 0xF8],
            "rgb565 非恒等路径应走 scale_channel"
        );
    }

    /// Tight+JPEG 编码：验证 wire 格式（encoding=7、压缩控制 0x90、长度前缀、JPEG 可解码）。
    #[test]
    fn tight_jpeg_update_produces_valid_wire_format() {
        use crate::framebuffer::BgraFrameView;
        use crate::framebuffer::RfbRectangle;

        let pixels = [
            10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255,
        ];
        let frame = BgraFrameView::new(RfbSize::new(2, 2).unwrap(), 8, &pixels).unwrap();
        let rectangle = RfbRectangle {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        };
        let mut output = Vec::new();
        let written = encode_tight_jpeg_update(frame, &[rectangle], 85, &mut output).unwrap();
        assert_eq!(written, output.len());

        // FramebufferUpdate header: message-type=0, num-rectangles=1。
        assert_eq!(&output[..4], &[0, 0, 0, 1]);
        // rect header: x=0, y=0, w=2, h=2, encoding=7 (Tight)。
        assert_eq!(&output[4..6], &[0, 0]); // x
        assert_eq!(&output[6..8], &[0, 0]); // y
        assert_eq!(&output[8..10], &[0, 2]); // width
        assert_eq!(&output[10..12], &[0, 2]); // height
        assert_eq!(
            i32::from_be_bytes([output[12], output[13], output[14], output[15]]),
            ENCODING_TIGHT
        );
        // 压缩控制字节：0x90（高半字节 0x09=JPEG）。
        assert_eq!(output[16], 0x90);
        // 长度前缀后是 JPEG 数据。JPEG 数据应以 SOI marker (0xFF 0xD8) 开头。
        // 长度前缀是变长的——读第一个长度字节。
        let len_byte = output[17];
        let jpeg_len = if len_byte & 0x80 != 0 {
            // 多字节长度：读第二个字节。
            ((len_byte & 0x7f) as usize) | (((output[18] & 0x7f) as usize) << 7)
        } else {
            (len_byte & 0x7f) as usize
        };
        let jpeg_start = if len_byte & 0x80 != 0 { 19 } else { 18 };
        let jpeg_data = &output[jpeg_start..];
        assert_eq!(jpeg_data.len(), jpeg_len);
        // JPEG SOI marker。
        assert_eq!(
            &jpeg_data[..2],
            &[0xFF, 0xD8],
            "JPEG data must start with SOI"
        );
    }

    /// Tight 变长长度编码。
    #[test]
    fn tight_length_encoding_handles_various_sizes() {
        let mut buf = Vec::new();
        write_tight_length(&mut buf, 0);
        assert_eq!(buf, &[0]);

        buf.clear();
        write_tight_length(&mut buf, 127);
        assert_eq!(buf, &[127]);

        buf.clear();
        write_tight_length(&mut buf, 128);
        assert_eq!(buf, &[0x80, 1]);

        buf.clear();
        write_tight_length(&mut buf, 16384);
        assert_eq!(buf, &[0x80, 0x80, 1]);
    }
}
