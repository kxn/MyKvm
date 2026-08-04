use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use futures_util::StreamExt;
use ipkvm_core::{InputResult, InputSink, KeyEvent, MouseMode, PointerEvent};
use ipkvm_headless::{
    frame_source::SwitchableFrameSource,
    rfb_connection::RfbConnectionGate,
    rfb_ws::RfbWebSocketConfig,
    session_manager::SessionManager,
    settings::SettingsStore,
    web::{HeadlessWebService, HeadlessWebServiceError, SessionFactory, SessionSelection},
};
use ipkvm_video::{
    FrameReceiver, FrameSource, MonotonicTimestamp, PixelFormat, SharedVideoFrame, VideoFrame,
    VideoSourceInfo, VideoSourceKind, mock::MockFrameSource,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::JoinHandle,
    time::timeout,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Error as WebSocketError, Message},
};

/// 记录型输入 sink：共享 `Arc<Mutex<Recorded>>`，供测试观察泵写入。
/// 与 session crate 内部测试 sink 同构，但本文件私有（不依赖 crate 内部）。
#[derive(Clone, Debug, Default)]
struct RecordingSink {
    recorded: Arc<std::sync::Mutex<Recorded>>,
}

#[derive(Clone, Debug, Default)]
struct Recorded {
    key_batches: usize,
    release_count: usize,
}

impl InputSink for RecordingSink {
    fn set_mouse_mode(&mut self, _mode: MouseMode) -> InputResult<()> {
        Ok(())
    }

    fn handle_key_batch(&mut self, _events: &[KeyEvent]) -> InputResult<()> {
        self.recorded.lock().unwrap().key_batches += 1;
        Ok(())
    }

    fn handle_pointer_batch(&mut self, _events: &[PointerEvent]) -> InputResult<()> {
        Ok(())
    }

    fn release_all(&mut self) -> InputResult<()> {
        self.recorded.lock().unwrap().release_count += 1;
        Ok(())
    }
}

/// 带可辨识名称的测试帧源：用来断言 `/api/session` 切换后 status/screenshot
/// 读取的是新帧源，而不是启动时固定的旧帧源。
#[derive(Debug)]
struct NamedFrameSource {
    inner: MockFrameSource,
    name: String,
}

impl NamedFrameSource {
    fn new(name: &str, frame: Option<SharedVideoFrame>) -> Self {
        let inner = MockFrameSource::new();
        if let Some(frame) = frame {
            inner.publish_frame(frame);
        }
        Self {
            inner,
            name: name.to_string(),
        }
    }

    fn publish_frame(&self, frame: SharedVideoFrame) {
        self.inner.publish_frame(frame);
    }
}

impl FrameSource for NamedFrameSource {
    fn latest_frame(&self) -> Option<SharedVideoFrame> {
        self.inner.latest_frame()
    }

    fn subscribe(&self) -> FrameReceiver {
        self.inner.subscribe()
    }

    fn source_info(&self) -> VideoSourceInfo {
        VideoSourceInfo {
            kind: VideoSourceKind::Generated,
            device_name: self.name.clone(),
            is_loop: false,
        }
    }
}

#[derive(Debug)]
struct DropAwareFrameSource {
    inner: NamedFrameSource,
    dropped: Arc<AtomicBool>,
}

impl DropAwareFrameSource {
    fn new(name: &str, frame: Option<SharedVideoFrame>, dropped: Arc<AtomicBool>) -> Self {
        Self {
            inner: NamedFrameSource::new(name, frame),
            dropped,
        }
    }
}

impl Drop for DropAwareFrameSource {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl FrameSource for DropAwareFrameSource {
    fn latest_frame(&self) -> Option<SharedVideoFrame> {
        self.inner.latest_frame()
    }

    fn subscribe(&self) -> FrameReceiver {
        self.inner.subscribe()
    }

