use std::sync::Arc;

use ipkvm_video::FrameSource;
use tokio::{
    net::TcpListener,
    sync::{mpsc, watch},
};

use super::{
    RfbClientId, RfbDisconnectReason, RfbTcpConfig, RfbTcpEvent, RfbTcpServerError,
    connection::{ConnectionEnd, RfbTcpConnectionError, run_connection},
};

pub struct RfbTcpServer<S> {
    listener: TcpListener,
    frame_source: Arc<S>,
    event_tx: mpsc::Sender<RfbTcpEvent>,
    config: RfbTcpConfig,
    next_client_id: Option<u64>,
}

impl<S: FrameSource + 'static> RfbTcpServer<S> {
    pub fn new(
        listener: TcpListener,
        frame_source: Arc<S>,
        event_tx: mpsc::Sender<RfbTcpEvent>,
        config: RfbTcpConfig,
    ) -> Result<Self, RfbTcpServerError> {
        config.validate()?;
        Ok(Self {
            listener,
            frame_source,
            event_tx,
            config,
            next_client_id: Some(1),
        })
    }

    pub async fn run(
        mut self,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), RfbTcpServerError> {
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
            let client_id = self.allocate_client_id()?;
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
            if matches!(&end, ConnectionEnd::ServerShutdown) {
                return Ok(());
            }
        }
    }

    fn allocate_client_id(&mut self) -> Result<RfbClientId, RfbTcpServerError> {
        let value = self
            .next_client_id
            .take()
            .ok_or(RfbTcpServerError::ClientIdOverflow)?;
        self.next_client_id = value.checked_add(1);
        Ok(RfbClientId(value))
    }

    async fn send_disconnected(
        &self,
        client_id: RfbClientId,
        peer_addr: std::net::SocketAddr,
        reason: RfbDisconnectReason,
    ) -> Result<(), RfbTcpServerError> {
        self.event_tx
            .send(RfbTcpEvent::Disconnected {
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

    async fn make_server() -> (RfbTcpServer<MockFrameSource>, mpsc::Receiver<RfbTcpEvent>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (event_tx, event_rx) = mpsc::channel(1);
        (
            RfbTcpServer::new(
                listener,
                Arc::new(MockFrameSource::new()),
                event_tx,
                RfbTcpConfig::default(),
            )
            .unwrap(),
            event_rx,
        )
    }

    #[tokio::test]
    async fn client_id_allocation_never_wraps() {
        let (mut server, _event_rx) = make_server().await;
        server.next_client_id = Some(u64::MAX);

        assert_eq!(server.allocate_client_id().unwrap().get(), u64::MAX);
        assert!(matches!(
            server.allocate_client_id(),
            Err(RfbTcpServerError::ClientIdOverflow)
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
