use std::{
    borrow::Cow,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{
        HeaderValue, StatusCode, Uri,
        header::{CACHE_CONTROL, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
    routing::{get, post},
};
use ipkvm_core::InputSink;
use ipkvm_session::console_session::InputOfflineInfo;
use ipkvm_session::session_manager::{SessionManager, SessionState};
use ipkvm_video::{FrameSource, PixelFormat, VideoFrame, VideoSourceKind};
use serde::Deserialize;
use thiserror::Error;
use tokio::{
    net::TcpListener,
    sync::{mpsc, watch},
};

use super::{assets::find_asset, auth};
use crate::frame_source::{EmptyFrameSource, SwitchableFrameSource};
use crate::rfb_ws::{RfbWebSocketConfig, RfbWebSocketService, RfbWebSocketServiceError};
use crate::settings::{SettingsStore, WebSettings, validate};
use ipkvm_session::rfb_connection::{RfbConnectionGate, RfbServerEvent, RfbTransportKind};

const CONTENT_TYPE_OPTIONS: &str = "x-content-type-options";
const JPEG_QUALITY: u8 = 85;
/// 视频断流判定阈值：超过该时长无新帧视为 stalled（与桌面端无信号阈值一致）。
const VIDEO_STALL_TIMEOUT_NS: u64 = 2_000_000_000;

pub struct HeadlessWebService<I: InputSink + Clone + Send + 'static> {
    rfb: RfbWebSocketService<SwitchableFrameSource>,
    api: Arc<ApiState<I>>,
    shutdown: watch::Receiver<bool>,
    auth: Option<String>,
}

/// `/api` 路由共享的服务状态：帧源元数据、连接闸门与会话管理器。
pub(super) struct ApiState<I: InputSink + Clone + Send + 'static> {
    pub(super) frame_source: Arc<SwitchableFrameSource>,
    pub(super) gate: RfbConnectionGate,
    pub(super) manager: Arc<tokio::sync::Mutex<SessionManager<I>>>,
    pub(super) selection: tokio::sync::Mutex<Option<SessionSelection>>,
    /// 会话工厂：按请求中的设备选择构造帧源与 sink。
    pub(super) factory: Arc<dyn SessionFactory<I> + Send + Sync>,
    /// 运行时设置存储：`/api/settings` 读写 + 会话组装分层取默认值。
    pub(super) settings: Arc<SettingsStore>,
    /// 手动停止标记：`stop` 置位，`create`/`restart` 清除，恢复循环尊重。
    pub(super) manual_stop: Arc<AtomicBool>,
}

/// 一次运行时会话选择。字段为 `None` 时由工厂沿用启动配置默认值。
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct SessionSelection {
    pub video: Option<String>,
    pub serial: Option<String>,
}

impl SessionSelection {
    fn with_overrides(&self, overrides: &Self) -> Self {
        Self {
            video: overrides.video.clone().or_else(|| self.video.clone()),
            serial: overrides.serial.clone().or_else(|| self.serial.clone()),
        }
    }
}

/// 会话工厂：按启动默认配置与运行时选择构造帧源与输入 sink。
pub trait SessionFactory<I>: Send + Sync {
    fn build(&self, selection: &SessionSelection) -> Result<(Arc<dyn FrameSource>, I), String>;
}

