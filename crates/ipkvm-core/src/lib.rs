mod ch9329;
mod geometry;
mod input;

pub use ch9329::Ch9329Frame;
pub use geometry::{Ch9329Point, Point, ViewRect, map_pointer_to_ch9329};
pub use input::{InputSink, KeyEvent, PointerButton, PointerEvent};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ch9329_frame_uses_header_length_and_sum_checksum() {
        let frame = Ch9329Frame::new(0x00, 0x02, &[0x00, 0x00, 0x04]);

        assert_eq!(
            frame.as_bytes(),
            &[0x57, 0xAB, 0x00, 0x02, 0x03, 0x00, 0x00, 0x04, 0x0B]
        );
    }

    #[test]
    fn pointer_mapping_clamps_to_ch9329_absolute_range() {
        let rect = ViewRect {
            x: 10,
            y: 20,
            width: 100,
            height: 50,
        };

        assert_eq!(
            map_pointer_to_ch9329(Point { x: 10, y: 20 }, rect),
            Ch9329Point { x: 0, y: 0 }
        );
        assert_eq!(
            map_pointer_to_ch9329(Point { x: 110, y: 70 }, rect),
            Ch9329Point { x: 4095, y: 4095 }
        );
    }
}
