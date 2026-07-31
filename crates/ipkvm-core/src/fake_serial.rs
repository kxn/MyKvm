use std::sync::Mutex;

use crate::{Ch9329Frame, SerialResult, SerialStats, SerialWriter};

#[derive(Debug, Default)]
pub struct FakeSerialWriter {
    frames: Mutex<Vec<Ch9329Frame>>,
}

impl FakeSerialWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn written_frames(&self) -> Vec<Ch9329Frame> {
        self.frames
            .lock()
            .expect("fake serial lock poisoned")
            .clone()
    }
}

impl SerialWriter for FakeSerialWriter {
    fn enqueue(&self, frame: Ch9329Frame) -> SerialResult<()> {
        self.frames
            .lock()
            .expect("fake serial lock poisoned")
            .push(frame);
        Ok(())
    }

    fn stats(&self) -> SerialStats {
        let frames = self.frames.lock().expect("fake serial lock poisoned");
        SerialStats {
            frames_written: frames.len() as u64,
            bytes_written: frames
                .iter()
                .map(|frame| frame.as_bytes().len() as u64)
                .sum(),
        }
    }
}
