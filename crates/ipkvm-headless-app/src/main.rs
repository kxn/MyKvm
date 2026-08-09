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
use std::sync::Arc;
use std::time::Duration;

use ipkvm_core::{
    Ch9329InputSink, InputResult, InputSink, KeyEvent, MouseMode, MouseProfile, PointerEvent,
    QueueStats, SerialCommandQueue, fake_serial::FakeCommandQueue,
};
use ipkvm_device::ProductionDeviceInventoryProvider;
use ipkvm_headless::config::{self, Options};
use ipkvm_headless::rfb_connection::{RfbConnectionGate, RfbConnectionSettings};
use ipkvm_headless::rfb_tcp::{RfbTcpConfig, RfbTcpServer, RfbTcpServerError};
use ipkvm_headless::rfb_ws::RfbWebSocketConfig;
use ipkvm_headless::settings::{SettingsStore, WebSettings};
use ipkvm_headless::supervisor::{RecoveryPolicy, SessionSupervisor};
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
/// `SessionSupervisor<S>` 要求 `S: InputSink + Clone + Send + 'static`；真实
/// 串口 (`Ch9329InputSink<SerialCommandQueue>`) 与模拟队列
/// (`Ch9329InputSink<FakeCommandQueue>`) 是不同具体类型，本枚举在组装层统一。
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

fn install_service(
    service_name: &str,
    options: &ipkvm_headless::config::Options,
) -> Result<(), String> {
    // 获取当前可执行文件路径
    let exe_path = std::env::current_exe().map_err(|e| format!("获取可执行文件路径失败：{e}"))?;

    // 构建启动命令参数
    let mut args = Vec::new();
    if let Some(camera) = &options.camera_name {
        args.push(format!("--camera '{}'", camera.replace('\'', "'\\''")));
    }
    if let Some(assets) = &options.assets_dir {
        args.push(format!("--assets '{}'", assets.display()));
    }
    if let Some(serial) = &options.serial_path {
        args.push(format!("--serial '{}'", serial));
    }
    if options.bind_address != "0.0.0.0" {
        args.push(format!("--bind '{}'", options.bind_address));
    }
    if options.tcp_port != 5900 {
        args.push(format!("--tcp {}", options.tcp_port));
    }
    if options.http_port != 80 {
        args.push(format!("--http {}", options.http_port));
    }
    if let Some(fps) = options.frames_per_second {
        args.push(format!("--fps {}", fps));
    }
    if let Some(token) = &options.token {
        args.push(format!("--token '{}'", token));
    }
    if let Some(password) = &options.vnc_password {
        args.push(format!("--vnc-password '{}'", password));
    }

    let exec_line = format!("{} {}", exe_path.display(), args.join(" "));

    // 检测 init 系统
    if std::path::Path::new("/run/systemd/system").exists() {
        // systemd
        let service_content = format!(
            "[Unit]
Description=MyKvm Headless Service
After=network.target

[Service]
Type=simple
ExecStart={}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
",
            exec_line
        );
        let service_path = format!("/etc/systemd/system/{}.service", service_name);
        std::fs::write(&service_path, service_content)
            .map_err(|e| format!("写入服务文件失败：{e}"))?;

        // 启用并启动服务
        std::process::Command::new("systemctl")
            .args(["daemon-reload"])
            .status()
            .map_err(|e| format!("systemctl daemon-reload 失败：{e}"))?;

        std::process::Command::new("systemctl")
            .args(["enable", service_name])
            .status()
            .map_err(|e| format!("systemctl enable 失败：{e}"))?;

        println!("已安装 systemd 服务：{service_name}");
        println!("服务文件：{service_path}");
        println!("启动命令：systemctl start {service_name}");
        println!("查看日志：journalctl -u {service_name} -f");
    } else if std::path::Path::new("/sbin/openrc").exists()
        || std::path::Path::new("/usr/sbin/openrc").exists()
    {
        // openrc
        // openrc 的 command 不能含空格，需要把参数放在 command_args
        let service_content = format!(
            "#!/sbin/openrc-run

description=\"MyKvm Headless Service\"
command=\"{exe}\"
command_args=\"{args}\"
command_background=\"yes\"
pidfile=\"/run/{name}.pid\"

depend() {{
    need net
}}

start_pre() {{
    checkpath -d -m 0755 -o root /run
}}
",
            exe = exe_path.display(),
            args = args.join(" "),
            name = service_name,
        );
        let service_path = format!("/etc/init.d/{}", service_name);
        std::fs::write(&service_path, service_content)
            .map_err(|e| format!("写入服务文件失败：{e}"))?;

        // 设置可执行权限
        std::process::Command::new("chmod")
            .args(["+x", &service_path])
            .status()
            .map_err(|e| format!("chmod 失败：{e}"))?;

        // 启用服务
        std::process::Command::new("rc-update")
            .args(["add", service_name, "default"])
            .status()
            .map_err(|e| format!("rc-update add 失败：{e}"))?;

        println!("已安装 openrc 服务：{service_name}");
        println!("服务文件：{service_path}");
        println!("启动命令：rc-service {service_name} start");
        println!("查看日志：tail -f /var/log/messages");
    } else {
        return Err("未检测到支持的 init 系统（需要 systemd 或 openrc）".to_string());
    }

    Ok(())
}

