//! 正式无头后台进程入口：同时提供 RFB TCP（标准 VNC 客户端）和嵌入式
//! noVNC 网页 + RFB WebSocket（浏览器）。
//!
//! 硬件未到货前使用 Y4M 循环播放模拟帧源和 `FakeCommandQueue`：画面是真实
//! 视频文件，键鼠事件进入模拟队列后被丢弃，不注入真实串口。
//!
//! 用法：
//!
//! ```text
//! ./scripts/fetch-demo-assets.sh   # 首次运行下载 Y4M 素材
//! cargo run -p ipkvm-headless --features demo --bin ipkvm-headless \
//!     --assets .cache/demo-assets --tcp 5900 --http 6080 --fps 10
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
use ipkvm_video::looping::LoopingVideoSource;
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
    assets_dir: PathBuf,
    bind_address: String,
    tcp_port: u16,
    http_port: u16,
    frames_per_second: u64,
}

const USAGE: &str = "\
用法：ipkvm-headless [--assets <目录>] [--bind <地址>] [--tcp <端口>] \
[--http <端口>] [--fps <帧率>]

  --assets <目录>   存放 *.y4m 素材的目录，默认 .cache/demo-assets
  --bind <地址>     监听地址，默认 127.0.0.1
  --tcp <端口>      RFB TCP 监听端口，默认 5900
  --http <端口>     HTTP/noVNC 监听端口，默认 6080
  --fps <帧率>      播放帧率，默认 10
";

fn parse_args() -> Result<Options, String> {
    let mut options = Options {
        assets_dir: PathBuf::from(".cache/demo-assets"),
        bind_address: "127.0.0.1".to_string(),
        tcp_port: 5900,
        http_port: 6080,
        frames_per_second: 10,
    };

    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--assets" => {
                options.assets_dir = PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--assets 需要一个目录参数".to_string())?,
                );
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

    let loaded = match load_assets(&options.assets_dir) {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let source = match LoopingVideoSource::new(loaded, options.frames_per_second) {
        Ok(source) => std::sync::Arc::new(source),
        Err(error) => {
            eprintln!("无法启动循环视频源：{error}");
            std::process::exit(1);
        }
    };

    if let Err(error) = run(source, options).await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run(
    source: std::sync::Arc<LoopingVideoSource>,
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
