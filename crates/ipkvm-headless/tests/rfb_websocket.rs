mod support;

use std::{net::SocketAddr, sync::Arc};

use axum::{
    http::{
        HeaderMap, HeaderName, HeaderValue, StatusCode,
        header::{CONNECTION, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_PROTOCOL, UPGRADE},
    },
    serve,
};
use futures_util::StreamExt;
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
use tokio_tungstenite::tungstenite::{
    Error as WebSocketError, Message, client::IntoClientRequest, http::Response, protocol::Role,
};
use tokio_tungstenite::{WebSocketStream, connect_async};

struct RawUpgradeResponse {
    status: StatusCode,
    headers: HeaderMap,
    stream: TcpStream,
}

struct TestWebSocketServer {
    address: SocketAddr,
    source: Arc<MockFrameSource>,
    events: mpsc::Receiver<RfbServerEvent>,
    shutdown: watch::Sender<bool>,
    task: Option<JoinHandle<std::io::Result<()>>>,
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
        let event_publisher = watch::channel(Some(event_tx)).1;
        let (shutdown, shutdown_rx) = watch::channel(false);
        let service = RfbWebSocketService::new(
            Arc::clone(&source),
            event_publisher,
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
            source,
            events,
            shutdown,
            task: Some(task),
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

    async fn rejected_response(&self) -> Response<Option<Vec<u8>>> {
        match connect_async(self.url()).await.unwrap_err() {
            WebSocketError::Http(response) => *response,
            error => panic!("expected rejected HTTP upgrade, got {error:?}"),
        }
    }

    async fn raw_upgrade_response_with_protocols(&self, protocols: &str) -> RawUpgradeResponse {
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
        let (status, headers) = parse_http_response_headers(&response);
        RawUpgradeResponse {
            status,
            headers,
            stream,
        }
    }

    async fn expect_connected(&mut self) -> RfbClientId {
        match self.events.recv().await.unwrap() {
            RfbServerEvent::Connected { client_id, .. } => client_id,
            event => panic!("expected connected event, got {event:?}"),
        }
    }

    async fn expect_disconnected(&mut self) -> (RfbClientId, RfbDisconnectReason) {
        loop {
            match self.events.recv().await.unwrap() {
                // 跳过统计通知事件（调研阶段 0 埋点），等到真正的 Disconnected。
                RfbServerEvent::FrameUpdateSent { .. } => continue,
                RfbServerEvent::Disconnected {
                    client_id, reason, ..
                } => return (client_id, reason),
                event => panic!("expected disconnected event, got {event:?}"),
            }
        }
    }

    async fn assert_event_queue_empty_after_reconnect_barrier(&mut self) {
        let (socket, response) = self.connect_without_protocol().await;
        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        let mut barrier_client = TestWebSocketRfbClient::new(socket);
        assert_eq!(barrier_client.read_banner().await, *b"RFB 003.008\n");
        assert!(matches!(
            self.events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        barrier_client.close().await;
        assert_eq!(
            self.expect_disconnected().await.1,
            RfbDisconnectReason::ClientClosed
        );
    }

    async fn stop(mut self) {
        let task = self.task.take().unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }
}

impl Drop for TestWebSocketServer {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
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

fn bgra_frame(seq: u64, bytes: [u8; 4]) -> Arc<VideoFrame> {
    Arc::new(VideoFrame::new(
        seq,
        MonotonicTimestamp::from_nanos(seq),
        1,
        1,
        4,
        PixelFormat::Bgra8888,
        Arc::from(bytes),
    ))
}

// noVNC 1.7.0 commit 63107bd06d9e1f6136ff21aeda8cd62cbf0d433e, without H.264.
const NOVNC_1_7_SET_PIXEL_FORMAT: [u8; 20] = [
    0, 0, 0, 0, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 0, 8, 16, 0, 0, 0,
];

const NOVNC_1_7_ENCODINGS_WITHOUT_H264: [i32; 24] = [
    1,
    7,
    -260,
    16,
    21,
    5,
    2,
    6,
    0,
    -26,
    -254,
    -223,
    -224,
    -258,
    -261,
    -308,
    -309,
    -312,
    -313,
    -307,
    0xc0a1e5ceu32 as i32,
    -316,
    0x574d5664,
    -239,
];

fn key_message(down: bool, keysym: u32) -> Vec<u8> {
    let mut message = vec![4, u8::from(down), 0, 0];
    message.extend_from_slice(&keysym.to_be_bytes());
    message
}

/// 独立实现 VNC 密码响应（RFC 6143 §7.2 + Erratum 4951），交叉验证服务器
/// 校验逻辑：密钥 = 密码补零到 8 字节后每字节位反转，DES-ECB 逐块加密。
/// 不调用产品 `vnc_key`，避免测试自证。
fn vnc_response(password: &[u8; 8], challenge: &[u8; 16]) -> [u8; 16] {
    use des::cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
    let mut key = [0_u8; 8];
    for (index, byte) in password.iter().copied().enumerate() {
        key[index] = byte.reverse_bits();
    }
    let cipher = des::Des::new_from_slice(&key).unwrap();
    let mut response = *challenge;
    for chunk in response.chunks_exact_mut(8) {
        cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
    }
    response
}

fn parse_http_response_headers(response: &[u8]) -> (StatusCode, HeaderMap) {
    let response = std::str::from_utf8(response).unwrap();
    let response = response.strip_suffix("\r\n\r\n").unwrap();
    let mut lines = response.split("\r\n");
    let mut status_line = lines.next().unwrap().splitn(3, ' ');
    assert_eq!(status_line.next(), Some("HTTP/1.1"));
    let status = StatusCode::from_bytes(status_line.next().unwrap().as_bytes()).unwrap();
    assert!(status_line.next().is_some());

    let mut headers = HeaderMap::new();
    for line in lines {
        let (name, value) = line.split_once(':').unwrap();
        headers.append(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value.trim()).unwrap(),
        );
    }
    (status, headers)
}

fn header_contains_token(headers: &HeaderMap, name: HeaderName, expected: &str) -> bool {
    headers.get_all(name).iter().any(|value| {
        value
            .to_str()
            .unwrap()
            .split(',')
            .any(|token| token.trim().eq_ignore_ascii_case(expected))
    })
}

fn assert_empty_rejection(response: &Response<Option<Vec<u8>>>, expected: StatusCode) {
    assert_eq!(response.status(), expected);
    assert!(response.body().as_ref().is_none_or(Vec::is_empty));
}

#[tokio::test]
async fn novnc_1_7_without_h264_receives_initial_and_incremental_raw_updates() {
    // 此测试验证 Raw 编码路径：服务端强制 Raw（即使客户端声明了 Tight）。
    let config = RfbWebSocketConfig {
        connection: RfbConnectionSettings {
            preferred_encoding: ipkvm_rfb::EncodingPreference::Raw,
            ..RfbConnectionSettings::default()
        },
    };
    let mut server = TestWebSocketServer::start_with_config(config).await;
    server
        .source
        .publish_frame(bgra_frame(2, [30, 20, 10, 255]));
    let (socket, response) = server.connect_without_protocol().await;
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    let mut client = TestWebSocketRfbClient::new(socket);

    let init = client.handshake(true).await;
    assert_eq!((init.width, init.height), (1, 1));
    let client_id = server.expect_connected().await;
    client.set_pixel_format(&NOVNC_1_7_SET_PIXEL_FORMAT).await;
    client
        .set_encodings(&NOVNC_1_7_ENCODINGS_WITHOUT_H264)
        .await;

    let mut initial_request = vec![3, 0, 0, 0, 0, 0];
    initial_request.extend_from_slice(&init.width.to_be_bytes());
    initial_request.extend_from_slice(&init.height.to_be_bytes());
    client.send_binary(&initial_request).await;
    let initial_update = client.read_update(4).await;
    assert_eq!(
        (
            initial_update.x,
            initial_update.y,
            initial_update.width,
            initial_update.height,
        ),
        (0, 0, 1, 1)
    );
    assert_eq!(initial_update.encoding, 0);
    assert_eq!(initial_update.pixels, [10, 20, 30, 0]);

    let mut incremental_request = vec![3, 1, 0, 0, 0, 0];
    incremental_request.extend_from_slice(&init.width.to_be_bytes());
    incremental_request.extend_from_slice(&init.height.to_be_bytes());
    client.send_binary(&incremental_request).await;
    server
        .source
        .publish_frame(bgra_frame(3, [60, 50, 40, 255]));
    let incremental_update = client.read_update(4).await;
    assert_eq!(
        (
            incremental_update.x,
            incremental_update.y,
            incremental_update.width,
            incremental_update.height,
        ),
        (0, 0, 1, 1)
    );
    assert_eq!(incremental_update.encoding, 0);
    assert_eq!(incremental_update.pixels, [40, 50, 60, 0]);

    client.close().await;
    assert_eq!(
        server.expect_disconnected().await,
        (client_id, RfbDisconnectReason::ClientClosed)
    );
    server.stop().await;
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
    assert_eq!(response.status, StatusCode::SWITCHING_PROTOCOLS);
    assert!(header_contains_token(
        &response.headers,
        CONNECTION,
        "upgrade"
    ));
    assert!(header_contains_token(
        &response.headers,
        UPGRADE,
        "websocket"
    ));
    assert_eq!(
        response.headers.get(SEC_WEBSOCKET_ACCEPT),
        Some(&HeaderValue::from_static("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="))
    );
    assert!(response.headers.get(SEC_WEBSOCKET_PROTOCOL).is_none());

    let mut socket = WebSocketStream::from_raw_socket(response.stream, Role::Client, None).await;
    assert!(
        matches!(
            socket.next().await,
            Some(Ok(Message::Binary(bytes))) if bytes.as_ref() == b"RFB 003.008\n"
        ),
        "upgraded socket did not carry the binary RFB banner"
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
    server
        .assert_event_queue_empty_after_reconnect_barrier()
        .await;
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
    server
        .assert_event_queue_empty_after_reconnect_barrier()
        .await;
}

#[tokio::test]
async fn second_upgrade_is_rejected_while_a_connection_is_active() {
    let mut server = TestWebSocketServer::start().await;
    let (socket, _) = server.connect_without_protocol().await;
    let mut first = TestWebSocketRfbClient::new(socket);
    first.read_banner().await;

    let response = server.rejected_response().await;
    assert_empty_rejection(&response, StatusCode::CONFLICT);
    assert!(matches!(
        server.events.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));

    first.close().await;
    assert_eq!(
        server.expect_disconnected().await.1,
        RfbDisconnectReason::ClientClosed
    );
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
async fn shutdown_before_upgrade_returns_empty_service_unavailable() {
    let server = TestWebSocketServer::start().await;
    server.shutdown.send(true).unwrap();

    let response = server.rejected_response().await;
    assert_empty_rejection(&response, StatusCode::SERVICE_UNAVAILABLE);
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

    let response = server.rejected_response().await;
    assert_empty_rejection(&response, StatusCode::SERVICE_UNAVAILABLE);
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
    assert!(matches!(
        client.read_message().await,
        Ok(Message::Close(_)) | Err(_)
    ));
}

#[tokio::test]
async fn vnc_password_handshake_succeeds_over_websocket() {
    let config = RfbWebSocketConfig {
        connection: RfbConnectionSettings {
            security: ipkvm_rfb::RfbSecurity::Vnc {
                password: *b"12345678",
            },
            ..RfbConnectionSettings::default()
        },
    };
    let mut server = TestWebSocketServer::start_with_config(config).await;

    // 升级成功后，RFB 握手跑在 WS 二进制消息上（与 TCP 同一路径）：
    // banner → [1,2] → 选 2 → challenge → 响应 → OK → ServerInit。
    let (socket, _) = server.connect_without_protocol().await;
    let mut client = TestWebSocketRfbClient::new(socket);
    assert_eq!(client.read_banner().await, *b"RFB 003.008\n");
    client.send_binary(b"RFB 003.008\n").await;
    assert_eq!(client.read_binary().await, [1, 2]);
    client.send_binary(&[2]).await;
    let challenge: [u8; 16] = client.read_binary().await.try_into().unwrap();
    client
        .send_binary(&vnc_response(b"12345678", &challenge))
        .await;
    assert_eq!(client.read_binary().await, [0, 0, 0, 0]);
    client.send_binary(&[1]).await;
    assert!(client.read_binary().await.len() >= 24);
    // RfbClientId 的元组字段是 pub(crate)，集成测试无法用 `RfbClientId(_)`
    // 模式匹配，改为直接断言 Connected 事件（与 rfb_tcp.rs 的断言一致）。
    assert!(matches!(
        server.events.recv().await,
        Some(RfbServerEvent::Connected { .. })
    ));

    server.stop().await;
}
