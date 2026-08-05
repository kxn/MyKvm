use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use ipkvm_session::rfb_connection::{RfbTransport, RfbTransportError, RfbTransportRead};

pub(super) struct TcpTransport {
    stream: TcpStream,
    read_buffer_bytes: usize,
}

impl TcpTransport {
    pub(super) fn new(stream: TcpStream, read_buffer_bytes: usize) -> Self {
        // 禁用 Nagle 算法：RFB 是请求/响应式小包协议，Nagle 的延迟合并会显著
        // 拖慢交互延迟。失败时忽略（NODELAY 是延迟优化，非正确性要求；loopback
        // 上几乎不会失败）。见调研阶段 1.1（issue #18）。
        let _ = stream.set_nodelay(true);
        Self {
            stream,
            read_buffer_bytes,
        }
    }

    /// 测试专用：读取底层 stream 的 nodelay 状态。
    #[cfg(test)]
    pub(super) fn nodelay(&self) -> std::io::Result<bool> {
        self.stream.nodelay()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn new_sets_tcp_nodelay() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let connect_task = tokio::spawn(async move { TcpStream::connect(addr).await.unwrap() });
        let (server_stream, _) = listener.accept().await.unwrap();
        let _client_stream = connect_task.await.unwrap();

        let transport = TcpTransport::new(server_stream, 4096);
        assert!(
            transport.nodelay().unwrap(),
            "TcpTransport::new 应设置 TCP_NODELAY"
        );
    }
}
