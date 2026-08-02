use std::{net::SocketAddr, sync::Arc};

use ipkvm_rfb::RfbSecurity;
use ipkvm_video::FrameSource;
use tokio::{
    net::TcpListener,
    sync::{mpsc, watch},
};

use super::{RfbTcpConfig, RfbTcpServerError, transport::TcpTransport};
use ipkvm_session::rfb_connection::{
    ConnectionEnd, RfbConnectionFinalizeError, RfbConnectionGate, RfbConnectionGateError,
    RfbServerEvent, RfbTransportKind, finalize_connection, run_managed_connection,
};

pub struct RfbTcpServer<S: ?Sized> {
    listener: TcpListener,
    frame_source: Arc<S>,
    event_tx: mpsc::Sender<RfbServerEvent>,
    config: RfbTcpConfig,
    gate: RfbConnectionGate,
    #[cfg(test)]
    gate_wait_notifier: Option<mpsc::UnboundedSender<std::net::SocketAddr>>,
}

impl<S: FrameSource + ?Sized + 'static> RfbTcpServer<S> {
    pub fn new(
        listener: TcpListener,
        frame_source: Arc<S>,
        event_tx: mpsc::Sender<RfbServerEvent>,
        config: RfbTcpConfig,
        gate: RfbConnectionGate,
    ) -> Result<Self, RfbTcpServerError> {
        config.validate()?;
        Ok(Self {
            listener,
            frame_source,
            event_tx,
            config,
            gate,
            #[cfg(test)]
            gate_wait_notifier: None,
        })
    }

    /// 顺序服务客户端，直到收到关闭信号或发生 server 级错误。
    ///
    /// 调用方在本方法返回前必须持续消费事件通道。事件交付采用无损反压；
    /// 如果接收端停止消费且通道已满，连接关闭和 `Disconnected` 事件也会等待容量。
    pub async fn run(self, mut shutdown: watch::Receiver<bool>) -> Result<(), RfbTcpServerError> {
        loop {
            if shutdown_is_requested(&shutdown) {
                return Ok(());
            }

            let (stream, peer_addr) = tokio::select! {
                result = self.listener.accept() => {
                    result.map_err(RfbTcpServerError::Accept)?
                }
                _ = wait_for_shutdown(&mut shutdown) => return Ok(()),
                _ = self.event_tx.closed() => {
                    return Err(RfbTcpServerError::EventChannelClosed);
                }
            };
            if !tcp_peer_allowed(peer_addr, &self.config.connection.security) {
                // 未配置密码：非回环来源直接关闭，不进入握手。
                continue;
            }
            #[cfg(test)]
            if let Some(notifier) = &self.gate_wait_notifier {
                let _ = notifier.send(peer_addr);
            }
            let reservation = tokio::select! {
                result = self.gate.acquire(RfbTransportKind::Tcp, peer_addr) => {
                    result.map_err(|error| match error {
                        RfbConnectionGateError::ClientIdOverflow => RfbTcpServerError::ClientIdOverflow,
                        RfbConnectionGateError::Poisoned => RfbTcpServerError::ConnectionGatePoisoned,
                        RfbConnectionGateError::Busy => unreachable!("awaited gate acquisition cannot be busy"),
                    })?
                }
                _ = wait_for_shutdown(&mut shutdown) => return Ok(()),
                _ = self.event_tx.closed() => return Err(RfbTcpServerError::EventChannelClosed),
            };
            let completion = run_managed_connection(
                reservation,
                peer_addr,
                TcpTransport::new(stream, self.config.read_buffer_bytes),
                self.frame_source.subscribe(),
                self.event_tx.clone(),
                self.config.connection.clone(),
                shutdown.clone(),
            )
            .await;
            let end = finalize_connection(&self.event_tx, completion).await?;
            if matches!(&end, ConnectionEnd::ServerShutdown) {
                return Ok(());
            }
        }
    }
}

