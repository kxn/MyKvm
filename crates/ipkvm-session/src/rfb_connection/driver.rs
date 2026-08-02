use std::net::SocketAddr;

use ipkvm_rfb::{
    FramebufferUpdateOutcome, FramebufferUpdateRequest, RfbConfigError, RfbConnectionConfig,
    RfbConnectionCore, RfbConnectionState, RfbEncodeError, RfbEvent, RfbProtocolError,
};
use ipkvm_video::{FrameReceiver, SharedVideoFrame};
use thiserror::Error;
use tokio::{
    sync::{mpsc, watch},
    time,
};

use super::{
    RfbClientId, RfbConnectionSettings, RfbDisconnectReason, RfbFrameError, RfbServerEvent,
    RfbTransport, RfbTransportError, RfbTransportRead, frame::frame_view,
    pending::PendingFramebufferRequest,
};

#[derive(Debug, Error)]
pub enum RfbConnectionError {
    #[error("RFB handshake timed out")]
    HandshakeTimeout,
    #[error("invalid RFB connection configuration: {0}")]
    CoreConfig(#[from] RfbConfigError),
    #[error("RFB protocol error: {0}")]
    Protocol(#[from] RfbProtocolError),
    #[error("RFB encoding error: {0}")]
    Encode(#[from] RfbEncodeError),
    #[error("video frame error: {0}")]
    Frame(#[from] RfbFrameError),
    #[error("RFB transport error: {0}")]
    Transport(#[from] RfbTransportError),
    #[error("RFB event receiver is closed")]
    EventChannelClosed,
}

#[derive(Debug)]
pub enum ConnectionEnd {
    ClientClosed,
    ServerShutdown,
    Failed(RfbConnectionError),
}

struct ConnectionState {
    client_id: RfbClientId,
    peer_addr: SocketAddr,
    core: RfbConnectionCore,
    frame_rx: FrameReceiver,
    pending: PendingFramebufferRequest,
    last_observed_seq: u64,
    last_sent_seq: Option<u64>,
}

impl ConnectionEnd {
    pub(crate) fn reason(&self) -> Option<RfbDisconnectReason> {
        Some(match self {
            Self::ClientClosed => RfbDisconnectReason::ClientClosed,
            Self::ServerShutdown => RfbDisconnectReason::ServerShutdown,
            Self::Failed(RfbConnectionError::HandshakeTimeout) => {
                RfbDisconnectReason::HandshakeTimeout
            }
            Self::Failed(RfbConnectionError::CoreConfig(error)) => {
                RfbDisconnectReason::CoreConfig(error.clone())
            }
            Self::Failed(RfbConnectionError::Protocol(error)) => {
                RfbDisconnectReason::Protocol(error.clone())
            }
            Self::Failed(RfbConnectionError::Encode(error)) => {
                RfbDisconnectReason::Encode(error.clone())
            }
            Self::Failed(RfbConnectionError::Frame(error)) => {
                RfbDisconnectReason::Frame(error.clone())
            }
            Self::Failed(RfbConnectionError::Transport(RfbTransportError::Io(error))) => {
                RfbDisconnectReason::Io(error.kind())
            }
            Self::Failed(RfbConnectionError::Transport(RfbTransportError::WebSocket {
                ..
            })) => RfbDisconnectReason::WebSocket,
            Self::Failed(RfbConnectionError::Transport(
                RfbTransportError::UnexpectedTextMessage,
            )) => RfbDisconnectReason::UnexpectedTextMessage,
            Self::Failed(RfbConnectionError::EventChannelClosed) => return None,
        })
    }
}

pub(crate) async fn run_connection<T: RfbTransport>(
    client_id: RfbClientId,
    peer_addr: SocketAddr,
    mut transport: T,
    frame_rx: FrameReceiver,
    event_tx: mpsc::Sender<RfbServerEvent>,
    settings: RfbConnectionSettings,
    shutdown: watch::Receiver<bool>,
) -> ConnectionEnd {
    let result = drive_connection(
        client_id,
        peer_addr,
        &mut transport,
        frame_rx,
        event_tx,
        settings,
        shutdown,
    )
    .await;
    transport.close().await;

    match result {
        Ok(end) => end,
        Err(error) => ConnectionEnd::Failed(error),
    }
}

async fn drive_connection<T: RfbTransport>(
    client_id: RfbClientId,
    peer_addr: SocketAddr,
    transport: &mut T,
    frame_rx: FrameReceiver,
    event_tx: mpsc::Sender<RfbServerEvent>,
    settings: RfbConnectionSettings,
    mut shutdown: watch::Receiver<bool>,
) -> Result<ConnectionEnd, RfbConnectionError> {
    if shutdown_is_requested(&shutdown) {
        return Ok(ConnectionEnd::ServerShutdown);
    }

    let initial_frame = frame_rx
        .borrow()
        .clone()
        .ok_or(RfbFrameError::FrameUnavailable)?;
    let initial_view = frame_view(&initial_frame)?;
    let mut core = RfbConnectionCore::new(RfbConnectionConfig {
        desktop_name: settings.desktop_name.clone(),
        initial_size: initial_view.size(),
        limits: settings.protocol_limits,
    })?;
    write_core_output(transport, &mut core).await?;

    let mut state = ConnectionState {
        client_id,
        peer_addr,
        core,
        frame_rx,
        pending: PendingFramebufferRequest::default(),
        last_observed_seq: initial_frame.seq,
        last_sent_seq: None,
    };
    let handshake_deadline = time::sleep(settings.handshake_timeout);
    tokio::pin!(handshake_deadline);
    let mut input = Vec::new();

    loop {
        let awaiting_handshake = state.awaiting_handshake();
        tokio::select! {
            read = transport.receive_into(&mut input) => {
                match read? {
                    RfbTransportRead::Data => {
                        debug_assert!(!input.is_empty());
                        let events = state.core.push_input(&input);
                        write_core_output(transport, &mut state.core).await?;
                        if let Some(error) = state.handle_core_events(transport, &event_tx, events).await? {
                            return Err(error.into());
                        }
                        write_core_output(transport, &mut state.core).await?;
                    }
                    RfbTransportRead::Continue => {
                        debug_assert!(input.is_empty());
                    }
                    RfbTransportRead::Closed => {
                        debug_assert!(input.is_empty());
                        return Ok(ConnectionEnd::ClientClosed);
                    }
                }
            }
            changed = state.frame_rx.changed(), if state.pending.get().is_some() => {
                changed.map_err(|_| RfbFrameError::FrameUnavailable)?;
                state.handle_frame_change(transport).await?;
            }
            _ = wait_for_shutdown(&mut shutdown) => {
                return Ok(ConnectionEnd::ServerShutdown);
            }
            _ = event_tx.closed() => {
                return Err(RfbConnectionError::EventChannelClosed);
            }
            _ = &mut handshake_deadline, if awaiting_handshake => {
                return Err(RfbConnectionError::HandshakeTimeout);
            }
        }
    }
}

impl ConnectionState {
    fn awaiting_handshake(&self) -> bool {
        self.core.state() != RfbConnectionState::Normal
    }

    async fn handle_core_events(
        &mut self,
        transport: &mut impl RfbTransport,
        event_tx: &mpsc::Sender<RfbServerEvent>,
        events: Vec<Result<RfbEvent, RfbProtocolError>>,
    ) -> Result<Option<RfbProtocolError>, RfbConnectionError> {
        for event in events {
            let event = match event {
                Ok(event) => event,
                Err(error) => return Ok(Some(error)),
            };
            let event = match event {
                RfbEvent::HandshakeCompleted { shared } => RfbServerEvent::Connected {
                    client_id: self.client_id,
                    peer_addr: self.peer_addr,
                    shared,
                },
                RfbEvent::FramebufferUpdateRequested(request) => {
                    self.handle_update_request(transport, request).await?;
                    continue;
                }
                RfbEvent::Key { down, keysym } => RfbServerEvent::Key {
                    client_id: self.client_id,
                    down,
                    keysym,
                },
                RfbEvent::Pointer {
                    button_mask,
                    x,
                    y,
                    framebuffer_size,
                } => RfbServerEvent::Pointer {
                    client_id: self.client_id,
                    button_mask,
                    x,
                    y,
                    framebuffer_size,
                },
                RfbEvent::CutText(bytes) => RfbServerEvent::CutText {
                    client_id: self.client_id,
                    bytes,
                },
                RfbEvent::EnableContinuousUpdates { enable, rectangle } => {
                    RfbServerEvent::ContinuousUpdates {
                        client_id: self.client_id,
                        enable,
                        rectangle,
                    }
                }
            };
            send_event(event_tx, event).await?;
        }
        Ok(None)
    }

    async fn handle_update_request(
        &mut self,
        transport: &mut impl RfbTransport,
        request: FramebufferUpdateRequest,
    ) -> Result<(), RfbConnectionError> {
        let frame = self.latest_frame()?;
        let size = frame_view(&frame)?.size();
        self.pending.merge(request, size);

        let should_send = self
            .pending
            .get()
            .is_some_and(|merged| !merged.incremental || self.last_sent_seq != Some(frame.seq));
        if should_send && let Some(request) = self.pending.take() {
            let outcome = queue_and_write_frame(transport, &mut self.core, &frame, request).await?;
            update_last_sent_sequence(&mut self.last_sent_seq, frame.seq, outcome);
        }
        Ok(())
    }

    async fn handle_frame_change(
        &mut self,
        transport: &mut impl RfbTransport,
    ) -> Result<(), RfbConnectionError> {
        let frame = self.latest_frame()?;
        if self.last_sent_seq != Some(frame.seq)
            && let Some(request) = self.pending.take()
        {
            let outcome = queue_and_write_frame(transport, &mut self.core, &frame, request).await?;
            update_last_sent_sequence(&mut self.last_sent_seq, frame.seq, outcome);
        }
        Ok(())
    }

    fn latest_frame(&mut self) -> Result<SharedVideoFrame, RfbFrameError> {
        let frame = self
            .frame_rx
            .borrow()
            .clone()
            .ok_or(RfbFrameError::FrameUnavailable)?;
        if frame.seq < self.last_observed_seq {
            return Err(RfbFrameError::FrameSequenceRegressed {
                previous: self.last_observed_seq,
                actual: frame.seq,
            });
        }
        self.last_observed_seq = frame.seq;
        Ok(frame)
    }
}

async fn write_core_output<T: RfbTransport>(
    transport: &mut T,
    core: &mut RfbConnectionCore,
) -> Result<(), RfbConnectionError> {
    let output = core.take_output();
    if !output.is_empty() {
        transport.send_binary(output).await?;
    }
    Ok(())
}

async fn queue_and_write_frame<T: RfbTransport>(
    transport: &mut T,
    core: &mut RfbConnectionCore,
    frame: &SharedVideoFrame,
    request: FramebufferUpdateRequest,
) -> Result<FramebufferUpdateOutcome, RfbConnectionError> {
    let outcome = core.queue_framebuffer_update(frame_view(frame)?, request)?;
    write_core_output(transport, core).await?;
    Ok(outcome)
}

fn update_last_sent_sequence(
    last_sent_seq: &mut Option<u64>,
    frame_seq: u64,
    outcome: FramebufferUpdateOutcome,
) {
    if !matches!(outcome, FramebufferUpdateOutcome::ResizeAnnounced { .. }) {
        *last_sent_seq = Some(frame_seq);
    }
}

async fn send_event(
    sender: &mpsc::Sender<RfbServerEvent>,
    event: RfbServerEvent,
) -> Result<(), RfbConnectionError> {
    sender
        .send(event)
        .await
        .map_err(|_| RfbConnectionError::EventChannelClosed)
}

fn shutdown_is_requested(shutdown: &watch::Receiver<bool>) -> bool {
    *shutdown.borrow()
}

async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    loop {
        if shutdown_is_requested(shutdown) {
            return;
        }
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        io,
        net::SocketAddr,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use ipkvm_rfb::{RfbEncodeError, RfbProtocolError, RfbSize};
    use ipkvm_video::{
        FrameSource, MonotonicTimestamp, PixelFormat, VideoFrame, mock::MockFrameSource,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::{mpsc, watch},
    };

    use super::super::*;
    use super::*;

    enum FakeRead {
        Data(Vec<u8>),
        Continue,
        Closed,
    }

    struct FakeTransport {
        reads: VecDeque<Result<FakeRead, RfbTransportError>>,
        sent: Arc<Mutex<Vec<Vec<u8>>>>,
        closed: Arc<Mutex<usize>>,
    }

    struct TestTransport {
        stream: TcpStream,
    }

    impl TestTransport {
        fn new(stream: TcpStream) -> Self {
            Self { stream }
        }
    }

    impl FakeTransport {
        fn from_reads(reads: impl IntoIterator<Item = FakeRead>) -> Self {
            Self {
                reads: reads.into_iter().map(Ok).collect(),
                sent: Arc::new(Mutex::new(Vec::new())),
                closed: Arc::new(Mutex::new(0)),
            }
        }

        fn from_error(error: RfbTransportError) -> Self {
            Self {
                reads: [Err(error)].into(),
                sent: Arc::new(Mutex::new(Vec::new())),
                closed: Arc::new(Mutex::new(0)),
            }
        }
    }

    impl RfbTransport for FakeTransport {
        async fn receive_into(
            &mut self,
            buffer: &mut Vec<u8>,
        ) -> Result<RfbTransportRead, RfbTransportError> {
            buffer.clear();
            match self
                .reads
                .pop_front()
                .expect("fake transport ran out of reads")?
            {
                FakeRead::Data(bytes) => {
                    buffer.extend(bytes);
                    Ok(RfbTransportRead::Data)
                }
                FakeRead::Continue => Ok(RfbTransportRead::Continue),
                FakeRead::Closed => Ok(RfbTransportRead::Closed),
            }
        }

        async fn send_binary(&mut self, bytes: Vec<u8>) -> Result<(), RfbTransportError> {
            self.sent.lock().unwrap().push(bytes);
            Ok(())
        }

        async fn close(&mut self) {
            *self.closed.lock().unwrap() += 1;
        }
    }

    impl RfbTransport for TestTransport {
        async fn receive_into(
            &mut self,
            buffer: &mut Vec<u8>,
        ) -> Result<RfbTransportRead, RfbTransportError> {
            buffer.clear();
            buffer.resize(16 * 1024, 0);
            let count = self.stream.read(buffer).await?;
            buffer.truncate(count);
            if count == 0 {
                Ok(RfbTransportRead::Closed)
            } else {
                Ok(RfbTransportRead::Data)
            }
        }

        async fn send_binary(&mut self, bytes: Vec<u8>) -> Result<(), RfbTransportError> {
            self.stream.write_all(&bytes).await?;
            Ok(())
        }

        async fn close(&mut self) {
            let _ = self.stream.shutdown().await;
        }
    }

    struct TestConnectionResult {
        end: ConnectionEnd,
        sent: Vec<Vec<u8>>,
        close_count: usize,
    }

    async fn run_test_connection(transport: FakeTransport) -> TestConnectionResult {
        let sent = Arc::clone(&transport.sent);
        let closed = Arc::clone(&transport.closed);
        let frame_source = MockFrameSource::new();
        frame_source.publish_frame(shared_bgra_frame(1, 1, 1, &[0; 4]));
        let (event_tx, _event_rx) = mpsc::channel(1);
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let end = run_connection(
            RfbClientId::for_test(1),
            "127.0.0.1:5900".parse().unwrap(),
            transport,
            frame_source.subscribe(),
            event_tx,
            RfbConnectionSettings::default(),
            shutdown_rx,
        )
        .await;
        let sent = sent.lock().unwrap().clone();
        let close_count = *closed.lock().unwrap();
        TestConnectionResult {
            end,
            sent,
            close_count,
        }
    }

    #[tokio::test]
    async fn control_messages_do_not_enter_the_rfb_core() {
        let transport = FakeTransport::from_reads([
            FakeRead::Continue,
            FakeRead::Data(b"RFB 003.008\n".to_vec()),
            FakeRead::Closed,
        ]);
        let result = run_test_connection(transport).await;
        assert!(result.sent.starts_with(&[b"RFB 003.008\n".to_vec()]));
        assert!(matches!(result.end, ConnectionEnd::ClientClosed));
        assert_eq!(result.close_count, 1);
    }

    #[tokio::test]
    async fn transport_text_error_has_a_stable_disconnect_reason() {
        let transport = FakeTransport::from_error(RfbTransportError::UnexpectedTextMessage);
        let result = run_test_connection(transport).await;
        assert_eq!(
            result.end.reason(),
            Some(RfbDisconnectReason::UnexpectedTextMessage)
        );
        assert_eq!(result.close_count, 1);
    }

    #[tokio::test]
    async fn transport_websocket_error_has_a_stable_disconnect_reason() {
        let transport = FakeTransport::from_error(RfbTransportError::websocket(io::Error::other(
            "test WebSocket transport failure",
        )));
        let result = run_test_connection(transport).await;
        assert_eq!(result.end.reason(), Some(RfbDisconnectReason::WebSocket));
    }

    fn shared_bgra_frame(seq: u64, width: u32, height: u32, data: &[u8]) -> Arc<VideoFrame> {
        Arc::new(VideoFrame::new(
            seq,
            MonotonicTimestamp::from_nanos(seq),
            width,
            height,
            width * 4,
            PixelFormat::Bgra8888,
            Arc::from(data.to_vec().into_boxed_slice()),
        ))
    }

    async fn tcp_pair() -> (TcpStream, TcpStream, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (server, peer_addr) = listener.accept().await.unwrap();
        (server, client, peer_addr)
    }

    async fn read_exact_vec(stream: &mut TcpStream, length: usize) -> Vec<u8> {
        let mut bytes = vec![0; length];
        stream.read_exact(&mut bytes).await.unwrap();
        bytes
    }

    async fn write_fragmented(stream: &mut TcpStream, bytes: &[u8]) {
        for byte in bytes {
            stream.write_all(&[*byte]).await.unwrap();
            tokio::task::yield_now().await;
        }
    }

    async fn finish_handshake(stream: &mut TcpStream, shared: bool) -> (u16, u16, String) {
        assert_eq!(read_exact_vec(stream, 12).await, b"RFB 003.008\n");
        write_fragmented(stream, b"RFB 003.008\n").await;
        assert_eq!(read_exact_vec(stream, 2).await, [1, 1]);
        stream.write_all(&[1]).await.unwrap();
        assert_eq!(read_exact_vec(stream, 4).await, [0, 0, 0, 0]);
        stream.write_all(&[u8::from(shared)]).await.unwrap();

        let header = read_exact_vec(stream, 24).await;
        let width = u16::from_be_bytes([header[0], header[1]]);
        let height = u16::from_be_bytes([header[2], header[3]]);
        let name_length =
            u32::from_be_bytes([header[20], header[21], header[22], header[23]]) as usize;
        let name = String::from_utf8(read_exact_vec(stream, name_length).await).unwrap();
        (width, height, name)
    }

    #[derive(Debug, Eq, PartialEq)]
    struct WireUpdate {
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        encoding: i32,
        pixels: Vec<u8>,
    }

    async fn read_update(stream: &mut TcpStream, pixel_bytes: usize) -> WireUpdate {
        tokio::time::timeout(Duration::from_secs(1), async {
            let header = read_exact_vec(stream, 16).await;
            assert_eq!(&header[..4], &[0, 0, 0, 1]);
            WireUpdate {
                x: u16::from_be_bytes([header[4], header[5]]),
                y: u16::from_be_bytes([header[6], header[7]]),
                width: u16::from_be_bytes([header[8], header[9]]),
                height: u16::from_be_bytes([header[10], header[11]]),
                encoding: i32::from_be_bytes([header[12], header[13], header[14], header[15]]),
                pixels: read_exact_vec(stream, pixel_bytes).await,
            }
        })
        .await
        .expect("server did not send a framebuffer update")
    }

    async fn send_update_request(
        stream: &mut TcpStream,
        incremental: bool,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
    ) {
        let mut message = vec![3, u8::from(incremental)];
        message.extend_from_slice(&x.to_be_bytes());
        message.extend_from_slice(&y.to_be_bytes());
        message.extend_from_slice(&width.to_be_bytes());
        message.extend_from_slice(&height.to_be_bytes());
        stream.write_all(&message).await.unwrap();
    }

    async fn send_set_encodings(stream: &mut TcpStream, encodings: &[i32]) {
        let mut message = vec![2, 0];
        message.extend_from_slice(&(encodings.len() as u16).to_be_bytes());
        for encoding in encodings {
            message.extend_from_slice(&encoding.to_be_bytes());
        }
        stream.write_all(&message).await.unwrap();
    }

    async fn send_key(stream: &mut TcpStream, down: bool, keysym: u32) {
        let mut message = vec![4, u8::from(down), 0, 0];
        message.extend_from_slice(&keysym.to_be_bytes());
        stream.write_all(&message).await.unwrap();
    }

    fn spawn_connection(
        client_id: RfbClientId,
        peer_addr: SocketAddr,
        stream: TcpStream,
        frame_source: &MockFrameSource,
        event_tx: mpsc::Sender<RfbServerEvent>,
        settings: RfbConnectionSettings,
        shutdown_rx: watch::Receiver<bool>,
    ) -> tokio::task::JoinHandle<ConnectionEnd> {
        let frame_rx = frame_source.subscribe();
        tokio::spawn(run_connection(
            client_id,
            peer_addr,
            TestTransport::new(stream),
            frame_rx,
            event_tx,
            settings,
            shutdown_rx,
        ))
    }

    async fn completed_connection(
        client_id: RfbClientId,
        frame_source: &MockFrameSource,
        settings: RfbConnectionSettings,
    ) -> (
        tokio::task::JoinHandle<ConnectionEnd>,
        TcpStream,
        mpsc::Receiver<RfbServerEvent>,
        watch::Sender<bool>,
    ) {
        let (server_stream, mut client_stream, peer_addr) = tcp_pair().await;
        let (event_tx, mut event_rx) = mpsc::channel(16);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = spawn_connection(
            client_id,
            peer_addr,
            server_stream,
            frame_source,
            event_tx,
            settings,
            shutdown_rx,
        );
        finish_handshake(&mut client_stream, true).await;
        assert!(matches!(
            event_rx.recv().await,
            Some(RfbServerEvent::Connected {
                client_id: actual,
                ..
            }) if actual == client_id
        ));
        (task, client_stream, event_rx, shutdown_tx)
    }

    #[tokio::test]
    async fn fragmented_handshake_emits_connected_event() {
        let frame_source = MockFrameSource::new();
        frame_source.publish_frame(shared_bgra_frame(1, 2, 1, &[1, 2, 3, 0, 4, 5, 6, 0]));
        let (server_stream, mut client_stream, peer_addr) = tcp_pair().await;
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let mut task = spawn_connection(
            RfbClientId(1),
            peer_addr,
            server_stream,
            &frame_source,
            event_tx,
            RfbConnectionSettings::default(),
            shutdown_rx,
        );

        let handshake = tokio::select! {
            handshake = finish_handshake(&mut client_stream, true) => handshake,
            end = &mut task => panic!("connection ended before banner: {end:?}"),
        };
        assert_eq!(handshake, (2, 1, "my_ipkvm".to_string()));
        assert!(matches!(
            event_rx.recv().await,
            Some(RfbServerEvent::Connected {
                client_id: RfbClientId(1),
                peer_addr: actual_peer,
                shared: true,
            }) if actual_peer == peer_addr
        ));

        drop(client_stream);
        assert!(matches!(task.await.unwrap(), ConnectionEnd::ClientClosed));
    }

    #[tokio::test]
    async fn pipelined_input_events_preserve_order() {
        let frame_source = MockFrameSource::new();
        frame_source.publish_frame(shared_bgra_frame(1, 2, 1, &[0; 8]));
        let (server_stream, mut client_stream, peer_addr) = tcp_pair().await;
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = spawn_connection(
            RfbClientId(7),
            peer_addr,
            server_stream,
            &frame_source,
            event_tx,
            RfbConnectionSettings::default(),
            shutdown_rx,
        );

        finish_handshake(&mut client_stream, false).await;
        assert!(matches!(
            event_rx.recv().await,
            Some(RfbServerEvent::Connected { shared: false, .. })
        ));

        let mut messages = vec![4, 1, 0, 0, 0, 0, 0, 0x41];
        messages.extend_from_slice(&[5, 3, 0, 10, 0, 20]);
        messages.extend_from_slice(&[6, 0, 0, 0, 0, 0, 0, 3, b'a', b'b', b'c']);
        messages.extend_from_slice(&[150, 1, 0, 1, 0, 2, 0, 3, 0, 4]);
        client_stream.write_all(&messages).await.unwrap();

        assert_eq!(
            event_rx.recv().await,
            Some(RfbServerEvent::Key {
                client_id: RfbClientId(7),
                down: true,
                keysym: 0x41,
            })
        );
        assert_eq!(
            event_rx.recv().await,
            Some(RfbServerEvent::Pointer {
                client_id: RfbClientId(7),
                button_mask: 3,
                x: 10,
                y: 20,
                framebuffer_size: RfbSize::new(2, 1).unwrap(),
            })
        );
        assert_eq!(
            event_rx.recv().await,
            Some(RfbServerEvent::CutText {
                client_id: RfbClientId(7),
                bytes: b"abc".to_vec(),
            })
        );
        assert!(matches!(
            event_rx.recv().await,
            Some(RfbServerEvent::ContinuousUpdates {
                client_id: RfbClientId(7),
                enable: true,
                rectangle,
            }) if rectangle.x == 1
                && rectangle.y == 2
                && rectangle.width == 3
                && rectangle.height == 4
        ));

        drop(client_stream);
        assert!(matches!(task.await.unwrap(), ConnectionEnd::ClientClosed));
    }

    #[tokio::test]
    async fn protocol_error_follows_prior_valid_event() {
        let frame_source = MockFrameSource::new();
        frame_source.publish_frame(shared_bgra_frame(1, 1, 1, &[0; 4]));
        let (server_stream, mut client_stream, peer_addr) = tcp_pair().await;
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = spawn_connection(
            RfbClientId(2),
            peer_addr,
            server_stream,
            &frame_source,
            event_tx,
            RfbConnectionSettings::default(),
            shutdown_rx,
        );

        finish_handshake(&mut client_stream, true).await;
        event_rx.recv().await.unwrap();
        client_stream
            .write_all(&[4, 1, 0, 0, 0, 0, 0, 0x42, 0xff])
            .await
            .unwrap();

        assert!(matches!(
            event_rx.recv().await,
            Some(RfbServerEvent::Key {
                down: true,
                keysym: 0x42,
                ..
            })
        ));
        assert!(matches!(
            task.await.unwrap(),
            ConnectionEnd::Failed(RfbConnectionError::Protocol(
                RfbProtocolError::UnsupportedClientMessageType(0xff)
            ))
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn handshake_timeout_uses_paused_clock() {
        let frame_source = MockFrameSource::new();
        frame_source.publish_frame(shared_bgra_frame(1, 1, 1, &[0; 4]));
        let (server_stream, mut client_stream, peer_addr) = tcp_pair().await;
        let (event_tx, _event_rx) = mpsc::channel(8);
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = spawn_connection(
            RfbClientId(3),
            peer_addr,
            server_stream,
            &frame_source,
            event_tx,
            RfbConnectionSettings::default(),
            shutdown_rx,
        );

        assert_eq!(
            read_exact_vec(&mut client_stream, 12).await,
            b"RFB 003.008\n"
        );
        tokio::time::advance(Duration::from_secs(10)).await;

        assert!(matches!(
            task.await.unwrap(),
            ConnectionEnd::Failed(RfbConnectionError::HandshakeTimeout)
        ));
    }

    #[tokio::test]
    async fn shutdown_ends_handshake() {
        let frame_source = MockFrameSource::new();
        frame_source.publish_frame(shared_bgra_frame(1, 1, 1, &[0; 4]));
        let (server_stream, mut client_stream, peer_addr) = tcp_pair().await;
        let (event_tx, _event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = spawn_connection(
            RfbClientId(4),
            peer_addr,
            server_stream,
            &frame_source,
            event_tx,
            RfbConnectionSettings::default(),
            shutdown_rx,
        );

        assert_eq!(
            read_exact_vec(&mut client_stream, 12).await,
            b"RFB 003.008\n"
        );
        shutdown_tx.send(true).unwrap();

        assert!(matches!(task.await.unwrap(), ConnectionEnd::ServerShutdown));
    }

    #[tokio::test]
    async fn non_incremental_request_resends_same_frame() {
        let frame_source = MockFrameSource::new();
        frame_source.publish_frame(shared_bgra_frame(1, 2, 1, &[1, 2, 3, 9, 4, 5, 6, 9]));
        let (task, mut client, _events, _shutdown) = completed_connection(
            RfbClientId(10),
            &frame_source,
            RfbConnectionSettings::default(),
        )
        .await;

        send_update_request(&mut client, false, 0, 0, 2, 1).await;
        let first = read_update(&mut client, 8).await;
        send_update_request(&mut client, false, 0, 0, 2, 1).await;
        let second = read_update(&mut client, 8).await;

        assert_eq!(first.encoding, 0);
        assert_eq!(first.pixels, [1, 2, 3, 0, 4, 5, 6, 0]);
        assert_eq!(second, first);
        drop(client);
        assert!(matches!(task.await.unwrap(), ConnectionEnd::ClientClosed));
    }

    #[tokio::test]
    async fn incremental_request_waits_for_new_sequence() {
        let frame_source = MockFrameSource::new();
        frame_source.publish_frame(shared_bgra_frame(1, 2, 1, &[0; 8]));
        let (task, mut client, mut events, _shutdown) = completed_connection(
            RfbClientId(11),
            &frame_source,
            RfbConnectionSettings::default(),
        )
        .await;
        send_update_request(&mut client, false, 0, 0, 2, 1).await;
        read_update(&mut client, 8).await;

        send_update_request(&mut client, true, 0, 0, 2, 1).await;
        send_key(&mut client, true, 0x41).await;
        assert!(matches!(
            events.recv().await,
            Some(RfbServerEvent::Key { keysym: 0x41, .. })
        ));
        let mut unexpected = [0; 1];
        assert!(matches!(
            client.try_read(&mut unexpected),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock
        ));

        frame_source.publish_frame(shared_bgra_frame(2, 2, 1, &[10, 20, 30, 9, 40, 50, 60, 9]));
        let update = read_update(&mut client, 8).await;
        assert_eq!(update.pixels, [10, 20, 30, 0, 40, 50, 60, 0]);

        drop(client);
        assert!(matches!(task.await.unwrap(), ConnectionEnd::ClientClosed));
    }

    #[tokio::test]
    async fn outstanding_incremental_requests_coalesce() {
        let frame_source = MockFrameSource::new();
        frame_source.publish_frame(shared_bgra_frame(1, 2, 1, &[0; 8]));
        let (task, mut client, mut events, _shutdown) = completed_connection(
            RfbClientId(12),
            &frame_source,
            RfbConnectionSettings::default(),
        )
        .await;
        send_update_request(&mut client, false, 0, 0, 2, 1).await;
        read_update(&mut client, 8).await;

        send_update_request(&mut client, true, 0, 0, 1, 1).await;
        send_update_request(&mut client, true, 1, 0, 1, 1).await;
        send_key(&mut client, true, 0x42).await;
        assert!(matches!(
            events.recv().await,
            Some(RfbServerEvent::Key { keysym: 0x42, .. })
        ));
        frame_source.publish_frame(shared_bgra_frame(2, 2, 1, &[1, 2, 3, 0, 4, 5, 6, 0]));

        let update = read_update(&mut client, 8).await;
        assert_eq!(
            (update.x, update.y, update.width, update.height),
            (0, 0, 2, 1)
        );
        assert_eq!(update.pixels, [1, 2, 3, 0, 4, 5, 6, 0]);

        drop(client);
        assert!(matches!(task.await.unwrap(), ConnectionEnd::ClientClosed));
    }

    #[tokio::test]
    async fn desktop_size_is_sent_before_new_pixels() {
        let frame_source = MockFrameSource::new();
        frame_source.publish_frame(shared_bgra_frame(1, 2, 1, &[0; 8]));
        let (task, mut client, _events, _shutdown) = completed_connection(
            RfbClientId(13),
            &frame_source,
            RfbConnectionSettings::default(),
        )
        .await;
        send_set_encodings(&mut client, &[-223]).await;
        send_update_request(&mut client, false, 0, 0, 2, 1).await;
        read_update(&mut client, 8).await;

        frame_source.publish_frame(shared_bgra_frame(2, 3, 1, &[1; 12]));
        send_update_request(&mut client, true, 0, 0, 2, 1).await;
        let resize = read_update(&mut client, 0).await;
        assert_eq!(
            (
                resize.x,
                resize.y,
                resize.width,
                resize.height,
                resize.encoding
            ),
            (0, 0, 3, 1, -223)
        );

        send_update_request(&mut client, false, 0, 0, 3, 1).await;
        let pixels = read_update(&mut client, 12).await;
        assert_eq!((pixels.width, pixels.height, pixels.encoding), (3, 1, 0));

        drop(client);
        assert!(matches!(task.await.unwrap(), ConnectionEnd::ClientClosed));
    }

    #[tokio::test]
    async fn pointer_events_keep_the_coordinate_size_from_their_input_epoch() {
        let frame_source = MockFrameSource::new();
        frame_source.publish_frame(shared_bgra_frame(1, 2, 1, &[0; 8]));
        let (task, mut client, mut events, _shutdown) = completed_connection(
            RfbClientId(17),
            &frame_source,
            RfbConnectionSettings::default(),
        )
        .await;
        send_set_encodings(&mut client, &[-223]).await;
        frame_source.publish_frame(shared_bgra_frame(2, 3, 1, &[1; 12]));

        let mut messages = vec![3, 1, 0, 0, 0, 0, 0, 2, 0, 1];
        messages.extend_from_slice(&[5, 0, 0, 1, 0, 0]);
        client.write_all(&messages).await.unwrap();

        assert_eq!(
            events.recv().await,
            Some(RfbServerEvent::Pointer {
                client_id: RfbClientId(17),
                button_mask: 0,
                x: 1,
                y: 0,
                framebuffer_size: RfbSize::new(2, 1).unwrap(),
            })
        );
        assert_eq!(read_update(&mut client, 0).await.encoding, -223);

        client.write_all(&[5, 0, 0, 2, 0, 0]).await.unwrap();
        assert_eq!(
            events.recv().await,
            Some(RfbServerEvent::Pointer {
                client_id: RfbClientId(17),
                button_mask: 0,
                x: 2,
                y: 0,
                framebuffer_size: RfbSize::new(3, 1).unwrap(),
            })
        );

        drop(client);
        assert!(matches!(task.await.unwrap(), ConnectionEnd::ClientClosed));
    }

    #[tokio::test]
    async fn resize_without_negotiation_ends_connection() {
        let frame_source = MockFrameSource::new();
        frame_source.publish_frame(shared_bgra_frame(1, 2, 1, &[0; 8]));
        let (task, mut client, _events, _shutdown) = completed_connection(
            RfbClientId(14),
            &frame_source,
            RfbConnectionSettings::default(),
        )
        .await;
        send_update_request(&mut client, false, 0, 0, 2, 1).await;
        read_update(&mut client, 8).await;

        frame_source.publish_frame(shared_bgra_frame(2, 3, 1, &[0; 12]));
        send_update_request(&mut client, true, 0, 0, 2, 1).await;

        assert!(matches!(
            task.await.unwrap(),
            ConnectionEnd::Failed(RfbConnectionError::Encode(
                RfbEncodeError::DesktopSizeNotNegotiated { .. }
            ))
        ));
    }

    #[tokio::test]
    async fn regressed_frame_sequence_ends_connection() {
        let frame_source = MockFrameSource::new();
        frame_source.publish_frame(shared_bgra_frame(2, 1, 1, &[0; 4]));
        let (task, mut client, _events, _shutdown) = completed_connection(
            RfbClientId(15),
            &frame_source,
            RfbConnectionSettings::default(),
        )
        .await;
        frame_source.publish_frame(shared_bgra_frame(1, 1, 1, &[0; 4]));
        send_update_request(&mut client, false, 0, 0, 1, 1).await;

        assert!(matches!(
            task.await.unwrap(),
            ConnectionEnd::Failed(RfbConnectionError::Frame(
                RfbFrameError::FrameSequenceRegressed {
                    previous: 2,
                    actual: 1,
                }
            ))
        ));
    }

    #[tokio::test]
    async fn framebuffer_limit_never_writes_partial_update() {
        let frame_source = MockFrameSource::new();
        frame_source.publish_frame(shared_bgra_frame(1, 1, 1, &[0; 4]));
        let mut config = RfbConnectionSettings::default();
        config.protocol_limits.max_framebuffer_bytes = 4;
        config.protocol_limits.max_queued_output_bytes = 128;
        let (task, mut client, _events, _shutdown) =
            completed_connection(RfbClientId(16), &frame_source, config).await;
        send_update_request(&mut client, false, 0, 0, 1, 1).await;
        read_update(&mut client, 4).await;

        frame_source.publish_frame(shared_bgra_frame(2, 2, 1, &[0; 8]));
        send_update_request(&mut client, false, 0, 0, 2, 1).await;
        assert!(matches!(
            task.await.unwrap(),
            ConnectionEnd::Failed(RfbConnectionError::Encode(
                RfbEncodeError::FramebufferTooLarge {
                    actual: 8,
                    maximum: 4,
                }
            ))
        ));
        let mut remaining = Vec::new();
        client.read_to_end(&mut remaining).await.unwrap();
        assert!(remaining.is_empty());
    }
}
