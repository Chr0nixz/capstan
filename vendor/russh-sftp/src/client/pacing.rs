//! Capstan: admission hook for pipelined file data requests.
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// Limits pipelined file data that shares one transport with other traffic.
///
/// A [`File`](super::fs::File) with a pacer asks before each read or write
/// request and always keeps at least one request of its own in flight, so a
/// transfer cannot stall on the pacer. Every admission is completed exactly
/// once: with its round trip when the reply is dispatched, or without one
/// when the request is dropped, times out or cannot be sent.
pub trait Pacer: Send + Sync {
    /// Reserves `bytes` for one more request and returns a token for
    /// [`Pacer::complete`], or `None` when enough data is in flight already.
    /// `force` admits a file's only request regardless of the limit.
    fn admit(&self, bytes: u32, force: bool) -> Option<u64>;
    /// Returns an admission. `round_trip` runs from admission to the reply.
    fn complete(&self, token: u64, bytes: u32, round_trip: Option<Duration>);
}

/// One admitted request. It travels with the request's reply slot, so the
/// reply dispatcher reports the round trip even while its file is not polled.
pub(crate) struct Ticket {
    pacer: Arc<dyn Pacer>,
    token: u64,
    bytes: u32,
    admitted: Instant,
    open: bool,
}

impl Ticket {
    pub(crate) fn admit(pacer: &Arc<dyn Pacer>, bytes: u32, force: bool) -> Option<Self> {
        let token = pacer.admit(bytes, force)?;
        Some(Self {
            pacer: pacer.clone(),
            token,
            bytes,
            admitted: Instant::now(),
            open: true,
        })
    }

    /// The reply arrived.
    pub(crate) fn finish(mut self) {
        self.open = false;
        self.pacer
            .complete(self.token, self.bytes, Some(self.admitted.elapsed()));
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        if self.open {
            self.pacer.complete(self.token, self.bytes, None);
        }
    }
}
