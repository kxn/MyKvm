mod ch9329;
mod geometry;
mod input;
mod serial;

/// CH9329 默认波特率。该协议常量不依赖具体串口实现，因此在无硬件 core 中可用。
pub const DEFAULT_BAUD_RATE: u32 = 9_600;

#[cfg(feature = "serial")]
mod serial_port;

#[cfg(any(feature = "test-support", test))]
pub mod fake_serial;

pub use ch9329::{
    AbsoluteMouseReport, Ch9329Command, Ch9329DecodeError, Ch9329Decoder, Ch9329Frame,
    Ch9329FrameError, Ch9329Info, Ch9329InputSink, Ch9329ReportError, Ch9329Response,
    Ch9329ResponseError, CommandStatus, KeyboardReport, LockLedState, MAX_DATA_LEN,
    RelativeMouseReport,
};
pub use geometry::map_framebuffer_axis;
pub use input::{
    FramebufferSize, InputError, InputResult, InputSink, KeyEvent, KeyboardUsage, MouseMode,
    PointerButton, PointerEvent,
};
pub use serial::{
    CommandBatch, CommandBatchError, CommandQueue, CommandQueueError, CommandQueueResult,
    QueueStats,
};

#[cfg(feature = "serial")]
pub use serial_port::{SerialCommandQueue, SerialCommandQueueError};

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
            Err(Ch9329FrameError::DataTooLong(256))
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

        fn handle_key_batch(&mut self, events: &[KeyEvent]) -> InputResult<()> {
            self.keys.extend_from_slice(events);
            Ok(())
        }

        fn handle_pointer_batch(&mut self, events: &[PointerEvent]) -> InputResult<()> {
            self.pointers.extend_from_slice(events);
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

        let usage = KeyboardUsage::new(0x04).unwrap();
        sink.handle_key(KeyEvent::Down { usage }).unwrap();
        sink.handle_key(KeyEvent::Up { usage }).unwrap();
        sink.release_all().unwrap();

        assert_eq!(
            sink.keys,
            vec![KeyEvent::Down { usage }, KeyEvent::Up { usage }]
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
}
