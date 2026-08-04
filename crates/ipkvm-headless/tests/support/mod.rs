#![allow(dead_code)]

use std::{
    io,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use futures_util::{SinkExt, StreamExt};
use ipkvm_headless::settings::SettingsStore;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Error as WebSocketError, Message},
};

/// 测试用独立设置存储（进程内自增目录，避免并行测试互踩）。
pub fn temp_settings_store() -> Arc<SettingsStore> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ipkvm-headless-settings-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    Arc::new(SettingsStore::load_from(dir).0)
}

#[derive(Debug, Eq, PartialEq)]
pub struct ServerInit {
    pub width: u16,
    pub height: u16,
    pub name: String,
}

#[derive(Debug, Eq, PartialEq)]
pub struct FramebufferUpdate {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub encoding: i32,
    pub pixels: Vec<u8>,
}

pub struct TestRfbClient {
    stream: TcpStream,
}

pub type ClientWebSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct TestWebSocketRfbClient {
    socket: ClientWebSocket,
}

impl TestRfbClient {
    pub async fn connect(address: SocketAddr) -> Self {
        Self {
            stream: TcpStream::connect(address).await.unwrap(),
        }
    }

    pub async fn read_banner(&mut self) -> io::Result<[u8; 12]> {
        let mut banner = [0; 12];
        self.stream.read_exact(&mut banner).await?;
        Ok(banner)
    }

    pub async fn handshake(&mut self, shared: bool) -> ServerInit {
        assert_eq!(self.read_banner().await.unwrap(), *b"RFB 003.008\n");
        self.stream.write_all(b"RFB 003.008\n").await.unwrap();
        assert_eq!(self.read_exact(2).await, [1, 1]);
        self.stream.write_all(&[1]).await.unwrap();
        assert_eq!(self.read_exact(4).await, [0, 0, 0, 0]);
        self.stream.write_all(&[u8::from(shared)]).await.unwrap();

        let header = self.read_exact(24).await;
        let width = u16::from_be_bytes([header[0], header[1]]);
        let height = u16::from_be_bytes([header[2], header[3]]);
        let name_length =
            u32::from_be_bytes([header[20], header[21], header[22], header[23]]) as usize;
        let name = String::from_utf8(self.read_exact(name_length).await).unwrap();
        ServerInit {
            width,
            height,
            name,
        }
    }

    pub async fn set_rgb565(&mut self) {
        let mut message = vec![0, 0, 0, 0, 16, 16, 0, 1];
        message.extend_from_slice(&31_u16.to_be_bytes());
        message.extend_from_slice(&63_u16.to_be_bytes());
        message.extend_from_slice(&31_u16.to_be_bytes());
        message.extend_from_slice(&[11, 5, 0, 0, 0, 0]);
        self.stream.write_all(&message).await.unwrap();
    }

    pub async fn request_update(
        &mut self,
        incremental: bool,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
    ) {
        let mut message = vec![3, u8::from(incremental)];
        message.extend_from_slice(&x.to_be_bytes());
        message.extend_from_slice(&y.to_be_bytes());
        message.extend_from_slice(&width.to_be_bytes());
        message.extend_from_slice(&height.to_be_bytes());
        self.stream.write_all(&message).await.unwrap();
    }

    pub async fn read_update(&mut self, pixel_bytes: usize) -> FramebufferUpdate {
        let header = self.read_exact(16).await;
        assert_eq!(&header[..4], &[0, 0, 0, 1]);
        FramebufferUpdate {
            x: u16::from_be_bytes([header[4], header[5]]),
            y: u16::from_be_bytes([header[6], header[7]]),
            width: u16::from_be_bytes([header[8], header[9]]),
            height: u16::from_be_bytes([header[10], header[11]]),
            encoding: i32::from_be_bytes([header[12], header[13], header[14], header[15]]),
            pixels: self.read_exact(pixel_bytes).await,
        }
    }

    /// 读取一条更新，Raw 按 4 字节像素、`DesktopSize` 按 0 像素读取。
    pub async fn read_update_any(&mut self) -> FramebufferUpdate {
        let header = self.read_exact(16).await;
        assert_eq!(&header[..4], &[0, 0, 0, 1]);
        let width = u16::from_be_bytes([header[8], header[9]]);
        let height = u16::from_be_bytes([header[10], header[11]]);
        let encoding = i32::from_be_bytes([header[12], header[13], header[14], header[15]]);
        let pixel_bytes = if encoding == 0 {
            usize::from(width) * usize::from(height) * 4
        } else {
            0
        };
        FramebufferUpdate {
            x: u16::from_be_bytes([header[4], header[5]]),
            y: u16::from_be_bytes([header[6], header[7]]),
            width,
            height,
            encoding,
            pixels: self.read_exact(pixel_bytes).await,
        }
    }

    pub async fn send_key(&mut self, down: bool, keysym: u32) {
        let mut message = vec![4, u8::from(down), 0, 0];
        message.extend_from_slice(&keysym.to_be_bytes());
        self.stream.write_all(&message).await.unwrap();
    }

