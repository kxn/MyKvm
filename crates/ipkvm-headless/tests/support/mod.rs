use std::{io, net::SocketAddr};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

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

    async fn read_exact(&mut self, length: usize) -> Vec<u8> {
        let mut bytes = vec![0; length];
        self.stream.read_exact(&mut bytes).await.unwrap();
        bytes
    }
}
