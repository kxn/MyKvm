use thiserror::Error;

use crate::Ch9329Frame;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SerialError {
    #[error("serial transport is disconnected")]
    Disconnected,
}

pub type SerialResult<T> = Result<T, SerialError>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SerialStats {
    pub frames_written: u64,
    pub bytes_written: u64,
}

pub trait SerialWriter {
    fn enqueue(&self, frame: Ch9329Frame) -> SerialResult<()>;
    fn stats(&self) -> SerialStats;
}
