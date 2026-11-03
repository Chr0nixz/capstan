//! Bounded, ordered metadata joins without abandoning admitted native I/O.
use crate::sftp::SftpError;
use std::{future::Future, task::Poll};

// Keep metadata within the headroom beyond three full 64-request data windows
// under the existing 208-request connection ceiling. File workers stay at three.
pub(super) const MAX_SNAPSHOTS: usize = 16;

pub(super) async fn join_snapshots<F, T>(
    work: impl IntoIterator<Item = F>,
) -> Result<Vec<T>, SftpError>
where
    F: Future<Output = Result<T, SftpError>>,
{
    let mut futures = work.into_iter().map(Box::pin).collect::<Vec<_>>();
    if futures.len() > MAX_SNAPSHOTS {
        // No future has been polled, so no snapshot has started.
        return Err(SftpError::ResourceLimit);
    }
    let mut results = (0..futures.len()).map(|_| None).collect::<Vec<_>>();
    let mut remaining = futures.len();
    std::future::poll_fn(|context| {
        for (future, result) in futures.iter_mut().zip(results.iter_mut()) {
            if result.is_none() {
                if let Poll::Ready(value) = future.as_mut().poll(context) {
                    *result = Some(value);
                    remaining -= 1;
                }
            }
        }
        if remaining == 0 {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
    // Collect errors only after every admitted snapshot has completed.
    results.into_iter().flatten().collect()
}
