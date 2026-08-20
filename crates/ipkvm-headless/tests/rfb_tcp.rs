mod support;

use std::{io, sync::Arc};

use ipkvm_headless::{
    rfb_connection::{
        RfbClientId, RfbConnectionGate, RfbConnectionSettings, RfbDisconnectReason, RfbServerEvent,
    },
    rfb_tcp::{RfbTcpConfig, RfbTcpServer, RfbTcpServerError},
};
use ipkvm_rfb::{RfbProtocolError, RfbSecurity, RfbSize};
use ipkvm_video::{MonotonicTimestamp, PixelFormat, VideoFrame, mock::MockFrameSource};
use support::TestRfbClient;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, watch},
};

fn frame(seq: u64, width: u32, height: u32, format: PixelFormat, bytes: &[u8]) -> Arc<VideoFrame> {
    Arc::new(VideoFrame::new(
        seq,
        MonotonicTimestamp::from_nanos(seq),
        width,
        height,
        width * 4,
        format,
        Arc::from(bytes.to_vec().into_boxed_slice()),
    ))
}

struct ServerFixture {
    address: std::net::SocketAddr,
    source: Arc<MockFrameSource>,
    events: mpsc::Receiver<RfbServerEvent>,
    shutdown: watch::Sender<bool>,
    task: tokio::task::JoinHandle<Result<(), RfbTcpServerError>>,
}

impl ServerFixture {
    async fn start(event_capacity: usize, initial_frame: Option<Arc<VideoFrame>>) -> Self {
        Self::start_with_security(event_capacity, initial_frame, RfbSecurity::None).await
    }

    async fn start_with_security(
        event_capacity: usize,
        initial_frame: Option<Arc<VideoFrame>>,
        security: RfbSecurity,
    ) -> Self {
        let (event_tx, events) = mpsc::channel(event_capacity);
        let event_publisher = watch::channel(Some(event_tx)).1;
        Self::start_with_event_publisher(initial_frame, security, event_publisher, events).await
    }

    async fn start_view_only(initial_frame: Option<Arc<VideoFrame>>) -> Self {
        let (_unused_tx, events) = mpsc::channel(16);
        let event_publisher = watch::channel(None).1;
        Self::start_with_event_publisher(initial_frame, RfbSecurity::None, event_publisher, events)
            .await
    }

    async fn start_with_event_publisher(
        initial_frame: Option<Arc<VideoFrame>>,
        security: RfbSecurity,
        event_publisher: watch::Receiver<Option<mpsc::Sender<RfbServerEvent>>>,
        events: mpsc::Receiver<RfbServerEvent>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let source = Arc::new(MockFrameSource::new());
        if let Some(initial_frame) = initial_frame {
            source.publish_frame(initial_frame);
        }
        let (shutdown, shutdown_rx) = watch::channel(false);
        let server = RfbTcpServer::new(
            listener,
            Arc::clone(&source),
            event_publisher,
            RfbTcpConfig {
                connection: RfbConnectionSettings {
                    security,
                    ..RfbConnectionSettings::default()
                },
                ..RfbTcpConfig::default()
            },
            RfbConnectionGate::new(),
        )
        .unwrap();
        let task = tokio::spawn(server.run(shutdown_rx));
        Self {
            address,
            source,
            events,
            shutdown,
            task,
        }
    }

    async fn expect_connected(&mut self) -> RfbClientId {
        match self.events.recv().await.unwrap() {
            RfbServerEvent::Connected { client_id, .. } => client_id,
            event => panic!("expected connected event, got {event:?}"),
        }
    }

    async fn expect_disconnected(&mut self, expected_id: RfbClientId) -> RfbDisconnectReason {
        match self.events.recv().await.unwrap() {
            RfbServerEvent::Disconnected {
                client_id, reason, ..
            } => {
                assert_eq!(client_id, expected_id);
                reason
            }
            event => panic!("expected disconnected event, got {event:?}"),
        }
    }

    async fn stop(self) -> Result<(), RfbTcpServerError> {
        self.shutdown.send(true).unwrap();
        self.task.await.unwrap()
    }
}

