mod connection;
mod frame;
mod pending;

use std::time::Duration;
use std::{io::ErrorKind, net::SocketAddr};

use ipkvm_rfb::{
    RfbConfigError, RfbEncodeError, RfbFramebufferError, RfbProtocolError, RfbProtocolLimits,
    RfbRectangle,
};
use ipkvm_video::PixelFormat;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RfbClientId(u64);

impl RfbClientId {
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RfbTcpEvent {
    Connected {
        client_id: RfbClientId,
        peer_addr: SocketAddr,
        shared: bool,
    },
    Key {
        client_id: RfbClientId,
        down: bool,
        keysym: u32,
    },
    Pointer {
        client_id: RfbClientId,
        button_mask: u8,
        x: u16,
        y: u16,
    },
    CutText {
        client_id: RfbClientId,
        bytes: Vec<u8>,
    },
    ContinuousUpdates {
        client_id: RfbClientId,
        enable: bool,
        rectangle: RfbRectangle,
    },
    Disconnected {
        client_id: RfbClientId,
        peer_addr: SocketAddr,
        reason: RfbDisconnectReason,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RfbDisconnectReason {
    ClientClosed,
    ServerShutdown,
    HandshakeTimeout,
    CoreConfig(RfbConfigError),
    Protocol(RfbProtocolError),
    Encode(RfbEncodeError),
    Frame(RfbTcpFrameError),
    Io(ErrorKind),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RfbTcpConfig {
    pub desktop_name: String,
    pub read_buffer_bytes: usize,
    pub handshake_timeout: Duration,
    pub protocol_limits: RfbProtocolLimits,
}

impl RfbTcpConfig {
    pub fn validate(&self) -> Result<(), RfbTcpConfigError> {
        if self.read_buffer_bytes == 0 {
            return Err(RfbTcpConfigError::ZeroReadBuffer);
        }
        if self.read_buffer_bytes > self.protocol_limits.max_buffered_input_bytes {
            return Err(RfbTcpConfigError::ReadBufferExceedsInputLimit {
                actual: self.read_buffer_bytes,
                maximum: self.protocol_limits.max_buffered_input_bytes,
            });
        }
        if self.handshake_timeout.is_zero() {
            return Err(RfbTcpConfigError::ZeroHandshakeTimeout);
        }
        Ok(())
    }
}

impl Default for RfbTcpConfig {
    fn default() -> Self {
        Self {
            desktop_name: "my_ipkvm".to_string(),
            read_buffer_bytes: 16 * 1024,
            handshake_timeout: Duration::from_secs(10),
            protocol_limits: RfbProtocolLimits::default(),
        }
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbTcpConfigError {
    #[error("TCP read buffer must be non-zero")]
    ZeroReadBuffer,
    #[error("TCP read buffer is {actual} bytes, protocol input limit is {maximum}")]
    ReadBufferExceedsInputLimit { actual: usize, maximum: usize },
    #[error("RFB handshake timeout must be non-zero")]
    ZeroHandshakeTimeout,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbTcpFrameError {
    #[error("no video frame is available")]
    FrameUnavailable,
    #[error("RFB TCP requires BGRA8888, got {0:?}")]
    UnsupportedPixelFormat(PixelFormat),
    #[error("video frame width {0} exceeds the RFB limit")]
    WidthOutOfRange(u32),
    #[error("video frame height {0} exceeds the RFB limit")]
    HeightOutOfRange(u32),
    #[error("video frame stride {0} cannot be represented on this platform")]
    StrideOutOfRange(u32),
    #[error("invalid BGRA8888 video frame: {0}")]
    InvalidBgraFrame(#[from] RfbFramebufferError),
    #[error("video frame sequence regressed from {previous} to {actual}")]
    FrameSequenceRegressed { previous: u64, actual: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tcp_config_is_bounded() {
        let config = RfbTcpConfig::default();

        assert_eq!(config.read_buffer_bytes, 16 * 1024);
        assert_eq!(config.handshake_timeout, Duration::from_secs(10));
        assert!(config.validate().is_ok());
    }

    #[test]
    fn tcp_config_rejects_invalid_read_buffers_and_timeout() {
        let mut zero = RfbTcpConfig::default();
        zero.read_buffer_bytes = 0;
        assert_eq!(zero.validate(), Err(RfbTcpConfigError::ZeroReadBuffer));

        let mut oversized = RfbTcpConfig::default();
        oversized.read_buffer_bytes = oversized.protocol_limits.max_buffered_input_bytes + 1;
        assert!(matches!(
            oversized.validate(),
            Err(RfbTcpConfigError::ReadBufferExceedsInputLimit { .. })
        ));

        let mut no_timeout = RfbTcpConfig::default();
        no_timeout.handshake_timeout = Duration::ZERO;
        assert_eq!(
            no_timeout.validate(),
            Err(RfbTcpConfigError::ZeroHandshakeTimeout)
        );
    }
}
