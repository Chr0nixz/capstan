//! Global admission for owned transfer payloads and their protocol write copy.
use crate::sftp::{SftpError, MAX_BUFFER_BYTES, REQUEST_BYTES};
#[cfg(any(test, feature = "benchmarks"))]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{ops::Deref, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

pub(super) struct Budget {
    permits: Arc<Semaphore>,
    capacity: usize,
    #[cfg(any(test, feature = "benchmarks"))]
    peak: AtomicUsize,
    #[cfg(any(test, feature = "benchmarks"))]
    current: AtomicUsize,
}
impl Default for Budget {
    fn default() -> Self {
        Self::new(MAX_BUFFER_BYTES)
    }
}
impl Budget {
    fn new(capacity: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(capacity)),
            capacity,
            #[cfg(any(test, feature = "benchmarks"))]
            peak: AtomicUsize::new(0),
            #[cfg(any(test, feature = "benchmarks"))]
            current: AtomicUsize::new(0),
        }
    }
    pub(super) async fn reserve(
        self: &Arc<Self>,
        bytes: usize,
        cancel: &CancellationToken,
    ) -> Result<Reservation, SftpError> {
        if bytes == 0 || bytes > REQUEST_BYTES {
            return Err(SftpError::ResourceLimit);
        }
        self.charge(bytes, cancel).await
    }
    pub(super) async fn window(
        self: &Arc<Self>,
        cancel: &CancellationToken,
    ) -> Result<Reservation, SftpError> {
        // Always cover the wire's maximum DATA size, including a malicious
        // reply larger than a negotiated/requested block. Each worker owns at
        // most one such window, including full readback verification.
        self.charge(REQUEST_BYTES * crate::sftp::REQUESTS_PER_FILE, cancel)
            .await
    }
    async fn charge(
        self: &Arc<Self>,
        bytes: usize,
        cancel: &CancellationToken,
    ) -> Result<Reservation, SftpError> {
        let charged = bytes
            .checked_mul(2)
            .filter(|size| *size <= self.capacity)
            .ok_or(SftpError::ResourceLimit)?;
        let permits = u32::try_from(charged).map_err(|_| SftpError::ResourceLimit)?;
        let permit = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(SftpError::OperationCanceled),
            result = self.permits.clone().acquire_many_owned(permits) => result.map_err(|_| SftpError::ResourceLimit)?,
        };
        #[cfg(any(test, feature = "benchmarks"))]
        self.peak.fetch_max(
            self.current.fetch_add(charged, Ordering::SeqCst) + charged,
            Ordering::SeqCst,
        );
        Ok(Reservation {
            _permit: permit,
            #[cfg(any(test, feature = "benchmarks"))]
            budget: self.clone(),
            bytes,
        })
    }
    #[cfg(any(test, feature = "benchmarks"))]
    pub(super) fn usage(&self) -> (usize, usize) {
        (
            self.current.load(Ordering::SeqCst),
            self.peak.load(Ordering::SeqCst),
        )
    }
}
pub(super) struct Reservation {
    _permit: OwnedSemaphorePermit,
    #[cfg(any(test, feature = "benchmarks"))]
    budget: Arc<Budget>,
    bytes: usize,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        // Decrement before the permit is returned, so another admitted owner
        // cannot transiently count both the old and new reservation.
        #[cfg(any(test, feature = "benchmarks"))]
        self.budget
            .current
            .fetch_sub(self.bytes * 2, Ordering::SeqCst);
    }
}
impl Reservation {
    pub(super) fn wrap(self, bytes: Vec<u8>) -> Result<Block, SftpError> {
        if bytes.capacity() > self.bytes {
            return Err(SftpError::ResourceLimit);
        }
        Ok(Block {
            bytes,
            _reservation: self,
        })
    }
}
pub(super) struct Block {
    // Drop payload before returning admission, including inside blocking I/O.
    bytes: Vec<u8>,
    _reservation: Reservation,
}
impl Deref for Block {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.bytes
    }
}
impl AsRef<[u8]> for Block {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn global_budget_bounds_multiple_owners_and_cancel_waits_without_leaking() {
        let budget = Arc::new(Budget::new(64));
        let cancel = CancellationToken::new();
        let first = budget
            .reserve(16, &cancel)
            .await
            .unwrap()
            .wrap(vec![1; 16])
            .unwrap();
        let second = budget
            .reserve(16, &cancel)
            .await
            .unwrap()
            .wrap(vec![2; 16])
            .unwrap();
        assert_eq!(budget.usage(), (64, 64));
        let pending = budget.reserve(1, &cancel);
        tokio::pin!(pending);
        tokio::select! { biased; _ = &mut pending => panic!("full budget admitted an owner"), () = tokio::task::yield_now() => {} }
        cancel.cancel();
        assert!(matches!(pending.await, Err(SftpError::OperationCanceled)));
        drop(first);
        drop(second);
        assert_eq!(budget.usage(), (0, 64));
    }
    #[tokio::test]
    async fn blocking_payload_keeps_admission_until_native_work_releases_it() {
        let budget = Arc::new(Budget::new(64));
        let payload = budget
            .reserve(32, &CancellationToken::new())
            .await
            .unwrap()
            .wrap(vec![0; 32])
            .unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        let task = tokio::task::spawn_blocking(move || {
            wait.recv().unwrap();
            drop(payload);
        });
        assert_eq!(budget.usage(), (64, 64));
        release.send(()).unwrap();
        task.await.unwrap();
        assert_eq!(budget.usage(), (0, 64));
    }
    #[tokio::test]
    async fn invalid_or_oversized_buffers_do_not_consume_admission() {
        let budget = Arc::new(Budget::default());
        let cancel = CancellationToken::new();
        for bytes in [0, REQUEST_BYTES + 1, usize::MAX] {
            assert!(matches!(
                budget.reserve(bytes, &cancel).await,
                Err(SftpError::ResourceLimit)
            ));
        }
        let reservation = budget.reserve(16, &cancel).await.unwrap();
        assert!(matches!(
            reservation.wrap(Vec::with_capacity(17)),
            Err(SftpError::ResourceLimit)
        ));
        assert_eq!(budget.usage(), (0, 32));
    }
}
