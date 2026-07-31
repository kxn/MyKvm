use super::pixel_format::RfbPixelFormat;
use super::wire::{write_u16, write_u32};
use crate::{RfbConfigError, RfbSize};

pub(crate) const PROTOCOL_VERSION: &[u8; 12] = b"RFB 003.008\n";
pub(crate) const NONE_SECURITY_TYPES: [u8; 2] = [1, 1];
pub(crate) const SECURITY_RESULT_OK: [u8; 4] = [0, 0, 0, 0];

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
    }
}
