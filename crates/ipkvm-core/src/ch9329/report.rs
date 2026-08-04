use thiserror::Error;

use super::{Ch9329Frame, Ch9329FrameError};

const VALID_BUTTON_BITS: u8 = 0b0000_0111;
const MAX_ABSOLUTE_COORDINATE: u16 = 4095;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum Ch9329ReportError {
    #[error("invalid CH9329 mouse button mask: {0:#04x}")]
    InvalidButtonMask(u8),
    #[error("CH9329 absolute {axis} coordinate is out of range: {value}")]
    CoordinateOutOfRange { axis: &'static str, value: u16 },
    #[error("CH9329 relative {field} value is out of range: {value}")]
    RelativeValueOutOfRange { field: &'static str, value: i8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyboardReport {
    modifiers: u8,
    keys: [u8; 6],
}

impl KeyboardReport {
    pub fn new(modifiers: u8, keys: [u8; 6]) -> Self {
        Self { modifiers, keys }
    }

    pub fn data(&self) -> [u8; 8] {
        let mut data = [0; 8];
        data[0] = self.modifiers;
        data[2..].copy_from_slice(&self.keys);
        data
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AbsoluteMouseReport {
    buttons: u8,
    x: u16,
    y: u16,
    wheel: i8,
}

impl AbsoluteMouseReport {
    pub fn new(buttons: u8, x: u16, y: u16, wheel: i8) -> Result<Self, Ch9329ReportError> {
        validate_buttons(buttons)?;
        validate_coordinate("x", x)?;
        validate_coordinate("y", y)?;
        validate_relative_value("wheel", wheel)?;
        Ok(Self {
            buttons,
            x,
            y,
            wheel,
        })
    }

    pub fn data(&self) -> [u8; 7] {
        let [x_low, x_high] = self.x.to_le_bytes();
        let [y_low, y_high] = self.y.to_le_bytes();
        [
            0x02,
            self.buttons,
            x_low,
            x_high,
            y_low,
            y_high,
            self.wheel as u8,
        ]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RelativeMouseReport {
    buttons: u8,
    dx: i8,
    dy: i8,
    wheel: i8,
}

impl RelativeMouseReport {
    pub fn new(buttons: u8, dx: i8, dy: i8, wheel: i8) -> Result<Self, Ch9329ReportError> {
        validate_buttons(buttons)?;
        validate_relative_value("dx", dx)?;
        validate_relative_value("dy", dy)?;
        validate_relative_value("wheel", wheel)?;
        Ok(Self {
            buttons,
            dx,
            dy,
            wheel,
        })
    }

    pub fn data(&self) -> [u8; 5] {
        [
            0x01,
            self.buttons,
            self.dx as u8,
            self.dy as u8,
            self.wheel as u8,
        ]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Ch9329Command {
    GetInfo,
    Reset,
    Keyboard(KeyboardReport),
    MouseAbsolute(AbsoluteMouseReport),
    MouseRelative(RelativeMouseReport),
}

impl Ch9329Command {
    pub fn to_frame(&self, address: u8) -> Result<Ch9329Frame, Ch9329FrameError> {
        match self {
            Self::GetInfo => Ch9329Frame::new(address, 0x01, &[]),
            Self::Reset => Ch9329Frame::new(address, 0x0f, &[]),
            Self::Keyboard(report) => Ch9329Frame::new(address, 0x02, &report.data()),
            Self::MouseAbsolute(report) => Ch9329Frame::new(address, 0x04, &report.data()),
            Self::MouseRelative(report) => Ch9329Frame::new(address, 0x05, &report.data()),
        }
    }
}

fn validate_buttons(buttons: u8) -> Result<(), Ch9329ReportError> {
    if buttons & !VALID_BUTTON_BITS != 0 {
        return Err(Ch9329ReportError::InvalidButtonMask(buttons));
    }
    Ok(())
}

fn validate_coordinate(axis: &'static str, value: u16) -> Result<(), Ch9329ReportError> {
    if value > MAX_ABSOLUTE_COORDINATE {
        return Err(Ch9329ReportError::CoordinateOutOfRange { axis, value });
    }
    Ok(())
}

fn validate_relative_value(field: &'static str, value: i8) -> Result<(), Ch9329ReportError> {
    if value == i8::MIN {
        return Err(Ch9329ReportError::RelativeValueOutOfRange { field, value });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_a_matches_vendor_frame() {
        let command = Ch9329Command::Keyboard(KeyboardReport::new(0, [4, 0, 0, 0, 0, 0]));
        assert_eq!(
            command.to_frame(0).unwrap().as_bytes(),
            &[0x57, 0xab, 0, 2, 8, 0, 0, 4, 0, 0, 0, 0, 0, 0x10]
        );
    }

    #[test]
    fn absolute_mouse_matches_vendor_example() {
        let report = AbsoluteMouseReport::new(0, 320, 533, 0).unwrap();
        assert_eq!(
            Ch9329Command::MouseAbsolute(report)
                .to_frame(0)
                .unwrap()
                .as_bytes(),
            &[0x57, 0xab, 0, 4, 7, 2, 0, 0x40, 1, 0x15, 2, 0, 0x67]
        );
    }

    #[test]
    fn rejects_invalid_mouse_button_mask() {
        assert_eq!(
            AbsoluteMouseReport::new(0x08, 0, 0, 0),
            Err(Ch9329ReportError::InvalidButtonMask(0x08))
        );
        assert_eq!(
            RelativeMouseReport::new(0x80, 0, 0, 0),
            Err(Ch9329ReportError::InvalidButtonMask(0x80))
        );
    }

    #[test]
    fn rejects_absolute_coordinates_above_protocol_range() {
        assert_eq!(
            AbsoluteMouseReport::new(0, 4096, 0, 0),
            Err(Ch9329ReportError::CoordinateOutOfRange {
                axis: "x",
                value: 4096,
            })
        );
    }

    #[test]
    fn rejects_negative_128_relative_value() {
        assert_eq!(
            RelativeMouseReport::new(0, -128, 0, 0),
            Err(Ch9329ReportError::RelativeValueOutOfRange {
                field: "dx",
                value: -128,
            })
        );
    }

    #[test]
    fn reset_command_matches_vendor_frame() {
        assert_eq!(
            Ch9329Command::Reset.to_frame(0).unwrap().as_bytes(),
            &[0x57, 0xab, 0, 0x0f, 0, 0x11]
        );
    }
}
