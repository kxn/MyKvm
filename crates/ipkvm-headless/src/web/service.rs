use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
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
use ipkvm_core::{InputSink, MouseMode, MouseProfile};
use ipkvm_device::{DeviceInventoryProvider, SerialDevice, VideoDevice};
use ipkvm_session::console_session::InputOfflineInfo;
use ipkvm_session::session_manager::{SessionManager, SessionState};
use ipkvm_video::{FrameSource, PixelFormat, VideoSourceKind};
use serde::Deserialize;
use thiserror::Error;
use tokio::{
    net::TcpListener,
    sync::{mpsc, watch},
    time::timeout,
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
    /// 设备枚举 provider：只描述设备，不持有相机或串口句柄。
    pub(super) device_provider: Arc<dyn DeviceInventoryProvider>,
    /// 运行时设置存储：`/api/settings` 读写 + 会话组装分层取默认值。
    pub(super) settings: Arc<SettingsStore>,
    /// 手动停止标记：`stop` 置位，`create`/`restart` 清除，恢复循环尊重。
    pub(super) manual_stop: Arc<AtomicBool>,
    /// 浏览器断开后等待超时的时间戳（None 表示没有断开）。
    pub(super) disconnect_deadline: Arc<tokio::sync::Mutex<Option<std::time::Instant>>>,
}

/// 一次运行时会话选择。字段为 `None` 时由工厂沿用启动配置默认值。
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct SessionSelection {
    pub video: Option<String>,
    pub serial: Option<String>,
    #[serde(default)]
    pub mouse_profile: Option<String>,
}

