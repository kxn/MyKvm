mod support;

use std::{io, net::SocketAddr, sync::Arc};

use axum::{http::StatusCode, serve};
use ipkvm_headless::{
    rfb_connection::{RfbClientId, RfbConnectionGate, RfbDisconnectReason, RfbServerEvent},
    rfb_tcp::{RfbTcpConfig, RfbTcpServer, RfbTcpServerError},
    rfb_ws::{RfbWebSocketConfig, RfbWebSocketService},
};
use ipkvm_video::{MonotonicTimestamp, PixelFormat, VideoFrame, mock::MockFrameSource};
use support::{ClientWebSocket, TestRfbClient};
use tokio::{
    net::TcpListener,
    sync::{mpsc, watch},
    task::JoinHandle,
};
use tokio_tungstenite::{connect_async, tungstenite::Error as WebSocketError};

struct TestDualTransportSystem {
    tcp_address: SocketAddr,
    websocket_address: SocketAddr,
    events: mpsc::Receiver<RfbServerEvent>,
    shutdown: watch::Sender<bool>,
    tcp_task: JoinHandle<Result<(), RfbTcpServerError>>,
    websocket_task: JoinHandle<io::Result<()>>,
}

impl TestDualTransportSystem {
    async fn start() -> Self {
        Self::start_with_event_capacity(16).await
    }

    async fn start_with_event_capacity(event_capacity: usize) -> Self {
        let tcp_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp_address = tcp_listener.local_addr().unwrap();
        let websocket_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let websocket_address = websocket_listener.local_addr().unwrap();
        let source = Arc::new(MockFrameSource::new());
        source.publish_frame(default_frame());
        let (event_tx, events) = mpsc::channel(event_capacity);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let gate = RfbConnectionGate::new();

        let tcp_server = RfbTcpServer::new(
            tcp_listener,
            Arc::clone(&source),
            event_tx.clone(),
            RfbTcpConfig::default(),
            gate.clone(),
        )
        .unwrap();
        let tcp_task = tokio::spawn(tcp_server.run(shutdown_rx.clone()));

        let websocket_service = RfbWebSocketService::new(
            source,
            event_tx,
            RfbWebSocketConfig::default(),
            shutdown_rx.clone(),
            gate,
        )
        .unwrap();
        let websocket_task = tokio::spawn(async move {
            serve(
                websocket_listener,
                websocket_service
                    .router()
                    .into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(wait_for_shutdown(shutdown_rx))
            .await
        });

        Self {
            tcp_address,
            websocket_address,
            events,
            shutdown,
            tcp_task,
            websocket_task,
        }
    }

    async fn connect_tcp(&self) -> TestRfbClient {
        TestRfbClient::connect(self.tcp_address).await
    }

    async fn connect_tcp_and_finish_handshake(&mut self) -> (TestRfbClient, RfbClientId) {
        let mut client = self.connect_tcp().await;
        client.handshake(true).await;
        let client_id = self.expect_connected().await;
        (client, client_id)
    }

    async fn try_connect_websocket(&self) -> Result<ClientWebSocket, WebSocketError> {
        connect_async(format!("ws://{}/rfb", self.websocket_address))
            .await
            .map(|(socket, _)| socket)
    }

    async fn expect_connected(&mut self) -> RfbClientId {
        match self.events.recv().await.unwrap() {
            RfbServerEvent::Connected { client_id, .. } => client_id,
            event => panic!("expected connected event, got {event:?}"),
        }
    }

    async fn expect_disconnected(&mut self, expected_id: RfbClientId) -> RfbDisconnectReason {
        let (client_id, reason) = self.expect_any_disconnected().await;
        assert_eq!(client_id, expected_id);
        reason
    }

    async fn expect_any_disconnected(&mut self) -> (RfbClientId, RfbDisconnectReason) {
        match self.events.recv().await.unwrap() {
            RfbServerEvent::Disconnected {
                client_id, reason, ..
            } => (client_id, reason),
            event => panic!("expected disconnected event, got {event:?}"),
        }
    }

    async fn stop(self) {
        let Self {
            mut events,
            shutdown,
            tcp_task,
            websocket_task,
            ..
        } = self;
        shutdown.send(true).unwrap();
        let drain_events = async move { while events.recv().await.is_some() {} };
        let (tcp_result, websocket_result, ()) =
            tokio::join!(tcp_task, websocket_task, drain_events);
        tcp_result.unwrap().unwrap();
        websocket_result.unwrap().unwrap();
    }
}

fn default_frame() -> Arc<VideoFrame> {
    Arc::new(VideoFrame::new(
        1,
        MonotonicTimestamp::from_nanos(1),
        1,
        1,
        4,
        PixelFormat::Bgra8888,
        Arc::from([30, 20, 10, 255]),
    ))
}

async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow() {
            return;
        }
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

fn websocket_rejection_status(error: WebSocketError) -> StatusCode {
    match error {
        WebSocketError::Http(response) => response.status(),
        error => panic!("expected HTTP upgrade rejection, got {error:?}"),
    }
}

#[tokio::test]
async fn active_tcp_rejects_websocket_upgrade() {
    let mut system = TestDualTransportSystem::start().await;
    let (tcp, tcp_id) = system.connect_tcp_and_finish_handshake().await;

    let error = match system.try_connect_websocket().await {
        Ok(_) => panic!("WebSocket upgrade succeeded while TCP was active"),
        Err(error) => error,
    };
    assert_eq!(websocket_rejection_status(error), StatusCode::CONFLICT);

    drop(tcp);
    assert_eq!(
        system.expect_disconnected(tcp_id).await,
        RfbDisconnectReason::ClientClosed
    );
    system.stop().await;
}
