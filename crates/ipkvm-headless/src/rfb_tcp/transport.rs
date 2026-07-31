use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use crate::rfb_connection::{RfbTransport, RfbTransportError, RfbTransportRead};

pub(super) struct TcpTransport {
    stream: TcpStream,
    read_buffer_bytes: usize,
}

impl TcpTransport {
    pub(super) fn new(stream: TcpStream, read_buffer_bytes: usize) -> Self {
        Self {
            stream,
            read_buffer_bytes,
        }
    }
}

impl RfbTransport for TcpTransport {
    async fn receive_into(
        &mut self,
        buffer: &mut Vec<u8>,
    ) -> Result<RfbTransportRead, RfbTransportError> {
        buffer.clear();
        buffer.resize(self.read_buffer_bytes, 0);
        let count = self.stream.read(buffer).await?;
        buffer.truncate(count);
        if count == 0 {
            Ok(RfbTransportRead::Closed)
        } else {
            Ok(RfbTransportRead::Data)
        }
    }

    async fn send_binary(&mut self, bytes: Vec<u8>) -> Result<(), RfbTransportError> {
        self.stream.write_all(&bytes).await?;
        Ok(())
    }

    async fn close(&mut self) {
        let _ = self.stream.shutdown().await;
    }
}
