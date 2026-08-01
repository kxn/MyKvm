use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use futures_util::StreamExt;
use ipkvm_headless::{
    rfb_connection::{RfbConnectionGate, RfbServerEvent},
    rfb_ws::RfbWebSocketConfig,
    web::{HeadlessWebService, HeadlessWebServiceError},
};
use ipkvm_video::{
    MonotonicTimestamp, PixelFormat, SharedVideoFrame, VideoFrame, mock::MockFrameSource,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, watch},
    task::JoinHandle,
    time::timeout,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Error as WebSocketError, Message},
};

struct HttpResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

struct TestWebServer {
    address: SocketAddr,
    source: Arc<MockFrameSource>,
    shutdown: watch::Sender<bool>,
    task: JoinHandle<Result<(), HeadlessWebServiceError>>,
    _events: mpsc::Receiver<RfbServerEvent>,
}

impl TestWebServer {
    async fn start() -> Self {
        Self::start_with_frame(Some(test_frame())).await
    }

    async fn start_with_frame(frame: Option<SharedVideoFrame>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let source = Arc::new(MockFrameSource::new());
        if let Some(frame) = frame {
            source.publish_frame(frame);
        }
        let (event_tx, events) = mpsc::channel(32);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let service = HeadlessWebService::new(
            Arc::clone(&source),
            event_tx,
            RfbWebSocketConfig::default(),
            shutdown_rx,
            RfbConnectionGate::new(),
        )
        .unwrap();
        let task = tokio::spawn(service.serve(listener));
        Self {
            address,
            source,
            shutdown,
            task,
            _events: events,
        }
    }

    fn websocket_url(&self) -> String {
        format!("ws://{}/rfb", self.address)
    }

    async fn request(&self, method: &str, path: &str) -> HttpResponse {
        let mut stream = TcpStream::connect(self.address).await.unwrap();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            self.address
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        parse_http_response(&response)
    }

    async fn stop(self) {
        self.shutdown.send_replace(true);
        timeout(Duration::from_secs(2), self.task)
            .await
            .expect("headless web service did not stop")
            .expect("headless web service task panicked")
            .expect("headless web service failed");
    }
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
    assert_eq!(status["video"]["source"]["device_name"], "mock");
    assert_eq!(status["video"]["source"]["is_loop"], false);
    assert_eq!(status["video"]["frame"]["width"], 2);
    assert_eq!(status["video"]["frame"]["height"], 1);
    assert_eq!(status["video"]["frame"]["pixel_format"], "bgra8888");
    assert_eq!(status["video"]["frame"]["seq"], 1);
    assert_eq!(status["controller"]["active"], false);
    assert!(status["controller"]["client_id"].is_null());
    assert!(status["controller"]["transport"].is_null());
    assert!(status["controller"]["peer_addr"].is_null());
    assert!(status["controller"]["connected_since_ms"].is_null());

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
    let server = TestWebServer::start_with_frame(None).await;

    let response = server.request("GET", "/api/screenshot").await;

    assert_eq!(response.status, 503);
    assert!(response.body.is_empty());

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
