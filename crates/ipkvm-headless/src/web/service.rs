use std::{borrow::Cow, net::SocketAddr, sync::Arc};

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{
        HeaderValue, StatusCode, Uri,
        header::{CACHE_CONTROL, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
    routing::get,
};
use ipkvm_video::{FrameSource, PixelFormat, VideoFrame, VideoSourceKind};
use thiserror::Error;
use tokio::{
    net::TcpListener,
    sync::{mpsc, watch},
};

use super::assets::find_asset;
use crate::rfb_ws::{RfbWebSocketConfig, RfbWebSocketService, RfbWebSocketServiceError};
use ipkvm_session::rfb_connection::{RfbConnectionGate, RfbServerEvent, RfbTransportKind};

const CONTENT_TYPE_OPTIONS: &str = "x-content-type-options";
const JPEG_QUALITY: u8 = 85;

pub struct HeadlessWebService<S: ?Sized> {
    rfb: RfbWebSocketService<S>,
    api: Arc<ApiState<S>>,
    shutdown: watch::Receiver<bool>,
}

/// `/api` 路由共享的服务状态：帧源元数据与连接闸门状态。
struct ApiState<S: ?Sized> {
    frame_source: Arc<S>,
    gate: RfbConnectionGate,
}

#[derive(Debug, Error)]
pub enum HeadlessWebServiceError {
    #[error("invalid RFB WebSocket service: {0}")]
    Rfb(#[from] RfbWebSocketServiceError),
    #[error("headless HTTP server failed")]
    Serve(#[source] std::io::Error),
}

impl<S: FrameSource + ?Sized + 'static> HeadlessWebService<S> {
    pub fn new(
        frame_source: Arc<S>,
        event_tx: mpsc::Sender<RfbServerEvent>,
        config: RfbWebSocketConfig,
        shutdown: watch::Receiver<bool>,
        gate: RfbConnectionGate,
    ) -> Result<Self, HeadlessWebServiceError> {
        let api = Arc::new(ApiState {
            frame_source: Arc::clone(&frame_source),
            gate: gate.clone(),
        });
        let rfb = RfbWebSocketService::new(frame_source, event_tx, config, shutdown.clone(), gate)?;
        Ok(Self { rfb, api, shutdown })
    }

    pub async fn serve(self, listener: TcpListener) -> Result<(), HeadlessWebServiceError> {
        let shutdown = self.shutdown;
        let router = static_router()
            .merge(self.rfb.router())
            .merge(api_router(self.api));
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

fn api_router<S: FrameSource + ?Sized + 'static>(api: Arc<ApiState<S>>) -> Router {
    Router::new()
        .route("/api/status", get(api_status::<S>))
        .route("/api/screenshot", get(api_screenshot::<S>))
        .with_state(api)
}

async fn serve_asset(uri: Uri) -> Response {
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

#[derive(serde::Serialize)]
struct StatusResponse {
    service: ServiceStatus,
    video: VideoStatus,
    controller: ControllerStatus,
}

#[derive(serde::Serialize)]
struct ServiceStatus {
    name: &'static str,
    version: &'static str,
}

#[derive(serde::Serialize)]
struct VideoStatus {
    source: SourceStatus,
    frame: Option<FrameStatus>,
}

#[derive(serde::Serialize)]
struct SourceStatus {
    kind: &'static str,
    device_name: String,
    is_loop: bool,
}

#[derive(serde::Serialize)]
struct FrameStatus {
    width: u32,
    height: u32,
    pixel_format: &'static str,
    seq: u64,
}

#[derive(serde::Serialize)]
struct ControllerStatus {
    active: bool,
    client_id: Option<u64>,
    transport: Option<&'static str>,
    peer_addr: Option<String>,
    connected_since_ms: Option<u64>,
}

async fn api_status<S: FrameSource + ?Sized + 'static>(
    State(state): State<Arc<ApiState<S>>>,
) -> Response {
    let source = state.frame_source.source_info();
    let frame = state.frame_source.latest_frame();
    let controller = match state.gate.active_controller() {
        Some(active) => ControllerStatus {
            active: true,
            client_id: Some(active.client_id.get()),
            transport: Some(transport_name(active.transport)),
            peer_addr: Some(active.peer_addr.to_string()),
            connected_since_ms: Some(active.connected_since_ms),
        },
        None => ControllerStatus {
            active: false,
            client_id: None,
            transport: None,
            peer_addr: None,
            connected_since_ms: None,
        },
    };
    let status = StatusResponse {
        service: ServiceStatus {
            name: env!("CARGO_PKG_NAME"),
            version: env!("CARGO_PKG_VERSION"),
        },
        video: VideoStatus {
            source: SourceStatus {
                kind: source_kind_name(source.kind),
                device_name: source.device_name,
                is_loop: source.is_loop,
            },
            frame: frame.map(|frame| FrameStatus {
                width: frame.width,
                height: frame.height,
                pixel_format: pixel_format_name(frame.pixel_format),
                seq: frame.seq,
            }),
        },
        controller,
    };
    let body = serde_json::to_vec(&status).expect("status serialization cannot fail");
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(Body::from(body))
        .expect("status response headers are valid")
}

async fn api_screenshot<S: FrameSource + ?Sized + 'static>(
    State(state): State<Arc<ApiState<S>>>,
) -> Response {
    let Some(frame) = state.frame_source.latest_frame() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if frame.pixel_format != PixelFormat::Bgra8888 {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let Ok(width) = u16::try_from(frame.width) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let Ok(height) = u16::try_from(frame.height) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let Some(bgra) = packed_bgra(&frame) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut jpeg = Vec::new();
    if jpeg_encoder::Encoder::new(&mut jpeg, JPEG_QUALITY)
        .encode(&bgra, width, height, jpeg_encoder::ColorType::Bgra)
        .is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, HeaderValue::from_static("image/jpeg"))
        .header(CACHE_CONTROL, HeaderValue::from_static("no-store"))
        .body(Body::from(jpeg))
        .expect("screenshot response headers are valid")
}

/// 把帧数据整理成 jpeg-encoder 需要的紧凑行布局（每行 width*4 字节）。
///
/// jpeg-encoder 按连续行读取像素，不感知 `VideoFrame.stride`；当前各帧源
/// 都产生紧凑帧（stride == width*4），这里对带行填充的帧做防御性重排。
fn packed_bgra(frame: &VideoFrame) -> Option<Cow<'_, [u8]>> {
    let width = frame.width as usize;
    let row_bytes = width * 4;
    if frame.stride as usize == row_bytes {
        return Some(Cow::Borrowed(&frame.data));
    }
    let stride = frame.stride as usize;
    if frame.data.len() < stride * frame.height as usize {
        return None;
    }
    let mut packed = Vec::with_capacity(row_bytes * frame.height as usize);
    for y in 0..frame.height as usize {
        let start = y * stride;
        packed.extend_from_slice(frame.data.get(start..start + row_bytes)?);
    }
    Some(Cow::Owned(packed))
}

fn source_kind_name(kind: VideoSourceKind) -> &'static str {
    match kind {
        VideoSourceKind::Camera => "camera",
        VideoSourceKind::VideoFile => "file",
        VideoSourceKind::Generated => "generated",
    }
}