#[derive(Debug, Error)]
pub enum HeadlessWebServiceError {
    #[error("invalid RFB WebSocket service: {0}")]
    Rfb(#[from] RfbWebSocketServiceError),
    #[error("headless HTTP server failed")]
    Serve(#[source] std::io::Error),
}

impl<I: InputSink + Clone + Send + 'static> HeadlessWebService<I> {
    /// `auth` 为 `[auth] token`（HTTP/WS 鉴权）；`None` 表示仅允许本机来源。
    ///
    /// `event_publisher` 来自 `SessionManager::event_publisher()`，传输层据此
    /// 在会话重启后拿到新事件发送端。
    #[allow(clippy::too_many_arguments)] // 组装依赖项，无法合理合并
    pub fn new(
        frame_source: Arc<SwitchableFrameSource>,
        manager: Arc<tokio::sync::Mutex<SessionManager<I>>>,
        factory: Arc<dyn SessionFactory<I> + Send + Sync>,
        event_publisher: watch::Receiver<Option<mpsc::Sender<RfbServerEvent>>>,
        config: RfbWebSocketConfig,
        shutdown: watch::Receiver<bool>,
        gate: RfbConnectionGate,
        auth: Option<String>,
        settings: Arc<SettingsStore>,
        manual_stop: Arc<AtomicBool>,
        initial_selection: Option<SessionSelection>,
    ) -> Result<Self, HeadlessWebServiceError> {
        let api = Arc::new(ApiState {
            frame_source: Arc::clone(&frame_source),
            gate: gate.clone(),
            manager,
            selection: tokio::sync::Mutex::new(initial_selection),
            factory,
            settings,
            manual_stop,
        });
        let rfb = RfbWebSocketService::new(
            frame_source,
            event_publisher,
            config,
            shutdown.clone(),
            gate,
        )?;
        Ok(Self {
            rfb,
            api,
            shutdown,
            auth,
        })
    }

    pub async fn serve(self, listener: TcpListener) -> Result<(), HeadlessWebServiceError> {
        // 自动恢复循环：输入泵失败/视频从未出帧时按退避重建会话。
        tokio::spawn(super::recovery::run_recovery_loop(
            Arc::clone(&self.api),
            self.shutdown.clone(),
            super::recovery::RecoveryPolicy::default(),
        ));
        let shutdown = self.shutdown;
        let router = static_router()
            .merge(self.rfb.router())
            .merge(api_router(self.api))
            .layer(axum::middleware::from_fn_with_state(
                auth::AuthState { token: self.auth },
                auth::require_auth,
            ));
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
        .route("/assets/modules/{*path}", get(serve_asset))
        .route("/licenses", get(serve_asset))
        .route("/licenses/", get(serve_asset))
        .route("/vendor/novnc/{*path}", get(serve_asset))
}

fn api_router<I: InputSink + Clone + Send + 'static>(api: Arc<ApiState<I>>) -> Router {
    Router::new()
        .route("/api/status", get(api_status::<I>))
        .route("/api/screenshot", get(api_screenshot::<I>))
        .route("/api/devices", get(api_devices))
        .route(
            "/api/settings",
            get(api_get_settings::<I>).post(api_post_settings::<I>),
        )
        .route("/api/session", post(api_session::<I>))
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

// ---- 结构化 JSON 错误响应 ----

/// 统一 JSON 错误响应：`{"error": "...", "detail": "..."}`。
fn json_error(code: StatusCode, error: &str, detail: Option<&str>) -> Response {
    #[derive(serde::Serialize)]
    struct ErrorBody<'a> {
        error: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<&'a str>,
    }
    let body = serde_json::to_vec(&ErrorBody { error, detail })
        .expect("error body serialization cannot fail");
    Response::builder()
        .status(code)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(Body::from(body))
        .expect("error response headers are valid")
}

/// 统一 JSON 成功响应。
fn json_response<T: serde::Serialize>(value: &T) -> Response {
    let body = serde_json::to_vec(value).expect("response serialization cannot fail");
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(Body::from(body))
        .expect("response headers are valid")
}

// ---- /api/status ----

#[derive(serde::Serialize)]
struct StatusResponse {
    service: ServiceStatus,
    video: VideoStatus,
    controller: ControllerStatus,
    session: SessionStatus,
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
    stalled: bool,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    last_frame_ns: Option<u64>,
}

#[derive(serde::Serialize)]
struct ControllerStatus {
    active: bool,
    client_id: Option<u64>,
    transport: Option<&'static str>,
    peer_addr: Option<String>,
    connected_since_ms: Option<u64>,
}

/// 会话状态：管理器状态 + 输入/丢帧/串口统计。会话未创建时各计数字段为 0。
#[derive(serde::Serialize)]
struct SessionStatus {
    state: &'static str,
    /// 手动停止标记：恒序列化（契约要求恒存在）。
    manual_stop: bool,
    input_events: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_input_ns: Option<u64>,
    dropped_frames: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    serial: Option<SerialStatsDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_offline: Option<InputOfflineDto>,
}

#[derive(serde::Serialize)]
struct SerialStatsDto {
    batches_accepted: u64,
    frames_accepted: u64,
}

#[derive(serde::Serialize)]
struct InputOfflineDto {
    reason: String,
    since_ns: u64,
}

impl From<&InputOfflineInfo> for InputOfflineDto {
    fn from(info: &InputOfflineInfo) -> Self {
        Self {
            reason: info.reason.clone(),
            since_ns: info.since_ns,
        }
    }
}

impl From<ipkvm_session::serial_stats::SerialStats> for SerialStatsDto {
    fn from(stats: ipkvm_session::serial_stats::SerialStats) -> Self {
        Self {
            batches_accepted: stats.batches_accepted,
            frames_accepted: stats.frames_accepted,
        }
    }
}

async fn api_status<I: InputSink + Clone + Send + 'static>(
    State(state): State<Arc<ApiState<I>>>,
) -> Response {
    let frame_source = state.frame_source.current();
    let source = frame_source.source_info();
    let frame = frame_source.latest_frame();
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
    // 会话统计：短暂持锁读取后立即 clone（守卫不可跨 await）。
    let (
        session_state,
        input_events,
        last_input_ns,
        dropped_frames,
        serial,
        last_frame_ns,
        input_offline,
    ) = {
        let mut manager = state.manager.lock().await;
        manager.refresh_stats();
        let state_name = session_state_name(manager.state());
        let (input_events, last_input_ns, dropped_frames, serial, last_frame_ns, input_offline) =
            match manager.session() {
                Some(session) => {
                    let stats = session.stats();
                    (
                        stats.input_events,
                        stats.last_input_ns,
                        stats.dropped_frames,
                        stats.serial.map(SerialStatsDto::from),
                        stats.last_frame_ns,
                        stats.input_offline.as_ref().map(InputOfflineDto::from),
                    )
                }
                None => (0, None, 0, None, None, None),
            };
        (
            state_name,
            input_events,
            last_input_ns,
            dropped_frames,
            serial,
            last_frame_ns,
            input_offline,
        )
    };
    let now_ns = ipkvm_session::now_ns();
    let stalled = match &frame {
        Some(_) => last_frame_ns
            .map(|last| now_ns.saturating_sub(last) > VIDEO_STALL_TIMEOUT_NS)
            .unwrap_or(true),
        None => session_state != "absent",
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
                last_frame_ns,
            }),
            stalled,
        },
        controller,
        session: SessionStatus {
            state: session_state,
            manual_stop: state.manual_stop.load(Ordering::Relaxed),
            input_events,
            last_input_ns,
            dropped_frames,
            serial,
            input_offline,
        },
    };
    json_response(&status)
}