fn default_frame() -> Arc<VideoFrame> {
    frame(1, 2, 1, PixelFormat::Rgb888, &[255, 0, 0, 0, 255, 0])
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

async fn complete_vnc_handshake(stream: &mut TcpStream, password: &[u8; 8], expect_success: bool) {
    let mut banner = [0_u8; 12];
    stream.read_exact(&mut banner).await.unwrap();
    assert_eq!(&banner, b"RFB 003.008\n");
    stream.write_all(b"RFB 003.008\n").await.unwrap();
    let mut security_types = [0_u8; 2];
    stream.read_exact(&mut security_types).await.unwrap();
    assert_eq!(security_types, [1, 2]);
    stream.write_all(&[2]).await.unwrap();
    let mut challenge = [0_u8; 16];
    stream.read_exact(&mut challenge).await.unwrap();
    stream
        .write_all(&vnc_response(password, &challenge))
        .await
        .unwrap();
    let mut result = [0_u8; 4];
    stream.read_exact(&mut result).await.unwrap();
    assert_eq!(result, if expect_success { [0; 4] } else { [0, 0, 0, 1] });
}

#[tokio::test]
async fn server_queues_second_client_until_first_disconnects() {
    let mut fixture = ServerFixture::start(16, Some(default_frame())).await;
    let mut first = TestRfbClient::connect(fixture.address).await;
    assert_eq!(first.handshake(true).await.width, 2);
    let first_id = fixture.expect_connected().await;

    let mut second = TestRfbClient::connect(fixture.address).await;
    let mut banner_byte = [0; 1];
    assert!(matches!(
        second.try_read(&mut banner_byte),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock
    ));

    drop(first);
    assert_eq!(
        fixture.expect_disconnected(first_id).await,
        RfbDisconnectReason::ClientClosed
    );
    assert_eq!(second.handshake(true).await.width, 2);
    let second_id = fixture.expect_connected().await;
    assert_ne!(first_id, second_id);

    fixture.stop().await.unwrap();
}

#[tokio::test]
async fn server_writes_negotiated_rgb565_bytes() {
    let mut fixture = ServerFixture::start(16, Some(default_frame())).await;
    let mut client = TestRfbClient::connect(fixture.address).await;
    client.handshake(true).await;
    fixture.expect_connected().await;

    client.set_rgb565().await;
    client.request_update(false, 0, 0, 2, 1).await;
    let update = client.read_update(4).await;

    assert_eq!(update.encoding, 0);
    assert_eq!(update.pixels, [0x00, 0xf8, 0xe0, 0x07]);
    fixture.stop().await.unwrap();
}

#[tokio::test]
async fn bounded_event_channel_preserves_input_order() {
    let mut fixture = ServerFixture::start(1, Some(default_frame())).await;
    let mut client = TestRfbClient::connect(fixture.address).await;
    client.handshake(true).await;
    let client_id = fixture.expect_connected().await;

    client.send_key(true, 0x41).await;
    client.send_key(false, 0x41).await;
    assert_eq!(
        fixture.events.recv().await,
        Some(RfbServerEvent::Key {
            client_id,
            down: true,
            keysym: 0x41,
        })
    );
    assert_eq!(
        fixture.events.recv().await,
        Some(RfbServerEvent::Key {
            client_id,
            down: false,
            keysym: 0x41,
        })
    );

    client.send_pointer(1, 20, 30).await;
    client.send_cut_text(b"abc").await;
    assert!(matches!(
        fixture.events.recv().await,
        Some(RfbServerEvent::Pointer {
            client_id: actual,
            button_mask: 1,
            x: 20,
            y: 30,
            framebuffer_size,
        }) if actual == client_id && framebuffer_size == RfbSize::new(2, 1).unwrap()
    ));
    assert_eq!(
        fixture.events.recv().await,
        Some(RfbServerEvent::CutText {
            client_id,
            bytes: b"abc".to_vec(),
        })
    );

    fixture.stop().await.unwrap();
}

#[tokio::test]
async fn missing_initial_frame_waits_and_completes_when_frame_arrives() {
    // #110：视频恢复/重启窗口内首帧暂时缺失时，服务器必须在握手超时预算内
    // 等待首帧而不是立即断开——刷新页面的自动重连恰好落入该窗口时不应被踢掉。
    let mut fixture = ServerFixture::start(16, None).await;
    let mut client = TestRfbClient::connect(fixture.address).await;
    let source = fixture.source.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        source.publish_frame(default_frame());
    });
    let init = client.handshake(true).await;
    assert_eq!(init.width, 2);
    fixture.expect_connected().await;
    fixture.stop().await.unwrap();
}

#[tokio::test]
async fn mjpeg_initial_frame_handshake_succeeds() {
    // MJPEG 帧现在可以用于握手（从元数据获取尺寸，跳过 frame_view）。
    let mjpeg = frame(1, 2, 2, PixelFormat::Mjpeg, &[0xFF, 0xD8, 0x00, 0x00]);
    let mut fixture = ServerFixture::start(16, Some(mjpeg)).await;
    let mut client = TestRfbClient::connect(fixture.address).await;
    let init = client.handshake(true).await;
    assert_eq!(init.width, 2);
    assert_eq!(init.height, 2);
    fixture.expect_connected().await;
    fixture.stop().await.unwrap();
}

