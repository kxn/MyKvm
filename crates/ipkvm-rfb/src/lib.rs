//! VNC/RFB 服务抽象。

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RfbServerConfig {
    pub tcp_port: u16,
}

impl Default for RfbServerConfig {
    fn default() -> Self {
        Self { tcp_port: 5900 }
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
}
