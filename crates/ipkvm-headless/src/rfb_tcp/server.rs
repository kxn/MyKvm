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
    RfbClientId, RfbConnectionGate, RfbConnectionGateError, RfbConnectionPermit,
    RfbDisconnectReason, RfbServerEvent,
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
            self.finish_connection(permit, client_id, peer_addr, reason)
                .await?;
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

    async fn finish_connection(
        &self,
        permit: RfbConnectionPermit,
        client_id: RfbClientId,
        peer_addr: std::net::SocketAddr,
        reason: RfbDisconnectReason,
    ) -> Result<(), RfbTcpServerError> {
        self.send_disconnected(client_id, peer_addr, reason).await?;
        drop(permit);
        Ok(())
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
    use std::{sync::Arc, task::Poll};

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
    async fn gate_stays_busy_while_disconnected_event_waits_for_capacity() {
        let (server, mut event_rx) = make_server().await;
        let gate = server.gate.clone();
        let peer_addr = "127.0.0.1:5900".parse().unwrap();
        server
            .event_tx
            .send(RfbServerEvent::Key {
                client_id: RfbClientId(99),
                down: true,
                keysym: 0x41,
            })
            .await
            .unwrap();

        let permit = gate.acquire().await.unwrap();
        let client_id = permit.client_id();
        let finish = server.finish_connection(
            permit,
            client_id,
            peer_addr,
            RfbDisconnectReason::ClientClosed,
        );
        tokio::pin!(finish);
        std::future::poll_fn(|context| match finish.as_mut().poll(context) {
            Poll::Pending => Poll::Ready(()),
            Poll::Ready(result) => panic!("full event channel completed disconnect: {result:?}"),
        })
        .await;

        assert_eq!(
            gate.try_acquire().unwrap_err(),
            RfbConnectionGateError::Busy
        );

        assert!(matches!(
            event_rx.recv().await,
            Some(RfbServerEvent::Key { .. })
        ));
        finish.await.unwrap();
        assert!(matches!(
            event_rx.recv().await,
            Some(RfbServerEvent::Disconnected {
                client_id: actual_client_id,
                peer_addr: actual_peer_addr,
                reason: RfbDisconnectReason::ClientClosed,
            }) if actual_client_id == client_id && actual_peer_addr == peer_addr
        ));
        assert_eq!(gate.try_acquire().unwrap().client_id().get(), 2);
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