impl SessionSelection {
    fn with_overrides(&self, overrides: &Self) -> Self {
        Self {
            video: overrides.video.clone().or_else(|| self.video.clone()),
            serial: overrides.serial.clone().or_else(|| self.serial.clone()),
            mouse_profile: overrides
                .mouse_profile
                .clone()
                .or_else(|| self.mouse_profile.clone()),
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
        device_provider: Arc<dyn DeviceInventoryProvider>,
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
            device_provider,
            settings,
            manual_stop,
            disconnect_deadline: Arc::new(tokio::sync::Mutex::new(None)),
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
        .route("/api/system", get(api_system))
        .route(
            "/api/settings",
            get(api_get_settings::<I>).post(api_post_settings::<I>),
        )
        .route("/api/input/mouse-profile", post(api_mouse_profile::<I>))
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
    /// 源侧采集/转换统计（仅支持统计的源才输出）。
    #[serde(skip_serializing_if = "Option::is_none")]
    source_stats: Option<SourceStatsDto>,
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
    /// 采集时间（统一时钟，来自 frame.timestamp）。
    #[serde(skip_serializing_if = "Option::is_none")]
    capture_ns: Option<u64>,
    /// 观察时间（统一时钟，/api/status 读到的时刻）。
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
    mouse_profile: &'static str,
    mouse_mode: &'static str,
    /// RFB 编码统计快照（encode 耗时与字节累计；仅活动连接存在后才有值）。
    #[serde(skip_serializing_if = "Option::is_none")]
    encode: Option<RfbEncodeStatsDto>,
    /// RFB 成功发送的 FramebufferUpdate 累计次数。
    updates_sent: u64,
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

/// 源侧采集/转换统计 DTO（从 `ipkvm_video::SourceStatsSnapshot` 映射）。
#[derive(serde::Serialize)]
struct SourceStatsDto {
    published_frames: u64,
    dropped_frames: u64,
    convert_ns_total: u64,
    convert_count: u64,
    capture_wait_ns_total: u64,
    capture_wait_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_capture_ns: Option<u64>,
}

impl From<ipkvm_video::SourceStatsSnapshot> for SourceStatsDto {
    fn from(s: ipkvm_video::SourceStatsSnapshot) -> Self {
        Self {
            published_frames: s.published_frames,
            dropped_frames: s.dropped_frames,
            convert_ns_total: s.convert_ns_total,
            convert_count: s.convert_count,
            capture_wait_ns_total: s.capture_wait_ns_total,
            capture_wait_count: s.capture_wait_count,
            last_capture_ns: s.last_capture_ns,
        }
    }
}

/// RFB 编码统计 DTO（从 `ipkvm_rfb::RfbEncodeStatsSnapshot` 映射）。
#[derive(serde::Serialize)]
struct RfbEncodeStatsDto {
    encode_ns_total: u64,
    encode_count: u64,
    encoded_bytes_total: u64,
}

impl From<ipkvm_rfb::RfbEncodeStatsSnapshot> for RfbEncodeStatsDto {
    fn from(s: ipkvm_rfb::RfbEncodeStatsSnapshot) -> Self {
        Self {
            encode_ns_total: s.encode_ns_total,
            encode_count: s.encode_count,
            encoded_bytes_total: s.encoded_bytes_total,
        }
    }
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
        encode,
        updates_sent,
        input_offline,
        actual_mouse_mode,
    ) = {
        let mut manager = state.manager.lock().await;
        manager.refresh_stats();
        let state_name = session_state_name(manager.state());
        let (
            input_events,
            last_input_ns,
            dropped_frames,
            serial,
            last_frame_ns,
            encode,
            updates_sent,
            input_offline,
        ) = match manager.session() {
            Some(session) => {
                let stats = session.stats();
                (
                    stats.input_events,
                    stats.last_input_ns,
                    stats.dropped_frames,
                    stats.serial.map(SerialStatsDto::from),
                    stats.last_frame_ns,
                    stats.encode.map(RfbEncodeStatsDto::from),
                    stats.updates_sent,
                    stats.input_offline.as_ref().map(InputOfflineDto::from),
                )
            }
            None => (0, None, 0, None, None, None, 0, None),
        };
        let actual_mouse_mode = manager
            .session()
            .and_then(|session| *session.mouse_mode().borrow());
        (
            state_name,
            input_events,
            last_input_ns,
            dropped_frames,
            serial,
            last_frame_ns,
            encode,
            updates_sent,
            input_offline,
            actual_mouse_mode,
        )
    };
    let now_ns = ipkvm_session::now_ns();
    let stalled = match &frame {
        Some(_) => last_frame_ns
            .map(|last| now_ns.saturating_sub(last) > VIDEO_STALL_TIMEOUT_NS)
            .unwrap_or(true),
        None => session_state != "absent",
    };
    let (mouse_profile, selected_mouse_mode) = {
        let selection = state.selection.lock().await;
        let profile = selection
            .as_ref()
            .and_then(|selection| selection.mouse_profile.as_deref())
            .and_then(|value| MouseProfile::parse(value).ok())
            .unwrap_or_else(|| state.settings.get().mouse_profile);
        (profile.as_str(), profile.resolve_mode())
    };
    let mouse_mode = actual_mouse_mode.unwrap_or(selected_mouse_mode);

    // 浏览器断开超时：如果会话运行中但没有活跃连接，设置 1 分钟超时。
    // 如果 1 分钟内有新连接，取消超时；超时后停止 session。
    if !controller.active
        && session_state == "running"
        && !state.manual_stop.load(Ordering::Relaxed)
    {
        let mut deadline = state.disconnect_deadline.lock().await;
        if deadline.is_none() {
            *deadline = Some(std::time::Instant::now() + std::time::Duration::from_secs(60));
        }
    } else if controller.active {
        // 有活跃连接，清除超时
        state.disconnect_deadline.lock().await.take();
    }

    // 检查是否超时
    {
        let deadline = state.disconnect_deadline.lock().await;
        if let Some(dl) = *deadline
            && std::time::Instant::now() >= dl
        {
            drop(deadline);
            // 超时：停止 session
            let mut manager = state.manager.lock().await;
            if manager.state() == ipkvm_session::session_manager::SessionState::Running {
                let _ = manager.stop();
                manager.wait_stopped().await;
                let _ = manager.stop_and_destroy().await;
                state
                    .frame_source
                    .set_current(Arc::new(EmptyFrameSource::new()));
            }
            state.disconnect_deadline.lock().await.take();
        }
    }

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
                capture_ns: Some(frame.timestamp.nanos),
                last_frame_ns,
            }),
            stalled,
            source_stats: frame_source.source_stats().map(SourceStatsDto::from),
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
            mouse_profile,
            mouse_mode: mouse_mode_name(mouse_mode),
            encode,
            updates_sent,
        },
    };
    json_response(&status)
}