// ---- /api/settings ----

async fn api_get_settings<I: InputSink + Clone + Send + 'static>(
    State(state): State<Arc<ApiState<I>>>,
) -> Response {
    json_response(&state.settings.get())
}

async fn api_post_settings<I: InputSink + Clone + Send + 'static>(
    State(state): State<Arc<ApiState<I>>>,
    body: Result<Json<WebSettings>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let settings = match body {
        Ok(Json(settings)) => settings,
        Err(rejection) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "invalid settings",
                Some(&rejection.to_string()),
            );
        }
    };
    if let Err(detail) = validate(&settings) {
        return json_error(StatusCode::BAD_REQUEST, "invalid settings", Some(&detail));
    }
    match state.settings.save(&settings).await {
        Ok(()) => json_response(&settings),
        Err(detail) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "settings save failed",
            Some(&detail),
        ),
    }
}

// ---- /api/screenshot ----

async fn api_screenshot<I: InputSink + Clone + Send + 'static>(
    State(state): State<Arc<ApiState<I>>>,
) -> Response {
    let frame_source = state.frame_source.current();
    let Some(frame) = frame_source.latest_frame() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    state.manager.lock().await.refresh_stats();
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

// ---- /api/devices ----

#[derive(serde::Serialize)]
struct DeviceListResponse {
    video: Vec<DeviceDto>,
    serial: Vec<DeviceDto>,
}

#[derive(serde::Serialize)]
struct DeviceDto {
    id: String,
    display_name: String,
    kind: &'static str,
}

async fn api_devices<I: InputSink + Clone + Send + 'static>(
    State(_state): State<Arc<ApiState<I>>>,
) -> Response {
    let video = match ipkvm_session::devices::list_video_devices() {
        Ok(devices) => devices
            .into_iter()
            .map(|d| DeviceDto {
                id: d.id,
                display_name: d.display_name,
                kind: "video",
            })
            .collect::<Vec<_>>(),
        Err(error) => {
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "video enumeration failed",
                Some(&error.to_string()),
            );
        }
    };
    let serial = match ipkvm_session::devices::list_serial_devices() {
        Ok(devices) => devices
            .into_iter()
            .map(|d| DeviceDto {
                id: d.path,
                display_name: d.display_name,
                kind: "serial",
            })
            .collect::<Vec<_>>(),
        Err(error) => {
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "serial enumeration failed",
                Some(&error.to_string()),
            );
        }
    };
    let body = serde_json::to_vec(&DeviceListResponse { video, serial })
        .expect("device list serialization cannot fail");
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(Body::from(body))
        .expect("device list response headers are valid")
}

