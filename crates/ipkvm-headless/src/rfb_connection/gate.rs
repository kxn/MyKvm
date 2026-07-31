use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::RfbClientId;

#[derive(Clone)]
pub struct RfbConnectionGate {
    pub(super) inner: Arc<GateInner>,
}

pub(super) struct GateInner {
    semaphore: Arc<Semaphore>,
    pub(super) next_client_id: AtomicU64,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum RfbConnectionGateError {
    #[error("an RFB connection is already active")]
    Busy,
    #[error("RFB client identifier space is exhausted")]
    ClientIdOverflow,
}

#[derive(Debug)]
pub(crate) struct RfbConnectionPermit {
    _semaphore_permit: OwnedSemaphorePermit,
    client_id: RfbClientId,
}

impl RfbConnectionGate {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(GateInner {
                semaphore: Arc::new(Semaphore::new(1)),
                next_client_id: AtomicU64::new(1),
            }),
        }
    }

    pub(crate) async fn acquire(&self) -> Result<RfbConnectionPermit, RfbConnectionGateError> {
        let semaphore_permit = self
            .inner
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("RFB connection gate semaphore is never closed");
        let client_id = self.allocate_client_id()?;
        Ok(RfbConnectionPermit {
            _semaphore_permit: semaphore_permit,
            client_id,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn try_acquire(&self) -> Result<RfbConnectionPermit, RfbConnectionGateError> {
        let semaphore_permit = self
            .inner
            .semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| RfbConnectionGateError::Busy)?;
        let client_id = self.allocate_client_id()?;
        Ok(RfbConnectionPermit {
            _semaphore_permit: semaphore_permit,
            client_id,
        })
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

impl RfbConnectionPermit {
    pub(crate) fn client_id(&self) -> RfbClientId {
        self.client_id
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;

    #[tokio::test]
    async fn gate_allows_exactly_one_permit() {
        let gate = RfbConnectionGate::new();
        let first = gate.try_acquire().unwrap();
        assert_eq!(first.client_id().get(), 1);
        assert_eq!(
            gate.try_acquire().unwrap_err(),
            RfbConnectionGateError::Busy
        );
        drop(first);
        assert_eq!(gate.try_acquire().unwrap().client_id().get(), 2);
    }

    #[tokio::test]
    async fn gate_allocates_u64_max_once_and_never_wraps() {
        let gate = RfbConnectionGate::new();
        gate.inner.next_client_id.store(u64::MAX, Ordering::Relaxed);
        let last = gate.try_acquire().unwrap();
        assert_eq!(last.client_id().get(), u64::MAX);
        drop(last);
        assert_eq!(
            gate.try_acquire().unwrap_err(),
            RfbConnectionGateError::ClientIdOverflow
        );
    }
}