impl From<RfbConnectionFinalizeError> for RfbTcpServerError {
    fn from(error: RfbConnectionFinalizeError) -> Self {
        match error {
            RfbConnectionFinalizeError::EventChannelClosed => Self::EventChannelClosed,
        }
    }
}

/// 未配置密码时，TCP 入口只允许回环来源（防默认暴露）；配置了 VNC 密码
/// 则来源不再限制，完全交给密码挑战校验。
fn tcp_peer_allowed(peer: SocketAddr, security: &RfbSecurity) -> bool {
    match security {
        RfbSecurity::None => peer.ip().is_loopback(),
        RfbSecurity::Vnc { .. } => true,
    }
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
    use std::{net::SocketAddr, sync::Arc, task::Poll};

    use axum::serve;
    use futures_util::{SinkExt, StreamExt};
    use ipkvm_video::{MonotonicTimestamp, PixelFormat, VideoFrame, mock::MockFrameSource};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::{mpsc, watch},
    };
    use tokio_tungstenite::{
        MaybeTlsStream, WebSocketStream, connect_async,
        tungstenite::{Error as WebSocketError, Message, protocol::frame::coding::CloseCode},
    };

    use super::*;
    use crate::{
        rfb_connection::RfbDisconnectReason,
        rfb_ws::{RfbWebSocketConfig, RfbWebSocketService},
    };

    type TestWebSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

    async fn make_server() -> (
        RfbTcpServer<MockFrameSource>,
        mpsc::Receiver<RfbServerEvent>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (event_tx, event_rx) = mpsc::channel(1);
        let frame_source = Arc::new(MockFrameSource::new());
        frame_source.publish_frame(Arc::new(VideoFrame::new(
            1,
            MonotonicTimestamp::from_nanos(1),
            1,
            1,
            4,
            PixelFormat::Bgra8888,
            Arc::from([0_u8, 0, 0, 0]),
        )));
        (
            RfbTcpServer::new(
                listener,
                frame_source,
                event_tx,
                RfbTcpConfig::default(),
                RfbConnectionGate::new(),
            )
            .unwrap(),
            event_rx,
        )
    }

    async fn receive_binary(socket: &mut TestWebSocket) -> Vec<u8> {
        match socket.next().await.unwrap().unwrap() {
            Message::Binary(bytes) => bytes.to_vec(),
            message => panic!("expected binary WebSocket message, got {message:?}"),
        }
    }

    async fn finish_websocket_handshake(socket: &mut TestWebSocket) {
        assert_eq!(receive_binary(socket).await, b"RFB 003.008\n");
        socket
            .send(Message::Binary(b"RFB 003.008\n".to_vec().into()))
            .await
            .unwrap();
        assert_eq!(receive_binary(socket).await, [1, 1]);
        socket.send(Message::Binary(vec![1].into())).await.unwrap();
        assert_eq!(receive_binary(socket).await, [0, 0, 0, 0]);
        socket.send(Message::Binary(vec![1].into())).await.unwrap();
        assert!(receive_binary(socket).await.len() >= 24);
    }

    async fn send_websocket_key(socket: &mut TestWebSocket, down: bool, keysym: u32) {
        let mut message = vec![4, u8::from(down), 0, 0];
        message.extend_from_slice(&keysym.to_be_bytes());
        socket.send(Message::Binary(message.into())).await.unwrap();
    }

    async fn expect_event(event_rx: &mut mpsc::Receiver<RfbServerEvent>) -> RfbServerEvent {
        event_rx.recv().await.expect("RFB event channel closed")
    }

    #[tokio::test]
    async fn aborting_connected_owner_poisons_gate() {
        let (server, mut event_rx) = make_server().await;
        let address = server.listener.local_addr().unwrap();
        let gate = server.gate.clone();
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let owner = tokio::spawn(server.run(shutdown_rx));
        let mut stream = TcpStream::connect(address).await.unwrap();
        let mut banner = [0_u8; 12];
        stream.read_exact(&mut banner).await.unwrap();
        assert_eq!(&banner, b"RFB 003.008\n");
        stream.write_all(b"RFB 003.008\n").await.unwrap();
        let mut security_types = [0_u8; 2];
        stream.read_exact(&mut security_types).await.unwrap();
        assert_eq!(security_types, [1, 1]);
        stream.write_all(&[1]).await.unwrap();
        let mut security_result = [0_u8; 4];
        stream.read_exact(&mut security_result).await.unwrap();
        assert_eq!(security_result, [0; 4]);
        stream.write_all(&[1]).await.unwrap();
        let mut server_init = [0_u8; 24];
        stream.read_exact(&mut server_init).await.unwrap();
        let name_length = u32::from_be_bytes(server_init[20..24].try_into().unwrap()) as usize;
        let mut name = vec![0_u8; name_length];
        stream.read_exact(&mut name).await.unwrap();
        assert_eq!(name, b"my_ipkvm");
        assert!(matches!(
            event_rx.recv().await,
            Some(RfbServerEvent::Connected { .. })
        ));

        owner.abort();
        assert!(owner.await.unwrap_err().is_cancelled());
        assert_eq!(
            gate.try_acquire(RfbTransportKind::Tcp, "127.0.0.1:5900".parse().unwrap())
                .unwrap_err(),
            RfbConnectionGateError::Poisoned
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let poisoned_address = listener.local_addr().unwrap();
        let frame_source = Arc::new(MockFrameSource::new());
        frame_source.publish_frame(Arc::new(VideoFrame::new(
            1,
            MonotonicTimestamp::from_nanos(1),
            1,
            1,
            4,
            PixelFormat::Bgra8888,
            Arc::from([0_u8, 0, 0, 0]),
        )));
        let (event_tx, _event_rx) = mpsc::channel(1);
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let poisoned_server = RfbTcpServer::new(
            listener,
            frame_source,
            event_tx,
            RfbTcpConfig::default(),
            gate,
        )
        .unwrap();
        let poisoned_task = tokio::spawn(poisoned_server.run(shutdown_rx));
        let _stream = TcpStream::connect(poisoned_address).await.unwrap();
        assert!(matches!(
            poisoned_task.await.unwrap(),
            Err(RfbTcpServerError::ConnectionGatePoisoned)
        ));
    }

    #[tokio::test]
    async fn websocket_disconnect_enqueues_before_waiting_tcp_receives_banner() {
        let tcp_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp_address = tcp_listener.local_addr().unwrap();
        let websocket_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let websocket_address = websocket_listener.local_addr().unwrap();
        let frame_source = Arc::new(MockFrameSource::new());
        frame_source.publish_frame(Arc::new(VideoFrame::new(
            1,
            MonotonicTimestamp::from_nanos(1),
            1,
            1,
            4,
            PixelFormat::Bgra8888,
            Arc::from([0_u8, 0, 0, 0]),
        )));
        let (event_tx, mut event_rx) = mpsc::channel(1);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let gate = RfbConnectionGate::new();
        let (gate_wait_tx, mut gate_wait_rx) = mpsc::unbounded_channel();

        let mut tcp_server = RfbTcpServer::new(
            tcp_listener,
            Arc::clone(&frame_source),
            event_tx.clone(),
            RfbTcpConfig::default(),
            gate.clone(),
        )
        .unwrap();
        tcp_server.gate_wait_notifier = Some(gate_wait_tx);
        let tcp_task = tokio::spawn(tcp_server.run(shutdown_rx.clone()));

        let websocket_service = RfbWebSocketService::new(
            frame_source,
            event_tx,
            RfbWebSocketConfig::default(),
            shutdown_rx.clone(),
            gate,
        )
        .unwrap();
        let websocket_task = tokio::spawn(async move {
            let mut shutdown_rx = shutdown_rx;
            let shutdown = async move { wait_for_shutdown(&mut shutdown_rx).await };
            serve(
                websocket_listener,
                websocket_service
                    .router()
                    .into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown)
            .await
        });

        let (mut websocket, _) = connect_async(format!("ws://{websocket_address}/rfb"))
            .await
            .unwrap();
        finish_websocket_handshake(&mut websocket).await;
        let websocket_id = match expect_event(&mut event_rx).await {
            RfbServerEvent::Connected { client_id, .. } => client_id,
            event => panic!("expected WebSocket Connected event, got {event:?}"),
        };

        send_websocket_key(&mut websocket, true, 0x41).await;
        assert_eq!(
            expect_event(&mut event_rx).await,
            RfbServerEvent::Key {
                client_id: websocket_id,
                down: true,
                keysym: 0x41,
            }
        );

        let mut tcp = TcpStream::connect(tcp_address).await.unwrap();
        gate_wait_rx
            .recv()
            .await
            .expect("TCP server did not reach the connection gate");
        let mut banner = [0_u8; 12];
        let mut banner_read = Box::pin(tcp.read_exact(&mut banner));
        std::future::poll_fn(|context| match banner_read.as_mut().poll(context) {
            Poll::Pending => Poll::Ready(()),
            Poll::Ready(result) => panic!("TCP received banner before gate release: {result:?}"),
        })
        .await;

        send_websocket_key(&mut websocket, false, 0x41).await;
        websocket.send(Message::Close(None)).await.unwrap();
        let close_result = websocket.next().await;
        assert!(
            matches!(
                &close_result,
                Some(Ok(Message::Close(Some(frame)))) if frame.code == CloseCode::Normal
            ) || matches!(
                &close_result,
                Some(Ok(Message::Close(None)))
                    | Some(Err(WebSocketError::ConnectionClosed))
                    | Some(Err(WebSocketError::Protocol(
                        tokio_tungstenite::tungstenite::error::ProtocolError::ResetWithoutClosingHandshake
                    )))
                    | None
            ),
            "unexpected WebSocket close result: {close_result:?}"
        );
        std::future::poll_fn(|context| match banner_read.as_mut().poll(context) {
            Poll::Pending => Poll::Ready(()),
            Poll::Ready(result) => {
                panic!("TCP received banner before Disconnected was enqueued: {result:?}")
            }
        })
        .await;

        assert_eq!(
            expect_event(&mut event_rx).await,
            RfbServerEvent::Key {
                client_id: websocket_id,
                down: false,
                keysym: 0x41,
            }
        );
        assert!(matches!(
            expect_event(&mut event_rx).await,
            RfbServerEvent::Disconnected {
                client_id,
                reason: RfbDisconnectReason::ClientClosed,
                ..
            } if client_id == websocket_id
        ));

        banner_read.await.unwrap();
        assert_eq!(&banner, b"RFB 003.008\n");
        drop(tcp);
        assert!(matches!(
            expect_event(&mut event_rx).await,
            RfbServerEvent::Disconnected {
                reason: RfbDisconnectReason::ClientClosed,
                ..
            }
        ));

        shutdown_tx.send(true).unwrap();
        drop(websocket);
        tcp_task.await.unwrap().unwrap();
        websocket_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn initial_or_closed_shutdown_stops_before_accept() {
        let (_shutdown_tx, shutdown_rx) = watch::channel(true);
        let (server, _event_rx) = make_server().await;
        assert!(server.run(shutdown_rx).await.is_ok());

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        drop(shutdown_tx);
        let (server, _event_rx) = make_server().await;
        assert!(server.run(shutdown_rx).await.is_ok());
    }

    #[test]
    fn tcp_peer_allowed_rejects_remote_without_password() {
        let loopback: SocketAddr = "127.0.0.1:5900".parse().unwrap();
        let remote: SocketAddr = "192.168.1.5:5900".parse().unwrap();
        assert!(tcp_peer_allowed(loopback, &RfbSecurity::None));
        assert!(!tcp_peer_allowed(remote, &RfbSecurity::None));
        // 配置密码后来源不再限制，完全交给密码校验。
        assert!(tcp_peer_allowed(
            remote,
            &RfbSecurity::Vnc { password: [0; 8] }
        ));
    }
}