#[tokio::test]
async fn protocol_error_disconnects_client_and_server_continues() {
    let mut fixture = ServerFixture::start(16, Some(default_frame())).await;
    let mut first = TestRfbClient::connect(fixture.address).await;
    first.handshake(true).await;
    let first_id = fixture.expect_connected().await;
    first.send_raw(&[0xff]).await;
    assert_eq!(
        fixture.expect_disconnected(first_id).await,
        RfbDisconnectReason::Protocol(RfbProtocolError::UnsupportedClientMessageType(0xff))
    );

    let mut second = TestRfbClient::connect(fixture.address).await;
    second.handshake(true).await;
    fixture.expect_connected().await;
    fixture.stop().await.unwrap();
}

#[tokio::test]
async fn missing_event_sender_allows_view_only_tcp_updates() {
    let mut fixture = ServerFixture::start_view_only(Some(default_frame())).await;
    let mut client = TestRfbClient::connect(fixture.address).await;

    let init = client.handshake(true).await;
    assert_eq!(
        (init.width, init.height, init.name.as_str()),
        (2, 1, "my_ipkvm")
    );
    client.send_key(true, 0x41).await;
    client.request_update(false, 0, 0, 2, 1).await;
    let update = client.read_update(8).await;
    assert_eq!(update.encoding, 0);
    assert_eq!(update.pixels, [0, 0, 255, 0, 0, 255, 0, 0]);
    assert!(matches!(
        fixture.events.try_recv(),
        Err(mpsc::error::TryRecvError::Disconnected)
    ));
    fixture.stop().await.unwrap();
}

#[tokio::test]
async fn closed_event_receiver_allows_view_only_tcp_updates() {
    let (event_tx, event_rx) = mpsc::channel(1);
    drop(event_rx);
    let replacement_events = mpsc::channel(1).1;
    let event_publisher = watch::channel(Some(event_tx)).1;
    let fixture = ServerFixture::start_with_event_publisher(
        Some(default_frame()),
        RfbSecurity::None,
        event_publisher,
        replacement_events,
    )
    .await;
    let mut client = TestRfbClient::connect(fixture.address).await;

    client.handshake(true).await;
    client.request_update(false, 0, 0, 2, 1).await;
    let update = client.read_update(8).await;
    assert_eq!(update.encoding, 0);
    fixture.stop().await.unwrap();
}

#[tokio::test]
async fn shutdown_ends_active_connection_and_server() {
    let mut fixture = ServerFixture::start(16, Some(default_frame())).await;
    let mut client = TestRfbClient::connect(fixture.address).await;
    client.handshake(true).await;
    let client_id = fixture.expect_connected().await;

    fixture.shutdown.send(true).unwrap();
    assert_eq!(
        fixture.expect_disconnected(client_id).await,
        RfbDisconnectReason::ServerShutdown
    );
    assert!(fixture.task.await.unwrap().is_ok());
    assert_eq!(
        client.read_banner().await.unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
}

#[tokio::test]
async fn vnc_password_handshake_succeeds_over_tcp() {
    let mut fixture = ServerFixture::start_with_security(
        16,
        Some(default_frame()),
        RfbSecurity::Vnc {
            password: *b"12345678",
        },
    )
    .await;
    let mut stream = TcpStream::connect(fixture.address).await.unwrap();

    complete_vnc_handshake(&mut stream, b"12345678", true).await;
    stream.write_all(&[1]).await.unwrap();
    let mut server_init = [0_u8; 24];
    stream.read_exact(&mut server_init).await.unwrap();
    assert!(matches!(
        fixture.events.recv().await,
        Some(RfbServerEvent::Connected { .. })
    ));

    drop(stream);
    fixture.stop().await.unwrap();
}

#[tokio::test]
async fn vnc_password_handshake_rejects_wrong_password_over_tcp() {
    let mut fixture = ServerFixture::start_with_security(
        16,
        Some(default_frame()),
        RfbSecurity::Vnc {
            password: *b"12345678",
        },
    )
    .await;
    let mut stream = TcpStream::connect(fixture.address).await.unwrap();

    complete_vnc_handshake(&mut stream, b"wrongpas", false).await;
    // 失败后服务器关闭连接：读返回 0。
    let mut tail = Vec::new();
    stream.read_to_end(&mut tail).await.unwrap();
    assert!(!matches!(
        fixture.events.recv().await,
        Some(RfbServerEvent::Connected { .. })
    ));

    drop(stream);
    fixture.stop().await.unwrap();
}