// ---- /api/session ----

/// `POST /api/session` 请求体：`restart` 可携带设备选择并重启会话。
#[derive(Deserialize)]
struct SessionRequest {
    action: String,
    #[serde(default)]
    video: Option<String>,
    #[serde(default)]
    serial: Option<String>,
}

#[derive(serde::Serialize)]
struct SessionResponse {
    state: &'static str,
}

async fn api_session<I: InputSink + Clone + Send + 'static>(
    State(state): State<Arc<ApiState<I>>>,
    Json(request): Json<SessionRequest>,
) -> Response {
    let mut manager = state.manager.lock().await;
    let selection = SessionSelection {
        video: request.video,
        serial: request.serial,
    };
    match request.action.as_str() {
        "restart" => {
            // 手动停止标记由 restart 清除：用户显式要求重建会话。
            state.manual_stop.store(false, Ordering::Relaxed);
            let (previous_selection, target_selection) = {
                let current = state.selection.lock().await;
                let base = current.clone().unwrap_or_default();
                (current.clone(), base.with_overrides(&selection))
            };
            if let Err(error) = manager.stop_and_destroy().await {
                return session_error(error);
            }
            state
                .frame_source
                .set_current(Arc::new(EmptyFrameSource::new()));
            let (frame_source, sink) = match state.factory.build(&target_selection) {
                Ok(built) => built,
                Err(detail) => {
                    let rollback = match &previous_selection {
                        Some(previous) => restore_previous_session(&state, &mut manager, previous)
                            .await
                            .map(|()| previous.clone()),
                        None => Err("没有上一成功会话选择可回滚".to_string()),
                    };
                    let detail = match rollback {
                        Ok(restored) => {
                            *state.selection.lock().await = Some(restored);
                            format!("{detail}; 已回滚到上一会话选择")
                        }
                        Err(rollback_detail) => {
                            *state.selection.lock().await = None;
                            format!("{detail}; 回滚上一会话选择失败：{rollback_detail}")
                        }
                    };
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "session creation failed",
                        Some(&detail),
                    );
                }
            };
            match create_and_start_session(&mut manager, &frame_source, sink, state.gate.clone()) {
                Ok(()) => {
                    state.frame_source.set_current(frame_source);
                    *state.selection.lock().await = Some(target_selection);
                    json_session_state(&manager)
                }
                Err(error) => session_error(error),
            }
        }
        "create" => {
            // 零初始会话首启：经工厂构造帧源/sink 后 create + start。
            if manager.state() != SessionState::Absent {
                return session_error(ipkvm_session::console_session::SessionError::AlreadyCreated);
            }
            // 手动停止标记由 create 清除：用户显式要求启动会话。
            state.manual_stop.store(false, Ordering::Relaxed);
            let target_selection = {
                let current = state.selection.lock().await;
                current
                    .clone()
                    .unwrap_or_default()
                    .with_overrides(&selection)
            };
            let (frame_source, sink) = match state.factory.build(&target_selection) {
                Ok(built) => built,
                Err(detail) => {
                    return json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "session creation failed",
                        Some(&detail),
                    );
                }
            };
            // create 用独立闸门副本（与传输层共享同一仲裁，clone 共享内部信号量）。
            if let Err(error) =
                create_and_start_session(&mut manager, &frame_source, sink, state.gate.clone())
            {
                return session_error(error);
            }
            state.frame_source.set_current(frame_source);
            *state.selection.lock().await = Some(target_selection);
            json_session_state(&manager)
        }
        "stop" => match manager.stop() {
            Ok(()) => {
                manager.wait_stopped().await;
                // 断开必须真正释放采集：销毁会话并把帧源切回空源，
                // 否则相机/素材源仍被持有，持续采集浪费 CPU。
                let _ = manager.stop_and_destroy().await;
                state
                    .frame_source
                    .set_current(Arc::new(EmptyFrameSource::new()));
                state.manual_stop.store(true, Ordering::Relaxed);
                json_session_state(&manager)
            }
            Err(error) => session_error(error),
        },
        other => json_error(
            StatusCode::BAD_REQUEST,
            "unknown action",
            Some(&format!(
                "action 必须是 restart/create/stop，得到 {other:?}"
            )),
        ),
    }
}

