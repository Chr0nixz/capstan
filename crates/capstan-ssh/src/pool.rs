use crate::{Result, SshError};
use std::{
    collections::HashMap,
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
    time::Duration,
};
use tokio::sync::{oneshot, watch, Mutex as AsyncMutex};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PoolKey {
    pub target: String,
    pub port: u16,
    pub username: String,
    pub credential_id: Uuid,
    pub credential_revision: u64,
    pub trust_fingerprint: String,
    pub vault_epoch: u64,
    pub agent_identity: Option<String>,
    pub agent_forwarding: bool,
    pub proxy_path: String,
}

struct Pending<T> {
    id: Uuid,
    response: oneshot::Sender<Result<Arc<T>>>,
}
enum EntryState<T> {
    Vacant,
    Building {
        build_id: Uuid,
        waiters: Vec<Pending<T>>,
        cancel: CancellationToken,
        completed: watch::Sender<bool>,
    },
    Ready {
        value: Arc<T>,
        leases: usize,
        idle_generation: u64,
        idle: CancellationToken,
    },
}
struct Entry<T> {
    state: Mutex<EntryState<T>>,
}
impl<T> Default for Entry<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new(EntryState::Vacant),
        }
    }
}

struct PoolInner<T> {
    entries: AsyncMutex<HashMap<PoolKey, Arc<Entry<T>>>>,
    idle_timeout: Duration,
}

