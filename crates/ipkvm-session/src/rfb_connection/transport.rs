use std::{error::Error, io};

use thiserror::Error;

#[allow(dead_code)]
pub enum RfbTransportRead {
    Data,
    Continue,
    Closed,
}

#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum RfbTransportError {
    #[error("TCP I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("WebSocket transport error")]
    WebSocket {
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    #[error("RFB over WebSocket does not accept text messages")]
    UnexpectedTextMessage,
}

impl RfbTransportError {
    pub fn websocket(error: impl Error + Send + Sync + 'static) -> Self {
        Self::WebSocket {
            source: Box::new(error),
        }
    }
}

/// RFB transport contract.
///
/// Each receive clears `buffer` before returning. `Data` requires a non-empty
/// buffer; `Continue` and `Closed` require an empty buffer. Message boundaries
/// are transport details and do not delimit RFB protocol input.
///
/// 迁入 ipkvm-session 前该 trait 在 headless 内为 `pub(crate)`，`async_fn_in_trait`
/// lint 不生效；迁入后为供 headless 重新导出的 `pub` 契约，但消费者只有 headless
/// 一个内部 crate。实际 Send 约束由 driver 的 `tokio::spawn(run_connection(..))`
/// 编译期保证，与迁移前一致，因此显式豁免该 lint（行为不变）。
#[allow(async_fn_in_trait)]
pub trait RfbTransport {
    async fn receive_into(
        &mut self,
        buffer: &mut Vec<u8>,
    ) -> Result<RfbTransportRead, RfbTransportError>;

    async fn send_binary(&mut self, bytes: Vec<u8>) -> Result<(), RfbTransportError>;

    async fn close(&mut self);
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    #[test]
    fn websocket_error_preserves_private_source() {
        let error =
            RfbTransportError::websocket(io::Error::other("test WebSocket transport failure"));

        assert_eq!(
            error.source().map(ToString::to_string),
            Some("test WebSocket transport failure".to_string())
        );
    }
}
