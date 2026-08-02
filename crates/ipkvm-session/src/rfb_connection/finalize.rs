use std::net::SocketAddr;

use ipkvm_video::FrameReceiver;
use thiserror::Error;
use tokio::sync::{mpsc, watch};

use super::{
    RfbConnectionSettings, RfbServerEvent, RfbTransport,
    driver::{ConnectionEnd, run_connection},
    gate::{RfbConnectionLease, RfbConnectionReservation},
};

#[derive(Debug)]
pub(crate) struct RfbConnectionCompletion {
    end: ConnectionEnd,
    lease: RfbConnectionLease,
    peer_addr: SocketAddr,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub(crate) enum RfbConnectionFinalizeError {
    #[error("RFB event receiver is closed")]
    EventChannelClosed,
}

pub(crate) async fn run_managed_connection<T: RfbTransport>(
    reservation: RfbConnectionReservation,
    peer_addr: SocketAddr,
    transport: T,
    frame_rx: FrameReceiver,
    event_tx: mpsc::Sender<RfbServerEvent>,
    settings: RfbConnectionSettings,
    shutdown: watch::Receiver<bool>,
) -> RfbConnectionCompletion {
    let lease = reservation.activate();
    let end = run_connection(
        lease.client_id(),
        peer_addr,
        transport,
        frame_rx,
        event_tx,
        settings,
        shutdown,
    )
    .await;
    RfbConnectionCompletion {
        end,
        lease,
        peer_addr,
    }
}

pub(crate) async fn finalize_connection(
    event_tx: &mpsc::Sender<RfbServerEvent>,
    completion: RfbConnectionCompletion,
) -> Result<ConnectionEnd, RfbConnectionFinalizeError> {
    let RfbConnectionCompletion {
        end,
        lease,
        peer_addr,
    } = completion;
    let reason = end
        .reason()
        .ok_or(RfbConnectionFinalizeError::EventChannelClosed)?;
    let client_id = lease.client_id();

    event_tx
        .send(RfbServerEvent::Disconnected {
            client_id,
            peer_addr,
            reason,
        })
        .await
        .map_err(|_| RfbConnectionFinalizeError::EventChannelClosed)?;
    lease.release();
    Ok(end)
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, task::Poll};

    use tokio::sync::mpsc;

    use super::*;
    use crate::rfb_connection::{
        RfbClientId, RfbConnectionGate, RfbConnectionGateError, RfbDisconnectReason,
        RfbServerEvent, RfbTransportKind, driver::RfbConnectionError,
    };

    fn peer() -> SocketAddr {
        "127.0.0.1:5900".parse().unwrap()
    }

    fn completion(
        gate: &RfbConnectionGate,
        peer_addr: SocketAddr,
        end: ConnectionEnd,
    ) -> RfbConnectionCompletion {
        let lease = gate
            .try_acquire(RfbTransportKind::Tcp, peer_addr)
            .unwrap()
            .activate();
        RfbConnectionCompletion {
            end,
            lease,
            peer_addr,
        }
    }

    #[tokio::test]
    async fn disconnect_backpressure_holds_gate_until_event_is_enqueued() {
        let gate = RfbConnectionGate::new();
        let peer_addr = "127.0.0.1:5900".parse().unwrap();
        let completion = completion(&gate, peer_addr, ConnectionEnd::ClientClosed);
        let client_id = completion.lease.client_id();
        let (event_tx, mut event_rx) = mpsc::channel(1);
        event_tx
            .send(RfbServerEvent::Key {
                client_id: RfbClientId(99),
                down: true,
                keysym: 0x41,
            })
            .await
            .unwrap();

        let mut finalizer = Box::pin(finalize_connection(&event_tx, completion));
        std::future::poll_fn(|context| match finalizer.as_mut().poll(context) {
            Poll::Pending => Poll::Ready(()),
            Poll::Ready(result) => panic!("full event channel completed disconnect: {result:?}"),
        })
        .await;
        assert_eq!(
            gate.try_acquire(RfbTransportKind::Tcp, peer()).unwrap_err(),
            RfbConnectionGateError::Busy
        );

        assert!(matches!(
            event_rx.recv().await,
            Some(RfbServerEvent::Key { .. })
        ));
        assert!(matches!(finalizer.await, Ok(ConnectionEnd::ClientClosed)));
        assert!(matches!(
            event_rx.recv().await,
            Some(RfbServerEvent::Disconnected {
                client_id: actual_client_id,
                peer_addr: actual_peer_addr,
                reason: RfbDisconnectReason::ClientClosed,
            }) if actual_client_id == client_id && actual_peer_addr == peer_addr
        ));

        let next = gate.try_acquire(RfbTransportKind::Tcp, peer()).unwrap();
        assert_eq!(
            gate.try_acquire(RfbTransportKind::Tcp, peer()).unwrap_err(),
            RfbConnectionGateError::Busy
        );
        drop(next);
    }

    #[tokio::test]
    async fn cancelling_pending_finalizer_poisons_gate() {
        let gate = RfbConnectionGate::new();
        let completion = completion(
            &gate,
            "127.0.0.1:5900".parse().unwrap(),
            ConnectionEnd::ClientClosed,
        );
        let (event_tx, _event_rx) = mpsc::channel(1);
        event_tx
            .send(RfbServerEvent::Key {
                client_id: RfbClientId(99),
                down: true,
                keysym: 0x41,
            })
            .await
            .unwrap();

        let mut finalizer = Box::pin(finalize_connection(&event_tx, completion));
        std::future::poll_fn(|context| match finalizer.as_mut().poll(context) {
            Poll::Pending => Poll::Ready(()),
            Poll::Ready(result) => panic!("full event channel completed disconnect: {result:?}"),
        })
        .await;
        drop(finalizer);

        assert_eq!(
            gate.try_acquire(RfbTransportKind::Tcp, peer()).unwrap_err(),
            RfbConnectionGateError::Poisoned
        );
    }

    #[tokio::test]
    async fn missing_disconnect_reason_poisons_gate_without_event() {
        let gate = RfbConnectionGate::new();
        let completion = completion(
            &gate,
            "127.0.0.1:5900".parse().unwrap(),
            ConnectionEnd::Failed(RfbConnectionError::EventChannelClosed),
        );
        let (event_tx, mut event_rx) = mpsc::channel(1);

        assert!(matches!(
            finalize_connection(&event_tx, completion).await,
            Err(RfbConnectionFinalizeError::EventChannelClosed)
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        assert_eq!(
            gate.try_acquire(RfbTransportKind::Tcp, peer()).unwrap_err(),
            RfbConnectionGateError::Poisoned
        );
    }

    #[tokio::test]
    async fn closed_event_receiver_poisons_gate_without_event() {
        let gate = RfbConnectionGate::new();
        let completion = completion(
            &gate,
            "127.0.0.1:5900".parse().unwrap(),
            ConnectionEnd::ClientClosed,
        );
        let (event_tx, event_rx) = mpsc::channel(1);
        drop(event_rx);

        assert!(matches!(
            finalize_connection(&event_tx, completion).await,
            Err(RfbConnectionFinalizeError::EventChannelClosed)
        ));
        assert_eq!(
            gate.try_acquire(RfbTransportKind::Tcp, peer()).unwrap_err(),
            RfbConnectionGateError::Poisoned
        );
    }
}
