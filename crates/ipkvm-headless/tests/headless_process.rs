//! 正式 ipkvm-headless 组装路径的集成测试。
//!
//! 验证 `RfbTcpServer` 和 `HeadlessWebService`（嵌入式 noVNC 页面 + `/rfb`
//! WebSocket）共享同一个 `RfbConnectionGate` 的组装路径：两个传输同时监听、
//! 单活动控制者互斥、静态资源可访问、关闭后干净退出。
//!
//! 这条路径与 `rfb_transport_exclusion.rs` 的区别在于使用 `HeadlessWebService`
//! 而非裸 `RfbWebSocketService`，从而覆盖静态资源路由和正式组装入口。

mod support;

use std::{net::SocketAddr, sync::Arc};

use futures_util::StreamExt;
use ipkvm_core::{Ch9329InputSink, MouseMode, fake_serial::FakeCommandQueue};
use ipkvm_headless::{
    frame_source::SwitchableFrameSource,
    rfb_connection::RfbConnectionGate,
    rfb_tcp::{RfbTcpConfig, RfbTcpServer, RfbTcpServerError},
    rfb_ws::RfbWebSocketConfig,
    session_manager::SessionManager,
    web::{HeadlessWebService, SessionFactory, SessionSelection},
};
use ipkvm_video::{MonotonicTimestamp, PixelFormat, VideoFrame, mock::MockFrameSource};
use support::{TestRfbClient, TestWebSocketRfbClient};
use tokio::{io::AsyncReadExt, net::TcpListener, sync::watch, task::JoinHandle, time::Duration};
use tokio_tungstenite::{connect_async, tungstenite::Error as WebSocketError};

/// 复刻正式 `src/main.rs` 的组装结构：TCP + HTTP/WS 共享 gate/source/event。
struct HeadlessAssembly {
    tcp_address: SocketAddr,
    http_address: SocketAddr,
    shutdown: watch::Sender<bool>,
    tcp_task: JoinHandle<Result<(), RfbTcpServerError>>,
    http_task: JoinHandle<Result<(), ipkvm_headless::web::HeadlessWebServiceError>>,
}

impl HeadlessAssembly {
    async fn start() -> Self {
        let tcp_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp_address = tcp_listener.local_addr().unwrap();
        let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_address = http_listener.local_addr().unwrap();

        let source = Arc::new(MockFrameSource::new());
        source.publish_frame(test_frame());

        let (shutdown, shutdown_rx) = watch::channel(false);
        let gate = RfbConnectionGate::new();

        // 会话管理器：构造并 start（内部 spawn 输入泵消费两个传输汇入的事件）。
        // `event_publisher()` 提供传输层共享的 watch 订阅端。
        let mut manager = SessionManager::new(
            Arc::clone(&source) as Arc<dyn ipkvm_video::FrameSource>,
            Ch9329InputSink::new(FakeCommandQueue::new(), 0, MouseMode::Absolute),
            gate.clone(),
        );
        manager.start().unwrap();
        let event_publisher = manager.event_publisher();
        let manager = Arc::new(tokio::sync::Mutex::new(manager));
        let factory = Arc::new(TestSessionFactory);
        let switchable_source = Arc::new(SwitchableFrameSource::new(
            Arc::clone(&source) as Arc<dyn ipkvm_video::FrameSource>
        ));

        // TCP 任务（clone gate）。
        let tcp_server = RfbTcpServer::new(
            tcp_listener,
            Arc::clone(&switchable_source),
            event_publisher.clone(),
            RfbTcpConfig::default(),
            gate.clone(),
        )
        .unwrap();
        let tcp_shutdown = shutdown_rx.clone();
        let tcp_task = tokio::spawn(async move { tcp_server.run(tcp_shutdown).await });

        // HTTP+WS 任务（move gate）：HeadlessWebService 提供静态资源 + /rfb。
        let web_service = HeadlessWebService::new(
            switchable_source,
            manager,
            factory,
            event_publisher,
            RfbWebSocketConfig::default(),
            shutdown_rx.clone(),
            gate,
            None, // auth：未配置 token，仅放行本机来源（本测试不覆盖鉴权）
        )
        .unwrap();
        let http_task = tokio::spawn(async move { web_service.serve(http_listener).await });

        Self {
            tcp_address,
            http_address,
            shutdown,
            tcp_task,
            http_task,
        }
    }