fn mouse_mode_name(mode: MouseMode) -> &'static str {
    match mode {
        MouseMode::Absolute => "absolute",
        MouseMode::Relative => "relative",
    }
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

// ---- /api/input/mouse-profile ----

#[derive(Deserialize)]
struct MouseProfileRequest {
    mouse_profile: String,
}

#[derive(serde::Serialize)]
struct MouseProfileResponse {
    mouse_profile: &'static str,
    mouse_mode: &'static str,
}

/// 在当前活动 RFB 输入泵中切换 profile。设备状态切换仍由泵统一执行，
/// API 只负责校验、选取当前控制器并更新会话选择覆盖值。
async fn api_mouse_profile<I: InputSink + Clone + Send + 'static>(
    State(state): State<Arc<ApiState<I>>>,
    body: Result<Json<MouseProfileRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Json(request) = match body {
        Ok(body) => body,
        Err(rejection) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "invalid mouse profile",
                Some(&rejection.to_string()),
            );
        }
    };
    let profile = match MouseProfile::parse(&request.mouse_profile) {
        Ok(profile) => profile,
        Err(error) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "invalid mouse profile",
                Some(&error.to_string()),
            );
        }
    };
    let Some(active) = state.gate.active_controller() else {
        return json_error(
            StatusCode::CONFLICT,
            "no active controller",
            Some("connect the remote display before changing the mouse profile"),
        );
    };
    let manager = state.manager.lock().await;
    if manager.state() != SessionState::Running {
        return json_error(StatusCode::CONFLICT, "session is not running", None);
    }
    let Some(session) = manager.session() else {
        return json_error(StatusCode::CONFLICT, "session is not running", None);
    };
    let event_tx = session.event_tx().clone();
    let mut mode_rx = session.mouse_mode();
    let current_profile = {
        let selection = state.selection.lock().await;
        selection
            .as_ref()
            .and_then(|selection| selection.mouse_profile.as_deref())
            .and_then(|value| MouseProfile::parse(value).ok())
            .unwrap_or_else(|| state.settings.get().mouse_profile)
    };
    if profile.resolve_mode() != current_profile.resolve_mode() {
        if event_tx
            .send(RfbServerEvent::SetMouseMode {
                client_id: active.client_id,
                mode: profile.resolve_mode(),
            })
            .await
            .is_err()
        {
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "mouse profile switch failed",
                Some("the input session stopped before the mode change was queued"),
            );
        }
        let applied = timeout(
            Duration::from_secs(1),
            wait_for_mouse_mode(&mut mode_rx, profile.resolve_mode()),
        )
        .await
        .unwrap_or(false);
        if !applied {
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "mouse profile switch failed",
                Some("the input sink did not confirm the requested mode"),
            );
        }
    }
    {
        let mut selection = state.selection.lock().await;
        let selection = selection.get_or_insert_with(SessionSelection::default);
        selection.mouse_profile = Some(profile.as_str().to_string());
    }
    json_response(&MouseProfileResponse {
        mouse_profile: profile.as_str(),
        mouse_mode: mouse_mode_name(profile.resolve_mode()),
    })
}

