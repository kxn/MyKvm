use thiserror::Error;

use super::{Ch9329Frame, Ch9329FrameError, MAX_DATA_LEN};

const HEADER_FIRST: u8 = 0x57;
const HEADER_SECOND: u8 = 0xab;
const FRAME_OVERHEAD: usize = 6;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum Ch9329DecodeError {
    #[error("discarded {0} noise bytes before a CH9329 frame")]
    NoiseDiscarded(usize),
    #[error("invalid CH9329 frame")]
    Frame(#[from] Ch9329FrameError),
}

#[derive(Clone, Debug, Default)]
pub struct Ch9329Decoder {
    buffer: Vec<u8>,
}

impl Ch9329Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<Result<Ch9329Frame, Ch9329DecodeError>> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();

        loop {
            if self.buffer.len() < 2 {
                if self
                    .buffer
                    .first()
                    .is_some_and(|byte| *byte != HEADER_FIRST)
                {
                    self.buffer.clear();
                    events.push(Err(Ch9329DecodeError::NoiseDiscarded(1)));
                }
                break;
            }

            let Some(header_position) = find_header(&self.buffer) else {
                let retained = usize::from(self.buffer.last() == Some(&HEADER_FIRST));
                let discarded = self.buffer.len() - retained;
                self.buffer.drain(..discarded);
                if discarded != 0 {
                    events.push(Err(Ch9329DecodeError::NoiseDiscarded(discarded)));
                }
                break;
            };

            if header_position != 0 {
                self.buffer.drain(..header_position);
                events.push(Err(Ch9329DecodeError::NoiseDiscarded(header_position)));
                continue;
            }

            if self.buffer.len() < 5 {
                break;
            }

            let declared = usize::from(self.buffer[4]);
            if declared > MAX_DATA_LEN {
                self.buffer.drain(..1);
                events.push(Err(Ch9329DecodeError::Frame(
                    Ch9329FrameError::DataTooLong(declared),
                )));
                continue;
            }

            let frame_length = FRAME_OVERHEAD + declared;
            if self.buffer.len() < frame_length {
                break;
            }

            match Ch9329Frame::parse(&self.buffer[..frame_length]) {
                Ok(frame) => {
                    self.buffer.drain(..frame_length);
                    events.push(Ok(frame));
                }
                Err(error) => {
                    self.buffer.drain(..1);
                    events.push(Err(Ch9329DecodeError::Frame(error)));
                }
            }
        }

        events
    }

    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }
}

fn find_header(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(2)
        .position(|window| window == [HEADER_FIRST, HEADER_SECOND])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded_frames(events: Vec<Result<Ch9329Frame, Ch9329DecodeError>>) -> Vec<Ch9329Frame> {
        events.into_iter().filter_map(Result::ok).collect()
    }

    #[test]
    fn decodes_frame_at_every_split_boundary() {
        let expected = Ch9329Frame::new(0, 1, &[]).unwrap();
        let bytes = expected.as_bytes().to_vec();
        for split in 0..bytes.len() {
            let mut decoder = Ch9329Decoder::new();
            assert!(decoded_frames(decoder.push(&bytes[..split])).is_empty());
            assert_eq!(
                decoded_frames(decoder.push(&bytes[split..])),
                vec![expected.clone()]
            );
        }
    }

    #[test]
    fn decodes_multiple_frames_from_one_chunk() {
        let first = Ch9329Frame::new(0, 1, &[]).unwrap();
        let second = Ch9329Frame::new(0, 2, &[0; 8]).unwrap();
        let mut bytes = first.as_bytes().to_vec();
        bytes.extend_from_slice(second.as_bytes());
        let mut decoder = Ch9329Decoder::new();
        assert_eq!(decoded_frames(decoder.push(&bytes)), vec![first, second]);
    }

    #[test]
    fn recovers_after_noise_and_bad_checksum() {
        let good = Ch9329Frame::new(0, 1, &[]).unwrap();
        let mut bytes = vec![1, 2, 3, 0x57, 0xab, 0, 1, 0, 0xff];
        bytes.extend_from_slice(good.as_bytes());
        let mut decoder = Ch9329Decoder::new();
        assert_eq!(decoded_frames(decoder.push(&bytes)), vec![good]);
        assert!(decoder.buffered_len() <= 1);
    }

    #[test]
    fn reports_noise_and_bounds_retained_buffer() {
        let mut decoder = Ch9329Decoder::new();
        let events = decoder.push(&vec![0x11; 1024]);
        assert!(matches!(
            events.as_slice(),
            [Err(Ch9329DecodeError::NoiseDiscarded(1024))]
        ));
        assert_eq!(decoder.buffered_len(), 0);

        decoder.push(&[0x11, 0x57]);
        assert_eq!(decoder.buffered_len(), 1);
    }
}
