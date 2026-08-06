//! 正式无头后台进程入口：同时提供 RFB TCP（标准 VNC 客户端）和嵌入式
//! noVNC 网页 + RFB WebSocket（浏览器）。
//!
//! 视频源按 CLI 参数选择：`--camera <名称>` 打开 Windows 相机（id 或显示名），
//! `--assets <目录>` 使用目录内 Y4M 文件伪设备（按文件名排序循环播放），
//! 未指定任何视频参数时启动空会话，等待网页选择设备。`--list-cameras` 只枚举
//! 设备并退出。相机未就绪时可用 `--assets` 的 Y4M 模拟帧源和 `FakeCommandQueue`
//! 验证画面与键鼠链路（键鼠事件进入模拟队列后被丢弃，不注入真实串口）。
//!
//! 用法：
//!
//! ```text
//! ./scripts/fetch-demo-assets.sh   # 首次运行下载 Y4M 素材
//! cargo run -p ipkvm-headless-app --bin ipkvm-headless \
//!     --assets .cache/demo-assets --tcp 5900 --http 6080 --fps 10
//! cargo run -p ipkvm-headless-app --bin ipkvm-headless \
//!     --camera "OBS Virtual Camera" --tcp 5900 --http 6080
//! ```
//!
//! 启动后用浏览器打开 `http://127.0.0.1:6080`，或用标准 VNC 客户端连接
//! `127.0.0.1:5900`。两个入口共享同一个单活动控制者连接闸门，同一时刻
//! 只有一个客户端能获得控制权。

use std::path::PathBuf;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Duration;

use ipkvm_core::{
    Ch9329InputSink, InputResult, InputSink, KeyEvent, MouseMode, MouseProfile, PointerEvent,
    QueueStats, SerialCommandQueue, fake_serial::FakeCommandQueue,
};
use ipkvm_device::ProductionDeviceInventoryProvider;
use ipkvm_headless::config::{self, Options};
use ipkvm_headless::frame_source::{EmptyFrameSource, SwitchableFrameSource};
use ipkvm_headless::rfb_connection::{RfbConnectionGate, RfbConnectionSettings};
use ipkvm_headless::rfb_tcp::{RfbTcpConfig, RfbTcpServer, RfbTcpServerError};
use ipkvm_headless::rfb_ws::RfbWebSocketConfig;
use ipkvm_headless::session_manager::SessionManager;
use ipkvm_headless::settings::{SettingsStore, WebSettings};
use ipkvm_headless::web::{
    HeadlessWebService, HeadlessWebServiceError, SessionFactory, SessionSelection,
};
use ipkvm_video::FrameSource;
use ipkvm_video::camera::CameraSource;
use ipkvm_video::file_source::FileVideoSource;
use ipkvm_video::y4m::Y4mAsset;
use thiserror::Error;
use tokio::{net::TcpListener, sync::watch, task::JoinError};

/// 优雅关闭的等待上限。超时后强制退出，避免卡死的连接阻止进程结束。
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// headless 统一输入 sink：枚举分发真实串口与模拟队列。
///
/// `SessionManager<S>` 要求 `S: InputSink + Clone + Send + 'static`；真实串口
/// (`Ch9329InputSink<SerialCommandQueue>`) 与模拟队列
/// (`Ch9329InputSink<FakeCommandQueue>`) 是不同具体类型，本枚举在组装层统一，
/// 使 `SessionManager<HeadlessSink>` 在整个进程生命周期单态化。运行时换 sink
/// 类型不在本轮范围（见 issue #32）——启动时决定，restart 复用相同变体。
#[derive(Clone)]
enum HeadlessSink {
    Serial(Ch9329InputSink<SerialCommandQueue>),
    Fake(Ch9329InputSink<FakeCommandQueue>),
}

impl InputSink for HeadlessSink {
    fn initial_mouse_mode(&self) -> Option<MouseMode> {
        Some(match self {
            HeadlessSink::Serial(sink) => sink.initial_mouse_mode().unwrap_or(MouseMode::Absolute),
            HeadlessSink::Fake(sink) => sink.initial_mouse_mode().unwrap_or(MouseMode::Absolute),
        })
    }