    fn source_info(&self) -> VideoSourceInfo {
        self.inner.source_info()
    }
}

/// 测试会话工厂：默认构造 replacement 源，模拟用户通过 video 字段选中新设备。
struct TestSessionFactory {
    replacement: Arc<NamedFrameSource>,
}

impl SessionFactory<RecordingSink> for TestSessionFactory {
    fn build(
        &self,
        selection: &SessionSelection,
    ) -> Result<(Arc<dyn FrameSource>, RecordingSink), String> {
        if selection.video.as_deref() == Some("__factory_fail__") {
            return Err("test factory failure".to_string());
        }
        Ok((
            Arc::clone(&self.replacement) as Arc<dyn FrameSource>,
            RecordingSink::default(),
        ))
    }
}

/// 独占资源测试工厂：只有旧帧源已释放后才允许构建新会话。
struct ReleaseCheckingFactory {
    old_released: Arc<AtomicBool>,
    replacement: Arc<NamedFrameSource>,
}

impl SessionFactory<RecordingSink> for ReleaseCheckingFactory {
    fn build(
        &self,
        _selection: &SessionSelection,
    ) -> Result<(Arc<dyn FrameSource>, RecordingSink), String> {
        if !self.old_released.load(Ordering::SeqCst) {
            return Err("old resource is still held".to_string());
        }
        Ok((
            Arc::clone(&self.replacement) as Arc<dyn FrameSource>,
            RecordingSink::default(),
        ))
    }
}

struct HttpResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

/// 测试专用临时设置目录：进程内自增避免并行测试互踩，Drop 时清理。
struct TempSettingsDir {
    path: PathBuf,
}

impl TempSettingsDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ipkvm-headless-web-http-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn store(&self) -> Arc<SettingsStore> {
        Arc::new(SettingsStore::load_from(self.path.clone()).0)
    }
}

impl Drop for TempSettingsDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

struct ReleaseOrderWebServer {
    address: SocketAddr,
    manager: Arc<tokio::sync::Mutex<SessionManager<RecordingSink>>>,
    shutdown: watch::Sender<bool>,
    task: JoinHandle<Result<(), HeadlessWebServiceError>>,
    _settings: Arc<SettingsStore>,
    _manual_stop: Arc<AtomicBool>,
    _settings_dir: TempSettingsDir,
}

impl ReleaseOrderWebServer {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let old_released = Arc::new(AtomicBool::new(false));
        let source: Arc<dyn FrameSource> = Arc::new(DropAwareFrameSource::new(
            "exclusive",
            Some(test_frame()),
            Arc::clone(&old_released),
        ));
        let gate = RfbConnectionGate::new();
        let mut manager =
            SessionManager::new(Arc::clone(&source), RecordingSink::default(), gate.clone());
        manager.start().unwrap();
        let event_publisher = manager.event_publisher();
        let manager = Arc::new(tokio::sync::Mutex::new(manager));
        let switchable = Arc::new(SwitchableFrameSource::new(source));
        let factory = Arc::new(ReleaseCheckingFactory {
            old_released,
            replacement: Arc::new(NamedFrameSource::new("wide", Some(wide_frame()))),
        });
        let settings_dir = TempSettingsDir::new();
        let settings = settings_dir.store();
        let manual_stop = Arc::new(AtomicBool::new(false));
        let (shutdown, shutdown_rx) = watch::channel(false);
        let service = HeadlessWebService::<RecordingSink>::new(
            switchable,
            Arc::clone(&manager),
            factory,
            event_publisher,
            RfbWebSocketConfig::default(),
            shutdown_rx,
            gate,
            None,
            Arc::clone(&settings),
            Arc::clone(&manual_stop),
            Some(SessionSelection::default()),
        )
        .unwrap();
        let task = tokio::spawn(service.serve(listener));
        Self {
            address,
            manager,
            shutdown,
            task,
            _settings: settings,
            _manual_stop: manual_stop,
            _settings_dir: settings_dir,
        }
    }

    async fn request_with_body(&self, method: &str, path: &str, body: &[u8]) -> HttpResponse {
        request_with_headers_and_body(
            self.address,
            method,
            path,
            &[("Content-Type", "application/json")],
            body,
        )
        .await
    }

    async fn stop(self) {
        {
            let mut manager = self.manager.lock().await;
            if manager.stop().is_ok() {
                manager.wait_stopped().await;
            }
        }
        self.shutdown.send_replace(true);
        timeout(Duration::from_secs(2), self.task)
            .await
            .expect("headless web service did not stop")
            .expect("headless web service task panicked")
            .expect("headless web service failed");
    }
}

struct TestWebServer {
    address: SocketAddr,
    source: Arc<NamedFrameSource>,
    manager: Arc<tokio::sync::Mutex<SessionManager<RecordingSink>>>,
    shutdown: watch::Sender<bool>,
    task: JoinHandle<Result<(), HeadlessWebServiceError>>,
    settings: Arc<SettingsStore>,
    manual_stop: Arc<AtomicBool>,
    _settings_dir: TempSettingsDir,
}

impl TestWebServer {
    async fn start() -> Self {
        Self::start_with_frame(Some(test_frame()), None).await
    }

