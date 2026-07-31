use std::sync::Arc;

use ipkvm_video::FrameSource;
use tokio::{
    net::TcpListener,
    sync::{mpsc, watch},
};

use super::{RfbTcpConfig, RfbTcpServerError, transport::TcpTransport};
use crate::rfb_connection::{
    ConnectionEnd, RfbConnectionFinalizeError, RfbConnectionGate, RfbConnectionGateError,
    RfbServerEvent, finalize_connection, run_managed_connection,
};

pub struct RfbTcpServer<S> {
    listener: TcpListener,
    frame_source: Arc<S>,
    event_tx: mpsc::Sender<RfbServerEvent>,
    config: RfbTcpConfig,
    gate: RfbConnectionGate,
}

impl<S: FrameSource + 'static> RfbTcpServer<S> {
    pub fn new(
        listener: TcpListener,
        frame_source: Arc<S>,
        event_tx: mpsc::Sender<RfbServerEvent>,
        config: RfbTcpConfig,
        gate: RfbConnectionGate,
    ) -> Result<Self, RfbTcpServerError> {
        config.validate()?;
        Ok(Self {
            listener,
            frame_source,
            event_tx,
            config,
            gate,
        })
    }

    /// 顺序服务客户端，直到收到关闭信号或发生 server 级错误。
    ///
    /// 调用方在本方法返回前必须持续消费事件通道。事件交付采用无损反压；
    /// 如果接收端停止消费且通道已满，连接关闭和 `Disconnected` 事件也会等待容量。
    pub async fn run(self, mut shutdown: watch::Receiver<bool>) -> Result<(), RfbTcpServerError> {
        loop {
            if shutdown_is_requested(&shutdown) {
                return Ok(());
            }

            let (stream, peer_addr) = tokio::select! {
                result = self.listener.accept() => {
                    result.map_err(RfbTcpServerError::Accept)?
                }
                _ = wait_for_shutdown(&mut shutdown) => return Ok(()),
                _ = self.event_tx.closed() => {
                    return Err(RfbTcpServerError::EventChannelClosed);
                }
            };
            let reservation = tokio::select! {
                result = self.gate.acquire() => {
                    result.map_err(|error| match error {
                        RfbConnectionGateError::ClientIdOverflow => RfbTcpServerError::ClientIdOverflow,
                        RfbConnectionGateError::Poisoned => RfbTcpServerError::ConnectionGatePoisoned,
                        RfbConnectionGateError::Busy => unreachable!("awaited gate acquisition cannot be busy"),
                    })?
                }
                _ = wait_for_shutdown(&mut shutdown) => return Ok(()),
                _ = self.event_tx.closed() => return Err(RfbTcpServerError::EventChannelClosed),
            };
            let completion = run_managed_connection(
                reservation,
                peer_addr,
                TcpTransport::new(stream, self.config.read_buffer_bytes),
                self.frame_source.subscribe(),
                self.event_tx.clone(),
                self.config.connection.clone(),
                shutdown.clone(),
            )
            .await;
            let end = finalize_connection(&self.event_tx, completion).await?;
            if matches!(&end, ConnectionEnd::ServerShutdown) {
                return Ok(());
            }
        }
    }
}

impl From<RfbConnectionFinalizeError> for RfbTcpServerError {
    fn from(error: RfbConnectionFinalizeError) -> Self {
        match error {
            RfbConnectionFinalizeError::EventChannelClosed => Self::EventChannelClosed,
        }
    }
}

fn shutdown_is_requested(shutdown: &watch::Receiver<bool>) -> bool {
    *shutdown.borrow()
}

async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    loop {
        if shutdown_is_requested(shutdown) {
            return;
        }
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ipkvm_video::{MonotonicTimestamp, PixelFormat, VideoFrame, mock::MockFrameSource};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::{mpsc, watch},
    };

    use super::*;

    async fn make_server() -> (
        RfbTcpServer<MockFrameSource>,
        mpsc::Receiver<RfbServerEvent>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (event_tx, event_rx) = mpsc::channel(1);
        let frame_source = Arc::new(MockFrameSource::new());
        frame_source.publish_frame(Arc::new(VideoFrame::new(
            1,
            MonotonicTimestamp::from_nanos(1),
            1,
            1,
            4,
            PixelFormat::Bgra8888,
            Arc::from([0_u8, 0, 0, 0]),
        )));
        (
            RfbTcpServer::new(
                listener,
                frame_source,
                event_tx,
                RfbTcpConfig::default(),
                RfbConnectionGate::new(),
            )
            .unwrap(),
            event_rx,
        )
    }

    #[tokio::test]
    async fn aborting_connected_owner_poisons_gate() {
        let (server, mut event_rx) = make_server().await;
        let address = server.listener.local_addr().unwrap();
        let gate = server.gate.clone();
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let owner = tokio::spawn(server.run(shutdown_rx));
        let mut stream = TcpStream::connect(address).await.unwrap();
        let mut banner = [0_u8; 12];
        stream.read_exact(&mut banner).await.unwrap();
        assert_eq!(&banner, b"RFB 003.008\n");
        stream.write_all(b"RFB 003.008\n").await.unwrap();
        let mut security_types = [0_u8; 2];
        stream.read_exact(&mut security_types).await.unwrap();
        assert_eq!(security_types, [1, 1]);
        stream.write_all(&[1]).await.unwrap();
        let mut security_result = [0_u8; 4];
        stream.read_exact(&mut security_result).await.unwrap();
        assert_eq!(security_result, [0; 4]);
        stream.write_all(&[1]).await.unwrap();
        let mut server_init = [0_u8; 24];
        stream.read_exact(&mut server_init).await.unwrap();
        let name_length = u32::from_be_bytes(server_init[20..24].try_into().unwrap()) as usize;
        let mut name = vec![0_u8; name_length];
        stream.read_exact(&mut name).await.unwrap();
        assert_eq!(name, b"my_ipkvm");
        assert!(matches!(
            event_rx.recv().await,
            Some(RfbServerEvent::Connected { .. })
        ));

        owner.abort();
        assert!(owner.await.unwrap_err().is_cancelled());
        assert_eq!(
            gate.try_acquire().unwrap_err(),
            RfbConnectionGateError::Poisoned
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let poisoned_address = listener.local_addr().unwrap();
        let frame_source = Arc::new(MockFrameSource::new());
        frame_source.publish_frame(Arc::new(VideoFrame::new(
            1,
            MonotonicTimestamp::from_nanos(1),
            1,
            1,
            4,
            PixelFormat::Bgra8888,
            Arc::from([0_u8, 0, 0, 0]),
        )));
        let (event_tx, _event_rx) = mpsc::channel(1);
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let poisoned_server = RfbTcpServer::new(
            listener,
            frame_source,
            event_tx,
            RfbTcpConfig::default(),
            gate,
        )
        .unwrap();
        let poisoned_task = tokio::spawn(poisoned_server.run(shutdown_rx));
        let _stream = TcpStream::connect(poisoned_address).await.unwrap();
        assert!(matches!(
            poisoned_task.await.unwrap(),
            Err(RfbTcpServerError::ConnectionGatePoisoned)
        ));
    }

    #[tokio::test]
    async fn initial_or_closed_shutdown_stops_before_accept() {
        let (_shutdown_tx, shutdown_rx) = watch::channel(true);
        let (server, _event_rx) = make_server().await;
        assert!(server.run(shutdown_rx).await.is_ok());

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        drop(shutdown_tx);
        let (server, _event_rx) = make_server().await;
        assert!(server.run(shutdown_rx).await.is_ok());
    }
}
