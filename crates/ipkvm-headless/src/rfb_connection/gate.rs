use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError, watch};

use super::RfbClientId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RfbTransportKind {
    Tcp,
    WebSocket,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveController {
    pub client_id: RfbClientId,
    pub transport: RfbTransportKind,
    pub peer_addr: SocketAddr,
    pub connected_since_ms: u64,
}

#[derive(Clone)]
pub struct RfbConnectionGate {
    pub(super) inner: Arc<GateInner>,
}

#[derive(Debug)]
pub(super) struct GateInner {
    semaphore: Arc<Semaphore>,
    pub(super) next_client_id: AtomicU64,
    pub(super) status: watch::Sender<Option<ActiveController>>,
    started_at: Instant,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum RfbConnectionGateError {
    #[error("an RFB connection is already active")]
    Busy,
    #[error("the RFB connection gate is poisoned")]
    Poisoned,
    #[error("RFB client identifier space is exhausted")]
    ClientIdOverflow,
}

#[derive(Debug)]
pub(crate) struct RfbConnectionReservation {
    semaphore_permit: Option<OwnedSemaphorePermit>,
    inner: Arc<GateInner>,
    client_id: RfbClientId,
    transport: RfbTransportKind,
    peer_addr: SocketAddr,
}

#[derive(Debug)]
pub(super) struct RfbConnectionLease {
    inner: Option<Arc<GateInner>>,
    client_id: RfbClientId,
}

impl RfbConnectionGate {
    pub fn new() -> Self {
        let (status, _) = watch::channel(None);
        Self {
            inner: Arc::new(GateInner {
                semaphore: Arc::new(Semaphore::new(1)),
                next_client_id: AtomicU64::new(1),
                status,
                started_at: Instant::now(),
            }),
        }
    }

    pub(crate) async fn acquire(
        &self,
        transport: RfbTransportKind,
        peer_addr: SocketAddr,
    ) -> Result<RfbConnectionReservation, RfbConnectionGateError> {
        let semaphore_permit = self
            .inner
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| RfbConnectionGateError::Poisoned)?;
        let client_id = self.allocate_client_id()?;
        Ok(RfbConnectionReservation {
            semaphore_permit: Some(semaphore_permit),
            inner: Arc::clone(&self.inner),
            client_id,
            transport,
            peer_addr,
        })
    }

    pub(crate) fn try_acquire(
        &self,
        transport: RfbTransportKind,
        peer_addr: SocketAddr,
    ) -> Result<RfbConnectionReservation, RfbConnectionGateError> {
        let semaphore_permit =
            self.inner
                .semaphore
                .clone()
                .try_acquire_owned()
                .map_err(|error| match error {
                    TryAcquireError::NoPermits => RfbConnectionGateError::Busy,
                    TryAcquireError::Closed => RfbConnectionGateError::Poisoned,
                })?;
        let client_id = self.allocate_client_id()?;
        Ok(RfbConnectionReservation {
            semaphore_permit: Some(semaphore_permit),
            inner: Arc::clone(&self.inner),
            client_id,
            transport,
            peer_addr,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn controller_status(&self) -> watch::Receiver<Option<ActiveController>> {
        self.inner.status.subscribe()
    }

    #[allow(dead_code)]
    pub(crate) fn active_controller(&self) -> Option<ActiveController> {
        self.inner.status.borrow().clone()
    }

    fn allocate_client_id(&self) -> Result<RfbClientId, RfbConnectionGateError> {
        let mut current = self.inner.next_client_id.load(Ordering::Relaxed);
        loop {
            if current == 0 {
                return Err(RfbConnectionGateError::ClientIdOverflow);
            }
            let next = current.checked_add(1).unwrap_or(0);
            match self.inner.next_client_id.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Ok(RfbClientId(current)),
                Err(actual) => current = actual,
            }
        }
    }
}

impl Default for RfbConnectionGate {
    fn default() -> Self {
        Self::new()
    }
}

impl RfbConnectionReservation {
    #[cfg(test)]
    pub(crate) fn client_id(&self) -> RfbClientId {
        self.client_id
    }

    pub(in crate::rfb_connection) fn activate(mut self) -> RfbConnectionLease {
        self.semaphore_permit
            .take()
            .expect("an RFB connection reservation is activated exactly once")
            .forget();
        self.inner.status.send_replace(Some(ActiveController {
            client_id: self.client_id,
            transport: self.transport,
            peer_addr: self.peer_addr,
            connected_since_ms: self.inner.started_at.elapsed().as_millis() as u64,
        }));
        RfbConnectionLease {
            inner: Some(Arc::clone(&self.inner)),
            client_id: self.client_id,
        }
    }
}

impl RfbConnectionLease {
    pub(in crate::rfb_connection) fn client_id(&self) -> RfbClientId {
        self.client_id
    }

    pub(in crate::rfb_connection) fn release(mut self) {
        let inner = self
            .inner
            .take()
            .expect("an RFB connection lease is released exactly once");
        inner.semaphore.add_permits(1);
        inner.status.send_replace(None);
    }
}

impl Drop for RfbConnectionLease {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            inner.semaphore.close();
            inner.status.send_replace(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, sync::atomic::Ordering, task::Poll, time::Duration};

    use super::*;

    fn peer_addr() -> SocketAddr {
        "127.0.0.1:1234".parse().unwrap()
    }

    #[test]
    fn default_creates_a_fresh_gate() {
        let gate = RfbConnectionGate::default();
        let permit = gate
            .try_acquire(RfbTransportKind::Tcp, peer_addr())
            .unwrap();

        assert_eq!(permit.client_id().get(), 1);
    }

    #[tokio::test]
    async fn gate_allows_exactly_one_permit() {
        let gate = RfbConnectionGate::new();
        let first = gate
            .try_acquire(RfbTransportKind::Tcp, peer_addr())
            .unwrap();
        assert_eq!(first.client_id().get(), 1);
        assert_eq!(
            gate.try_acquire(RfbTransportKind::Tcp, peer_addr())
                .unwrap_err(),
            RfbConnectionGateError::Busy
        );
        drop(first);
        assert_eq!(
            gate.try_acquire(RfbTransportKind::Tcp, peer_addr())
                .unwrap()
                .client_id()
                .get(),
            2
        );
    }

    #[tokio::test]
    async fn activated_lease_releases_only_when_explicitly_finalized() {
        let gate = RfbConnectionGate::new();

        for _ in 0..16 {
            let lease = gate
                .try_acquire(RfbTransportKind::Tcp, peer_addr())
                .unwrap()
                .activate();
            assert_eq!(
                gate.try_acquire(RfbTransportKind::Tcp, peer_addr())
                    .unwrap_err(),
                RfbConnectionGateError::Busy
            );
            lease.release();

            let next = gate
                .try_acquire(RfbTransportKind::Tcp, peer_addr())
                .unwrap();
            assert_eq!(
                gate.try_acquire(RfbTransportKind::Tcp, peer_addr())
                    .unwrap_err(),
                RfbConnectionGateError::Busy
            );
            next.activate().release();
        }
    }

    #[tokio::test]
    async fn dropping_activated_lease_poisons_gate_and_wakes_waiter() {
        let gate = RfbConnectionGate::new();
        let lease = gate
            .try_acquire(RfbTransportKind::Tcp, peer_addr())
            .unwrap()
            .activate();
        let waiter_gate = gate.clone();
        let mut waiter = Box::pin(waiter_gate.acquire(RfbTransportKind::Tcp, peer_addr()));
        std::future::poll_fn(|context| match waiter.as_mut().poll(context) {
            Poll::Pending => Poll::Ready(()),
            Poll::Ready(result) => panic!("active gate did not block waiter: {result:?}"),
        })
        .await;

        drop(lease);

        assert_eq!(waiter.await.unwrap_err(), RfbConnectionGateError::Poisoned);
        assert_eq!(
            gate.try_acquire(RfbTransportKind::Tcp, peer_addr())
                .unwrap_err(),
            RfbConnectionGateError::Poisoned
        );
    }

    #[tokio::test]
    async fn gate_exposes_controller_status_on_acquire_and_release() {
        let gate = RfbConnectionGate::new();
        let mut rx = gate.controller_status();
        assert!(rx.borrow().is_none());
        let peer = "127.0.0.1:1234".parse().unwrap();
        let reservation = gate.try_acquire(RfbTransportKind::Tcp, peer).unwrap();
        // 单调时钟在新鲜闸门上 elapsed 为 0，让时间流逝后再激活
        tokio::time::sleep(Duration::from_millis(5)).await;
        // 激活后状态为 Some
        let lease = reservation.activate();
        let active = gate.active_controller().unwrap();
        assert_eq!(active.transport, RfbTransportKind::Tcp);
        assert_eq!(active.peer_addr, peer);
        assert!(active.connected_since_ms > 0);
        rx.changed().await.unwrap();
        assert_eq!(rx.borrow().as_ref().unwrap().client_id, active.client_id);

        drop(lease);
        assert!(gate.active_controller().is_none());
        rx.changed().await.unwrap();
        assert!(rx.borrow().is_none());
    }

    #[tokio::test]
    async fn gate_allocates_u64_max_once_and_never_wraps() {
        let gate = RfbConnectionGate::new();
        gate.inner.next_client_id.store(u64::MAX, Ordering::Relaxed);
        let last = gate
            .try_acquire(RfbTransportKind::Tcp, peer_addr())
            .unwrap();
        assert_eq!(last.client_id().get(), u64::MAX);
        drop(last);
        assert_eq!(
            gate.try_acquire(RfbTransportKind::Tcp, peer_addr())
                .unwrap_err(),
            RfbConnectionGateError::ClientIdOverflow
        );
    }
}
