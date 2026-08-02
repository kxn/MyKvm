mod driver;
mod finalize;
mod frame;
mod gate;
mod pending;
mod transport;

use std::{io::ErrorKind, net::SocketAddr, time::Duration};

use ipkvm_rfb::{
    RfbConfigError, RfbEncodeError, RfbFramebufferError, RfbProtocolError, RfbProtocolLimits,
    RfbRectangle, RfbSecurity, RfbSize,
};
use ipkvm_video::PixelFormat;
use thiserror::Error;

pub use driver::ConnectionEnd;
pub use finalize::{RfbConnectionFinalizeError, finalize_connection, run_managed_connection};
pub use gate::RfbConnectionReservation;
pub use gate::{ActiveController, RfbConnectionGate, RfbConnectionGateError, RfbTransportKind};
pub use transport::{RfbTransport, RfbTransportError, RfbTransportRead};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RfbConnectionSettings {
    pub desktop_name: String,
    pub handshake_timeout: Duration,
    pub protocol_limits: RfbProtocolLimits,
    pub security: RfbSecurity,
}

impl RfbConnectionSettings {
    pub fn validate(&self) -> Result<(), RfbConnectionSettingsError> {
        if self.handshake_timeout.is_zero() {
            return Err(RfbConnectionSettingsError::ZeroHandshakeTimeout);
        }
        Ok(())
    }
}

impl Default for RfbConnectionSettings {
    fn default() -> Self {
        Self {
            desktop_name: "my_ipkvm".to_string(),
            handshake_timeout: Duration::from_secs(10),
            protocol_limits: RfbProtocolLimits::default(),
            security: RfbSecurity::None,
        }
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbConnectionSettingsError {
    #[error("RFB handshake timeout must be non-zero")]
    ZeroHandshakeTimeout,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RfbClientId(pub(crate) u64);

impl RfbClientId {
    pub fn get(self) -> u64 {
        self.0
    }

    #[cfg(test)]
    pub(crate) fn for_test(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RfbServerEvent {
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
        framebuffer_size: RfbSize,
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
    AuthenticationFailed,
    CoreConfig(RfbConfigError),
    Protocol(RfbProtocolError),
    Encode(RfbEncodeError),
    Frame(RfbFrameError),
    Io(ErrorKind),
    WebSocket,
    UnexpectedTextMessage,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RfbFrameError {
    #[error("no video frame is available")]
    FrameUnavailable,
    #[error("RFB requires BGRA8888, got {0:?}")]
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
    use std::time::Duration;

    use super::*;

    #[test]
    fn default_connection_settings_are_bounded() {
        let settings = RfbConnectionSettings::default();
        assert_eq!(settings.desktop_name, "my_ipkvm");
        assert_eq!(settings.handshake_timeout, Duration::from_secs(10));
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn zero_handshake_timeout_is_rejected() {
        let settings = RfbConnectionSettings {
            handshake_timeout: Duration::ZERO,
            ..RfbConnectionSettings::default()
        };
        assert_eq!(
            settings.validate(),
            Err(RfbConnectionSettingsError::ZeroHandshakeTimeout)
        );
    }

    #[test]
    fn default_connection_settings_use_none_security() {
        let settings = RfbConnectionSettings::default();
        assert_eq!(settings.security, ipkvm_rfb::RfbSecurity::None);
    }

    #[test]
    fn vnc_security_is_derivable_from_connection_settings() {
        let settings = RfbConnectionSettings {
            security: ipkvm_rfb::RfbSecurity::Vnc {
                password: *b"secret12",
            },
            ..RfbConnectionSettings::default()
        };
        assert_eq!(
            settings.security,
            ipkvm_rfb::RfbSecurity::Vnc {
                password: *b"secret12"
            }
        );
    }
}