pub struct ConnectionPool<T> {
    inner: Arc<PoolInner<T>>,
}
impl<T> Clone for ConnectionPool<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}
impl<T: Send + Sync + 'static> ConnectionPool<T> {
    pub fn new(idle_timeout: Duration) -> Self {
        Self {
            inner: Arc::new(PoolInner {
                entries: AsyncMutex::new(HashMap::new()),
                idle_timeout,
            }),
        }
    }

    pub async fn acquire<F, Fut>(
        &self,
        key: PoolKey,
        cancellation: CancellationToken,
        factory: F,
    ) -> Result<ConnectionLease<T>>
    where
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
    {
        self.acquire_checked(key, cancellation, |_| true, factory)
            .await
    }

    /// Retire a known-dead ready entry, including its aliases, before an
    /// explicitly requested acquisition. Existing leases remain bound to the
    /// retired entry and cannot decrement a replacement connection's count.
    /// The liveness predicate is synchronous and runs under the pool lock.
    pub async fn acquire_checked<F, Fut>(
        &self,
        key: PoolKey,
        cancellation: CancellationToken,
        reusable: impl Fn(&T) -> bool + Send,
        factory: F,
    ) -> Result<ConnectionLease<T>>
    where
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
    {
        if cancellation.is_cancelled() {
            return Err(SshError::Canceled);
        }
        let waiter_id = Uuid::new_v4();
        let (response, receiver) = oneshot::channel();
        let mut factory = Some(factory);
        let mut build_cancel = None;
        let mut ready = None;
        let entry = {
            // Keep the map lock through the state transition so a vacant entry
            // cannot be removed after this acquire has found it but before it
            // registers itself as a waiter.
            let mut entries = self.inner.entries.lock().await;
            if let Some(current) = entries.get(&key) {
                let retired = {
                    let state = current.state.lock().map_err(|_| SshError::Protocol)?;
                    matches!(&*state, EntryState::Ready { value, .. } if !reusable(value))
                };
                if retired {
                    let current = current.clone();
                    entries.retain(|_, entry| !Arc::ptr_eq(entry, &current));
                }
            }
            let entry = entries
                .entry(key.clone())
                .or_insert_with(|| Arc::new(Entry::default()))
                .clone();
            let mut state = entry.state.lock().map_err(|_| SshError::Protocol)?;
            match &mut *state {
                EntryState::Ready {
                    value,
                    leases,
                    idle_generation,
                    idle,
                } => {
                    if *leases == 0 {
                        *idle = CancellationToken::new();
                    }
                    *leases += 1;
                    *idle_generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
                    ready = Some(value.clone());
                }
                EntryState::Building { waiters, .. } => waiters.push(Pending {
                    id: waiter_id,
                    response,
                }),
                EntryState::Vacant => {
                    let cancel = CancellationToken::new();
                    let build_id = Uuid::new_v4();
                    let completed = watch::channel(false).0;
                    build_cancel = Some((cancel.clone(), build_id, completed.clone()));
                    *state = EntryState::Building {
                        build_id,
                        waiters: vec![Pending {
                            id: waiter_id,
                            response,
                        }],
                        cancel,
                        completed,
                    };
                }
            }
            drop(state);
            drop(entries);
            entry
        };
        if let Some(value) = ready {
            let token = self.lease_token(key, entry);
            return Ok(ConnectionLease {
                value,
                _token: token,
            });
        }
        if let Some((build_cancel, build_id, completed)) = build_cancel {
            let factory = factory.take().ok_or(SshError::Protocol)?;
            let weak_entry = Arc::downgrade(&entry);
            let weak_pool = Arc::downgrade(&self.inner);
            let cleanup_key = key.clone();
            tokio::spawn(async move {
                let result = tokio::select! {
                    biased;
                    () = build_cancel.cancelled() => Err(SshError::Canceled),
                    result = factory(build_cancel.clone()) => result.map(Arc::new),
                };
                if let Some(entry) = weak_entry.upgrade() {
                    if let Ok(mut state) = entry.state.lock() {
                        if matches!(&*state, EntryState::Building { build_id: current, .. } if *current == build_id)
                        {
                            let previous = std::mem::replace(&mut *state, EntryState::Vacant);
                            let EntryState::Building { waiters, .. } = previous else {
                                return;
                            };
                            if let Ok(value) = &result {
                                *state = EntryState::Ready {
                                    value: value.clone(),
                                    leases: 0,
                                    idle_generation: NEXT_GENERATION
                                        .fetch_add(1, Ordering::Relaxed),
                                    idle: CancellationToken::new(),
                                };
                            }
                            for waiter in waiters {
                                let _ = waiter.response.send(result.clone());
                            }
                        }
                    }
                    if result.is_err() {
                        remove_vacant_entry(weak_pool, cleanup_key, entry).await;
                    } else {
                        let generation = entry.state.lock().ok().and_then(|state| match &*state {
                            EntryState::Ready {
                                leases: 0,
                                idle_generation,
                                ..
                            } => Some(*idle_generation),
                            _ => None,
                        });
                        if let Some(generation) = generation {
                            schedule_idle(weak_pool, entry, cleanup_key, generation);
                        }
                    }
                }
                completed.send_replace(true);
            });
        }
        let response = tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                if let Some(mut completed) = self.remove_waiter(&key, &entry, waiter_id) {
                    while !*completed.borrow_and_update() {
                        if completed.changed().await.is_err() { break; }
                    }
                }
                return Err(SshError::Canceled);
            }
            response = receiver => response.map_err(|_| SshError::Canceled)?,
        }?;
        let value = {
            let mut state = entry.state.lock().map_err(|_| SshError::Protocol)?;
            match &mut *state {
                EntryState::Ready {
                    value: ready,
                    leases,
                    idle_generation,
                    idle,
                } if Arc::ptr_eq(ready, &response) => {
                    if *leases == 0 {
                        *idle = CancellationToken::new();
                    }
                    *leases += 1;
                    *idle_generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
                    ready.clone()
                }
                _ => return Err(SshError::Canceled),
            }
        };
        let token = self.lease_token(key, entry);
        Ok(ConnectionLease {
            value,
            _token: token,
        })
    }

    fn lease_token(&self, key: PoolKey, entry: Arc<Entry<T>>) -> Arc<LeaseToken<T>> {
        Arc::new(LeaseToken {
            key,
            entry,
            pool: Arc::downgrade(&self.inner),
        })
    }

    fn remove_waiter(
        &self,
        key: &PoolKey,
        entry: &Arc<Entry<T>>,
        waiter_id: Uuid,
    ) -> Option<watch::Receiver<bool>> {
        let mut cancel_build = None;
        let mut completion = None;
        let mut idle_generation = None;
        if let Ok(mut state) = entry.state.lock() {
            match &mut *state {
                EntryState::Building {
                    waiters,
                    cancel,
                    completed,
                    ..
                } => {
                    waiters.retain(|waiter| waiter.id != waiter_id);
                    if waiters.is_empty() {
                        cancel_build = Some(cancel.clone());
                        completion = Some(completed.subscribe());
                        *state = EntryState::Vacant;
                    }
                }
                EntryState::Ready {
                    leases: 0,
                    idle_generation: generation,
                    ..
                } => idle_generation = Some(*generation),
                _ => {}
            }
        }
        if let Some(cancel) = cancel_build {
            cancel.cancel();
        }
        if let Some(generation) = idle_generation {
            schedule_idle(
                Arc::downgrade(&self.inner),
                entry.clone(),
                key.clone(),
                generation,
            );
        }
        completion
    }

    pub async fn entry_count(&self) -> usize {
        self.inner.entries.lock().await.len()
    }
    /// Remove all entries, cancel in-flight builds and return each ready connection once.
    pub async fn drain(&self) -> Vec<Arc<T>> {
        let mut entries = self.inner.entries.lock().await;
        let drained: Vec<_> = entries.drain().map(|(_, entry)| entry).collect();
        let mut connections = Vec::new();
        let mut pending = Vec::new();
        for entry in drained {
            let Ok(mut state) = entry.state.lock() else {
                continue;
            };
            let previous = std::mem::replace(&mut *state, EntryState::Vacant);
            match previous {
                EntryState::Ready { value, idle, .. } => {
                    idle.cancel();
                    if !connections
                        .iter()
                        .any(|existing| Arc::ptr_eq(existing, &value))
                    {
                        connections.push(value);
                    }
                }
                EntryState::Building {
                    waiters,
                    cancel,
                    completed,
                    ..
                } => {
                    cancel.cancel();
                    pending.push(completed.subscribe());
                    for waiter in waiters {
                        let _ = waiter.response.send(Err(SshError::Canceled));
                    }
                }
                EntryState::Vacant => {}
            }
        }
        drop(entries);
        for mut completed in pending {
            while !*completed.borrow_and_update() {
                if completed.changed().await.is_err() {
                    break;
                }
            }
        }
        connections
    }
    pub async fn alias(&self, lease: &ConnectionLease<T>, key: PoolKey) {
        self.inner
            .entries
            .lock()
            .await
            .entry(key)
            .or_insert_with(|| lease._token.entry.clone());
    }
}

