use std::io;

use thiserror::Error;

#[allow(dead_code)]
pub(crate) enum RfbTransportRead {
    Data,
    Continue,
    Closed,
}

#[allow(dead_code)]
#[derive(Debug, Error)]
pub(crate) enum RfbTransportError {
    #[error("TCP I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("WebSocket transport error")]
    WebSocket,
    #[error("RFB over WebSocket does not accept text messages")]
    UnexpectedTextMessage,
}

/// RFB transport contract.
///
/// Each receive clears `buffer` before returning. `Data` requires a non-empty
/// buffer; `Continue` and `Closed` require an empty buffer. Message boundaries
/// are transport details and do not delimit RFB protocol input.
pub(crate) trait RfbTransport {
    async fn receive_into(
        &mut self,
        buffer: &mut Vec<u8>,
    ) -> Result<RfbTransportRead, RfbTransportError>;

    async fn send_binary(&mut self, bytes: Vec<u8>) -> Result<(), RfbTransportError>;

    async fn close(&mut self);
}