    pub async fn send_pointer(&mut self, buttons: u8, x: u16, y: u16) {
        let mut message = vec![5, buttons];
        message.extend_from_slice(&x.to_be_bytes());
        message.extend_from_slice(&y.to_be_bytes());
        self.stream.write_all(&message).await.unwrap();
    }

    pub async fn send_cut_text(&mut self, bytes: &[u8]) {
        let mut message = vec![6, 0, 0, 0];
        message.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        message.extend_from_slice(bytes);
        self.stream.write_all(&message).await.unwrap();
    }

    pub async fn send_raw(&mut self, bytes: &[u8]) {
        self.stream.write_all(bytes).await.unwrap();
    }

    pub fn try_read(&self, bytes: &mut [u8]) -> io::Result<usize> {
        self.stream.try_read(bytes)
    }

    pub async fn read_one(&mut self) -> io::Result<usize> {
        let mut byte = [0; 1];
        self.stream.read(&mut byte).await
    }

    async fn read_exact(&mut self, length: usize) -> Vec<u8> {
        let mut bytes = vec![0; length];
        self.stream.read_exact(&mut bytes).await.unwrap();
        bytes
    }
}

impl TestWebSocketRfbClient {
    pub fn new(socket: ClientWebSocket) -> Self {
        Self { socket }
    }

    pub async fn read_banner(&mut self) -> [u8; 12] {
        self.read_binary().await.try_into().unwrap()
    }

    pub async fn handshake(&mut self, shared: bool) -> ServerInit {
        assert_eq!(self.read_banner().await, *b"RFB 003.008\n");
        self.send_binary(b"RFB 003.008\n").await;
        assert_eq!(self.read_binary().await, [1, 1]);
        self.send_binary(&[1]).await;
        assert_eq!(self.read_binary().await, [0, 0, 0, 0]);
        self.send_binary(&[u8::from(shared)]).await;

        let message = self.read_binary().await;
        assert!(message.len() >= 24);
        let width = u16::from_be_bytes([message[0], message[1]]);
        let height = u16::from_be_bytes([message[2], message[3]]);
        let name_length =
            u32::from_be_bytes([message[20], message[21], message[22], message[23]]) as usize;
        assert_eq!(message.len(), 24 + name_length);
        let name = String::from_utf8(message[24..].to_vec()).unwrap();
        ServerInit {
            width,
            height,
            name,
        }
    }

    pub async fn request_update(
        &mut self,
        incremental: bool,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
    ) {
        let mut message = vec![3, u8::from(incremental)];
        message.extend_from_slice(&x.to_be_bytes());
        message.extend_from_slice(&y.to_be_bytes());
        message.extend_from_slice(&width.to_be_bytes());
        message.extend_from_slice(&height.to_be_bytes());
        self.send_binary(&message).await;
    }

    pub async fn set_pixel_format(&mut self, message: &[u8; 20]) {
        self.send_binary(message).await;
    }

    pub async fn set_encodings(&mut self, encodings: &[i32]) {
        let count = u16::try_from(encodings.len()).unwrap();
        let mut message = vec![2, 0];
        message.extend_from_slice(&count.to_be_bytes());
        for encoding in encodings {
            message.extend_from_slice(&encoding.to_be_bytes());
        }
        self.send_binary(&message).await;
    }

    pub async fn send_key(&mut self, down: bool, keysym: u32) {
        let mut message = vec![4, u8::from(down), 0, 0];
        message.extend_from_slice(&keysym.to_be_bytes());
        self.send_binary(&message).await;
    }

    pub async fn read_update(&mut self, pixel_bytes: usize) -> FramebufferUpdate {
        let message = self.read_binary().await;
        assert_eq!(message.len(), 16 + pixel_bytes);
        assert_eq!(&message[..4], &[0, 0, 0, 1]);
        FramebufferUpdate {
            x: u16::from_be_bytes([message[4], message[5]]),
            y: u16::from_be_bytes([message[6], message[7]]),
            width: u16::from_be_bytes([message[8], message[9]]),
            height: u16::from_be_bytes([message[10], message[11]]),
            encoding: i32::from_be_bytes([message[12], message[13], message[14], message[15]]),
            pixels: message[16..].to_vec(),
        }
    }

    pub async fn send_binary(&mut self, bytes: &[u8]) {
        self.socket
            .send(Message::Binary(bytes.to_vec().into()))
            .await
            .unwrap();
    }

    pub async fn send_text(&mut self, text: &str) {
        self.socket
            .send(Message::Text(text.to_string().into()))
            .await
            .unwrap();
    }

    pub async fn send_ping(&mut self, bytes: &[u8]) {
        self.socket
            .send(Message::Ping(bytes.to_vec().into()))
            .await
            .unwrap();
    }

    pub async fn close(&mut self) {
        self.socket.send(Message::Close(None)).await.unwrap();
    }

    pub async fn read_message(&mut self) -> Result<Message, WebSocketError> {
        self.socket
            .next()
            .await
            .unwrap_or(Err(WebSocketError::ConnectionClosed))
    }

    pub async fn read_binary(&mut self) -> Vec<u8> {
        match self.read_message().await.unwrap() {
            Message::Binary(bytes) => bytes.to_vec(),
            message => panic!("expected binary WebSocket message, got {message:?}"),
        }
    }
}
