mod support;

use std::{net::SocketAddr, sync::Arc};

use axum::{
    http::{HeaderValue, StatusCode, header::SEC_WEBSOCKET_PROTOCOL},
    serve,
};
use ipkvm_headless::{
    rfb_connection::{
        RfbClientId, RfbConnectionGate, RfbConnectionSettings, RfbDisconnectReason, RfbServerEvent,
    },
    rfb_ws::{RfbWebSocketConfig, RfbWebSocketService},
};
use ipkvm_video::{MonotonicTimestamp, PixelFormat, VideoFrame, mock::MockFrameSource};
use support::{ClientWebSocket, TestWebSocketRfbClient};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, watch},
    task::JoinHandle,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Error as WebSocketError, Message, client::IntoClientRequest, http::Response},
};

struct TestWebSocketServer {
    address: SocketAddr,
    events: mpsc::Receiver<RfbServerEvent>,
    shutdown: watch::Sender<bool>,
    task: JoinHandle<std::io::Result<()>>,
}

impl TestWebSocketServer {
    async fn start() -> Self {
        Self::start_with_config(RfbWebSocketConfig::default()).await
    }

    async fn start_with_config(config: RfbWebSocketConfig) -> Self {
        let (event_tx, events) = mpsc::channel(32);
        Self::start_with_channels(config, event_tx, events).await
    }

