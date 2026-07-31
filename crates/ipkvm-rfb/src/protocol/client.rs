use super::pixel_format::{RfbPixelFormat, RfbPixelFormatError};
use super::wire::{read_i32, read_u16, read_u32};
use crate::{RfbProtocolLimits, RfbRectangle};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FramebufferUpdateRequest {
    pub incremental: bool,
    pub rectangle: RfbRectangle,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbProtocolError {
    #[error("unsupported RFB protocol version {0:?}")]
    UnsupportedVersion([u8; 12]),
    #[error("unsupported RFB security type {0}")]
    UnsupportedSecurityType(u8),
    #[error("unsupported client message type {0}")]
    UnsupportedClientMessageType(u8),
    #[error("client declared {declared} encodings, maximum is {maximum}")]
    TooManyEncodings { declared: usize, maximum: usize },
    #[error("client cut text has {declared} bytes, maximum is {maximum}")]
    CutTextTooLong { declared: usize, maximum: usize },
    #[error("input buffer would grow to {attempted} bytes, maximum is {maximum}")]
    InputBufferLimitExceeded { attempted: usize, maximum: usize },
    #[error("invalid client pixel format: {0}")]
    InvalidPixelFormat(#[from] RfbPixelFormatError),
    #[error("protocol message length overflow")]
    LengthOverflow,
    #[error("connection is already in the failed state")]
    ConnectionFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ClientMessage {
    SetPixelFormat(RfbPixelFormat),
    SetEncodings(Vec<i32>),
    FramebufferUpdateRequest(FramebufferUpdateRequest),
    Key {
        down: bool,
        keysym: u32,
    },
    Pointer {
        button_mask: u8,
        x: u16,
        y: u16,
    },
    CutText(Vec<u8>),
    EnableContinuousUpdates {
        enable: bool,
        rectangle: RfbRectangle,
    },
}

pub(crate) struct ClientMessageDecoder {
    limits: RfbProtocolLimits,
    buffer: Vec<u8>,
    failed: bool,
}

impl ClientMessageDecoder {
    pub(crate) fn new(limits: RfbProtocolLimits) -> Self {
        Self {
            limits,
            buffer: Vec::new(),
            failed: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn buffered_len(&self) -> usize {
        self.buffer.len()
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<Result<ClientMessage, RfbProtocolError>> {
        if self.failed {
            return vec![Err(RfbProtocolError::ConnectionFailed)];
        }

        let Some(attempted) = self.buffer.len().checked_add(bytes.len()) else {
            self.failed = true;
            self.buffer.clear();
            return vec![Err(RfbProtocolError::LengthOverflow)];
        };
        if attempted > self.limits.max_buffered_input_bytes {
            self.failed = true;
            self.buffer.clear();
            return vec![Err(RfbProtocolError::InputBufferLimitExceeded {
                attempted,
                maximum: self.limits.max_buffered_input_bytes,
            })];
        }

        self.buffer.extend_from_slice(bytes);
        let mut results = Vec::new();
        let mut consumed = 0;
        while consumed < self.buffer.len() {
            match decode_one(&self.buffer[consumed..], self.limits) {
                DecodeOne::Incomplete => break,
                DecodeOne::Message { value, length } => {
                    results.push(Ok(value));
                    consumed += length;
                }
                DecodeOne::Error(error) => {
                    results.push(Err(error));
                    self.failed = true;
                    self.buffer.clear();
                    return results;
                }
            }
        }
        if consumed != 0 {
            self.buffer.drain(..consumed);
        }
        results
    }
}

enum DecodeOne {
    Incomplete,
    Message { value: ClientMessage, length: usize },
    Error(RfbProtocolError),
}

fn decode_one(bytes: &[u8], limits: RfbProtocolLimits) -> DecodeOne {
    let Some(message_type) = bytes.first().copied() else {
        return DecodeOne::Incomplete;
    };
    match message_type {
        0 => decode_pixel_format(bytes),
        2 => decode_encodings(bytes, limits),
        3 => decode_update_request(bytes),
        4 => decode_key(bytes),
        5 => decode_pointer(bytes),
        6 => decode_cut_text(bytes, limits),
        150 => decode_continuous_updates(bytes),
        other => DecodeOne::Error(RfbProtocolError::UnsupportedClientMessageType(other)),
    }
}

fn decode_pixel_format(bytes: &[u8]) -> DecodeOne {
    if bytes.len() < 20 {
        return DecodeOne::Incomplete;
    }
    let mut wire = [0_u8; 16];
    wire.copy_from_slice(&bytes[4..20]);
    match RfbPixelFormat::from_wire(&wire) {
        Ok(format) => DecodeOne::Message {
            value: ClientMessage::SetPixelFormat(format),
            length: 20,
        },
        Err(error) => DecodeOne::Error(error.into()),
    }
}

fn decode_encodings(bytes: &[u8], limits: RfbProtocolLimits) -> DecodeOne {
    if bytes.len() < 4 {
        return DecodeOne::Incomplete;
    }
    let Some(count) = read_u16(bytes, 2).map(usize::from) else {
        return DecodeOne::Error(RfbProtocolError::LengthOverflow);
    };
    if count > limits.max_encodings {
        return DecodeOne::Error(RfbProtocolError::TooManyEncodings {
            declared: count,
            maximum: limits.max_encodings,
        });
    }
    let Some(length) = count
        .checked_mul(4)
        .and_then(|body_length| body_length.checked_add(4))
    else {
        return DecodeOne::Error(RfbProtocolError::LengthOverflow);
    };
    if bytes.len() < length {
        return DecodeOne::Incomplete;
    }

    let mut encodings = Vec::with_capacity(count);
    for offset in (4..length).step_by(4) {
        let Some(encoding) = read_i32(bytes, offset) else {
            return DecodeOne::Error(RfbProtocolError::LengthOverflow);
        };
        encodings.push(encoding);
    }
    DecodeOne::Message {
        value: ClientMessage::SetEncodings(encodings),
        length,
    }
}

fn decode_cut_text(bytes: &[u8], limits: RfbProtocolLimits) -> DecodeOne {
    if bytes.len() < 8 {
        return DecodeOne::Incomplete;
    }
    let Some(body_length) = read_u32(bytes, 4).and_then(|value| usize::try_from(value).ok()) else {
        return DecodeOne::Error(RfbProtocolError::LengthOverflow);
    };
    if body_length > limits.max_cut_text_bytes {
        return DecodeOne::Error(RfbProtocolError::CutTextTooLong {
            declared: body_length,
            maximum: limits.max_cut_text_bytes,
        });
    }
    let Some(length) = body_length.checked_add(8) else {
        return DecodeOne::Error(RfbProtocolError::LengthOverflow);
    };
    if bytes.len() < length {
        return DecodeOne::Incomplete;
    }
    DecodeOne::Message {
        value: ClientMessage::CutText(bytes[8..length].to_vec()),
        length,
    }
}

fn decode_update_request(bytes: &[u8]) -> DecodeOne {
    if bytes.len() < 10 {
        return DecodeOne::Incomplete;
    }
    let Some(rectangle) = read_rectangle(bytes, 2) else {
        return DecodeOne::Error(RfbProtocolError::LengthOverflow);
    };
    DecodeOne::Message {
        value: ClientMessage::FramebufferUpdateRequest(FramebufferUpdateRequest {
            incremental: bytes[1] != 0,
            rectangle,
        }),
        length: 10,
    }
}

fn decode_key(bytes: &[u8]) -> DecodeOne {
    if bytes.len() < 8 {
        return DecodeOne::Incomplete;
    }
    let Some(keysym) = read_u32(bytes, 4) else {
        return DecodeOne::Error(RfbProtocolError::LengthOverflow);
    };
    DecodeOne::Message {
        value: ClientMessage::Key {
            down: bytes[1] != 0,
            keysym,
        },
        length: 8,
    }
}

fn decode_pointer(bytes: &[u8]) -> DecodeOne {
    if bytes.len() < 6 {
        return DecodeOne::Incomplete;
    }
    let (Some(x), Some(y)) = (read_u16(bytes, 2), read_u16(bytes, 4)) else {
        return DecodeOne::Error(RfbProtocolError::LengthOverflow);
    };
    DecodeOne::Message {
        value: ClientMessage::Pointer {
            button_mask: bytes[1],
            x,
            y,
        },
        length: 6,
    }
}

fn decode_continuous_updates(bytes: &[u8]) -> DecodeOne {
    if bytes.len() < 10 {
        return DecodeOne::Incomplete;
    }
    let Some(rectangle) = read_rectangle(bytes, 2) else {
        return DecodeOne::Error(RfbProtocolError::LengthOverflow);
    };
    DecodeOne::Message {
        value: ClientMessage::EnableContinuousUpdates {
            enable: bytes[1] != 0,
            rectangle,
        },
        length: 10,
    }
}

fn read_rectangle(bytes: &[u8], offset: usize) -> Option<RfbRectangle> {
    Some(RfbRectangle {
        x: read_u16(bytes, offset)?,
        y: read_u16(bytes, offset.checked_add(2)?)?,
        width: read_u16(bytes, offset.checked_add(4)?)?,
        height: read_u16(bytes, offset.checked_add(6)?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn representative_client_messages() -> Vec<Vec<u8>> {
        let mut pixel_format = vec![0, 9, 8, 7];
        pixel_format.extend_from_slice(&RfbPixelFormat::default_bgrx8888().to_wire());

        let mut encodings = vec![2, 0, 0, 3];
        encodings.extend_from_slice(&0_i32.to_be_bytes());
        encodings.extend_from_slice(&(-223_i32).to_be_bytes());
        encodings.extend_from_slice(&12_345_i32.to_be_bytes());

        vec![
            pixel_format,
            encodings,
            vec![3, 1, 0, 1, 0, 2, 0, 3, 0, 4],
            vec![4, 1, 0, 0, 0, 0, 0xff, 0x0d],
            vec![5, 3, 0, 10, 0, 20],
            vec![6, 0, 0, 0, 0, 0, 0, 2, 0x41, 0xff],
            vec![150, 1, 0, 5, 0, 6, 0, 7, 0, 8],
        ]
    }

    fn representative_client_message_stream() -> Vec<u8> {
        representative_client_messages()
            .into_iter()
            .flatten()
            .collect()
    }

    #[test]
    fn decodes_fixed_messages_and_nonzero_booleans() {
        let bytes = [
            3,
            2,
            0,
            1,
            0,
            2,
            0,
            3,
            0,
            4,
            4,
            1,
            0,
            0,
            0,
            0,
            0xff,
            0x0d,
            5,
            0b0000_0101,
            0,
            10,
            0,
            20,
            150,
            7,
            0,
            1,
            0,
            2,
            0,
            3,
            0,
            4,
        ];
        let mut decoder = ClientMessageDecoder::new(RfbProtocolLimits::default());
        let messages = decoder.push(&bytes);

        assert_eq!(
            messages,
            vec![
                Ok(ClientMessage::FramebufferUpdateRequest(
                    FramebufferUpdateRequest {
                        incremental: true,
                        rectangle: RfbRectangle {
                            x: 1,
                            y: 2,
                            width: 3,
                            height: 4,
                        },
                    }
                )),
                Ok(ClientMessage::Key {
                    down: true,
                    keysym: 0xff0d,
                }),
                Ok(ClientMessage::Pointer {
                    button_mask: 5,
                    x: 10,
                    y: 20,
                }),
                Ok(ClientMessage::EnableContinuousUpdates {
                    enable: true,
                    rectangle: RfbRectangle {
                        x: 1,
                        y: 2,
                        width: 3,
                        height: 4,
                    },
                }),
            ]
        );
    }

    #[test]
    fn decodes_set_pixel_format_set_encodings_and_cut_text() {
        let mut bytes = vec![0, 9, 8, 7];
        bytes.extend_from_slice(&RfbPixelFormat::default_bgrx8888().to_wire());
        bytes.extend_from_slice(&[
            2, 0, 0, 3, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0x21, 0xff, 0xff, 0xfe, 0xc7, 6, 1, 2, 3, 0,
            0, 0, 3, 0x41, 0x80, 0xff,
        ]);

        let mut decoder = ClientMessageDecoder::new(RfbProtocolLimits::default());
        assert_eq!(
            decoder.push(&bytes),
            vec![
                Ok(ClientMessage::SetPixelFormat(
                    RfbPixelFormat::default_bgrx8888()
                )),
                Ok(ClientMessage::SetEncodings(vec![0, -223, -313])),
                Ok(ClientMessage::CutText(vec![0x41, 0x80, 0xff])),
            ]
        );
    }

    #[test]
    fn waits_for_complete_variable_body() {
        let mut decoder = ClientMessageDecoder::new(RfbProtocolLimits::default());
        assert!(decoder.push(&[2, 0, 0, 1, 0, 0]).is_empty());
        assert_eq!(decoder.buffered_len(), 6);
        assert_eq!(
            decoder.push(&[0, 0]),
            vec![Ok(ClientMessage::SetEncodings(vec![0]))]
        );
    }

    #[test]
    fn returns_complete_messages_and_keeps_an_incomplete_tail() {
        let mut decoder = ClientMessageDecoder::new(RfbProtocolLimits::default());
        let mut bytes = vec![4, 1, 0, 0, 0, 0, 0xff, 0x0d];
        bytes.extend_from_slice(&[5, 3, 0]);

        assert_eq!(
            decoder.push(&bytes),
            vec![Ok(ClientMessage::Key {
                down: true,
                keysym: 0xff0d,
            })]
        );
        assert_eq!(decoder.buffered_len(), 3);
        assert_eq!(
            decoder.push(&[10, 0, 20]),
            vec![Ok(ClientMessage::Pointer {
                button_mask: 3,
                x: 10,
                y: 20,
            })]
        );
    }

    #[test]
    fn rejects_oversized_lengths_before_waiting_for_bodies() {
        let limits = RfbProtocolLimits {
            max_encodings: 1,
            max_cut_text_bytes: 2,
            ..RfbProtocolLimits::default()
        };
        let mut encodings = ClientMessageDecoder::new(limits);
        assert_eq!(
            encodings.push(&[2, 0, 0, 2]),
            vec![Err(RfbProtocolError::TooManyEncodings {
                declared: 2,
                maximum: 1,
            })]
        );

        let mut cut_text = ClientMessageDecoder::new(limits);
        assert_eq!(
            cut_text.push(&[6, 0, 0, 0, 0, 0, 0, 3]),
            vec![Err(RfbProtocolError::CutTextTooLong {
                declared: 3,
                maximum: 2,
            })]
        );
    }

    #[test]
    fn input_limit_is_checked_before_append() {
        let limits = RfbProtocolLimits {
            max_buffered_input_bytes: 5,
            ..RfbProtocolLimits::default()
        };
        let mut decoder = ClientMessageDecoder::new(limits);
        assert_eq!(
            decoder.push(&[5, 0, 0, 0, 0, 0]),
            vec![Err(RfbProtocolError::InputBufferLimitExceeded {
                attempted: 6,
                maximum: 5,
            })]
        );
        assert_eq!(decoder.buffered_len(), 0);
    }

    #[test]
    fn unknown_type_is_fatal_and_does_not_resynchronize() {
        let mut decoder = ClientMessageDecoder::new(RfbProtocolLimits::default());
        assert_eq!(
            decoder.push(&[99, 4, 1, 0, 0, 0, 0, 0xff, 0x0d]),
            vec![Err(RfbProtocolError::UnsupportedClientMessageType(99))]
        );
        assert_eq!(
            decoder.push(&[4, 1, 0, 0, 0, 0, 0xff, 0x0d]),
            vec![Err(RfbProtocolError::ConnectionFailed)]
        );
    }

    #[test]
    fn invalid_pixel_format_is_wrapped_as_a_fatal_protocol_error() {
        let mut wire = RfbPixelFormat::default_bgrx8888().to_wire();
        wire[3] = 0;
        let mut message = vec![0, 0, 0, 0];
        message.extend_from_slice(&wire);
        let mut decoder = ClientMessageDecoder::new(RfbProtocolLimits::default());

        assert_eq!(
            decoder.push(&message),
            vec![Err(RfbProtocolError::InvalidPixelFormat(
                RfbPixelFormatError::ColorMapUnsupported
            ))]
        );
        assert_eq!(
            decoder.push(&[]),
            vec![Err(RfbProtocolError::ConnectionFailed)]
        );
    }

    #[test]
    fn every_message_decodes_at_every_split_boundary() {
        for bytes in representative_client_messages() {
            let mut single = ClientMessageDecoder::new(RfbProtocolLimits::default());
            let expected = single.push(&bytes);
            for split in 0..=bytes.len() {
                let mut chunked = ClientMessageDecoder::new(RfbProtocolLimits::default());
                let mut actual = chunked.push(&bytes[..split]);
                actual.extend(chunked.push(&bytes[split..]));
                assert_eq!(actual, expected, "split={split}, bytes={bytes:?}");
            }
        }
    }

    proptest! {
        #[test]
        fn arbitrary_chunking_matches_single_push(
            chunks in proptest::collection::vec(1_usize..16, 1..64)
        ) {
            let bytes = representative_client_message_stream();
            let mut single = ClientMessageDecoder::new(RfbProtocolLimits::default());
            let expected = single.push(&bytes);

            let mut chunked = ClientMessageDecoder::new(RfbProtocolLimits::default());
            let mut actual = Vec::new();
            let mut offset = 0;
            for requested in chunks {
                if offset == bytes.len() {
                    break;
                }
                let end = offset.saturating_add(requested).min(bytes.len());
                actual.extend(chunked.push(&bytes[offset..end]));
                offset = end;
            }
            actual.extend(chunked.push(&bytes[offset..]));

            prop_assert_eq!(actual, expected);
        }

        #[test]
        fn random_client_bytes_never_panic(
            bytes in proptest::collection::vec(any::<u8>(), 0..4096)
        ) {
            let limits = RfbProtocolLimits::default();
            let mut decoder = ClientMessageDecoder::new(limits);
            let _ = decoder.push(&bytes);
            prop_assert!(decoder.buffered_len() <= limits.max_buffered_input_bytes);
        }
    }
}