pub(super) fn create_and_start_session<I: InputSink + Clone + Send + 'static>(
    manager: &mut SessionManager<I>,
    frame_source: &Arc<dyn FrameSource>,
    sink: I,
    gate: RfbConnectionGate,
) -> Result<(), ipkvm_session::console_session::SessionError> {
    manager.create(Arc::clone(frame_source), sink, gate)?;
    manager.start()?;
    Ok(())
}

async fn restore_previous_session<I: InputSink + Clone + Send + 'static>(
    state: &ApiState<I>,
    manager: &mut SessionManager<I>,
    selection: &SessionSelection,
) -> Result<(), String> {
    let (frame_source, sink) = state.factory.build(selection)?;
    create_and_start_session(manager, &frame_source, sink, state.gate.clone())
        .map_err(|error| error.to_string())?;
    state.frame_source.set_current(frame_source);
    Ok(())
}

fn json_session_state<I: InputSink + Clone + Send + 'static>(
    manager: &SessionManager<I>,
) -> Response {
    let body = serde_json::to_vec(&SessionResponse {
        state: session_state_name(manager.state()),
    })
    .expect("session response serialization cannot fail");
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(Body::from(body))
        .expect("session response headers are valid")
}

/// 会话级错误映射到 HTTP 状态码与结构化 JSON。
fn session_error(error: ipkvm_session::console_session::SessionError) -> Response {
    use ipkvm_session::console_session::SessionError;
    match error {
        SessionError::AlreadyRunning | SessionError::AlreadyCreated => json_error(
            StatusCode::CONFLICT,
            "session conflict",
            Some(&error.to_string()),
        ),
        SessionError::NotRunning => json_error(
            StatusCode::CONFLICT,
            "session conflict",
            Some(&error.to_string()),
        ),
        SessionError::Input(error) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "input pump failed",
            Some(&error.to_string()),
        ),
    }
}

pub(super) fn session_state_name(state: SessionState) -> &'static str {
    match state {
        SessionState::Absent => "absent",
        SessionState::Stopped => "stopped",
        SessionState::Running => "running",
    }
}

fn source_kind_name(kind: VideoSourceKind) -> &'static str {
    match kind {
        VideoSourceKind::None => "none",
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
    use super::*;

    #[test]
    fn json_error_serializes_error_and_optional_detail() {
        let response = json_error(StatusCode::BAD_REQUEST, "bad", Some("why"));
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn session_state_name_covers_all_variants() {
        assert_eq!(session_state_name(SessionState::Absent), "absent");
        assert_eq!(session_state_name(SessionState::Stopped), "stopped");
        assert_eq!(session_state_name(SessionState::Running), "running");
    }
}
