mod ch9329;
mod geometry;
mod input;
mod serial;

#[cfg(feature = "mock")]
pub mod fake_serial;

pub use ch9329::{Ch9329Error, Ch9329Frame};
pub use geometry::{Ch9329Point, Point, ViewRect, map_pointer_to_ch9329};
pub use input::{
    FramebufferSize, InputError, InputResult, InputSink, KeyEvent, MouseMode, PointerButton,
    PointerEvent,
};
pub use serial::{SerialError, SerialResult, SerialStats, SerialWriter};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ch9329_frame_uses_header_length_and_sum_checksum() {
        let frame = Ch9329Frame::new(0x00, 0x02, &[0x00, 0x00, 0x04]).unwrap();

        assert_eq!(
            frame.as_bytes(),
            &[0x57, 0xAB, 0x00, 0x02, 0x03, 0x00, 0x00, 0x04, 0x0B]
        );
    }

    #[test]
    fn ch9329_frame_rejects_data_longer_than_one_byte_length() {
        let data = vec![0u8; 256];

        assert_eq!(
            Ch9329Frame::new(0x00, 0x02, &data),
            Err(Ch9329Error::DataTooLong(256))
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

    #[derive(Default)]
    struct RecordingSink {
        keys: Vec<KeyEvent>,
        pointers: Vec<PointerEvent>,
        release_count: usize,
    }

    impl InputSink for RecordingSink {
        fn set_mouse_mode(&mut self, _mode: MouseMode) -> InputResult<()> {
            Ok(())
        }

        fn handle_key(&mut self, event: KeyEvent) -> InputResult<()> {
            self.keys.push(event);
            Ok(())
        }

        fn handle_pointer(&mut self, event: PointerEvent) -> InputResult<()> {
            self.pointers.push(event);
            Ok(())
        }

        fn type_text(&mut self, _text: &str) -> InputResult<()> {
            Ok(())
        }

        fn release_all(&mut self) -> InputResult<()> {
            self.release_count += 1;
            Ok(())
        }
    }

    #[test]
    fn input_sink_uses_unified_result_returning_contract() {
        let mut sink = RecordingSink::default();

        sink.handle_key(KeyEvent::Down { hid_usage: 0x04 }).unwrap();
        sink.handle_key(KeyEvent::Up { hid_usage: 0x04 }).unwrap();
        sink.release_all().unwrap();

        assert_eq!(
            sink.keys,
            vec![
                KeyEvent::Down { hid_usage: 0x04 },
                KeyEvent::Up { hid_usage: 0x04 }
            ]
        );
        assert_eq!(sink.release_count, 1);
    }

    #[test]
    fn pointer_move_keeps_framebuffer_pixel_semantics() {
        let mut sink = RecordingSink::default();
        let size = FramebufferSize {
            width: 1920,
            height: 1080,
        };

        sink.handle_pointer(PointerEvent::AbsoluteMove {
            x: 960,
            y: 540,
            framebuffer_size: size,
        })
        .unwrap();

        assert_eq!(
            sink.pointers,
            vec![PointerEvent::AbsoluteMove {
                x: 960,
                y: 540,
                framebuffer_size: size
            }]
        );
    }

    #[cfg(feature = "mock")]
    #[test]
    fn fake_serial_records_frames_in_write_order() {
        use crate::fake_serial::FakeSerialWriter;

        let writer = FakeSerialWriter::new();
        let first = Ch9329Frame::new(0x00, 0x02, &[0x01]).unwrap();
        let second = Ch9329Frame::new(0x00, 0x04, &[0x02]).unwrap();

        writer.enqueue(first.clone()).unwrap();
        writer.enqueue(second.clone()).unwrap();

        assert_eq!(writer.written_frames(), vec![first, second]);
        assert_eq!(
            writer.stats(),
            SerialStats {
                frames_written: 2,
                bytes_written: 14,
            }
        );
    }
}
