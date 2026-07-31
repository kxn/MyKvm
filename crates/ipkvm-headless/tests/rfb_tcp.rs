mod support;

use std::{io, sync::Arc};

use ipkvm_headless::rfb_tcp::{
    RfbClientId, RfbDisconnectReason, RfbTcpConfig, RfbTcpEvent, RfbTcpServer, RfbTcpServerError,
};
use ipkvm_rfb::{RfbProtocolError, RfbSize};
use ipkvm_video::{MonotonicTimestamp, PixelFormat, VideoFrame, mock::MockFrameSource};
use support::TestRfbClient;
use tokio::{
    net::TcpListener,
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
    events: mpsc::Receiver<RfbTcpEvent>,
    shutdown: watch::Sender<bool>,
    task: tokio::task::JoinHandle<Result<(), RfbTcpServerError>>,
}

impl ServerFixture {
    async fn start(event_capacity: usize, initial_frame: Option<Arc<VideoFrame>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let source = Arc::new(MockFrameSource::new());
        if let Some(initial_frame) = initial_frame {
            source.publish_frame(initial_frame);
        }
        let (event_tx, events) = mpsc::channel(event_capacity);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let server = RfbTcpServer::new(
            listener,
            Arc::clone(&source),
            event_tx,
            RfbTcpConfig::default(),
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
            RfbTcpEvent::Connected { client_id, .. } => client_id,
            event => panic!("expected connected event, got {event:?}"),
        }
    }

    async fn expect_disconnected(&mut self, expected_id: RfbClientId) -> RfbDisconnectReason {
        match self.events.recv().await.unwrap() {
            RfbTcpEvent::Disconnected {
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
    frame(
        1,
        2,
        1,
        PixelFormat::Bgra8888,
        &[0, 0, 255, 0, 0, 255, 0, 0],
    )
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
        Some(RfbTcpEvent::Key {
            client_id,
            down: true,
            keysym: 0x41,
        })
    );
    assert_eq!(
        fixture.events.recv().await,
        Some(RfbTcpEvent::Key {
            client_id,
            down: false,
            keysym: 0x41,
        })
    );

    client.send_pointer(1, 20, 30).await;
    client.send_cut_text(b"abc").await;
    assert!(matches!(
        fixture.events.recv().await,
        Some(RfbTcpEvent::Pointer {
            client_id: actual,
            button_mask: 1,
            x: 20,
            y: 30,
            framebuffer_size,
        }) if actual == client_id && framebuffer_size == RfbSize::new(2, 1).unwrap()
    ));
    assert_eq!(
        fixture.events.recv().await,
        Some(RfbTcpEvent::CutText {
            client_id,
            bytes: b"abc".to_vec(),
        })
    );

    fixture.stop().await.unwrap();
}

#[tokio::test]
async fn missing_initial_frame_disconnects_then_valid_frame_reconnects() {
    let mut fixture = ServerFixture::start(16, None).await;
    let mut first = TestRfbClient::connect(fixture.address).await;
    assert_eq!(
        first.read_banner().await.unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    let reason = match fixture.events.recv().await.unwrap() {
        RfbTcpEvent::Disconnected { reason, .. } => reason,
        event => panic!("expected disconnected event, got {event:?}"),
    };
    assert_eq!(
        reason,
        RfbDisconnectReason::Frame(ipkvm_headless::rfb_tcp::RfbTcpFrameError::FrameUnavailable)
    );

    fixture.source.publish_frame(default_frame());
    let mut second = TestRfbClient::connect(fixture.address).await;
    assert_eq!(second.handshake(true).await.width, 2);
    fixture.expect_connected().await;
    fixture.stop().await.unwrap();
}

#[tokio::test]
async fn invalid_initial_frame_disconnects_then_valid_frame_reconnects() {
    let invalid = frame(1, 1, 1, PixelFormat::Mjpeg, &[0; 4]);
    let mut fixture = ServerFixture::start(16, Some(invalid)).await;
    let mut first = TestRfbClient::connect(fixture.address).await;
    assert_eq!(
        first.read_banner().await.unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    let reason = match fixture.events.recv().await.unwrap() {
        RfbTcpEvent::Disconnected { reason, .. } => reason,
        event => panic!("expected disconnected event, got {event:?}"),
    };
    assert!(matches!(
        reason,
        RfbDisconnectReason::Frame(
            ipkvm_headless::rfb_tcp::RfbTcpFrameError::UnsupportedPixelFormat(PixelFormat::Mjpeg)
        )
    ));

    fixture.source.publish_frame(default_frame());
    let mut second = TestRfbClient::connect(fixture.address).await;
    assert_eq!(second.handshake(true).await.width, 2);
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
async fn closed_event_receiver_is_a_server_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let source = Arc::new(MockFrameSource::new());
    source.publish_frame(default_frame());
    let (event_tx, event_rx) = mpsc::channel(1);
    drop(event_rx);
    let (_shutdown, shutdown_rx) = watch::channel(false);
    let server = RfbTcpServer::new(listener, source, event_tx, RfbTcpConfig::default()).unwrap();

    assert!(matches!(
        server.run(shutdown_rx).await,
        Err(RfbTcpServerError::EventChannelClosed)
    ));
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
