use std::{net::SocketAddr, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{
        HeaderValue, Response, StatusCode, Uri,
        header::{CACHE_CONTROL, CONTENT_TYPE},
    },
    response::IntoResponse,
    routing::get,
};
use ipkvm_video::FrameSource;
use thiserror::Error;
use tokio::{
    net::TcpListener,
    sync::{mpsc, watch},
};

use super::assets::find_asset;
use crate::{
    rfb_connection::{RfbConnectionGate, RfbServerEvent},
    rfb_ws::{RfbWebSocketConfig, RfbWebSocketService, RfbWebSocketServiceError},
};

const CONTENT_TYPE_OPTIONS: &str = "x-content-type-options";

pub struct HeadlessWebService<S> {
    rfb: RfbWebSocketService<S>,
    shutdown: watch::Receiver<bool>,
}

#[derive(Debug, Error)]
pub enum HeadlessWebServiceError {
    #[error("invalid RFB WebSocket service: {0}")]
    Rfb(#[from] RfbWebSocketServiceError),
    #[error("headless HTTP server failed")]
    Serve(#[source] std::io::Error),
}

impl<S: FrameSource + 'static> HeadlessWebService<S> {
    pub fn new(
        frame_source: Arc<S>,
        event_tx: mpsc::Sender<RfbServerEvent>,
        config: RfbWebSocketConfig,
        shutdown: watch::Receiver<bool>,
        gate: RfbConnectionGate,
    ) -> Result<Self, HeadlessWebServiceError> {
        let rfb = RfbWebSocketService::new(frame_source, event_tx, config, shutdown.clone(), gate)?;
        Ok(Self { rfb, shutdown })
    }

    pub async fn serve(self, listener: TcpListener) -> Result<(), HeadlessWebServiceError> {
        let shutdown = self.shutdown;
        let router = static_router().merge(self.rfb.router());
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(wait_for_shutdown(shutdown))
        .await
        .map_err(HeadlessWebServiceError::Serve)
    }
}

fn static_router() -> Router {
    Router::new()
        .route("/", get(serve_asset))
        .route("/index.html", get(serve_asset))
        .route("/assets/app.css", get(serve_asset))
        .route("/assets/app.js", get(serve_asset))
        .route("/licenses", get(serve_asset))
        .route("/licenses/", get(serve_asset))
        .route("/vendor/novnc/{*path}", get(serve_asset))
}

async fn serve_asset(uri: Uri) -> Response<Body> {
    let Some(asset) = find_asset(uri.path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, HeaderValue::from_static(asset.content_type()))
        .header(CACHE_CONTROL, HeaderValue::from_static("no-cache"))
        .header(CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"))
        .body(Body::from(asset.bytes()))
        .expect("static response headers are valid")
}

async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow_and_update() {
            return;
        }
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closed_shutdown_sender_stops_the_waiter() {
        let (sender, receiver) = watch::channel(false);
        drop(sender);

        wait_for_shutdown(receiver).await;
    }

    #[tokio::test]
    async fn pre_requested_shutdown_stops_the_waiter() {
        let (sender, receiver) = watch::channel(true);

        wait_for_shutdown(receiver).await;
        drop(sender);
    }
}
