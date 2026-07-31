use axum::extract::ws::{Message, WebSocket};

use crate::rfb_connection::{RfbTransport, RfbTransportError, RfbTransportRead};

pub(super) struct WebSocketTransport {
    socket: WebSocket,
}

impl WebSocketTransport {
    pub(super) fn new(socket: WebSocket) -> Self {
        Self { socket }
    }
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
            Some(Err(error)) => Err(RfbTransportError::websocket(error)),
        }
    }

    async fn send_binary(&mut self, bytes: Vec<u8>) -> Result<(), RfbTransportError> {
        self.socket
            .send(Message::Binary(bytes.into()))
            .await
            .map_err(RfbTransportError::websocket)
    }

    async fn close(&mut self) {
        let _ = self.socket.send(Message::Close(None)).await;
    }
}
