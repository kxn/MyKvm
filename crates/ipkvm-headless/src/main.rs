//! 正式无头后台进程入口：同时提供 RFB TCP（标准 VNC 客户端）和嵌入式
//! noVNC 网页 + RFB WebSocket（浏览器）。
//!
//! 视频源按 CLI 参数选择：`--camera <名称>` 打开 Windows 相机（id 或显示名），
//! `--assets <目录>` 使用目录内 Y4M 文件伪设备（按文件名排序循环播放），
//! 未指定任何视频参数时默认打开枚举到的第一台相机。`--list-cameras` 只枚举
//! 设备并退出。相机未就绪时可用 `--assets` 的 Y4M 模拟帧源和 `FakeCommandQueue`
//! 验证画面与键鼠链路（键鼠事件进入模拟队列后被丢弃，不注入真实串口）。
//!
//! 用法：
//!
//! ```text
//! ./scripts/fetch-demo-assets.sh   # 首次运行下载 Y4M 素材
//! cargo run -p ipkvm-headless --features demo --bin ipkvm-headless \
//!     --assets .cache/demo-assets --tcp 5900 --http 6080 --fps 10
//! cargo run -p ipkvm-headless --features demo --bin ipkvm-headless \
//!     --camera "OBS Virtual Camera" --tcp 5900 --http 6080
//! ```
//!
//! 启动后用浏览器打开 `http://127.0.0.1:6080`，或用标准 VNC 客户端连接
//! `127.0.0.1:5900`。两个入口共享同一个单活动控制者连接闸门，同一时刻
//! 只有一个客户端能获得控制权。

use std::path::PathBuf;
use std::time::Duration;

use ipkvm_core::{Ch9329InputSink, MouseMode, fake_serial::FakeCommandQueue};
use ipkvm_headless::rfb_connection::RfbConnectionGate;
use ipkvm_headless::rfb_input::{RfbInputPump, RfbInputRunError};
use ipkvm_headless::rfb_tcp::{RfbTcpConfig, RfbTcpServer, RfbTcpServerError};
use ipkvm_headless::rfb_ws::RfbWebSocketConfig;
use ipkvm_headless::web::{HeadlessWebService, HeadlessWebServiceError};
use ipkvm_video::FrameSource;
use ipkvm_video::camera::CameraSource;
use ipkvm_video::file_source::FileVideoSource;
use ipkvm_video::y4m::Y4mAsset;
use thiserror::Error;
use tokio::{
    net::TcpListener,
    sync::{mpsc, watch},
    task::JoinError,
};

/// 优雅关闭的等待上限。超时后强制退出，避免卡死的连接阻止进程结束。
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

struct Options {
    /// 显式 --assets：文件伪设备目录。
    assets_dir: Option<PathBuf>,
    /// 显式 --camera：按 id 或显示名打开的相机。
    camera_name: Option<String>,
    /// --list-cameras：枚举设备后立即退出。
    list_cameras: bool,
    bind_address: String,
    tcp_port: u16,
    http_port: u16,
    frames_per_second: u64,
}

const USAGE: &str = "\
用法：ipkvm-headless [--list-cameras] [--camera <名称>|--assets <目录>] \
[--bind <地址>] [--tcp <端口>] [--http <端口>] [--fps <帧率>]

  --list-cameras   枚举视频采集设备并退出
  --camera <名称>  按名称（id 或显示名）打开相机；与 --assets 互斥
  --assets <目录>  存放 *.y4m 素材的目录（文件伪设备，按文件名排序循环）
  --bind <地址>    监听地址，默认 127.0.0.1
  --tcp <端口>     RFB TCP 监听端口，默认 5900
  --http <端口>    HTTP/noVNC 监听端口，默认 6080
  --fps <帧率>     播放帧率，默认 10
";

