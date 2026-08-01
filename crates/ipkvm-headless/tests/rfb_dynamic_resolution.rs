mod support;

use std::sync::Arc;

use ipkvm_headless::rfb_connection::{RfbConnectionGate, RfbServerEvent};
use ipkvm_headless::rfb_tcp::{RfbTcpConfig, RfbTcpServer};
use ipkvm_video::looping::LoopingVideoSource;
use ipkvm_video::y4m::Y4mAsset;
use support::TestRfbClient;
use tokio::{
    net::TcpListener,
    sync::{mpsc, watch},
    time::{Duration, timeout},
};

fn asset(width: u32, height: u32, luminance: u8, frame_count: usize) -> Y4mAsset {
    let y_len = (width * height) as usize;
    let uv_len = (width.div_ceil(2) * height.div_ceil(2)) as usize;
    let mut bytes = format!("YUV4MPEG2 W{width} H{height} F10:1 Ip A1:1 C420\n").into_bytes();
    for _ in 0..frame_count {
        bytes.extend_from_slice(b"FRAME\n");
        bytes.extend(std::iter::repeat_n(luminance, y_len));
        bytes.extend(std::iter::repeat_n(128, 2 * uv_len));
    }
    Y4mAsset::parse(&bytes).unwrap()
}

struct ServerFixture {
    address: std::net::SocketAddr,
    events: mpsc::Receiver<RfbServerEvent>,
    shutdown: watch::Sender<bool>,
    task: tokio::task::JoinHandle<Result<(), ipkvm_headless::rfb_tcp::RfbTcpServerError>>,
}

impl ServerFixture {
    async fn start(source: Arc<LoopingVideoSource>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (event_tx, events) = mpsc::channel(16);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let server = RfbTcpServer::new(
            listener,
            source,
            event_tx,
            RfbTcpConfig::default(),
            RfbConnectionGate::new(),
        )
        .unwrap();
        let task = tokio::spawn(server.run(shutdown_rx));
        Self {
            address,
            events,
            shutdown,
            task,
        }
    }

    async fn expect_connected(&mut self) {
        match self.events.recv().await.unwrap() {
            RfbServerEvent::Connected { .. } => {}
            event => panic!("expected connected event, got {event:?}"),
        }
    }

    async fn stop(self) {
        self.shutdown.send(true).unwrap();
        self.task.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn looping_source_announces_desktop_size_over_real_tcp() {
    let small = asset(4, 2, 0, 100);
    let large = asset(2, 4, 255, 100);
    let source = Arc::new(LoopingVideoSource::new(vec![small, large], 1_000).unwrap());
    let mut fixture = ServerFixture::start(Arc::clone(&source)).await;

    let mut client = TestRfbClient::connect(fixture.address).await;
    let server_init = client.handshake(true).await;
    let (initial_width, initial_height) = (server_init.width, server_init.height);
    let (target_width, target_height) = if (initial_width, initial_height) == (4, 2) {
        (2, 4)
    } else {
        assert_eq!(
            (initial_width, initial_height),
            (2, 4),
            "初始尺寸应为 4x2 或 2x4"
        );
        (4, 2)
    };
    fixture.expect_connected().await;

    // 声明 Raw 与 DesktopSize 伪编码；未协商时服务端按协议拒绝尺寸变化。
    client
        .send_raw(&[2, 0, 0, 2, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0x21])
        .await;

    client
        .request_update(false, 0, 0, initial_width, initial_height)
        .await;
    let initial = client
        .read_update(usize::from(initial_width) * usize::from(initial_height) * 4)
        .await;
    assert_eq!(initial.encoding, 0);
    assert_eq!(
        (initial.width, initial.height),
        (initial_width, initial_height)
    );

    let desktop_size = timeout(Duration::from_secs(10), async {
        loop {
            client
                .request_update(true, 0, 0, initial_width, initial_height)
                .await;
            let update = client.read_update_any().await;
            if update.encoding == -223 {
                return update;
            }
        }
    })
    .await
    .expect("timed out waiting for DesktopSize");

    assert_eq!(
        (
            desktop_size.x,
            desktop_size.y,
            desktop_size.width,
            desktop_size.height
        ),
        (0, 0, target_width, target_height)
    );

    client
        .request_update(false, 0, 0, target_width, target_height)
        .await;
    let resized = client
        .read_update(usize::from(target_width) * usize::from(target_height) * 4)
        .await;
    assert_eq!(resized.encoding, 0);
    assert_eq!(
        (resized.width, resized.height),
        (target_width, target_height)
    );
    let luminance = if (target_width, target_height) == (2, 4) {
        255
    } else {
        0
    };
    let pixel_count = usize::from(target_width) * usize::from(target_height);
    assert_eq!(
        resized.pixels,
        [luminance, luminance, luminance, 0].repeat(pixel_count)
    );

    fixture.stop().await;
}