    fn set_mouse_mode(&mut self, mode: MouseMode) -> InputResult<()> {
        match self {
            HeadlessSink::Serial(sink) => sink.set_mouse_mode(mode),
            HeadlessSink::Fake(sink) => sink.set_mouse_mode(mode),
        }
    }

    fn handle_key_batch(&mut self, events: &[KeyEvent]) -> InputResult<()> {
        match self {
            HeadlessSink::Serial(sink) => sink.handle_key_batch(events),
            HeadlessSink::Fake(sink) => sink.handle_key_batch(events),
        }
    }

    fn handle_pointer_batch(&mut self, events: &[PointerEvent]) -> InputResult<()> {
        match self {
            HeadlessSink::Serial(sink) => sink.handle_pointer_batch(events),
            HeadlessSink::Fake(sink) => sink.handle_pointer_batch(events),
        }
    }

    fn release_all(&mut self) -> InputResult<()> {
        match self {
            HeadlessSink::Serial(sink) => sink.release_all(),
            HeadlessSink::Fake(sink) => sink.release_all(),
        }
    }

    fn queue_stats(&self) -> Option<QueueStats> {
        Some(match self {
            HeadlessSink::Serial(sink) => sink.queue_stats(),
            HeadlessSink::Fake(sink) => sink.queue_stats(),
        })
    }
}

fn load_assets(directory: &PathBuf) -> Result<Vec<Y4mAsset>, String> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(directory)
        .map_err(|error| format!("无法读取素材目录 {}：{error}", directory.display()))?
    {
        let path = entry
            .map_err(|error| format!("无法读取目录项：{error}"))?
            .path();
        if path.extension().is_some_and(|extension| extension == "y4m") {
            paths.push(path);
        }
    }
    paths.sort();

    if paths.is_empty() {
        return Err(format!(
            "素材目录 {} 中没有 *.y4m 文件",
            directory.display()
        ));
    }

    let mut assets = Vec::new();
    for path in paths {
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
        let asset = Y4mAsset::parse(&bytes)
            .map_err(|error| format!("解析 {} 失败：{error}", path.display()))?;
        println!(
            "素材 {}：{}x{}，{} 帧",
            path.display(),
            asset.width(),
            asset.height(),
            asset.frame_count()
        );
        assets.push(asset);
    }
    Ok(assets)
}

/// 枚举视频采集设备并打印（格式同 `camera_probe` 示例）。
/// 枚举成功即返回 Ok(())，即使一台设备都没有；枚举失败（如非 Windows
/// 平台的 UnsupportedPlatform stub）由调用方报错退出。
fn print_cameras() -> Result<(), String> {
    let cameras =
        ipkvm_video::camera::list_cameras().map_err(|error| format!("枚举相机失败：{error}"))?;
    println!("{} camera(s):", cameras.len());
    for (index, camera) in cameras.iter().enumerate() {
        println!(
            "  [{index}] id={:?} display_name={:?}",
            camera.id, camera.display_name
        );
    }
    Ok(())
}

/// 按 CLI 参数选择视频源并统一包装成 `Arc<dyn FrameSource>`。
///
/// 优先级：`--assets`（文件伪设备）> `--camera`（按名打开）。
fn build_source(
    options: &Options,
    frames_per_second: u64,
) -> Result<std::sync::Arc<dyn FrameSource>, String> {
    let source: std::sync::Arc<dyn FrameSource> = match (&options.assets_dir, &options.camera_name)
    {
        (Some(directory), _) => {
            let assets = load_assets(directory)?;
            std::sync::Arc::new(
                FileVideoSource::new(assets, frames_per_second)
                    .map_err(|error| format!("无法启动文件视频源：{error}"))?,
            )
        }
        (None, Some(name)) => std::sync::Arc::new(
            CameraSource::open(name, frames_per_second)
                .map_err(|error| format!("无法打开相机 {name}：{error}"))?,
        ),
        (None, None) => {
            return Err(
                "未指定视频源，请在网页连接页选择设备，或使用 --camera/--assets 启动".to_string(),
            );
        }
    };
    println!("视频源：{:?}", source.source_info());
    Ok(source)
}

fn initial_video_source_requested(options: &Options) -> bool {
    options.assets_dir.is_some() || options.camera_name.is_some()
}

