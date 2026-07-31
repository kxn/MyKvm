use thiserror::Error;

const HEADER: [u8; 2] = [0x57, 0xab];
const FRAME_OVERHEAD: usize = 6;

pub const MAX_DATA_LEN: usize = 64;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum Ch9329FrameError {
    #[error("CH9329 data length {0} exceeds the 64-byte protocol limit")]
    DataTooLong(usize),
    #[error("CH9329 frame is too short: {0} bytes")]
    FrameTooShort(usize),
    #[error("invalid CH9329 frame header: {0:02x?}")]
    InvalidHeader([u8; 2]),
    #[error("CH9329 frame declares {declared} data bytes but contains {actual}")]
    LengthMismatch { declared: usize, actual: usize },
    #[error("CH9329 checksum mismatch: expected {expected:#04x}, got {actual:#04x}")]
    ChecksumMismatch { expected: u8, actual: u8 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ch9329Frame {
    bytes: Vec<u8>,
}

impl Ch9329Frame {
    pub fn new(address: u8, command: u8, data: &[u8]) -> Result<Self, Ch9329FrameError> {
        if data.len() > MAX_DATA_LEN {
            return Err(Ch9329FrameError::DataTooLong(data.len()));
        }

        let mut bytes = Vec::with_capacity(FRAME_OVERHEAD + data.len());
        bytes.extend_from_slice(&HEADER);
        bytes.push(address);
        bytes.push(command);
        bytes.push(data.len() as u8);
        bytes.extend_from_slice(data);
        bytes.push(checksum(&bytes));
        Ok(Self { bytes })
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, Ch9329FrameError> {
        if bytes.len() < FRAME_OVERHEAD {
            return Err(Ch9329FrameError::FrameTooShort(bytes.len()));
        }

        let header = [bytes[0], bytes[1]];
        if header != HEADER {
            return Err(Ch9329FrameError::InvalidHeader(header));
        }

        let declared = usize::from(bytes[4]);
        if declared > MAX_DATA_LEN {
            return Err(Ch9329FrameError::DataTooLong(declared));
        }

        let actual = bytes.len() - FRAME_OVERHEAD;
        if actual != declared {
            return Err(Ch9329FrameError::LengthMismatch { declared, actual });
        }

        let checksum_index = bytes.len() - 1;
        let expected = checksum(&bytes[..checksum_index]);
        let actual = bytes[checksum_index];
        if actual != expected {
            return Err(Ch9329FrameError::ChecksumMismatch { expected, actual });
        }

        Ok(Self {
            bytes: bytes.to_vec(),
        })
    }

    pub fn address(&self) -> u8 {
        self.bytes[2]
    }

    pub fn command(&self) -> u8 {
        self.bytes[3]
    }

    pub fn data(&self) -> &[u8] {
        &self.bytes[5..self.bytes.len() - 1]
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn rejects_payload_larger_than_protocol_limit() {
        assert_eq!(
            Ch9329Frame::new(0, 2, &[0; 65]),
            Err(Ch9329FrameError::DataTooLong(65))
        );
    }

    #[test]
    fn parses_vendor_keyboard_frame() {
        let bytes = [0x57, 0xab, 0, 2, 8, 0, 0, 4, 0, 0, 0, 0, 0, 0x10];
        let frame = Ch9329Frame::parse(&bytes).unwrap();
        assert_eq!(frame.address(), 0);
        assert_eq!(frame.command(), 2);
        assert_eq!(frame.data(), &[0, 0, 4, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn rejects_bad_checksum() {
        let bytes = [0x57, 0xab, 0, 1, 0, 0xff];
        assert!(matches!(
            Ch9329Frame::parse(&bytes),
            Err(Ch9329FrameError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn rejects_short_invalid_header_and_length_mismatch() {
        assert_eq!(
            Ch9329Frame::parse(&[0x57, 0xab]),
            Err(Ch9329FrameError::FrameTooShort(2))
        );
        assert_eq!(
            Ch9329Frame::parse(&[0, 0, 0, 1, 0, 0]),
            Err(Ch9329FrameError::InvalidHeader([0, 0]))
        );
        assert_eq!(
            Ch9329Frame::parse(&[0x57, 0xab, 0, 1, 1, 0]),
            Err(Ch9329FrameError::LengthMismatch {
                declared: 1,
                actual: 0,
            })
        );
    }

    proptest! {
        #[test]
        fn encoded_frames_round_trip(
            address in any::<u8>(),
            command in any::<u8>(),
            data in proptest::collection::vec(any::<u8>(), 0..=MAX_DATA_LEN),
        ) {
            let frame = Ch9329Frame::new(address, command, &data).unwrap();
            prop_assert_eq!(Ch9329Frame::parse(frame.as_bytes()).unwrap(), frame);
        }
    }
}
