mod service;
mod transport;

use thiserror::Error;

use ipkvm_session::rfb_connection::{RfbConnectionSettings, RfbConnectionSettingsError};

pub use service::RfbWebSocketService;

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct RfbWebSocketConfig {
    pub connection: RfbConnectionSettings,
}

impl RfbWebSocketConfig {
    fn validate(&self) -> Result<(), RfbWebSocketServiceError> {
        self.connection.validate()?;
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum RfbWebSocketServiceError {
    #[error("invalid RFB WebSocket configuration: {0}")]
    Config(#[from] RfbConnectionSettingsError),
}
