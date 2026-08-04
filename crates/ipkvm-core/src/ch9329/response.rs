use thiserror::Error;

use super::Ch9329Frame;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LockLedState {
    pub num_lock: bool,
    pub caps_lock: bool,
    pub scroll_lock: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ch9329Info {
    pub version: u8,
    pub usb_enumerated: bool,
    pub leds: LockLedState,
    pub reserved: [u8; 5],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandStatus {
    Success,
    SerialTimeout,
    InvalidHeader,
    InvalidCommand,
    ChecksumError,
    InvalidParameters,
    OperationFailed,
    Unknown(u8),
}

impl From<u8> for CommandStatus {
    fn from(value: u8) -> Self {
        match value {
            0x00 => Self::Success,
            0xe1 => Self::SerialTimeout,
            0xe2 => Self::InvalidHeader,
            0xe3 => Self::InvalidCommand,
            0xe4 => Self::ChecksumError,
            0xe5 => Self::InvalidParameters,
            0xe6 => Self::OperationFailed,
            value => Self::Unknown(value),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Ch9329Response {
    Info(Ch9329Info),
    Acknowledgement { command: u8, status: CommandStatus },
    Error { command: u8, status: CommandStatus },
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum Ch9329ResponseError {
    #[error("unexpected CH9329 response command: {0:#04x}")]
    UnexpectedCommand(u8),
    #[error(
        "CH9329 response command {command:#04x} requires {expected} data bytes but received {actual}"
    )]
    InvalidDataLength {
        command: u8,
        expected: usize,
        actual: usize,
    },
}

impl Ch9329Response {
    pub fn parse(frame: &Ch9329Frame) -> Result<Self, Ch9329ResponseError> {
        let command = frame.command();
        match command {
            0x81 => parse_info(frame),
            0x82 | 0x84 | 0x85 | 0x8f => {
                require_data_length(frame, 1)?;
                Ok(Self::Acknowledgement {
                    command: command & 0x7f,
                    status: frame.data()[0].into(),
                })
            }
            0xc1 | 0xc2 | 0xc4 | 0xc5 | 0xcf => {
                require_data_length(frame, 1)?;
                Ok(Self::Error {
                    command: command & 0x3f,
                    status: frame.data()[0].into(),
                })
            }
            _ => Err(Ch9329ResponseError::UnexpectedCommand(command)),
        }
    }
}

fn parse_info(frame: &Ch9329Frame) -> Result<Ch9329Response, Ch9329ResponseError> {
    require_data_length(frame, 8)?;
    let data = frame.data();
    let led_bits = data[2];
    let mut reserved = [0; 5];
    reserved.copy_from_slice(&data[3..]);
    Ok(Ch9329Response::Info(Ch9329Info {
        version: data[0],
        usb_enumerated: data[1] != 0,
        leds: LockLedState {
            num_lock: led_bits & 0x01 != 0,
            caps_lock: led_bits & 0x02 != 0,
            scroll_lock: led_bits & 0x04 != 0,
        },
        reserved,
    }))
}

fn require_data_length(frame: &Ch9329Frame, expected: usize) -> Result<(), Ch9329ResponseError> {
    let actual = frame.data().len();
    if actual != expected {
        return Err(Ch9329ResponseError::InvalidDataLength {
            command: frame.command(),
            expected,
            actual,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_get_info_led_state() {
        let frame = Ch9329Frame::new(0, 0x81, &[0x31, 1, 0b0000_0011, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(
            Ch9329Response::parse(&frame).unwrap(),
            Ch9329Response::Info(Ch9329Info {
                version: 0x31,
                usb_enumerated: true,
                leds: LockLedState {
                    num_lock: true,
                    caps_lock: true,
                    scroll_lock: false,
                },
                reserved: [0; 5],
            })
        );
    }

    #[test]
    fn parses_keyboard_acknowledgement() {
        let frame = Ch9329Frame::new(0, 0x82, &[0]).unwrap();
        assert_eq!(
            Ch9329Response::parse(&frame).unwrap(),
            Ch9329Response::Acknowledgement {
                command: 2,
                status: CommandStatus::Success,
            }
        );
    }

    #[test]
    fn preserves_unknown_status_code() {
        let frame = Ch9329Frame::new(0, 0xc2, &[0xaa]).unwrap();
        assert!(matches!(
            Ch9329Response::parse(&frame),
            Ok(Ch9329Response::Error {
                command: 2,
                status: CommandStatus::Unknown(0xaa)
            })
        ));
    }

    #[test]
    fn rejects_response_with_wrong_data_length() {
        let frame = Ch9329Frame::new(0, 0x82, &[]).unwrap();
        assert_eq!(
            Ch9329Response::parse(&frame),
            Err(Ch9329ResponseError::InvalidDataLength {
                command: 0x82,
                expected: 1,
                actual: 0,
            })
        );
    }

    #[test]
    fn rejects_unknown_response_command() {
        let frame = Ch9329Frame::new(0, 0x83, &[0]).unwrap();
        assert_eq!(
            Ch9329Response::parse(&frame),
            Err(Ch9329ResponseError::UnexpectedCommand(0x83))
        );
    }

    #[test]
    fn parses_reset_ack_and_error() {
        let ok = Ch9329Frame::new(0, 0x8f, &[0]).unwrap();
        let error = Ch9329Frame::new(0, 0xcf, &[0xe6]).unwrap();

        assert_eq!(
            Ch9329Response::parse(&ok).unwrap(),
            Ch9329Response::Acknowledgement {
                command: 0x0f,
                status: CommandStatus::Success,
            }
        );
        assert_eq!(
            Ch9329Response::parse(&error).unwrap(),
            Ch9329Response::Error {
                command: 0x0f,
                status: CommandStatus::OperationFailed,
            }
        );
    }
}