async fn wait_for_mouse_mode(
    mode_rx: &mut watch::Receiver<Option<MouseMode>>,
    expected: MouseMode,
) -> bool {
    loop {
        if *mode_rx.borrow() == Some(expected) {
            return true;
        }
        if mode_rx.changed().await.is_err() {
            return false;
        }
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

    // MJPEG 透传：帧已经是 JPEG，直接返回。
    if frame.pixel_format == PixelFormat::Mjpeg {
        return Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, HeaderValue::from_static("image/jpeg"))
            .header(CACHE_CONTROL, HeaderValue::from_static("no-store"))
            .body(Body::from(frame.data.to_vec()))
            .expect("screenshot response headers are valid");
    }

    // RGB：编码为 JPEG。
    if frame.pixel_format != PixelFormat::Rgb888 {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let Ok(width) = u16::try_from(frame.width) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let Ok(height) = u16::try_from(frame.height) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };

    let mut jpeg = Vec::new();
    if jpeg_encoder::Encoder::new(&mut jpeg, JPEG_QUALITY)
        .encode(&frame.data, width, height, jpeg_encoder::ColorType::Rgb)
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

use std::sync::Mutex;

/// CPU 使用率采样器：记录上一次的 ticks，计算增量百分比。
struct CpuSampler {
    last_proc_ticks: u64,
    last_total_ticks: u64,
}

impl CpuSampler {
    fn new() -> Self {
        let (proc, total) = read_cpu_ticks();
        Self {
            last_proc_ticks: proc,
            last_total_ticks: total,
        }
    }

    fn sample(&mut self) -> f64 {
        let (proc, total) = read_cpu_ticks();
        let proc_delta = proc.saturating_sub(self.last_proc_ticks);
        let total_delta = total.saturating_sub(self.last_total_ticks);
        self.last_proc_ticks = proc;
        self.last_total_ticks = total;
        if total_delta == 0 {
            0.0
        } else {
            (proc_delta as f64 / total_delta as f64) * 100.0
        }
    }
}

fn read_cpu_ticks() -> (u64, u64) {
    let proc_ticks = std::fs::read_to_string("/proc/self/stat")
        .ok()
        .and_then(|s| {
            let fields: Vec<&str> = s.split_whitespace().collect();
            let utime: u64 = fields.get(13)?.parse().ok()?;
            let stime: u64 = fields.get(14)?.parse().ok()?;
            Some(utime + stime)
        })
        .unwrap_or(0);

    let total_ticks = std::fs::read_to_string("/proc/stat")
        .ok()
        .and_then(|s| {
            let line = s.lines().next()?;
            let fields: Vec<&str> = line.split_whitespace().collect();
            Some(
                fields[1..]
                    .iter()
                    .filter_map(|v| v.parse::<u64>().ok())
                    .sum(),
            )
        })
        .unwrap_or(0);

    (proc_ticks, total_ticks)
}

static CPU_SAMPLER: std::sync::OnceLock<Mutex<CpuSampler>> = std::sync::OnceLock::new();

async fn api_system() -> Response {
    let mem_info = read_mem_info();
    let load_avg = read_load_avg();
    let cpu_percent = CPU_SAMPLER
        .get_or_init(|| Mutex::new(CpuSampler::new()))
        .lock()
        .map(|mut s| s.sample())
        .unwrap_or(0.0);

    #[derive(serde::Serialize)]
    struct SystemInfo {
        mem_total_kb: u64,
        mem_available_kb: u64,
        mem_used_kb: u64,
        load_1m: f64,
        load_5m: f64,
        load_15m: f64,
        cpu_percent: f64,
    }

    let info = SystemInfo {
        mem_total_kb: mem_info.0,
        mem_available_kb: mem_info.1,
        mem_used_kb: mem_info.0.saturating_sub(mem_info.1),
        load_1m: load_avg.0,
        load_5m: load_avg.1,
        load_15m: load_avg.2,
        cpu_percent,
    };

    match serde_json::to_vec(&info) {
        Ok(body) => Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(Body::from(body))
            .unwrap(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn read_mem_info() -> (u64, u64) {
    let content = match std::fs::read_to_string("/proc/meminfo") {
        Ok(c) => c,
        Err(_) => return (0, 0),
    };
    let mut total = 0u64;
    let mut available = 0u64;
    for line in content.lines() {
        if line.starts_with("MemTotal:") {
            total = parse_mem_value(line);
        } else if line.starts_with("MemAvailable:") {
            available = parse_mem_value(line);
        }
    }
    (total, available)
}

fn parse_mem_value(line: &str) -> u64 {
    line.split_whitespace()
        .nth(1)
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
}

fn read_load_avg() -> (f64, f64, f64) {
    let content = match std::fs::read_to_string("/proc/loadavg") {
        Ok(c) => c,
        Err(_) => return (0.0, 0.0, 0.0),
    };
    let parts: Vec<&str> = content.split_whitespace().collect();
    let load1 = parts.first().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let load5 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let load15 = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0.0);
    (load1, load5, load15)
}

async fn api_devices<I: InputSink + Clone + Send + 'static>(
    State(state): State<Arc<ApiState<I>>>,
) -> Response {
    let video = match state.device_provider.list_video_devices() {
        Ok(devices) => devices
            .into_iter()
            .map(video_device_dto)
            .collect::<Vec<_>>(),
        Err(error) => {
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "video enumeration failed",
                Some(&error.to_string()),
            );
        }
    };
    let serial = match state.device_provider.list_serial_devices() {
        Ok(devices) => devices
            .into_iter()
            .map(serial_device_dto)
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

fn video_device_dto(device: VideoDevice) -> DeviceDto {
    DeviceDto {
        id: device.id,
        display_name: device.display_name,
        kind: "video",
    }
}

fn serial_device_dto(device: SerialDevice) -> DeviceDto {
    DeviceDto {
        id: device.path,
        display_name: device.display_name,
        kind: "serial",
    }
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
    #[serde(default)]
    mouse_profile: Option<String>,
    #[serde(default)]
    mouse_mode: Option<String>,
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
    let mouse_profile =
        match normalize_session_mouse_profile(request.mouse_profile, request.mouse_mode) {
            Ok(profile) => profile,
            Err(detail) => {
                return json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid mouse profile",
                    Some(&detail),
                );
            }
        };
    let selection = SessionSelection {
        video: request.video,
        serial: request.serial,
        mouse_profile,
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
        "stop" => {
            // 用户手动断开：立即停止 session
            state.manual_stop.store(true, Ordering::Relaxed);
            match manager.stop() {
                Ok(()) => {
                    manager.wait_stopped().await;
                    let _ = manager.stop_and_destroy().await;
                    state
                        .frame_source
                        .set_current(Arc::new(EmptyFrameSource::new()));
                    json_session_state(&manager)
                }
                Err(error) => session_error(error),
            }
        }
        other => json_error(
            StatusCode::BAD_REQUEST,
            "unknown action",
            Some(&format!(
                "action 必须是 restart/create/stop，得到 {other:?}"
            )),
        ),
    }
}

fn normalize_session_mouse_profile(
    profile: Option<String>,
    legacy_mode: Option<String>,
) -> Result<Option<String>, String> {
    if let Some(profile) = profile {
        let parsed = MouseProfile::parse(&profile).map_err(|error| error.to_string())?;
        return Ok(Some(parsed.as_str().to_string()));
    }
    let Some(mode) = legacy_mode else {
        return Ok(None);
    };
    let profile = match mode.as_str() {
        "absolute" | "Absolute" => MouseProfile::RawAbsolute,
        "relative" | "Relative" => MouseProfile::RawRelative,
        other => return Err(format!("unknown mouse_mode: {other}")),
    };
    Ok(Some(profile.as_str().to_string()))
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
        PixelFormat::Rgb888 => "rgb888",
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

    #[test]
    fn session_selection_override_keeps_mouse_profile_with_devices() {
        let base = SessionSelection {
            video: Some("cam0".into()),
            serial: Some("COM9".into()),
            mouse_profile: Some("windows".into()),
        };
        let override_selection = SessionSelection {
            video: None,
            serial: None,
            mouse_profile: Some("linux".into()),
        };
        assert_eq!(
            base.with_overrides(&override_selection),
            SessionSelection {
                video: Some("cam0".into()),
                serial: Some("COM9".into()),
                mouse_profile: Some("linux".into()),
            }
        );
    }

    #[test]
    fn mouse_mode_name_matches_wire_values() {
        assert_eq!(mouse_mode_name(MouseMode::Absolute), "absolute");
        assert_eq!(mouse_mode_name(MouseMode::Relative), "relative");
    }

    #[tokio::test]
    async fn wait_for_mouse_mode_requires_confirmed_value() {
        let (tx, mut rx) = watch::channel(None);
        tokio::spawn(async move {
            tx.send(Some(MouseMode::Relative)).unwrap();
        });
        assert!(wait_for_mouse_mode(&mut rx, MouseMode::Relative).await);
    }

    #[test]
    fn session_mouse_mode_compatibility_maps_to_raw_profiles() {
        assert_eq!(
            normalize_session_mouse_profile(None, Some("relative".into())).unwrap(),
            Some("raw_relative".into())
        );
        assert_eq!(
            normalize_session_mouse_profile(Some("linux".into()), Some("absolute".into())).unwrap(),
            Some("linux".into())
        );
        assert!(normalize_session_mouse_profile(None, Some("banana".into())).is_err());
    }
}