/// 未指定任何视频参数时默认打开枚举到的第一台相机。
fn parse_args() -> Result<Options, String> {
    let mut options = Options {
        assets_dir: None,
        camera_name: None,
        list_cameras: false,
        bind_address: "127.0.0.1".to_string(),
        tcp_port: 5900,
        http_port: 6080,
        frames_per_second: 10,
    };

    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--list-cameras" => options.list_cameras = true,
            "--camera" => {
                options.camera_name = Some(
                    args.next()
                        .ok_or_else(|| "--camera 需要一个名称参数".to_string())?,
                );
            }
            "--assets" => {
                options.assets_dir = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--assets 需要一个目录参数".to_string())?,
                ));
            }
            "--bind" => {
                options.bind_address = args
                    .next()
                    .ok_or_else(|| "--bind 需要一个地址参数".to_string())?;
            }
            "--tcp" => {
                options.tcp_port = args
                    .next()
                    .ok_or_else(|| "--tcp 需要一个端口参数".to_string())?
                    .parse()
                    .map_err(|error| format!("无效端口：{error}"))?;
            }
            "--http" => {
                options.http_port = args
                    .next()
                    .ok_or_else(|| "--http 需要一个端口参数".to_string())?
                    .parse()
                    .map_err(|error| format!("无效端口：{error}"))?;
            }
            "--fps" => {
                options.frames_per_second = args
                    .next()
                    .ok_or_else(|| "--fps 需要一个帧率参数".to_string())?
                    .parse()
                    .map_err(|error| format!("无效帧率：{error}"))?;
            }
            "--help" | "-h" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("未知参数：{other}")),
        }
    }

    if options.assets_dir.is_some() && options.camera_name.is_some() {
        return Err("--assets 与 --camera 互斥，只能指定其中一个".to_string());
    }
    Ok(options)
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
/// 优先级：`--assets`（文件伪设备）> `--camera`（按名打开）> 默认第一台相机。
fn build_source(options: &Options) -> Result<std::sync::Arc<dyn FrameSource>, String> {
    let source: std::sync::Arc<dyn FrameSource> = match (&options.assets_dir, &options.camera_name)
    {
        (Some(directory), _) => {
            let assets = load_assets(directory)?;
            std::sync::Arc::new(
                FileVideoSource::new(assets, options.frames_per_second)
                    .map_err(|error| format!("无法启动文件视频源：{error}"))?,
            )
        }
        (None, Some(name)) => std::sync::Arc::new(
            CameraSource::open(name, options.frames_per_second)
                .map_err(|error| format!("无法打开相机 {name}：{error}"))?,
        ),
        (None, None) => {
            let cameras = ipkvm_video::camera::list_cameras()
                .map_err(|error| format!("枚举相机失败：{error}"))?;
            let first = cameras
                .first()
                .ok_or_else(|| "未找到任何相机，请用 --assets 指定素材目录".to_string())?;
            println!(
                "未指定视频源，默认使用第一台相机：{}（{}）",
                first.display_name, first.id
            );
            std::sync::Arc::new(
                CameraSource::open(&first.id, options.frames_per_second)
                    .map_err(|error| format!("无法打开相机 {}：{error}", first.id))?,
            )
        }
    };
    println!("视频源：{:?}", source.source_info());
    Ok(source)
}

#[derive(Debug, Error)]
enum HeadlessRunError {
    #[error("I/O 失败")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    TcpServer(#[from] RfbTcpServerError),
    #[error(transparent)]
    Web(#[from] HeadlessWebServiceError),
    #[error(transparent)]
    Input(#[from] RfbInputRunError),
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
    let options = match parse_args() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("参数错误：{error}");
            eprint!("{USAGE}");
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

    let source = match build_source(&options) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };

    if let Err(error) = run(source, options).await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run(
    source: std::sync::Arc<dyn FrameSource>,
    options: Options,
) -> Result<(), HeadlessRunError> {
    let tcp_listener = TcpListener::bind((options.bind_address.as_str(), options.tcp_port)).await?;
    let tcp_local = tcp_listener.local_addr()?;
    let http_listener =
        TcpListener::bind((options.bind_address.as_str(), options.http_port)).await?;
    let http_local = http_listener.local_addr()?;

    let (event_tx, event_rx) = mpsc::channel(64);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // 单一连接闸门：clone 给 TCP，move 给 HTTP/WS。两者共享同一个信号量，
    // 因此同一时刻只有一个活跃 RFB 控制连接，无论它来自哪个传输层。
    let gate = RfbConnectionGate::new();

    let sink = Ch9329InputSink::new(FakeCommandQueue::new(), 0, MouseMode::Absolute);
    let mut pump = RfbInputPump::new(sink);
    let mut pump_events = event_rx;
    let input_task = tokio::spawn(async move {
        pump.run(&mut pump_events, |notice| {
            println!("输入事件：{notice:?}");
        })
        .await
    });

    let tcp_server = RfbTcpServer::new(
        tcp_listener,
        std::sync::Arc::clone(&source),
        event_tx.clone(),
        RfbTcpConfig::default(),
        gate.clone(),
    )?;
    let tcp_shutdown = shutdown_rx.clone();
    let mut tcp_task = tokio::spawn(async move { tcp_server.run(tcp_shutdown).await });

    let web_service = HeadlessWebService::new(
        source,
        event_tx,
        RfbWebSocketConfig::default(),
        shutdown_rx.clone(),
        gate,
    )?;
    let mut http_task = tokio::spawn(async move { web_service.serve(http_listener).await });

    let mut ctrl_c = tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
    });

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

    // 优雅关闭：等待三个任务全部结束，超时则强制退出。
    let join = async {
        let tcp = flatten(tcp_task.await);
        let http = flatten(http_task.await);
        let input = flatten(input_task.await);
        tcp.and(http).and(input)
    };
    match tokio::time::timeout(SHUTDOWN_TIMEOUT, join).await {
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
