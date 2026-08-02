use ipkvm_rfb::RfbServerConfig;

pub use ipkvm_session::{rfb_connection, rfb_input};
pub mod rfb_tcp;
pub mod rfb_ws;
pub mod web;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadlessConfig {
    pub bind_address: String,
    pub http_port: u16,
    pub rfb: RfbServerConfig,
}

impl Default for HeadlessConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1".to_string(),
            http_port: 6080,
            rfb: RfbServerConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_config_defaults_to_localhost_and_standard_http_port() {
        let config = HeadlessConfig::default();

        assert_eq!(config.bind_address, "127.0.0.1");
        assert_eq!(config.http_port, 6080);
        assert_eq!(config.rfb.tcp_port, 5900);
    }
}
