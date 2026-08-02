use std::{any::type_name, sync::Arc, time::Duration};

use ipkvm_core::{
    Ch9329InputSink, CommandQueue, InputResult, InputSink, KeyEvent, MouseMode, PointerEvent,
    fake_serial::FakeCommandQueue,
};
use ipkvm_headless::rfb_input::{
    RfbControllerReleaseReason, RfbInputError, RfbInputEventError, RfbInputEventKind,
    RfbInputLifecycleError, RfbInputNotice, RfbInputOperation, RfbInputPump, RfbInputRunError,
    RfbKeyboardRejection,
};
use ipkvm_headless::{
    rfb_connection::{RfbConnectionGate, RfbDisconnectReason},
    rfb_tcp::{RfbTcpConfig, RfbTcpServer},
};
use ipkvm_video::{MonotonicTimestamp, PixelFormat, VideoFrame, mock::MockFrameSource};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, watch},
};

struct EndToEndClient {
    stream: TcpStream,
}

impl EndToEndClient {
    async fn connect(address: std::net::SocketAddr) -> Self {
        Self {
            stream: TcpStream::connect(address).await.unwrap(),
        }
    }

    async fn handshake(&mut self) -> (u16, u16) {
        assert_eq!(self.read_exact(12).await, b"RFB 003.008\n");
        self.stream.write_all(b"RFB 003.008\n").await.unwrap();
        assert_eq!(self.read_exact(2).await, [1, 1]);
        self.stream.write_all(&[1]).await.unwrap();
        assert_eq!(self.read_exact(4).await, [0, 0, 0, 0]);
        self.stream.write_all(&[1]).await.unwrap();

        let header = self.read_exact(24).await;
        let width = u16::from_be_bytes([header[0], header[1]]);
        let height = u16::from_be_bytes([header[2], header[3]]);
        let name_length =
            u32::from_be_bytes([header[20], header[21], header[22], header[23]]) as usize;
        self.read_exact(name_length).await;
        (width, height)
    }

    async fn send_key(&mut self, down: bool, keysym: u32) {
        let mut message = vec![4, u8::from(down), 0, 0];
        message.extend_from_slice(&keysym.to_be_bytes());
        self.stream.write_all(&message).await.unwrap();
    }

    async fn send_pointer(&mut self, buttons: u8, x: u16, y: u16) {
        let mut message = vec![5, buttons];
        message.extend_from_slice(&x.to_be_bytes());
        message.extend_from_slice(&y.to_be_bytes());
        self.stream.write_all(&message).await.unwrap();
    }

    async fn read_exact(&mut self, length: usize) -> Vec<u8> {
        let mut bytes = vec![0; length];
        self.stream.read_exact(&mut bytes).await.unwrap();
        bytes
    }
}

struct NoopSink;

impl InputSink for NoopSink {
    fn set_mouse_mode(&mut self, _mode: MouseMode) -> InputResult<()> {
        Ok(())
    }

    fn handle_key_batch(&mut self, _events: &[KeyEvent]) -> InputResult<()> {
        Ok(())
    }

    fn handle_pointer_batch(&mut self, _events: &[PointerEvent]) -> InputResult<()> {
        Ok(())
    }

    fn release_all(&mut self) -> InputResult<()> {
        Ok(())
    }
}

#[test]
fn public_input_pump_contract_types_are_available() {
    // 契约可用性由上方 `use ipkvm_headless::rfb_input::…` 成功编译证明（headless
    // 重新导出 ipkvm_session 的类型）。std::any::type_name 返回类型**定义处**路径
    // （现为 ipkvm_session::rfb_input），与重新导出路径无关，因此不断言前缀。
    let names = [
        type_name::<RfbControllerReleaseReason>(),
        type_name::<RfbInputError>(),
        type_name::<RfbInputEventError>(),
        type_name::<RfbInputEventKind>(),
        type_name::<RfbInputLifecycleError>(),
        type_name::<RfbInputNotice>(),
        type_name::<RfbInputOperation>(),
        type_name::<RfbInputPump<NoopSink>>(),
        type_name::<RfbInputRunError>(),
        type_name::<RfbKeyboardRejection>(),
    ];

    assert!(names.iter().all(|name| !name.is_empty()));
}

#[tokio::test]
async fn real_tcp_client_drives_ch9329_input_and_disconnect_release() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let source = Arc::new(MockFrameSource::new());
    source.publish_frame(Arc::new(VideoFrame::new(
        1,
        MonotonicTimestamp::from_nanos(1),
        2,
        1,
        8,
        PixelFormat::Bgra8888,
        Arc::from(vec![0_u8; 8].into_boxed_slice()),
    )));
    let (event_tx, mut event_rx) = mpsc::channel(2);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server = RfbTcpServer::new(
        listener,
        Arc::clone(&source),
        event_tx,
        RfbTcpConfig::default(),
        RfbConnectionGate::new(),
    )
    .unwrap();
    let server_task = tokio::spawn(server.run(shutdown_rx));

    let queue = FakeCommandQueue::new();
    let pump_queue = queue.clone();
    let pump_task = tokio::spawn(async move {
        let sink = Ch9329InputSink::new(pump_queue, 0, MouseMode::Absolute);
        let mut pump = RfbInputPump::new(sink);
        let mut notices = Vec::new();
        pump.run(&mut event_rx, |notice| notices.push(notice.clone()))
            .await
            .unwrap();
        (notices, pump.active_client())
    });

    let mut client = EndToEndClient::connect(address).await;
    assert_eq!(client.handshake().await, (2, 1));
    client.send_key(true, 0x61).await;
    client.send_pointer(1, 1, 0).await;
    drop(client);

    // 键盘、指针、pump 释放、文本服务取消时的释放，共 4 个批次。
    tokio::time::timeout(Duration::from_secs(1), async {
        while queue.stats().batches_accepted < 4 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("RFB disconnect did not reach the input pump");

    shutdown_tx.send(true).unwrap();
    assert!(server_task.await.unwrap().is_ok());
    let (notices, active_client) = pump_task.await.unwrap();

    assert_eq!(active_client, None);
    assert_eq!(queue.accepted_batches().len(), 4);
    assert!(matches!(
        notices.as_slice(),
        [
            RfbInputNotice::ControllerAcquired { .. },
            RfbInputNotice::Keyboard { .. },
            RfbInputNotice::Pointer { .. },
            RfbInputNotice::ControllerReleased {
                reason: RfbControllerReleaseReason::Disconnected(RfbDisconnectReason::ClientClosed),
                ..
            },
        ]
    ));
    let release = queue.accepted_batches();
    let release_frames = release.last().unwrap().frames();
    assert_eq!(release_frames[0].data(), &[0; 8]);
    assert_eq!(release_frames[1].data()[1], 0);
}