    async fn stop(self) {
        let Self {
            shutdown,
            tcp_task,
            http_task,
            ..
        } = self;
        shutdown.send(true).unwrap();
        tcp_task.await.unwrap().unwrap();
        http_task.await.unwrap().unwrap();
    }

    async fn post_session(&self, body: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(self.http_address)
            .await
            .unwrap();
        let request = format!(
            "POST /api/session HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            self.http_address,
            body.len(),
            body
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        read_http_status_line(&mut stream).await
    }
}

/// 会话工厂：`POST /api/session {action:"create"}` 首启时按当前帧源/sink 构造。
/// 本测试不触发该路径，工厂仅占位以满足 `HeadlessWebService::new` 签名。
struct TestSessionFactory;

impl SessionFactory<Ch9329InputSink<FakeCommandQueue>> for TestSessionFactory {
    fn build(
        &self,
        _selection: &SessionSelection,
    ) -> Result<
        (
            Arc<dyn ipkvm_video::FrameSource>,
            Ch9329InputSink<FakeCommandQueue>,
        ),
        String,
    > {
        let source = Arc::new(MockFrameSource::new());
        source.publish_frame(test_frame());
        Ok((
            source as Arc<dyn ipkvm_video::FrameSource>,
            Ch9329InputSink::new(FakeCommandQueue::new(), 0, MouseMode::Absolute),
        ))
    }
}

fn test_frame() -> Arc<VideoFrame> {
    Arc::new(VideoFrame::new(
        1,
        MonotonicTimestamp::from_nanos(1),
        2,
        2,
        8,
        PixelFormat::Bgra8888,
        // 两行各 8 字节（2 像素 × 4 字节 BGRA），无行填充。
        Arc::from([
            10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255,
        ]),
    ))
}

/// 读取 HTTP 响应的起始行和首部，返回状态行。
async fn read_http_status_line(stream: &mut tokio::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = stream.read(&mut byte).await.unwrap();
        if n == 0 {
            break;
        }
        buf.push(byte[0]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let text = String::from_utf8(buf).unwrap();
    text.lines().next().unwrap().to_string()
}

#[tokio::test]
async fn tcp_transport_serves_rfb_banner() {
    let system = HeadlessAssembly::start().await;

    let mut client = TestRfbClient::connect(system.tcp_address).await;
    let banner = client.read_banner().await.unwrap();
    assert_eq!(&banner, b"RFB 003.008\n");

    system.stop().await;
}

#[tokio::test]
async fn http_root_serves_embedded_novnc_console() {
    let system = HeadlessAssembly::start().await;

    let mut stream = tokio::net::TcpStream::connect(system.http_address)
        .await
        .unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let status = read_http_status_line(&mut stream).await;
    assert!(status.starts_with("HTTP/1.1 200"), "root status: {status}");

    // 读取完整响应体，断言含中文控制台页面标志。
    let mut body = Vec::new();
    stream.read_to_end(&mut body).await.unwrap();
    let body = String::from_utf8(body).unwrap();
    assert!(
        body.contains("lang=\"zh-CN\""),
        "missing zh-CN console root"
    );
    assert!(
        body.contains("/assets/app.js"),
        "page does not load the console application script"
    );
    assert!(
        body.contains("id=\"console\""),
        "page does not contain the console root element"
    );

    system.stop().await;
}

#[tokio::test]
async fn websocket_upgrade_reaches_rfb_handshake() {
    let system = HeadlessAssembly::start().await;

    let (mut socket, _response) = connect_async(format!("ws://{}/rfb", system.http_address))
        .await
        .expect("WebSocket upgrade should succeed when gate is free");

    // noVNC 裸 WebSocket：收到 RFB banner 证明进入协议握手。
    use tokio_tungstenite::tungstenite::Message;
    let banner = match socket.next().await {
        Some(Ok(Message::Binary(bytes))) => bytes,
        Some(other) => panic!("expected binary banner, got {other:?}"),
        None => panic!("WebSocket closed before banner"),
    };
    assert_eq!(&banner[..], b"RFB 003.008\n");

    system.stop().await;
}

#[tokio::test]
async fn active_tcp_blocks_websocket_with_conflict() {
    let system = HeadlessAssembly::start().await;

    // TCP 先握手，占用 gate。
    let mut tcp = TestRfbClient::connect(system.tcp_address).await;
    tcp.handshake(true).await;

    // 此时 WebSocket 升级应被拒绝（gate 被占用）。
    let error = match connect_async(format!("ws://{}/rfb", system.http_address)).await {
        Ok(_) => panic!("WebSocket upgrade succeeded while TCP held the gate"),
        Err(error) => error,
    };
    let status = match error {
        WebSocketError::Http(response) => response.status(),
        other => panic!("expected HTTP rejection, got {other:?}"),
    };
    assert_eq!(status, axum::http::StatusCode::CONFLICT);

    drop(tcp);
    system.stop().await;
}

#[tokio::test]
async fn session_restart_while_tcp_active_releases_gate_and_keeps_tcp_server_alive() {
    let system = HeadlessAssembly::start().await;

    let mut tcp = TestRfbClient::connect(system.tcp_address).await;
    tcp.handshake(true).await;

    let status = system.post_session(r#"{"action":"restart"}"#).await;
    assert!(
        status.starts_with("HTTP/1.1 200"),
        "restart status: {status}"
    );
    let old_closed = tokio::time::timeout(Duration::from_secs(1), tcp.read_one())
        .await
        .expect("active TCP connection should close during session restart")
        .expect("old TCP read should complete");
    assert_eq!(old_closed, 0, "old TCP connection should reach EOF");

    let mut next = TestRfbClient::connect(system.tcp_address).await;
    assert_eq!(next.read_banner().await.unwrap(), *b"RFB 003.008\n");

    system.stop().await;
}

#[tokio::test]
async fn session_restart_while_websocket_active_releases_gate_for_new_websocket() {
    let system = HeadlessAssembly::start().await;

    let (socket, _response) = connect_async(format!("ws://{}/rfb", system.http_address))
        .await
        .expect("initial WebSocket upgrade should succeed");
    let mut websocket = TestWebSocketRfbClient::new(socket);
    websocket.handshake(true).await;

    let status = system.post_session(r#"{"action":"restart"}"#).await;
    assert!(
        status.starts_with("HTTP/1.1 200"),
        "restart status: {status}"
    );
    let _old_end = tokio::time::timeout(Duration::from_secs(1), websocket.read_message())
        .await
        .expect("active WebSocket should close during session restart");

    let (socket, _response) = connect_async(format!("ws://{}/rfb", system.http_address))
        .await
        .expect("new WebSocket upgrade should succeed after restart");
    let mut next = TestWebSocketRfbClient::new(socket);
    assert_eq!(next.read_banner().await, *b"RFB 003.008\n");

    system.stop().await;
}

#[tokio::test]
async fn shutdown_stops_both_transports_cleanly() {
    let system = HeadlessAssembly::start().await;
    let tcp_address = system.tcp_address;

    system.stop().await;

    // 关闭后新 TCP 连接应被拒绝（监听已关闭）。
    let after = tokio::net::TcpStream::connect(tcp_address).await;
    assert!(
        after.is_err(),
        "TCP listener should be closed after shutdown"
    );
}

// re-export 用于 write_all 的 trait。
use tokio::io::AsyncWriteExt;

/// `--list-cameras` 枚举成功即退出 0，即使枚举到 0 台设备（避免依赖 OBS 是否在运行）。
/// 仅 Windows 断言：非 Windows 的 stub 返回 UnsupportedPlatform、退出非 0，属预期行为。
#[test]
fn headless_list_cameras_succeeds_on_windows() {
    #[cfg(windows)]
    {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_ipkvm-headless"))
            .arg("--list-cameras")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "--list-cameras failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("camera(s)"),
            "--list-cameras should report camera count, got: {stdout}"
        );
    }
}

/// `--camera <不存在的设备>`：Windows 上设备未找到、非 Windows 上
/// UnsupportedPlatform——两种情况都应非 0 退出（跨平台可测）。
#[test]
fn headless_camera_with_unknown_device_fails() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ipkvm-headless"))
        .arg("--camera")
        .arg("0:no-such-camera")
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "--camera with an unknown device should fail, stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// `--assets` 与 `--camera` 互斥：参数错误退出码 2。
#[test]
fn headless_assets_and_camera_are_mutually_exclusive() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ipkvm-headless"))
        .arg("--assets")
        .arg("some/dir")
        .arg("--camera")
        .arg("OBS Virtual Camera")
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "--assets with --camera should exit 2, stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
