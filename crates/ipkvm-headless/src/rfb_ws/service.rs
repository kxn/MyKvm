use std::{net::SocketAddr, sync::Arc};

use axum::{
    Router,
    extract::{ConnectInfo, State, WebSocketUpgrade, ws::WebSocket},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::any,
};
use ipkvm_video::FrameSource;
use tokio::sync::{mpsc, watch};

use super::{RfbWebSocketConfig, RfbWebSocketServiceError, transport::WebSocketTransport};
use ipkvm_session::rfb_connection::{
    RfbConnectionGate, RfbConnectionGateError, RfbConnectionReservation, RfbServerEvent,
    RfbTransportKind, finalize_connection, finalize_connection_after_session_end,
    run_managed_connection,
};

pub struct RfbWebSocketService<S: ?Sized> {
    state: Arc<ServiceState<S>>,
}

struct ServiceState<S: ?Sized> {
    frame_source: Arc<S>,
    /// 当前活动事件出口订阅端：每个 WS upgrade 读取最新 sender；
    /// `None`（会话未启动/已停止）时拒绝升级。会话重启后自动读到新 channel。
    event_publisher: watch::Receiver<Option<mpsc::Sender<RfbServerEvent>>>,
    config: RfbWebSocketConfig,
    shutdown: watch::Receiver<bool>,
    gate: RfbConnectionGate,
}

impl<S: FrameSource + ?Sized + 'static> RfbWebSocketService<S> {
    pub fn new(
        frame_source: Arc<S>,
        event_publisher: watch::Receiver<Option<mpsc::Sender<RfbServerEvent>>>,
        config: RfbWebSocketConfig,
        shutdown: watch::Receiver<bool>,
        gate: RfbConnectionGate,
    ) -> Result<Self, RfbWebSocketServiceError> {
        config.validate()?;
        Ok(Self {
            state: Arc::new(ServiceState {
                frame_source,
                event_publisher,
                config,
                shutdown,
                gate,
            }),
        })
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/rfb", any(handle_upgrade::<S>))
            .with_state(Arc::clone(&self.state))
    }
}

async fn handle_upgrade<S: FrameSource + ?Sized + 'static>(
    state: State<Arc<ServiceState<S>>>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    ws: WebSocketUpgrade,
) -> Response {
    if shutdown_is_requested(&state.shutdown) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    // 读取当前活动事件出口；无活动会话时拒绝升级（等下一次 start）。
    let event_tx = match state.event_publisher.borrow().clone() {
        Some(tx) => tx,
        None => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    if event_tx.is_closed() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }

    let permit = match state
        .gate
        .try_acquire(RfbTransportKind::WebSocket, peer_addr)
    {
        Ok(permit) => permit,
        Err(error) => return gate_error_status(error).into_response(),
    };
    let limit = state
        .config
        .connection
        .protocol_limits
        .max_buffered_input_bytes;

    ws.protocols(["binary"])
        .max_message_size(limit)
        .max_frame_size(limit)
        .on_upgrade(move |socket| {
            run_upgraded_connection(Arc::clone(&state), socket, peer_addr, permit, event_tx)
        })
        .into_response()
}

async fn run_upgraded_connection<S: FrameSource + ?Sized + 'static>(
    state: Arc<ServiceState<S>>,
    socket: WebSocket,
    peer_addr: SocketAddr,
    reservation: RfbConnectionReservation,
    event_tx: mpsc::Sender<RfbServerEvent>,
) {
    let completion = run_managed_connection(
        reservation,
        peer_addr,
        WebSocketTransport::new(socket),
        state.frame_source.subscribe(),
        event_tx.clone(),
        state.config.connection.clone(),
        state.shutdown.clone(),
    )
    .await;
    if event_sender_is_current(&state.event_publisher, &event_tx) {
        let _ = finalize_connection(&event_tx, completion).await;
    } else {
        finalize_connection_after_session_end(&event_tx, completion).await;
    }
}

fn shutdown_is_requested(shutdown: &watch::Receiver<bool>) -> bool {
    *shutdown.borrow() || shutdown.has_changed().is_err()
}

fn gate_error_status(error: RfbConnectionGateError) -> StatusCode {
    match error {
        RfbConnectionGateError::Busy => StatusCode::CONFLICT,
        RfbConnectionGateError::Poisoned | RfbConnectionGateError::ClientIdOverflow => {
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

fn event_sender_is_current(
    publisher: &watch::Receiver<Option<mpsc::Sender<RfbServerEvent>>>,
    sender: &mpsc::Sender<RfbServerEvent>,
) -> bool {
    publisher
        .borrow()
        .as_ref()
        .is_some_and(|current| sender.same_channel(current))
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;

    use super::*;

    #[tokio::test]
    async fn client_id_exhaustion_maps_to_empty_service_unavailable() {
        let response = gate_error_status(RfbConnectionGateError::ClientIdOverflow).into_response();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn poisoned_gate_maps_to_empty_service_unavailable() {
        let response = gate_error_status(RfbConnectionGateError::Poisoned).into_response();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