    /// 以零初始会话启动（manager empty）：供 `POST /api/session {create}` 测试。
    async fn start_empty() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let source = Arc::new(NamedFrameSource::new("initial", Some(test_frame())));
        let gate = RfbConnectionGate::new();
        // empty：不构造会话；event_publisher 初始为 None，传输层拒绝连接。
        let manager = Arc::new(tokio::sync::Mutex::new(
            SessionManager::<RecordingSink>::empty(),
        ));
        let factory = Arc::new(TestSessionFactory {
            replacement: Arc::new(NamedFrameSource::new("wide", Some(wide_frame()))),
        });
        let settings_dir = TempSettingsDir::new();
        let settings = settings_dir.store();
        let manual_stop = Arc::new(AtomicBool::new(false));
        let (shutdown, shutdown_rx) = watch::channel(false);
        let switchable = Arc::new(SwitchableFrameSource::new(
            Arc::clone(&source) as Arc<dyn FrameSource>
        ));
        let service = HeadlessWebService::<RecordingSink>::new(
            switchable,
            Arc::clone(&manager),
            factory,
            manager.lock().await.event_publisher(),
            RfbWebSocketConfig::default(),
            shutdown_rx,
            gate,
            None,
            Arc::clone(&settings),
            Arc::clone(&manual_stop),
            Some(SessionSelection::default()),
        )
        .unwrap();
        let task = tokio::spawn(service.serve(listener));
        Self {
            address,
            source,
            manager,
            shutdown,
            task,
            settings,
            manual_stop,
            _settings_dir: settings_dir,
        }
    }

    async fn start_with_frame(frame: Option<SharedVideoFrame>, auth: Option<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let source = Arc::new(NamedFrameSource::new("initial", frame));
        let sink = RecordingSink::default();
        let gate = RfbConnectionGate::new();
        // 启动即绑定：构造并 start 会话，传输层立即接受连接。
        let mut manager = SessionManager::new(
            Arc::clone(&source) as Arc<dyn FrameSource>,
            sink.clone(),
            gate.clone(),
        );
        manager.start().unwrap();
        let event_publisher = manager.event_publisher();
        let manager = Arc::new(tokio::sync::Mutex::new(manager));
        let factory = Arc::new(TestSessionFactory {
            replacement: Arc::new(NamedFrameSource::new("wide", Some(wide_frame()))),
        });
        let settings_dir = TempSettingsDir::new();
        let settings = settings_dir.store();
        let manual_stop = Arc::new(AtomicBool::new(false));
        let (shutdown, shutdown_rx) = watch::channel(false);
        let switchable = Arc::new(SwitchableFrameSource::new(
            Arc::clone(&source) as Arc<dyn FrameSource>
        ));
        let service = HeadlessWebService::<RecordingSink>::new(
            switchable,
            Arc::clone(&manager),
            factory,
            event_publisher,
            RfbWebSocketConfig::default(),
            shutdown_rx,
            gate,
            auth, // HTTP 鉴权 token；None 表示仅放行本机来源
            Arc::clone(&settings),
            Arc::clone(&manual_stop),
            Some(SessionSelection::default()),
        )
        .unwrap();
        let task = tokio::spawn(service.serve(listener));
        Self {
            address,
            source,
            manager,
            shutdown,
            task,
            settings,
            manual_stop,
            _settings_dir: settings_dir,
        }
    }

    fn websocket_url(&self) -> String {
        format!("ws://{}/rfb", self.address)
    }

    async fn request(&self, method: &str, path: &str) -> HttpResponse {
        self.request_with_body(method, path, &[]).await
    }

    async fn request_with_body(&self, method: &str, path: &str, body: &[u8]) -> HttpResponse {
        // 测试用 body 均为 JSON：带 content-type 让 axum Json extractor 放行。
        self.request_with_headers_and_body(
            method,
            path,
            &[("Content-Type", "application/json")],
            body,
        )
        .await
    }

    async fn request_with_headers(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
    ) -> HttpResponse {
        self.request_with_headers_and_body(method, path, headers, &[])
            .await
    }

    async fn request_with_headers_and_body(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> HttpResponse {
        request_with_headers_and_body(self.address, method, path, headers, body).await
    }

    async fn stop(self) {
        // 先停会话（释放泵），再触发 web 服务优雅关闭。
        {
            let mut manager = self.manager.lock().await;
            if manager.stop().is_ok() {
                manager.wait_stopped().await;
            }
        }
        self.shutdown.send_replace(true);
        timeout(Duration::from_secs(2), self.task)
            .await
            .expect("headless web service did not stop")
            .expect("headless web service task panicked")
            .expect("headless web service failed");
    }
}