async fn remove_vacant_entry<T: Send + Sync + 'static>(
    pool: Weak<PoolInner<T>>,
    key: PoolKey,
    entry: Arc<Entry<T>>,
) {
    let Some(pool) = pool.upgrade() else {
        return;
    };
    let mut entries = pool.entries.lock().await;
    let still_vacant = entry
        .state
        .lock()
        .is_ok_and(|state| matches!(&*state, EntryState::Vacant));
    if still_vacant
        && entries
            .get(&key)
            .is_some_and(|current| Arc::ptr_eq(current, &entry))
    {
        entries.remove(&key);
    }
}

pub struct ConnectionLease<T: Send + Sync + 'static> {
    value: Arc<T>,
    _token: Arc<LeaseToken<T>>,
}
impl<T: Send + Sync + 'static> Clone for ConnectionLease<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            _token: self._token.clone(),
        }
    }
}
impl<T: Send + Sync + 'static> ConnectionLease<T> {
    pub fn connection(&self) -> &T {
        &self.value
    }
    /// Cleanup registries can observe a transport without retaining its lease.
    pub fn downgrade_connection(&self) -> Weak<T> {
        Arc::downgrade(&self.value)
    }

    /// Observes this busy interval without retaining a pool lease.
    /// The token stays canceled even if the same connection is reused.
    pub fn idle_token(&self) -> Result<CancellationToken> {
        let state = self
            ._token
            .entry
            .state
            .lock()
            .map_err(|_| SshError::Protocol)?;
        match &*state {
            EntryState::Ready { idle, .. } => Ok(idle.child_token()),
            _ => Err(SshError::Canceled),
        }
    }

    /// The exact target used to acquire this authenticated pool entry.
    /// Callers cannot substitute a hostname supplied by terminal output.
    pub fn authenticated_target(&self) -> &str {
        &self._token.key.target
    }

    /// Rust-owned authentication/trust policy for explicit workflow rebinds.
    /// It contains identifiers and fingerprints, never credential plaintext.
    pub fn policy(&self) -> &PoolKey {
        &self._token.key
    }
}

