use axum::extract::ws::{Message, WebSocket};

use ipkvm_session::rfb_connection::{RfbTransport, RfbTransportError, RfbTransportRead};

pub(super) struct WebSocketTransport {
    socket: WebSocket,
}

impl WebSocketTransport {
    pub(super) fn new(socket: WebSocket) -> Self {
        Self { socket }
    }
}

fn map_websocket_error(error: axum::Error) -> RfbTransportError {
    RfbTransportError::websocket(error)
}

impl RfbTransport for WebSocketTransport {
    async fn receive_into(
        &mut self,
        buffer: &mut Vec<u8>,
    ) -> Result<RfbTransportRead, RfbTransportError> {
        buffer.clear();
        match self.socket.recv().await {
            Some(Ok(Message::Binary(bytes))) if !bytes.is_empty() => {
                buffer.extend_from_slice(&bytes);
                Ok(RfbTransportRead::Data)
            }
            Some(Ok(Message::Binary(_)))
            | Some(Ok(Message::Ping(_)))
            | Some(Ok(Message::Pong(_))) => Ok(RfbTransportRead::Continue),
            Some(Ok(Message::Close(_))) | None => Ok(RfbTransportRead::Closed),
            Some(Ok(Message::Text(_))) => Err(RfbTransportError::UnexpectedTextMessage),
            Some(Err(error)) => Err(map_websocket_error(error)),
        }
    }

    async fn send_binary(&mut self, bytes: Vec<u8>) -> Result<(), RfbTransportError> {
        self.socket
            .send(Message::Binary(bytes.into()))
            .await
            .map_err(map_websocket_error)
    }

    async fn close(&mut self) {
        let _ = self.socket.send(Message::Close(None)).await;
    }
}

#[cfg(test)]
mod tests {
    use tokio_tungstenite::tungstenite::Error as TungsteniteError;

    use super::*;

    #[test]
    fn websocket_mapping_preserves_axum_and_tungstenite_sources() {
        let error = map_websocket_error(axum::Error::new(TungsteniteError::ConnectionClosed));
        let mut current: &(dyn std::error::Error + 'static) = &error;
        let mut saw_axum = false;
        let mut saw_tungstenite = false;

        while let Some(source) = current.source() {
            saw_axum |= source.downcast_ref::<axum::Error>().is_some();
            saw_tungstenite |= source.downcast_ref::<TungsteniteError>().is_some();
            current = source;
        }

        assert!(saw_axum);
        assert!(saw_tungstenite);
    }
}