type SessionComponents = (Arc<dyn FrameSource>, HeadlessSink);

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> Options {
        Options {
            assets_dir: None,
            camera_name: None,
            list_cameras: false,
            serial_path: None,
            serial_baud: None,
            bind_address: "127.0.0.1".to_string(),
            tcp_port: 5900,
            http_port: 6080,
            frames_per_second: None,
            encoding: None,
            jpeg_quality: None,
            token: None,
            vnc_password: None,
        }
    }

    #[test]
    fn no_explicit_video_source_does_not_start_initial_session() {
        assert!(!initial_video_source_requested(&options()));
    }

    #[test]
    fn explicit_camera_or_assets_starts_initial_session() {
        let mut camera = options();
        camera.camera_name = Some("Fixture Camera".to_string());
        assert!(initial_video_source_requested(&camera));

        let mut assets = options();
        assets.assets_dir = Some(PathBuf::from("assets"));
        assert!(initial_video_source_requested(&assets));
    }
}

#[derive(Debug, Error)]
enum HeadlessRunError {
    #[error("I/O 失败")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    TcpServer(#[from] RfbTcpServerError),
    #[error(transparent)]
    Web(#[from] HeadlessWebServiceError),
    #[error("后台任务失败")]
    Join(#[from] JoinError),
    #[error("会话失败：{0}")]
    Session(String),
}

type TaskResult<T> = Result<T, HeadlessRunError>;

/// 把 `JoinError` 和内部错误统一折叠成 `HeadlessRunError`。
fn flatten<T, E>(result: Result<Result<T, E>, JoinError>) -> TaskResult<T>
where
    E: Into<HeadlessRunError>,
{
    let inner = result?;
    inner.map_err(Into::into)
}

#[tokio::main]
async fn main() {
    let cli = match config::parse_cli() {
        Ok(cli) => cli,
        Err(error) => {
            eprintln!("参数错误：{error}");
            eprint!("{}", config::USAGE);
            std::process::exit(2);
        }
    };
    let file = match cli
        .config_path
        .as_deref()
        .map(config::load_config)
        .transpose()
    {
        Ok(file) => file,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let options = match config::resolve(cli, file) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("配置错误：{error}");
            eprint!("{}", config::USAGE);
            std::process::exit(2);
        }
    };

    if options.list_cameras {
        match print_cameras() {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }

    // 运行时设置存储：构造一次，注入 Web API 与会话工厂；损坏/缺失回退默认。
    let (settings_store, settings_warning) = SettingsStore::load();
    if let Some(warning) = settings_warning {
        eprintln!("{warning}");
    }
    let settings_store = Arc::new(settings_store);

    let initial = match build_initial_session_components(&options, &settings_store.get()) {
        Ok(built) => built,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };

    if let Err(error) = run(initial, options, settings_store).await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn build_initial_session_components(
    options: &Options,
    runtime: &WebSettings,
) -> Result<Option<SessionComponents>, String> {
    if !initial_video_source_requested(options) {
        println!("未指定视频源，启动空会话，等待网页选择设备");
        return Ok(None);
    }
    build_session_components(options, runtime, None).map(Some)
}

/// 按 CLI 参数与运行时设置构造会话所需的帧源与输入 sink。
///
/// 返回的 `(frame_source, sink)` 同时用于启动即绑定（`run` 首启）与零初始
/// 会话启动后的工厂 `build()`。两条 sink 路径（真实串口/模拟队列）经
/// `HeadlessSink` 统一类型。`serial_baud`/`frames_per_second` 未指定时取
/// 运行时设置（`auto_baud` 本单仅存储，不在 headless 使用）。
fn build_session_components(
    options: &Options,
    runtime: &WebSettings,
    profile_override: Option<&str>,
) -> Result<SessionComponents, String> {
    let frames_per_second = options.frames_per_second.unwrap_or(runtime.preview_fps);
    let baud = options.serial_baud.unwrap_or(runtime.baud_rate);
    let source = build_source(options, frames_per_second)?;
    let profile = profile_override
        .map(MouseProfile::parse)
        .transpose()
        .map_err(|error| error.to_string())?
        .unwrap_or(runtime.mouse_profile);
    let sink = build_sink(options, baud, profile.resolve_mode())?;
    Ok((source, sink))
}

/// 按 `--serial` 选择输入 sink：真实串口或模拟队列。
fn build_sink(options: &Options, baud: u32, mouse_mode: MouseMode) -> Result<HeadlessSink, String> {
    if let Some(serial_path) = &options.serial_path {
        let queue = SerialCommandQueue::open(serial_path, baud)
            .map_err(|e| format!("无法打开串口 {serial_path}@{baud}：{e}"))?;
        println!("输入：CH9329 串口 {serial_path}@{baud}（8N1）");
        Ok(HeadlessSink::Serial(Ch9329InputSink::new(
            queue, 0, mouse_mode,
        )))
    } else {
        println!("输入：模拟队列（无 --serial，键鼠事件不注入真实串口）");
        Ok(HeadlessSink::Fake(Ch9329InputSink::new(
            FakeCommandQueue::new(),
            0,
            mouse_mode,
        )))
    }
}

/// 会话工厂：捕获启动配置作为默认值，供运行时 `POST /api/session`
/// create/restart 按设备选择重新构造帧源/sink。
struct HeadlessSessionFactory {
    options: Options,
    settings: Arc<SettingsStore>,
}

impl SessionFactory<HeadlessSink> for HeadlessSessionFactory {
    fn build(
        &self,
        selection: &SessionSelection,
    ) -> Result<(Arc<dyn FrameSource>, HeadlessSink), String> {
        let mut options = self.options.clone();
        if let Some(video) = &selection.video
            && !video.trim().is_empty()
        {
            options.camera_name = Some(video.clone());
            options.assets_dir = None;
        }
        if let Some(serial) = &selection.serial {
            options.serial_path = if serial.trim().is_empty() {
                None
            } else {
                Some(serial.clone())
            };
        }
        build_session_components(
            &options,
            &self.settings.get(),
            selection.mouse_profile.as_deref(),
        )
    }
}

async fn run(
    initial: Option<(Arc<dyn FrameSource>, HeadlessSink)>,
    options: Options,
    settings: Arc<SettingsStore>,
) -> Result<(), HeadlessRunError> {
    let tcp_listener = TcpListener::bind((options.bind_address.as_str(), options.tcp_port)).await?;
    let tcp_local = tcp_listener.local_addr()?;
    let http_listener =
        TcpListener::bind((options.bind_address.as_str(), options.http_port)).await?;
    let http_local = http_listener.local_addr()?;

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // 单一连接闸门：clone 给 TCP、HTTP/WS 与会话管理器。三者共享同一个信号量，
    // 因此同一时刻只有一个活跃 RFB 控制连接，无论它来自哪个传输层。
    let gate = RfbConnectionGate::new();

    let (switchable_source, manager, initial_selection) = match initial {
        Some((source, sink)) => {
            let switchable_source = Arc::new(SwitchableFrameSource::new(Arc::clone(&source)));
            // 显式指定视频源时启动即绑定；无参数路径使用下面的 empty manager。
            let mut manager = SessionManager::new(Arc::clone(&source), sink, gate.clone());
            manager
                .start()
                .map_err(|error| HeadlessRunError::Session(format!("启动会话失败：{error}")))?;
            let selection = Some(SessionSelection {
                video: options.camera_name.clone(),
                serial: options.serial_path.clone(),
                mouse_profile: Some(settings.get().mouse_profile.as_str().to_string()),
            });
            (switchable_source, manager, selection)
        }
        None => {
            let empty = Arc::new(EmptyFrameSource::new());
            (
                Arc::new(SwitchableFrameSource::new(empty)),
                SessionManager::empty(),
                None,
            )
        }
    };
    let event_publisher = manager.event_publisher();
    let manager = Arc::new(tokio::sync::Mutex::new(manager));
    let factory = Arc::new(HeadlessSessionFactory {
        options: options.clone(),
        settings: Arc::clone(&settings),
    });

    // 鉴权注入：VNC 密码（若有）同时作用于 TCP 与 WS 两条 RFB 传输，
    // 未配置时保持 RfbSecurity::None（连接闸门侧按本机回环限制来源）。
    let security = config::vnc_security(options.vnc_password.as_deref());
    let preferred_encoding = config::parse_encoding(options.encoding.as_deref());
    let jpeg_quality = options.jpeg_quality.unwrap_or(85);
    let tcp_config = RfbTcpConfig {
        connection: RfbConnectionSettings {
            security: security.clone(),
            preferred_encoding,
            jpeg_quality,
            ..RfbConnectionSettings::default()
        },
        ..RfbTcpConfig::default()
    };
    let ws_config = RfbWebSocketConfig {
        connection: RfbConnectionSettings {
            security,
            preferred_encoding,
            jpeg_quality,
            ..RfbConnectionSettings::default()
        },
    };

    let tcp_server = RfbTcpServer::new(
        tcp_listener,
        Arc::clone(&switchable_source),
        event_publisher.clone(),
        tcp_config,
        gate.clone(),
    )?;
    let tcp_shutdown = shutdown_rx.clone();
    let mut tcp_task = tokio::spawn(async move { tcp_server.run(tcp_shutdown).await });

    let web_service = HeadlessWebService::<HeadlessSink>::new(
        switchable_source,
        Arc::clone(&manager),
        factory,
        Arc::new(ProductionDeviceInventoryProvider),
        event_publisher,
        ws_config,
        shutdown_rx.clone(),
        gate,
        options.token.clone(), // HTTP/WS 鉴权 token（[auth] token）
        settings,
        Arc::new(AtomicBool::new(false)),
        initial_selection,
    )?;
    let mut http_task = tokio::spawn(async move { web_service.serve(http_listener).await });

    let mut ctrl_c = tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
    });

    if options.token.is_some() {
        println!("HTTP 鉴权：已启用（Bearer/cookie/query token）");
    } else {
        println!("HTTP 鉴权：未配置 token，仅允许本机来源访问");
    }
    if options.vnc_password.is_some() {
        println!("RFB 鉴权：已启用（VNC 密码挑战）");
    } else {
        println!("RFB 鉴权：未配置 VNC 密码，TCP 仅允许本机来源连接");
    }

    println!(
        "ipkvm-headless 已启动：RFB TCP 监听 {tcp_local}，noVNC 网页 http://{http_local}（Ctrl+C 退出）"
    );

    let early_tcp;
    let early_http;
    tokio::select! {
        _ = &mut ctrl_c => {
            early_tcp = None;
            early_http = None;
        }
        result = &mut tcp_task => {
            early_tcp = Some(flatten(result));
            early_http = None;
        }
        result = &mut http_task => {
            early_tcp = None;
            early_http = Some(flatten(result));
        }
    }

    shutdown_tx.send_replace(true);
    ctrl_c.abort();

    // 优雅关闭：先停会话（释放输入泵、release_all），再等待传输任务结束。
    let session_stop = async {
        let mut manager = manager.lock().await;
        if manager.stop().is_ok() {
            manager.wait_stopped().await;
        }
    };
    let join = async {
        let tcp = flatten(tcp_task.await);
        let http = flatten(http_task.await);
        tcp.and(http)
    };
    match tokio::time::timeout(SHUTDOWN_TIMEOUT, async {
        session_stop.await;
        join.await
    })
    .await
    {
        Ok(Ok(())) => {
            println!("ipkvm-headless 已停止");
            Ok(())
        }
        Ok(Err(error)) => {
            // 某个任务返回错误。若触发是 Ctrl+C，这里仍报告错误；若触发是某任务
            // 提前停止，combine 报告第一个错误并附带"提前停止"诊断。
            report_early(early_tcp, early_http);
            Err(error)
        }
        Err(_) => {
            eprintln!("关闭超时（{SHUTDOWN_TIMEOUT:?}），部分连接可能未优雅断开，强制退出");
            std::process::exit(1);
        }
    }
}

/// 某个传输任务在关闭信号前就停止时，补充说明是哪个任务触发的。
fn report_early(early_tcp: Option<TaskResult<()>>, early_http: Option<TaskResult<()>>) {
    if let Some(Err(error)) = early_tcp {
        eprintln!("RFB TCP 服务提前停止：{error}");
    }
    if let Some(Err(error)) = early_http {
        eprintln!("HTTP 服务提前停止：{error}");
    }
}
