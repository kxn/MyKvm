use std::sync::Arc;

use ipkvm_video::FrameSource;
use tokio::{
    net::TcpListener,
    sync::{mpsc, watch},
};

use super::{
    RfbTcpConfig, RfbTcpServerError,
    connection::{ConnectionEnd, RfbTcpConnectionError, run_connection},
};
use crate::rfb_connection::{
    RfbClientId, RfbConnectionGate, RfbConnectionGateError, RfbDisconnectReason, RfbServerEvent,
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
            let permit = tokio::select! {
                result = self.gate.acquire() => {
                    result.map_err(|error| match error {
                        RfbConnectionGateError::ClientIdOverflow => RfbTcpServerError::ClientIdOverflow,
                        RfbConnectionGateError::Busy => unreachable!("awaited gate acquisition cannot be busy"),
                    })?
                }
                _ = wait_for_shutdown(&mut shutdown) => return Ok(()),
                _ = self.event_tx.closed() => return Err(RfbTcpServerError::EventChannelClosed),
            };
            let client_id = permit.client_id();
            let end = run_connection(
                client_id,
                peer_addr,
                stream,
                self.frame_source.subscribe(),
                self.event_tx.clone(),
                self.config.clone(),
                shutdown.clone(),
            )
            .await;

            if matches!(
                &end,
                ConnectionEnd::Failed(RfbTcpConnectionError::EventChannelClosed)
            ) {
                return Err(RfbTcpServerError::EventChannelClosed);
            }
            let reason = end.reason().ok_or(RfbTcpServerError::EventChannelClosed)?;
            self.send_disconnected(client_id, peer_addr, reason).await?;
            drop(permit);
            if matches!(&end, ConnectionEnd::ServerShutdown) {
                return Ok(());
            }
        }
    }

    async fn send_disconnected(
        &self,
        client_id: RfbClientId,
        peer_addr: std::net::SocketAddr,
        reason: RfbDisconnectReason,
    ) -> Result<(), RfbTcpServerError> {
        self.event_tx
            .send(RfbServerEvent::Disconnected {
                client_id,
                peer_addr,
                reason,
            })
            .await
            .map_err(|_| RfbTcpServerError::EventChannelClosed)
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

    use ipkvm_video::mock::MockFrameSource;
    use tokio::{
        net::TcpListener,
        sync::{mpsc, watch},
    };

    use super::*;

    async fn make_server() -> (
        RfbTcpServer<MockFrameSource>,
        mpsc::Receiver<RfbServerEvent>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (event_tx, event_rx) = mpsc::channel(1);
        (
            RfbTcpServer::new(
                listener,
                Arc::new(MockFrameSource::new()),
                event_tx,
                RfbTcpConfig::default(),
                RfbConnectionGate::new(),
            )
            .unwrap(),
            event_rx,
        )
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