struct LeaseToken<T: Send + Sync + 'static> {
    key: PoolKey,
    entry: Arc<Entry<T>>,
    pool: Weak<PoolInner<T>>,
}
impl<T: Send + Sync + 'static> Drop for LeaseToken<T> {
    fn drop(&mut self) {
        let generation = if let Ok(mut state) = self.entry.state.lock() {
            match &mut *state {
                EntryState::Ready {
                    leases,
                    idle_generation,
                    idle,
                    ..
                } => {
                    *leases = leases.saturating_sub(1);
                    if *leases == 0 {
                        idle.cancel();
                        *idle_generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
                        Some(*idle_generation)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        } else {
            None
        };
        if let Some(generation) = generation {
            schedule_idle(
                self.pool.clone(),
                self.entry.clone(),
                self.key.clone(),
                generation,
            );
        }
    }
}

fn schedule_idle<T: Send + Sync + 'static>(
    pool: Weak<PoolInner<T>>,
    entry: Arc<Entry<T>>,
    key: PoolKey,
    generation: u64,
) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let Some(inner) = pool.upgrade() else {
        return;
    };
    // Count from lease release, not from the spawned cleanup task's first
    // poll, which can be delayed by other transport/subsystem cleanup.
    let deadline = tokio::time::Instant::now() + inner.idle_timeout;
    runtime.spawn(async move {
        tokio::time::sleep_until(deadline).await;
        let mut entries = inner.entries.lock().await;
        let Some(current) = entries.get(&key) else { return; };
        if !Arc::ptr_eq(current, &entry) { return; }
        if let Ok(state) = entry.state.lock() {
            if matches!(&*state, EntryState::Ready { leases: 0, idle_generation: current_generation, .. } if *current_generation == generation) {
                entries.retain(|_, value| !Arc::ptr_eq(value, &entry));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn key() -> PoolKey {
        PoolKey {
            target: "example.test".into(),
            port: 22,
            username: "alice".into(),
            credential_id: Uuid::new_v4(),
            credential_revision: 1,
            trust_fingerprint: "SHA256:test".into(),
            vault_epoch: 3,
            agent_identity: None,
            agent_forwarding: false,
            proxy_path: String::new(),
        }
    }
    #[tokio::test]
    async fn concurrent_waiters_share_one_connection_build_and_independent_cancel() {
        let pool = ConnectionPool::new(Duration::from_secs(30));
        let build_count = Arc::new(AtomicUsize::new(0));
        let first_cancel = CancellationToken::new();
        let second_cancel = CancellationToken::new();
        let cancel_first = first_cancel.clone();
        let pool_key = key();
        let first_key = pool_key.clone();
        let first_pool = pool.clone();
        let first_count = build_count.clone();
        let first = tokio::spawn(async move {
            first_pool
                .acquire(first_key, first_cancel, move |_| async move {
                    first_count.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Ok(41_u64)
                })
                .await
        });
        tokio::task::yield_now().await;
        let second_pool = pool.clone();
        let second_count = build_count.clone();
        let second_key = pool_key.clone();
        let second = tokio::spawn(async move {
            second_pool
                .acquire(second_key, second_cancel, move |_| async move {
                    second_count.fetch_add(1, Ordering::SeqCst);
                    Ok(42_u64)
                })
                .await
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        cancel_first.cancel();
        assert!(matches!(
            first.await.expect("first waiter"),
            Err(SshError::Canceled)
        ));
        let lease = second.await.expect("second waiter").expect("shared build");
        assert_eq!(*lease.connection(), 41);
        assert_eq!(build_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn idle_expiry_is_generation_bound_and_closes_after_thirty_seconds() {
        let pool = ConnectionPool::new(Duration::from_secs(30));
        let pool_key = key();
        let lease = pool
            .acquire(pool_key.clone(), CancellationToken::new(), |_| async {
                Ok(1_u64)
            })
            .await
            .expect("build");
        let clone = lease.clone();
        let idle = lease.idle_token().expect("busy interval");
        drop(lease);
        assert!(!idle.is_cancelled());
        tokio::time::advance(Duration::from_secs(29)).await;
        tokio::task::yield_now().await;
        assert_eq!(pool.entry_count().await, 1);
        drop(clone);
        assert!(idle.is_cancelled());
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(29)).await;
        let reused = pool
            .acquire(pool_key, CancellationToken::new(), |_| async { Ok(2_u64) })
            .await
            .expect("reuse before expiry");
        assert_eq!(*reused.connection(), 1);
        assert!(idle.is_cancelled());
        assert!(!reused
            .idle_token()
            .expect("new busy interval")
            .is_cancelled());
        drop(reused);
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(30)).await;
        tokio::task::yield_now().await;
        assert_eq!(pool.entry_count().await, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn idle_timeout_starts_at_release_even_if_the_reaper_is_not_polled_yet() {
        let pool = ConnectionPool::new(Duration::from_secs(30));
        let lease = pool
            .acquire(key(), CancellationToken::new(), |_| async { Ok(1_u64) })
            .await
            .expect("build");
        drop(lease);
        // Intentionally advance before the newly spawned reaper can register
        // its sleep. Scheduling delay must not reset the idle deadline.
        tokio::time::advance(Duration::from_secs(29)).await;
        assert_eq!(pool.entry_count().await, 1);
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(pool.entry_count().await, 0);
    }

    #[tokio::test]
    async fn last_waiter_cancellation_stops_connection_creation() {
        let pool = ConnectionPool::new(Duration::from_secs(30));
        let cancellation = CancellationToken::new();
        let cancel_factory = cancellation.clone();
        let task_pool = pool.clone();
        let task = tokio::spawn(async move {
            task_pool
                .acquire(key(), cancellation, move |shared_cancel| async move {
                    tokio::select! {
                        () = shared_cancel.cancelled() => Err(SshError::Canceled),
                        () = tokio::time::sleep(Duration::from_secs(10)) => Ok(1_u64),
                    }
                })
                .await
        });
        tokio::task::yield_now().await;
        cancel_factory.cancel();
        assert!(matches!(
            task.await.expect("waiter result"),
            Err(SshError::Canceled)
        ));
        for _ in 0..10 {
            if pool.entry_count().await == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(pool.entry_count().await, 0);
    }

    #[tokio::test]
    async fn cancellation_returns_only_after_the_last_factory_has_been_dropped() {
        struct Finished(Arc<AtomicUsize>);
        impl Drop for Finished {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let pool = ConnectionPool::new(Duration::from_secs(30));
        let canceled = CancellationToken::new();
        let dropped = Arc::new(AtomicUsize::new(0));
        let factory_dropped = dropped.clone();
        let (started, ready) = oneshot::channel();
        let token = canceled.clone();
        let task = tokio::spawn(async move {
            pool.acquire(key(), token, move |_| async move {
                let _finished = Finished(factory_dropped);
                let _ = started.send(());
                std::future::pending::<Result<u64>>().await
            })
            .await
        });
        ready.await.unwrap();
        canceled.cancel();
        assert!(matches!(task.await.unwrap(), Err(SshError::Canceled)));
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn drain_cancels_builds_and_returns_ready_entries_once_across_aliases() {
        let pool = ConnectionPool::new(Duration::from_secs(30));
        let pool_key = key();
        let lease = pool
            .acquire(pool_key.clone(), CancellationToken::new(), |_| async {
                Ok(41_u8)
            })
            .await
            .unwrap();
        pool.alias(
            &lease,
            PoolKey {
                target: "alias.example.test".into(),
                ..pool_key
            },
        )
        .await;
        let value = lease.connection() as *const u8;
        let drained = pool.drain().await;
        assert_eq!(drained.len(), 1);
        assert_eq!(Arc::as_ptr(&drained[0]), value);
        assert_eq!(pool.entry_count().await, 0);
        drop(lease);

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (proceed_tx, proceed_rx) = tokio::sync::oneshot::channel();
        let build_pool = pool.clone();
        let build = tokio::spawn(async move {
            build_pool
                .acquire(
                    PoolKey {
                        target: "building.example.test".into(),
                        ..key()
                    },
                    CancellationToken::new(),
                    move |cancel| async move {
                        let _ = started_tx.send(());
                        tokio::select! {
                            () = cancel.cancelled() => Err(SshError::Canceled),
                            _ = proceed_rx => Ok(9_u8),
                        }
                    },
                )
                .await
        });
        started_rx.await.unwrap();
        let drained = pool.drain().await;
        let _ = proceed_tx.send(());
        assert!(drained.is_empty());
        assert!(matches!(build.await.unwrap(), Err(SshError::Canceled)));
        assert_eq!(pool.entry_count().await, 0);
    }

    #[tokio::test]
    async fn failed_build_is_removed_and_a_later_attempt_can_build_again() {
        let pool = ConnectionPool::new(Duration::from_secs(30));
        let pool_key = key();
        let failed = pool
            .acquire(pool_key.clone(), CancellationToken::new(), |_| async {
                Err::<u64, _>(SshError::Connection)
            })
            .await;
        assert!(matches!(failed, Err(SshError::Connection)));
        for _ in 0..10 {
            if pool.entry_count().await == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(pool.entry_count().await, 0);

        let recovered = pool
            .acquire(pool_key, CancellationToken::new(), |_| async { Ok(42_u64) })
            .await
            .expect("retry succeeds");
        assert_eq!(*recovered.connection(), 42);
    }
}
