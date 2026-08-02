mod server;
mod transport;

use std::io;

use thiserror::Error;

use ipkvm_session::rfb_connection::{RfbConnectionSettings, RfbConnectionSettingsError};

pub use server::RfbTcpServer;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RfbTcpConfig {
    pub connection: RfbConnectionSettings,
    pub read_buffer_bytes: usize,
}

impl RfbTcpConfig {
    pub fn validate(&self) -> Result<(), RfbTcpConfigError> {
        self.connection.validate()?;
        if self.read_buffer_bytes == 0 {
            return Err(RfbTcpConfigError::ZeroReadBuffer);
        }
        if self.read_buffer_bytes > self.connection.protocol_limits.max_buffered_input_bytes {
            return Err(RfbTcpConfigError::ReadBufferExceedsInputLimit {
                actual: self.read_buffer_bytes,
                maximum: self.connection.protocol_limits.max_buffered_input_bytes,
            });
        }
        Ok(())
    }
}

impl Default for RfbTcpConfig {
    fn default() -> Self {
        Self {
            connection: RfbConnectionSettings::default(),
            read_buffer_bytes: 16 * 1024,
        }
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbTcpConfigError {
    #[error("invalid RFB connection configuration: {0}")]
    Connection(#[from] RfbConnectionSettingsError),
    #[error("TCP read buffer must be non-zero")]
    ZeroReadBuffer,
    #[error("TCP read buffer is {actual} bytes, protocol input limit is {maximum}")]
    ReadBufferExceedsInputLimit { actual: usize, maximum: usize },
}

#[derive(Debug, Error)]
pub enum RfbTcpServerError {
    #[error("invalid RFB TCP configuration: {0}")]
    Config(#[from] RfbTcpConfigError),
    #[error("failed to accept RFB TCP client: {0}")]
    Accept(#[source] io::Error),
    #[error("RFB client identifier space is exhausted")]
    ClientIdOverflow,
    #[error("the RFB connection gate is poisoned")]
    ConnectionGatePoisoned,
    #[error("RFB event receiver is closed")]
    EventChannelClosed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tcp_config_is_bounded() {
        let config = RfbTcpConfig::default();

        assert_eq!(config.read_buffer_bytes, 16 * 1024);
        assert_eq!(
            config.connection.handshake_timeout,
            std::time::Duration::from_secs(10)
        );
        assert!(config.validate().is_ok());
    }

    #[test]
    fn tcp_config_rejects_invalid_read_buffers_and_timeout() {
        let zero = RfbTcpConfig {
            read_buffer_bytes: 0,
            ..RfbTcpConfig::default()
        };
        assert_eq!(zero.validate(), Err(RfbTcpConfigError::ZeroReadBuffer));

        let mut oversized = RfbTcpConfig::default();
        oversized.read_buffer_bytes = oversized
            .connection
            .protocol_limits
            .max_buffered_input_bytes
            + 1;
        assert!(matches!(
            oversized.validate(),
            Err(RfbTcpConfigError::ReadBufferExceedsInputLimit { .. })
        ));

        let no_timeout = RfbTcpConfig {
            connection: RfbConnectionSettings {
                handshake_timeout: std::time::Duration::ZERO,
                ..RfbConnectionSettings::default()
            },
            ..RfbTcpConfig::default()
        };
        assert_eq!(
            no_timeout.validate(),
            Err(RfbTcpConfigError::Connection(
                RfbConnectionSettingsError::ZeroHandshakeTimeout
            ))
        );
    }
}