fn pixel_format_name(pixel_format: PixelFormat) -> &'static str {
    match pixel_format {
        PixelFormat::Yuy2 => "yuy2",
        PixelFormat::Nv12 => "nv12",
        PixelFormat::Bgra8888 => "bgra8888",
        PixelFormat::Mjpeg => "mjpeg",
        PixelFormat::H264 => "h264",
        PixelFormat::Unknown => "unknown",
    }
}

fn transport_name(transport: RfbTransportKind) -> &'static str {
    match transport {
        RfbTransportKind::Tcp => "tcp",
        RfbTransportKind::WebSocket => "ws",
    }
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
    use ipkvm_video::MonotonicTimestamp;

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

    #[test]
    fn packed_bgra_borrows_tightly_packed_frames() {
        let frame = bgra_frame(2, 1, 8, vec![0, 0, 255, 255, 0, 255, 0, 255]);

        match packed_bgra(&frame) {
            Some(Cow::Borrowed(bytes)) => {
                assert!(std::ptr::eq(bytes.as_ptr(), frame.data.as_ptr()));
                assert_eq!(bytes.len(), frame.data.len());
            }
            _ => panic!("tight frame must not be copied"),
        }
    }

    #[test]
    fn packed_bgra_tightens_padded_rows() {
        let frame = bgra_frame(
            2,
            2,
            12,
            vec![
                0, 0, 255, 255, 0, 255, 0, 255, // 第 0 行
                1, 1, 1, 1, // 行填充
                3, 3, 3, 3, 4, 4, 4, 4, // 第 1 行
                2, 2, 2, 2, // 行填充
            ],
        );

        let packed = packed_bgra(&frame).unwrap();
        assert_eq!(
            packed.as_ref(),
            &[0, 0, 255, 255, 0, 255, 0, 255, 3, 3, 3, 3, 4, 4, 4, 4]
        );
    }

    #[test]
    fn packed_bgra_rejects_truncated_data() {
        let frame = bgra_frame(4, 1, 20, vec![0; 4]);

        assert!(packed_bgra(&frame).is_none());
    }

    #[test]
    fn packed_bgra_rejects_data_shorter_than_stride_times_height() {
        // 像素行与行间填充齐备、仅末行填充缺失：数据短于 stride*height，
        // 视为畸形帧拒绝（行校验先于按元数据分配，避免畸形帧触发大分配）。
        let frame = bgra_frame(
            2,
            2,
            12,
            vec![
                0, 0, 255, 255, 0, 255, 0, 255, // 第 0 行
                1, 1, 1, 1, // 行填充
                3, 3, 3, 3, 4, 4, 4, 4, // 第 1 行
            ],
        );

        assert!(packed_bgra(&frame).is_none());
    }

    fn bgra_frame(width: u32, height: u32, stride: u32, data: Vec<u8>) -> VideoFrame {
        VideoFrame::new(
            1,
            MonotonicTimestamp::from_nanos(1),
            width,
            height,
            stride,
            PixelFormat::Bgra8888,
            Arc::from(data.into_boxed_slice()),
        )
    }
}
