use super::pixel_format::RfbPixelFormat;
use super::wire::{write_i32, write_u16, write_u32};
use crate::{BgraFrameView, RfbConfigError, RfbEncodeError, RfbRectangle, RfbSize};

pub(crate) const PROTOCOL_VERSION: &[u8; 12] = b"RFB 003.008\n";
pub(crate) const NONE_SECURITY_TYPES: [u8; 2] = [1, 1];
pub(crate) const VNC_SECURITY_TYPES: [u8; 2] = [1, 2];
pub(crate) const SECURITY_RESULT_OK: [u8; 4] = [0, 0, 0, 0];
pub(crate) const SECURITY_RESULT_FAILED: [u8; 4] = [0, 0, 0, 1];
pub(crate) const AUTH_FAILED_REASON: &[u8] = b"authentication failed";

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

pub(crate) fn encode_raw_update(
    frame: BgraFrameView<'_>,
    rectangle: RfbRectangle,
    pixel_format: RfbPixelFormat,
) -> Result<Vec<u8>, RfbEncodeError> {
    let length = checked_raw_message_len(
        usize::from(rectangle.width),
        usize::from(rectangle.height),
        pixel_format.bytes_per_pixel(),
    )?;
    let mut output = Vec::with_capacity(length);
    output.extend_from_slice(&[0, 0, 0, 1]);
    write_u16(&mut output, rectangle.x);
    write_u16(&mut output, rectangle.y);
    write_u16(&mut output, rectangle.width);
    write_u16(&mut output, rectangle.height);
    write_i32(&mut output, 0);

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
    for y in rectangle.y..end_y {
        for pixel in frame.row(y)[start_x..end_x].chunks_exact(4) {
            pixel_format.write_bgr(&mut output, pixel[0], pixel[1], pixel[2]);
        }
    }
    debug_assert_eq!(output.len(), length);
    Ok(output)
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
}
