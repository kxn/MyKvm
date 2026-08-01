//! 演示入口：循环播放两个不同分辨率的 Y4M 素材，并提供 RFB TCP 服务。
//!
//! 用法：
//!
//! ```text
//! cargo run -p ipkvm-headless --features demo --bin ipkvm-demo \
//!     --assets .cache/demo-assets --tcp 5900 --fps 10
//! ```
//!
//! 素材按文件名排序依次播放并无限循环；素材之间分辨率不同时，
//! 已连接客户端会收到 `DesktopSize` 更新。

use std::path::PathBuf;
use std::sync::Arc;

use ipkvm_core::{Ch9329InputSink, MouseMode, fake_serial::FakeCommandQueue};
use ipkvm_headless::rfb_connection::RfbConnectionGate;
use ipkvm_headless::rfb_input::RfbInputPump;
use ipkvm_headless::rfb_tcp::{RfbTcpConfig, RfbTcpServer};
use ipkvm_video::looping::LoopingVideoSource;
use ipkvm_video::y4m::Y4mAsset;
use tokio::{
    net::TcpListener,
    sync::{mpsc, watch},
};

struct DemoOptions {
    assets_dir: PathBuf,
    tcp_port: u16,
    frames_per_second: u64,
}

const USAGE: &str = "\
用法：ipkvm-demo [--assets <目录>] [--tcp <端口>] [--fps <帧率>]

  --assets <目录>   存放 *.y4m 素材的目录，默认 .cache/demo-assets
  --tcp <端口>      RFB TCP 监听端口，默认 5900
  --fps <帧率>      播放帧率，默认 10
";

fn parse_args() -> Result<DemoOptions, String> {
    let mut options = DemoOptions {
        assets_dir: PathBuf::from(".cache/demo-assets"),
        tcp_port: 5900,
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
            "--tcp" => {
                options.tcp_port = args
                    .next()
                    .ok_or_else(|| "--tcp 需要一个端口参数".to_string())?
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

fn load_assets(directory: &PathBuf) -> Result<Vec<(PathBuf, Y4mAsset)>, String> {
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
        assets.push((path, asset));
    }
    Ok(assets)
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
    let assets: Vec<Y4mAsset> = loaded.into_iter().map(|(_, asset)| asset).collect();
    let source = match LoopingVideoSource::new(assets, options.frames_per_second) {
        Ok(source) => Arc::new(source),
        Err(error) => {
            eprintln!("无法启动循环视频源：{error}");
            std::process::exit(1);
        }
    };

    let listener = match TcpListener::bind(("127.0.0.1", options.tcp_port)).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("无法监听 127.0.0.1:{}：{error}", options.tcp_port);
            std::process::exit(1);
        }
    };
    let address = listener.local_addr().unwrap();

    let (event_tx, event_rx) = mpsc::channel(64);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let sink = Ch9329InputSink::new(FakeCommandQueue::new(), 0, MouseMode::Absolute);
    let mut pump = RfbInputPump::new(sink);
    let mut pump_events = event_rx;
    tokio::spawn(async move {
        let result = pump
            .run(&mut pump_events, |notice| println!("输入事件：{notice:?}"))
            .await;
        if let Err(error) = result {
            eprintln!("输入事件泵错误：{error}");
        }
    });

    let server = match RfbTcpServer::new(
        listener,
        source,
        event_tx,
        RfbTcpConfig::default(),
        RfbConnectionGate::new(),
    ) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("无法启动 RFB TCP 服务：{error}");
            std::process::exit(1);
        }
    };

    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = shutdown_tx.send(true);
        }
    });

    println!("RFB TCP 服务监听 {address}（Ctrl+C 退出）");
    match server.run(shutdown_rx).await {
        Ok(()) => println!("服务已停止"),
        Err(error) => {
            eprintln!("RFB TCP 服务错误：{error}");
            std::process::exit(1);
        }
    }
}
