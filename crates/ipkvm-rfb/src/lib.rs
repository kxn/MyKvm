//! VNC/RFB 服务抽象。

mod connection;
mod framebuffer;
mod protocol;
mod security;

pub use connection::{
    EncodingPreference, FramebufferUpdateOutcome, RfbConfigError, RfbConnectionConfig,
    RfbConnectionCore, RfbConnectionState, RfbEncodeError, RfbEncodeStatsSnapshot, RfbEvent,
};
pub use framebuffer::{BgraFrameView, RfbFramebufferError, RfbRectangle, RfbSize, RgbFrameView};
pub use protocol::client::{FramebufferUpdateRequest, RfbPointerMode, RfbProtocolError};
pub use protocol::pixel_format::{RfbColorChannel, RfbPixelFormat, RfbPixelFormatError};
pub use security::RfbSecurity;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RfbServerConfig {
    pub tcp_port: u16,
}

impl Default for RfbServerConfig {
    fn default() -> Self {
        Self { tcp_port: 5900 }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RfbProtocolLimits {
    pub max_desktop_name_bytes: usize,
    pub max_encodings: usize,
    pub max_cut_text_bytes: usize,
    pub max_buffered_input_bytes: usize,
    pub max_queued_output_bytes: usize,
    pub max_framebuffer_bytes: usize,
}

impl Default for RfbProtocolLimits {
    fn default() -> Self {
        Self {
            max_desktop_name_bytes: 1024,
            max_encodings: 4096,
            max_cut_text_bytes: 1024 * 1024,
            max_buffered_input_bytes: 2 * 1024 * 1024,
            max_queued_output_bytes: 256 * 1024 * 1024,
            max_framebuffer_bytes: 128 * 1024 * 1024,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfb_config_has_standard_default_tcp_port() {
        let config = RfbServerConfig::default();

        assert_eq!(config.tcp_port, 5900);
    }

    #[test]
    fn protocol_limits_have_documented_defaults() {
        let limits = RfbProtocolLimits::default();

        assert_eq!(limits.max_desktop_name_bytes, 1024);
        assert_eq!(limits.max_encodings, 4096);
        assert_eq!(limits.max_cut_text_bytes, 1024 * 1024);
        assert_eq!(limits.max_buffered_input_bytes, 2 * 1024 * 1024);
        assert_eq!(limits.max_queued_output_bytes, 256 * 1024 * 1024);
        assert_eq!(limits.max_framebuffer_bytes, 128 * 1024 * 1024);
    }
}