fn uninstall_service(service_name: &str) -> Result<(), String> {
    if std::path::Path::new("/run/systemd/system").exists() {
        // systemd
        std::process::Command::new("systemctl")
            .args(["stop", service_name])
            .status()
            .ok();

        std::process::Command::new("systemctl")
            .args(["disable", service_name])
            .status()
            .ok();

        let service_path = format!("/etc/systemd/system/{}.service", service_name);
        std::fs::remove_file(&service_path).map_err(|e| format!("删除服务文件失败：{e}"))?;

        std::process::Command::new("systemctl")
            .args(["daemon-reload"])
            .status()
            .map_err(|e| format!("systemctl daemon-reload 失败：{e}"))?;

        println!("已卸载 systemd 服务：{service_name}");
    } else if std::path::Path::new("/sbin/openrc").exists()
        || std::path::Path::new("/usr/sbin/openrc").exists()
    {
        // openrc
        std::process::Command::new("rc-service")
            .args([service_name, "stop"])
            .status()
            .ok();

        std::process::Command::new("rc-update")
            .args(["del", service_name, "default"])
            .status()
            .ok();

        let service_path = format!("/etc/init.d/{}", service_name);
        std::fs::remove_file(&service_path).map_err(|e| format!("删除服务文件失败：{e}"))?;

        println!("已卸载 openrc 服务：{service_name}");
    } else {
        return Err("未检测到支持的 init 系统（需要 systemd 或 openrc）".to_string());
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
            let tile = if options.dirty_rects {
                Some(options.dirty_rect_tile_size.unwrap_or(32))
            } else {
                None
            };
            std::sync::Arc::new(
                FileVideoSource::new_with_dirty_rects(assets, frames_per_second, tile)
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

fn configure_input_log(options: &Options) -> Result<(), String> {
    let config = input_log_config(
        options,
        std::env::var_os("IPKVM_LOG_FILE").map(PathBuf::from),
        std::env::var("IPKVM_LOG_LEVEL").ok(),
        std::env::var("IPKVM_LOG_CATEGORIES").ok(),
    )?;
    let Some(config) = config else {
        return Ok(());
    };
    let path = config.path().to_path_buf();
    ipkvm_core::diag::configure(config)
        .map_err(|error| format!("打开输入诊断日志 {} 失败：{error}", path.display()))?;
    ipkvm_core::diag::log(
        ipkvm_core::diag::DiagLevel::Info,
        ipkvm_core::diag::DiagCategory::LIFECYCLE,
        "headless.app",
        "input_log",
        &[
            ("result", "enabled".into()),
            ("path", path.display().to_string()),
        ],
    );
    Ok(())
}

fn input_log_config(
    options: &Options,
    env_file: Option<PathBuf>,
    env_level: Option<String>,
    env_categories: Option<String>,
) -> Result<Option<ipkvm_core::diag::DiagConfig>, String> {
    let Some(path) = options.log_file.clone().or(env_file) else {
        return Ok(None);
    };
    let level_text = options
        .log_level
        .as_deref()
        .or(env_level.as_deref())
        .unwrap_or("trace");
    let Some(level) = ipkvm_core::diag::DiagLevel::parse(level_text) else {
        return Err(format!(
            "无效日志级别：{level_text}（可用 error/warn/info/debug/trace）"
        ));
    };
    let categories_text = options
        .log_categories
        .as_deref()
        .or(env_categories.as_deref())
        .unwrap_or("input,pointer,queue,serial,lifecycle");
    let categories = ipkvm_core::diag::DiagCategory::parse_list(categories_text)
        .map_err(|category| format!("无效日志类别：{category}"))?;
    Ok(Some(
        ipkvm_core::diag::DiagConfig::file(path)
            .level(level)
            .categories(categories),
    ))
}

type SessionComponents = (Arc<dyn FrameSource>, HeadlessSink);

#[cfg(test)]
mod tests {
    use super::*;

    fn initial_video_source_requested(options: &Options) -> bool {
        options.assets_dir.is_some() || options.camera_name.is_some()
    }

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
            dirty_rects: false,
            dirty_rect_tile_size: None,
            log_file: None,
            log_level: None,
            log_categories: None,
            install_service: None,
            uninstall_service: None,
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

    #[test]
    fn input_log_config_uses_file_level_and_categories() {
        let mut options = options();
        options.log_file = Some(PathBuf::from("/tmp/ipkvm-headless-input.log"));
        options.log_level = Some("debug".to_string());
        options.log_categories = Some("pointer,queue".to_string());

        let config = input_log_config(&options, None, None, None)
            .unwrap()
            .unwrap();

        assert_eq!(
            config.path(),
            PathBuf::from("/tmp/ipkvm-headless-input.log")
        );
        assert_eq!(
            config.configured_level(),
            ipkvm_core::diag::DiagLevel::Debug
        );
        assert!(
            config
                .configured_categories()
                .contains(ipkvm_core::diag::DiagCategory::POINTER)
        );
        assert!(
            config
                .configured_categories()
                .contains(ipkvm_core::diag::DiagCategory::QUEUE)
        );
        assert!(
            !config
                .configured_categories()
                .contains(ipkvm_core::diag::DiagCategory::SERIAL)
        );
    }

    #[test]
    fn input_log_config_can_be_enabled_from_environment_values() {
        let config = input_log_config(
            &options(),
            Some(PathBuf::from("/tmp/ipkvm-headless-env.log")),
            Some("trace".to_string()),
            Some("all".to_string()),
        )
        .unwrap()
        .unwrap();

        assert_eq!(config.path(), PathBuf::from("/tmp/ipkvm-headless-env.log"));
        assert_eq!(
            config.configured_level(),
            ipkvm_core::diag::DiagLevel::Trace
        );
        assert!(
            config
                .configured_categories()
                .contains(ipkvm_core::diag::DiagCategory::SERIAL)
        );
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
    if let Err(error) = configure_input_log(&options) {
        eprintln!("配置错误：{error}");
        std::process::exit(2);
    }

    if options.list_cameras {
        match print_cameras() {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }

    if let Some(service_name) = &options.install_service {
        match install_service(service_name, &options) {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                eprintln!("安装服务失败：{error}");
                std::process::exit(1);
            }
        }
    }

    if let Some(service_name) = &options.uninstall_service {
        match uninstall_service(service_name) {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                eprintln!("卸载服务失败：{error}");
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
    _options: &Options,
    _runtime: &WebSettings,
) -> Result<Option<SessionComponents>, String> {
    // 启动时不自动启动 session，等待用户手动连接
    println!("启动空会话，等待用户连接");
    Ok(None)
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
        Ok((self.build_video(selection)?, self.build_control(selection)?))
    }

    fn build_video(&self, selection: &SessionSelection) -> Result<Arc<dyn FrameSource>, String> {
        let mut options = self.options.clone();
        if let Some(video) = &selection.video
            && !video.trim().is_empty()
        {
            options.camera_name = Some(video.clone());
            options.assets_dir = None;
        }
        let frames_per_second = options
            .frames_per_second
            .unwrap_or(self.settings.get().preview_fps);
        build_source(&options, frames_per_second)
    }

    fn build_control(&self, selection: &SessionSelection) -> Result<HeadlessSink, String> {
        let mut options = self.options.clone();
        if let Some(serial) = &selection.serial {
            options.serial_path = if serial.trim().is_empty() {
                None
            } else {
                Some(serial.clone())
            };
        }
        let runtime = self.settings.get();
        let baud = options.serial_baud.unwrap_or(runtime.baud_rate);
        let profile = selection
            .mouse_profile
            .as_deref()
            .map(MouseProfile::parse)
            .transpose()
            .map_err(|error| error.to_string())?
            .unwrap_or(runtime.mouse_profile);
        build_sink(&options, baud, profile.resolve_mode())
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

    let mut supervisor =
        SessionSupervisor::<HeadlessSink>::new(gate.clone(), RecoveryPolicy::default());
    let initial_selection = match initial {
        Some((source, sink)) => {
            let selection = Some(SessionSelection {
                video: options.camera_name.clone(),
                serial: options.serial_path.clone(),
                mouse_profile: Some(settings.get().mouse_profile.as_str().to_string()),
            });
            let source_for_start = Arc::clone(&source);
            let sink_for_start = sink.clone();
            supervisor
                .start_at(
                    move || Ok(Arc::clone(&source_for_start)),
                    move || Ok(sink_for_start.clone()),
                    std::time::Instant::now(),
                )
                .await;
            selection
        }
        None => None,
    };
    let frame_hub = Arc::new(supervisor.frame_source());
    let event_publisher = supervisor.event_publisher();
    let supervisor = Arc::new(tokio::sync::Mutex::new(supervisor));
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
        Arc::clone(&frame_hub),
        event_publisher.clone(),
        tcp_config,
        gate.clone(),
    )?;
    let tcp_shutdown = shutdown_rx.clone();
    let mut tcp_task = tokio::spawn(async move { tcp_server.run(tcp_shutdown).await });

    let web_service = HeadlessWebService::<HeadlessSink>::new(
        frame_hub,
        Arc::clone(&supervisor),
        factory,
        Arc::new(ProductionDeviceInventoryProvider),
        event_publisher,
        ws_config,
        shutdown_rx.clone(),
        gate,
        options.token.clone(), // HTTP/WS 鉴权 token（[auth] token）
        settings,
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
        let mut supervisor = supervisor.lock().await;
        let _ = supervisor.stop_manual().await;
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