async fn request_with_headers_and_body(
    address: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> HttpResponse {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    if !body.is_empty() {
        stream.write_all(body).await.unwrap();
    }
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    parse_http_response(&response)
}

#[tokio::test]
async fn serves_fixed_novnc_modules_with_explicit_headers() {
    let server = TestWebServer::start().await;

    let response = server.request("GET", "/vendor/novnc/core/rfb.js").await;

    assert_eq!(response.status, 200);
    assert_eq!(
        response.headers.get("content-type").map(String::as_str),
        Some("text/javascript; charset=utf-8")
    );
    assert_eq!(
        response.headers.get("cache-control").map(String::as_str),
        Some("no-cache")
    );
    assert_eq!(
        response
            .headers
            .get("x-content-type-options")
            .map(String::as_str),
        Some("nosniff")
    );
    assert!(
        std::str::from_utf8(&response.body)
            .unwrap()
            .contains("export default class RFB")
    );

    server.stop().await;
}

#[tokio::test]
async fn serves_web_modules_with_explicit_headers() {
    let server = TestWebServer::start().await;

    let response = server
        .request("GET", "/assets/modules/special-keys.js")
        .await;

    assert_eq!(response.status, 200);
    assert_eq!(
        response.headers.get("content-type").map(String::as_str),
        Some("text/javascript; charset=utf-8")
    );
    assert_eq!(
        response.headers.get("cache-control").map(String::as_str),
        Some("no-cache")
    );
    assert_eq!(
        response
            .headers
            .get("x-content-type-options")
            .map(String::as_str),
        Some("nosniff")
    );
    assert!(
        std::str::from_utf8(&response.body)
            .unwrap()
            .contains("special keys")
    );

    server.stop().await;
}

#[tokio::test]
async fn serves_the_console_and_chinese_license_page() {
    let server = TestWebServer::start().await;

    let console = server.request("GET", "/").await;
    assert_eq!(console.status, 200);
    assert_eq!(
        console.headers.get("content-type").map(String::as_str),
        Some("text/html; charset=utf-8")
    );
    assert!(
        std::str::from_utf8(&console.body)
            .unwrap()
            .contains("my_ipkvm 控制台")
    );

    let licenses = server.request("GET", "/licenses/").await;
    assert_eq!(licenses.status, 200);
    assert!(
        std::str::from_utf8(&licenses.body)
            .unwrap()
            .contains("第三方组件与许可证")
    );

    server.stop().await;
}

#[tokio::test]
async fn missing_assets_and_wrong_methods_are_not_html_fallbacks() {
    let server = TestWebServer::start().await;

    let missing = server.request("GET", "/vendor/novnc/core/missing.js").await;
    assert_eq!(missing.status, 404);
    assert!(missing.body.is_empty());

    let wrong_method = server.request("POST", "/vendor/novnc/core/rfb.js").await;
    assert_eq!(wrong_method.status, 405);
    assert!(wrong_method.body.is_empty());

    server.stop().await;
}

#[tokio::test]
async fn official_serve_entry_injects_connect_info_for_rfb() {
    let server = TestWebServer::start().await;

    let (mut socket, response) = connect_async(server.websocket_url()).await.unwrap();

    assert_eq!(response.status().as_u16(), 101);
    match socket.next().await.unwrap().unwrap() {
        Message::Binary(bytes) => assert_eq!(bytes.as_ref(), b"RFB 003.008\n"),
        message => panic!("expected RFB banner, got {message:?}"),
    }
    socket.close(None).await.unwrap();
    server.stop().await;
}

#[tokio::test]
async fn static_routes_preserve_the_single_active_rfb_gate() {
    let server = TestWebServer::start().await;
    let (mut first, _) = connect_async(server.websocket_url()).await.unwrap();
    match first.next().await.unwrap().unwrap() {
        Message::Binary(bytes) => assert_eq!(bytes.as_ref(), b"RFB 003.008\n"),
        message => panic!("expected RFB banner, got {message:?}"),
    }

    let response = match connect_async(server.websocket_url()).await.unwrap_err() {
        WebSocketError::Http(response) => response,
        error => panic!("expected HTTP conflict, got {error:?}"),
    };
    assert_eq!(response.status().as_u16(), 409);

    first.close(None).await.unwrap();
    server.stop().await;
}

#[tokio::test]
async fn api_status_reports_video_and_controller() {
    let server = TestWebServer::start().await;

    let response = server.request("GET", "/api/status").await;

    assert_eq!(response.status, 200);
    assert!(
        response
            .headers
            .get("content-type")
            .is_some_and(|value| value.starts_with("application/json"))
    );
    let status: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(status["service"]["name"], "ipkvm-headless");
    assert_eq!(status["service"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(status["video"]["source"]["kind"], "generated");
    assert_eq!(status["video"]["source"]["device_name"], "initial");
    assert_eq!(status["video"]["source"]["is_loop"], false);
    assert_eq!(status["video"]["frame"]["width"], 2);
    assert_eq!(status["video"]["frame"]["height"], 1);
    assert_eq!(status["video"]["frame"]["pixel_format"], "bgra8888");
    assert_eq!(status["video"]["frame"]["seq"], 1);
    assert_eq!(status["video"]["stalled"], false);
    assert!(status["video"]["frame"]["last_frame_ns"].is_number());
    assert_eq!(status["controller"]["active"], false);
    assert!(status["controller"]["client_id"].is_null());
    assert!(status["controller"]["transport"].is_null());
    assert!(status["controller"]["peer_addr"].is_null());
    assert!(status["controller"]["connected_since_ms"].is_null());
    // 会话状态：启动即绑定 → running；无输入时计数字段为 0，last_input_ns 因
    // skip_serializing_if 不出现（None 时不序列化）。
    assert_eq!(status["session"]["state"], "running");
    assert_eq!(status["session"]["input_events"], 0);
    assert_eq!(status["session"]["dropped_frames"], 0);
    assert!(
        status["session"].get("last_input_ns").is_none(),
        "无输入时 last_input_ns 不应序列化"
    );
    assert!(
        status["session"].get("input_offline").is_none(),
        "会话正常时 input_offline 不应序列化"
    );
    assert_eq!(
        status["session"]["manual_stop"], false,
        "默认未手动停止时 manual_stop 必须恒序列化为 false"
    );

    server.stop().await;
}

#[tokio::test]
async fn api_screenshot_returns_red_jpeg_when_bgra_frame_available() {
    let server = TestWebServer::start().await;
    // 纯红 BGRA 像素 [B,G,R,A] = [0,0,255,255]：B/R 顺序错换会解码成蓝色
    server.source.publish_frame(red_frame());

    let response = server.request("GET", "/api/screenshot").await;

    assert_eq!(response.status, 200);
    assert_eq!(
        response.headers.get("content-type").map(String::as_str),
        Some("image/jpeg")
    );
    assert_eq!(
        response.headers.get("cache-control").map(String::as_str),
        Some("no-store")
    );
    assert_eq!(&response.body[..2], &[0xFF, 0xD8]);

    // 解码 JPEG 验证 B/R 通道：纯红帧必须解码为红色而非蓝色
    let mut decoder = zune_jpeg::JpegDecoder::new(std::io::Cursor::new(&response.body));
    let pixels = decoder.decode().unwrap();
    let info = decoder.info().unwrap();
    assert_eq!((info.width, info.height), (RED_FRAME_SIZE, RED_FRAME_SIZE));
    assert!(
        pixels[0] > 100,
        "red channel of decoded pixel, got {}",
        pixels[0]
    );
    assert!(
        pixels[2] < 100,
        "blue channel of decoded pixel, got {}",
        pixels[2]
    );
    assert!(pixels[0] > pixels[2]);

    server.stop().await;
}

#[tokio::test]
async fn api_screenshot_503_when_no_frame() {
    let server = TestWebServer::start_with_frame(None, None).await;

    let response = server.request("GET", "/api/screenshot").await;

    assert_eq!(response.status, 503);
    assert!(response.body.is_empty());

    server.stop().await;
}

// ---- GET /api/devices ----

#[tokio::test]
async fn api_devices_returns_lists_with_video_and_serial_kinds() {
    let server = TestWebServer::start().await;

    let response = server.request("GET", "/api/devices").await;
    assert_eq!(response.status, 200);
    assert!(
        response
            .headers
            .get("content-type")
            .is_some_and(|value| value.starts_with("application/json"))
    );
    let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    // mock/无硬件环境下枚举可能返回空列表或失败；成功时结构必须正确。
    assert!(body["video"].is_array(), "video 字段必须是数组");
    assert!(body["serial"].is_array(), "serial 字段必须是数组");
    // 若枚举到视频设备，每项必须有 id/display_name/kind 字段。
    if let Some(device) = body["video"].as_array().unwrap().first() {
        assert!(device["kind"] == "video");
        assert!(device["display_name"].is_string());
    }

    server.stop().await;
}

// ---- POST /api/session ----

#[tokio::test]
async fn api_session_restart_returns_running_state() {
    let server = TestWebServer::start().await;

    let response = server
        .request_with_body("POST", "/api/session", br#"{"action":"restart"}"#)
        .await;
    assert_eq!(response.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["state"], "running");

    server.stop().await;
}

#[tokio::test]
async fn api_session_restart_starts_from_empty_manager() {
    let server = TestWebServer::start_empty().await;

    // 零初始会话：restart 可按当前默认选择直接首启。
    let restart = server
        .request_with_body("POST", "/api/session", br#"{"action":"restart"}"#)
        .await;
    assert_eq!(restart.status, 200);
    let restart_body: serde_json::Value = serde_json::from_slice(&restart.body).unwrap();
    assert_eq!(restart_body["state"], "running");

    server.stop().await;
}

#[tokio::test]
async fn api_session_create_starts_from_empty_manager() {
    let server = TestWebServer::start_empty().await;

    let create = server
        .request_with_body("POST", "/api/session", br#"{"action":"create"}"#)
        .await;
    assert_eq!(create.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&create.body).unwrap();
    assert_eq!(body["state"], "running");

    // create 后 status 反映 running。
    let status = server.request("GET", "/api/status").await;
    let status_body: serde_json::Value = serde_json::from_slice(&status.body).unwrap();
    assert_eq!(status_body["session"]["state"], "running");

    // 重复 create 报告冲突；即使请求携带会导致工厂失败的设备名，也必须先
    // 按“会话已存在”处理，避免把幂等/冲突语义泄露成设备打开 500。
    let again = server
        .request_with_body(
            "POST",
            "/api/session",
            br#"{"action":"create","video":"__factory_fail__"}"#,
        )
        .await;
    assert_eq!(again.status, 409);

    server.stop().await;
}

#[tokio::test]
async fn api_session_stop_then_restart_cycles_state() {
    let server = TestWebServer::start().await;

    let stop = server
        .request_with_body("POST", "/api/session", br#"{"action":"stop"}"#)
        .await;
    assert_eq!(stop.status, 200);
    let stop_body: serde_json::Value = serde_json::from_slice(&stop.body).unwrap();
    assert_eq!(stop_body["state"], "absent");

    // 断开必须真正停止并释放采集：会话销毁、帧源切空、manual_stop 置位。
    let status_after: serde_json::Value =
        serde_json::from_slice(&server.request("GET", "/api/status").await.body).unwrap();
    assert_eq!(status_after["session"]["state"], "absent");
    assert_eq!(status_after["session"]["manual_stop"], true);
    assert_eq!(status_after["video"]["source"]["kind"], "none");
    assert_eq!(status_after["video"]["source"]["device_name"], "none");

    // 停止后再 restart：按当前选择重新启动会话。
    let restart = server
        .request_with_body("POST", "/api/session", br#"{"action":"restart"}"#)
        .await;
    assert_eq!(restart.status, 200);
    let restart_body: serde_json::Value = serde_json::from_slice(&restart.body).unwrap();
    assert_eq!(restart_body["state"], "running");
    let status_restarted: serde_json::Value =
        serde_json::from_slice(&server.request("GET", "/api/status").await.body).unwrap();
    assert_eq!(status_restarted["session"]["manual_stop"], false);

    server.stop().await;
}

#[tokio::test]
async fn api_session_restart_with_video_switches_current_frame_source() {
    let server = TestWebServer::start().await;

    let response = server
        .request_with_body(
            "POST",
            "/api/session",
            br#"{"action":"restart","video":"wide"}"#,
        )
        .await;
    assert_eq!(response.status, 200);
    let restart: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(restart["state"], "running");

    let status = server.request("GET", "/api/status").await;
    assert_eq!(status.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&status.body).unwrap();
    assert_eq!(body["video"]["source"]["device_name"], "wide");
    assert_eq!(body["video"]["frame"]["width"], 4);
    assert_eq!(body["video"]["frame"]["height"], 1);
    assert_eq!(body["video"]["frame"]["seq"], 9);

    server.stop().await;
}

#[tokio::test]
async fn api_session_restart_releases_old_resources_before_factory_build() {
    let server = ReleaseOrderWebServer::start().await;

    let response = server
        .request_with_body("POST", "/api/session", br#"{"action":"restart"}"#)
        .await;
    assert_eq!(
        response.status,
        200,
        "restart should build after releasing the old resource: {}",
        String::from_utf8_lossy(&response.body)
    );

    server.stop().await;
}

#[tokio::test]
async fn api_session_rejects_unknown_action() {
    let server = TestWebServer::start().await;

    let response = server
        .request_with_body("POST", "/api/session", br#"{"action":"nuke"}"#)
        .await;
    assert_eq!(response.status, 400);
    let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["error"], "unknown action");

    server.stop().await;
}

// ---- GET/POST /api/settings ----

#[tokio::test]
async fn api_settings_returns_defaults_matching_contract() {
    let server = TestWebServer::start().await;

    let response = server.request("GET", "/api/settings").await;
    assert_eq!(response.status, 200);
    let settings: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(settings["baud_rate"], 9600);
    assert_eq!(settings["auto_baud"], true);
    assert_eq!(settings["preview_fps"], 30);
    assert_eq!(settings["mouse_mode"], "absolute");
    assert_eq!(settings["relative_sensitivity"], 1.0);
    assert_eq!(settings["scale_mode"], "fit_window");

    server.stop().await;
}

#[tokio::test]
async fn api_settings_post_roundtrips_and_persists_to_file() {
    let server = TestWebServer::start().await;

    let body = br#"{"baud_rate":57600,"auto_baud":false,"preview_fps":15,
        "mouse_mode":"relative","relative_sensitivity":2.5,"scale_mode":"resize_to_video"}"#;
    let response = server
        .request_with_body("POST", "/api/settings", body)
        .await;
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let saved: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(saved["baud_rate"], 57600);
    assert_eq!(saved["auto_baud"], false);
    assert_eq!(saved["preview_fps"], 15);
    assert_eq!(saved["mouse_mode"], "relative");
    assert_eq!(saved["relative_sensitivity"], 2.5);
    assert_eq!(saved["scale_mode"], "resize_to_video");

    let response = server.request("GET", "/api/settings").await;
    let loaded: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(loaded, saved);

    // 持久化落盘：从设置文件所在目录重新加载存储可见相同值。
    let dir = server.settings.path().parent().unwrap().to_path_buf();
    let (reloaded, warning) = SettingsStore::load_from(dir);
    assert!(warning.is_none(), "重新加载不应有警告：{warning:?}");
    assert_eq!(reloaded.get().baud_rate, 57_600);
    assert_eq!(reloaded.get().preview_fps, 15);
    assert_eq!(reloaded.get().mouse_mode, ipkvm_core::MouseMode::Relative);

    server.stop().await;
}

#[tokio::test]
async fn api_settings_rejects_invalid_values_with_400() {
    let server = TestWebServer::start().await;

    let cases: [(&[u8], &str); 5] = [
        (
            br#"{"baud_rate":960,"auto_baud":true,"preview_fps":30,"mouse_mode":"absolute","relative_sensitivity":1.0,"scale_mode":"fit_window"}"#.as_slice(),
            "baud_rate",
        ),
        (
            br#"{"baud_rate":115200,"auto_baud":true,"preview_fps":0,"mouse_mode":"absolute","relative_sensitivity":1.0,"scale_mode":"fit_window"}"#.as_slice(),
            "preview_fps",
        ),
        (
            br#"{"baud_rate":115200,"auto_baud":true,"preview_fps":30,"mouse_mode":"absolute","relative_sensitivity":6.0,"scale_mode":"fit_window"}"#.as_slice(),
            "relative_sensitivity",
        ),
        (
            br#"{"baud_rate":115200,"auto_baud":true,"preview_fps":30,"mouse_mode":"banana","relative_sensitivity":1.0,"scale_mode":"fit_window"}"#.as_slice(),
            "mouse_mode",
        ),
        (
            br#"{"baud_rate":115200,"auto_baud":true,"preview_fps":30,"mouse_mode":"absolute","relative_sensitivity":1.0,"scale_mode":"bogus"}"#.as_slice(),
            "scale_mode",
        ),
    ];
    for (body, detail_contains) in cases {
        let response = server
            .request_with_body("POST", "/api/settings", body)
            .await;
        assert_eq!(
            response.status,
            400,
            "非法设置应 400：{}",
            String::from_utf8_lossy(body)
        );
        let error: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(error["error"], "invalid settings");
        let detail = error["detail"].as_str().unwrap();
        assert!(
            detail.contains(detail_contains),
            "detail 应指明字段 {detail_contains}：{detail}"
        );
    }

    server.stop().await;
}

// ---- 手动停止标记 ----

#[tokio::test]
async fn api_status_exposes_manual_stop_and_stop_restart_cycle_flips_it() {
    let server = TestWebServer::start().await;

    let status = server.request("GET", "/api/status").await;
    let body: serde_json::Value = serde_json::from_slice(&status.body).unwrap();
    assert_eq!(body["session"]["manual_stop"], false);

    let stop = server
        .request_with_body("POST", "/api/session", br#"{"action":"stop"}"#)
        .await;
    assert_eq!(stop.status, 200);

    let status = server.request("GET", "/api/status").await;
    let body: serde_json::Value = serde_json::from_slice(&status.body).unwrap();
    assert_eq!(body["session"]["state"], "absent");
    assert_eq!(body["session"]["manual_stop"], true);

    let restart = server
        .request_with_body("POST", "/api/session", br#"{"action":"restart"}"#)
        .await;
    assert_eq!(restart.status, 200);

    let status = server.request("GET", "/api/status").await;
    let body: serde_json::Value = serde_json::from_slice(&status.body).unwrap();
    assert_eq!(body["session"]["state"], "running");
    assert_eq!(body["session"]["manual_stop"], false);

    server.stop().await;
}

#[tokio::test]
async fn api_session_create_clears_manual_stop() {
    let server = TestWebServer::start_empty().await;
    // 预置手动停止标记（模拟先前 stop 后尚未 create 的状态）。
    server.manual_stop.store(true, Ordering::SeqCst);

    let create = server
        .request_with_body("POST", "/api/session", br#"{"action":"create"}"#)
        .await;
    assert_eq!(create.status, 200);

    let status = server.request("GET", "/api/status").await;
    let body: serde_json::Value = serde_json::from_slice(&status.body).unwrap();
    assert_eq!(body["session"]["state"], "running");
    assert_eq!(body["session"]["manual_stop"], false);

    server.stop().await;
}

#[tokio::test]
async fn api_devices_and_session_require_token_when_configured() {
    let server =
        TestWebServer::start_with_frame(Some(test_frame()), Some("secret".to_string())).await;

    // 新增管理路由同样受全局鉴权层保护。
    let devices = server.request("GET", "/api/devices").await;
    assert_eq!(devices.status, 401);

    let session = server
        .request_with_body("POST", "/api/session", br#"{"action":"restart"}"#)
        .await;
    assert_eq!(session.status, 401);

    // 带 token 放行。
    let devices_ok = server
        .request_with_headers_and_body(
            "GET",
            "/api/devices",
            &[("Authorization", "Bearer secret")],
            &[],
        )
        .await;
    assert_eq!(devices_ok.status, 200);

    server.stop().await;
}

#[tokio::test]
async fn configured_token_requires_credentials_everywhere() {
    let server =
        TestWebServer::start_with_frame(Some(test_frame()), Some("secret".to_string())).await;

    let anonymous = server.request("GET", "/api/status").await;
    assert_eq!(anonymous.status, 401);
    assert_eq!(
        anonymous
            .headers
            .get("www-authenticate")
            .map(String::as_str),
        Some("Bearer")
    );

    let wrong = server
        .request_with_headers("GET", "/api/status", &[("Authorization", "Bearer wrong")])
        .await;
    assert_eq!(wrong.status, 401);

    let correct = server
        .request_with_headers("GET", "/api/status", &[("Authorization", "Bearer secret")])
        .await;
    assert_eq!(correct.status, 200);

    server.stop().await;
}

#[tokio::test]
async fn configured_token_accepts_cookie_and_query_and_sets_cookie() {
    let server =
        TestWebServer::start_with_frame(Some(test_frame()), Some("secret".to_string())).await;

    let cookie = server
        .request_with_headers("GET", "/api/status", &[("Cookie", "ipkvm_token=secret")])
        .await;
    assert_eq!(cookie.status, 200);

    let query = server.request("GET", "/?token=secret").await;
    assert_eq!(query.status, 200);
    let set_cookie = query.headers.get("set-cookie").cloned().unwrap_or_default();
    assert!(
        set_cookie.contains("ipkvm_token=secret"),
        "应种下 ipkvm_token cookie：{set_cookie}"
    );

    // 静态页与 /rfb 升级同样被中间件覆盖。
    let page_without_credentials = server.request("GET", "/").await;
    assert_eq!(page_without_credentials.status, 401);
    assert!(
        server
            .request_with_headers(
                "GET",
                "/vendor/novnc/core/rfb.js",
                &[("Authorization", "Bearer secret")],
            )
            .await
            .status
            == 200
    );

    server.stop().await;
}

const RED_FRAME_SIZE: u16 = 8;

fn red_frame() -> Arc<VideoFrame> {
    let width = RED_FRAME_SIZE as u32;
    let height = RED_FRAME_SIZE as u32;
    let red_bgra = [0, 0, 255, 255].repeat((width * height) as usize);
    Arc::new(VideoFrame::new(
        2,
        MonotonicTimestamp::from_nanos(2),
        width,
        height,
        width * 4,
        PixelFormat::Bgra8888,
        Arc::from(red_bgra.into_boxed_slice()),
    ))
}

fn test_frame() -> Arc<VideoFrame> {
    Arc::new(VideoFrame::new(
        1,
        MonotonicTimestamp::from_nanos(1),
        2,
        1,
        8,
        PixelFormat::Bgra8888,
        Arc::from(vec![0, 0, 255, 255, 0, 255, 0, 255].into_boxed_slice()),
    ))
}

fn wide_frame() -> Arc<VideoFrame> {
    Arc::new(VideoFrame::new(
        9,
        MonotonicTimestamp::from_nanos(9),
        4,
        1,
        16,
        PixelFormat::Bgra8888,
        Arc::from(
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ]
            .into_boxed_slice(),
        ),
    ))
}

fn parse_http_response(bytes: &[u8]) -> HttpResponse {
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP response lacks a header terminator");
    let header = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let mut lines = header.split("\r\n");
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .map(|line| {
            let (name, value) = line.split_once(':').unwrap();
            (name.to_ascii_lowercase(), value.trim().to_string())
        })
        .collect();
    HttpResponse {
        status,
        headers,
        body: bytes[header_end + 4..].to_vec(),
    }
}