    async fn start_with_channels(
        config: RfbWebSocketConfig,
        event_tx: mpsc::Sender<RfbServerEvent>,
        events: mpsc::Receiver<RfbServerEvent>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let source = Arc::new(MockFrameSource::new());
        source.publish_frame(default_frame());
        let (shutdown, shutdown_rx) = watch::channel(false);
        let service = RfbWebSocketService::new(
            source,
            event_tx,
            config,
            shutdown_rx,
            RfbConnectionGate::new(),
        )
        .unwrap();
        let task = tokio::spawn(async move {
            serve(
                listener,
                service
                    .router()
                    .into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
        });
        Self {
            address,
            events,
            shutdown,
            task,
        }
    }

    fn url(&self) -> String {
        format!("ws://{}/rfb", self.address)
    }

    async fn connect_without_protocol(&self) -> (ClientWebSocket, Response<Option<Vec<u8>>>) {
        connect_async(self.url()).await.unwrap()
    }

    async fn connect_with_protocols(
        &self,
        protocols: &str,
    ) -> (ClientWebSocket, Response<Option<Vec<u8>>>) {
        let mut request = self.url().into_client_request().unwrap();
        request.headers_mut().insert(
            SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_str(protocols).unwrap(),
        );
        connect_async(request).await.unwrap()
    }

    async fn rejected_status(&self) -> StatusCode {
        match connect_async(self.url()).await.unwrap_err() {
            WebSocketError::Http(response) => response.status(),
            error => panic!("expected rejected HTTP upgrade, got {error:?}"),
        }
    }

    async fn raw_upgrade_response_with_protocols(&self, protocols: &str) -> String {
        let mut stream = TcpStream::connect(self.address).await.unwrap();
        let request = format!(
            "GET /rfb HTTP/1.1\r\nHost: {}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Protocol: {protocols}\r\n\r\n",
            self.address
        );
        stream.write_all(request.as_bytes()).await.unwrap();

        let mut response = Vec::new();
        while !response.ends_with(b"\r\n\r\n") {
            assert!(
                response.len() < 8 * 1024,
                "upgrade response headers are too large"
            );
            let mut byte = [0];
            stream.read_exact(&mut byte).await.unwrap();
            response.push(byte[0]);
        }
        String::from_utf8(response).unwrap()
    }

    async fn expect_connected(&mut self) -> RfbClientId {
        match self.events.recv().await.unwrap() {
            RfbServerEvent::Connected { client_id, .. } => client_id,
            event => panic!("expected connected event, got {event:?}"),
        }
    }

    async fn expect_disconnected(&mut self) -> (RfbClientId, RfbDisconnectReason) {
        match self.events.recv().await.unwrap() {
            RfbServerEvent::Disconnected {
                client_id, reason, ..
            } => (client_id, reason),
            event => panic!("expected disconnected event, got {event:?}"),
        }
    }
}

impl Drop for TestWebSocketServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn default_frame() -> Arc<VideoFrame> {
    Arc::new(VideoFrame::new(
        1,
        MonotonicTimestamp::from_nanos(1),
        2,
        1,
        8,
        PixelFormat::Bgra8888,
        Arc::from([0, 0, 255, 0, 0, 255, 0, 0]),
    ))
}

fn key_message(down: bool, keysym: u32) -> Vec<u8> {
    let mut message = vec![4, u8::from(down), 0, 0];
    message.extend_from_slice(&keysym.to_be_bytes());
    message
}

#[tokio::test]
async fn websocket_upgrade_does_not_require_a_subprotocol() {
    let server = TestWebSocketServer::start().await;
    let (socket, response) = server.connect_without_protocol().await;
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    assert!(response.headers().get(SEC_WEBSOCKET_PROTOCOL).is_none());
    drop(socket);
}

#[tokio::test]
async fn websocket_upgrade_selects_binary_only_when_requested() {
    let server = TestWebSocketServer::start().await;
    let (socket, response) = server.connect_with_protocols("chat, binary").await;
    assert_eq!(
        response.headers()[SEC_WEBSOCKET_PROTOCOL],
        HeaderValue::from_static("binary")
    );
    drop(socket);
}

#[tokio::test]
async fn an_unrelated_protocol_is_not_echoed() {
    let server = TestWebSocketServer::start().await;
    let response = server.raw_upgrade_response_with_protocols("chat").await;
    assert!(response.starts_with("HTTP/1.1 101 "));
    assert!(
        !response
            .to_ascii_lowercase()
            .contains("\r\nsec-websocket-protocol:")
    );
}

#[tokio::test]
async fn rfb_output_is_sent_as_binary_messages() {
    let mut server = TestWebSocketServer::start().await;
    let (socket, _) = server.connect_without_protocol().await;
    let mut client = TestWebSocketRfbClient::new(socket);

    let init = client.handshake(true).await;
    assert_eq!(
        (init.width, init.height, init.name.as_str()),
        (2, 1, "my_ipkvm")
    );
    server.expect_connected().await;
    client.request_update(false, 0, 0, 2, 1).await;
    let update = client.read_update(8).await;
    assert_eq!(
        (update.x, update.y, update.width, update.height),
        (0, 0, 2, 1)
    );
    assert_eq!(update.encoding, 0);
    assert_eq!(update.pixels, [0, 0, 255, 0, 0, 255, 0, 0]);
}

#[tokio::test]
async fn fragmented_binary_messages_complete_the_rfb_handshake() {
    let mut server = TestWebSocketServer::start().await;
    let (socket, _) = server.connect_without_protocol().await;
    let mut client = TestWebSocketRfbClient::new(socket);

    assert_eq!(client.read_banner().await, *b"RFB 003.008\n");
    for byte in b"RFB 003.008\n" {
        client.send_binary(&[*byte]).await;
    }
    assert_eq!(client.read_binary().await, [1, 1]);
    client.send_binary(&[1]).await;
    assert_eq!(client.read_binary().await, [0, 0, 0, 0]);
    client.send_binary(&[1]).await;
    assert!(!client.read_binary().await.is_empty());
    server.expect_connected().await;
}

#[tokio::test]
async fn one_binary_message_preserves_multiple_rfb_events_in_order() {
    let mut server = TestWebSocketServer::start().await;
    let (socket, _) = server.connect_without_protocol().await;
    let mut client = TestWebSocketRfbClient::new(socket);
    client.handshake(true).await;
    let client_id = server.expect_connected().await;

    let mut input = key_message(true, 0x41);
    input.extend_from_slice(&key_message(false, 0x41));
    client.send_binary(&input).await;

    assert_eq!(
        server.events.recv().await,
        Some(RfbServerEvent::Key {
            client_id,
            down: true,
            keysym: 0x41,
        })
    );
    assert_eq!(
        server.events.recv().await,
        Some(RfbServerEvent::Key {
            client_id,
            down: false,
            keysym: 0x41,
        })
    );
}

#[tokio::test]
async fn ping_does_not_interrupt_the_rfb_handshake() {
    let mut server = TestWebSocketServer::start().await;
    let (socket, _) = server.connect_without_protocol().await;
    let mut client = TestWebSocketRfbClient::new(socket);

    assert_eq!(client.read_banner().await, *b"RFB 003.008\n");
    client.send_ping(b"probe").await;
    client.send_binary(b"RFB 003.008\n").await;
    let mut saw_pong = false;
    let security_types = loop {
        match client.read_message().await.unwrap() {
            Message::Pong(bytes) => {
                assert_eq!(bytes.as_ref(), b"probe");
                saw_pong = true;
            }
            Message::Binary(bytes) => break bytes,
            message => panic!("unexpected WebSocket message: {message:?}"),
        }
    };
    assert!(saw_pong);
    assert_eq!(security_types.as_ref(), &[1, 1]);
    client.send_binary(&[1]).await;
    assert_eq!(client.read_binary().await, [0, 0, 0, 0]);
    client.send_binary(&[1]).await;
    assert!(!client.read_binary().await.is_empty());
    server.expect_connected().await;
}

#[tokio::test]
async fn text_message_disconnects_once_with_unexpected_text_reason() {
    let mut server = TestWebSocketServer::start().await;
    let (socket, _) = server.connect_without_protocol().await;
    let mut client = TestWebSocketRfbClient::new(socket);
    client.read_banner().await;

    client.send_text("not RFB").await;
    let (_, reason) = server.expect_disconnected().await;
    assert_eq!(reason, RfbDisconnectReason::UnexpectedTextMessage);
    assert!(matches!(
        server.events.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn close_message_disconnects_once_with_client_closed_reason() {
    let mut server = TestWebSocketServer::start().await;
    let (socket, _) = server.connect_without_protocol().await;
    let mut client = TestWebSocketRfbClient::new(socket);
    client.read_banner().await;

    client.close().await;
    let (_, reason) = server.expect_disconnected().await;
    assert_eq!(reason, RfbDisconnectReason::ClientClosed);
    assert!(matches!(
        server.events.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn second_upgrade_is_rejected_while_a_connection_is_active() {
    let server = TestWebSocketServer::start().await;
    let (socket, _) = server.connect_without_protocol().await;
    let mut first = TestWebSocketRfbClient::new(socket);
    first.read_banner().await;

    assert_eq!(server.rejected_status().await, StatusCode::CONFLICT);
}

#[tokio::test]
async fn gate_reopens_after_disconnected_event_is_delivered() {
    let mut server = TestWebSocketServer::start().await;
    let (socket, _) = server.connect_without_protocol().await;
    let mut first = TestWebSocketRfbClient::new(socket);
    first.read_banner().await;
    first.close().await;
    assert_eq!(
        server.expect_disconnected().await.1,
        RfbDisconnectReason::ClientClosed
    );

    let (second, response) = server.connect_without_protocol().await;
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    drop(second);
}

#[tokio::test]
async fn shutdown_disconnects_an_active_connection() {
    let mut server = TestWebSocketServer::start().await;
    let (socket, _) = server.connect_without_protocol().await;
    let mut client = TestWebSocketRfbClient::new(socket);
    client.handshake(true).await;
    let client_id = server.expect_connected().await;

    server.shutdown.send(true).unwrap();
    assert_eq!(
        server.expect_disconnected().await,
        (client_id, RfbDisconnectReason::ServerShutdown)
    );
}

#[tokio::test]
async fn closed_event_receiver_rejects_upgrade_with_service_unavailable() {
    let (event_tx, events) = mpsc::channel(1);
    drop(events);
    let replacement_events = mpsc::channel(1).1;
    let server = TestWebSocketServer::start_with_channels(
        RfbWebSocketConfig::default(),
        event_tx,
        replacement_events,
    )
    .await;

    assert_eq!(
        server.rejected_status().await,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn oversized_websocket_message_disconnects_with_websocket_reason() {
    let mut connection = RfbConnectionSettings::default();
    connection.protocol_limits.max_cut_text_bytes = 12;
    connection.protocol_limits.max_encodings = 4;
    connection.protocol_limits.max_buffered_input_bytes = 20;
    let mut server =
        TestWebSocketServer::start_with_config(RfbWebSocketConfig { connection }).await;
    let (socket, _) = server.connect_without_protocol().await;
    let mut client = TestWebSocketRfbClient::new(socket);
    client.read_banner().await;

    client.send_binary(&[0; 21]).await;
    assert_eq!(
        server.expect_disconnected().await.1,
        RfbDisconnectReason::WebSocket
    );
}
