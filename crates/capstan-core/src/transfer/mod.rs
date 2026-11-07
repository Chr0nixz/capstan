//! Bounded single-file workflows, independent of the browser and Tauri.
mod batch;
mod budget;
mod engine;
mod local;
#[cfg(windows)]
mod native;
mod preflight;
#[cfg(windows)]
mod tree;
#[cfg(windows)]
pub use tree::{TreeChoice, TreeDecision, TreePlanSummary, TreeResult};
mod recovery;
pub use batch::BatchSummary;
#[cfg(windows)]
pub use local::WindowsDestination;
pub use local::{LocalDestination, LocalSource, LocalVersion};
#[cfg(windows)]
pub use native::{ExportDrag, ExportSnapshot, NativeDropResult, PreparedExport};
pub use recovery::{RecoveryState, TransferRecoverySnapshot, TransferRecoveryView};

use crate::sftp::{
    Binding, ExactU64, Metadata, RemoteEntryId, RemotePath, SftpError, MAX_QUEUED_TRANSFERS,
    TRANSFER_WORKERS,
};
use capstan_ssh::sftp::SftpClient;
use recovery::{make_record, valid_cleanup_size, RecoveryDraft, RecoveryStore};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, Notify, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TransferTarget {
    Upload {
        parent: RemotePath,
        name: String,
    },
    UploadTree {
        parent: RemotePath,
        relative: String,
    },
    /// Internal trusted tree-download item. This variant is never accepted
    /// from the WebView's generic enqueue command; it is constructed only
    /// from a Rust-owned `SftpService` tree selection.
    DownloadTree {
        path: RemotePath,
        relative: String,
    },
    Download {
        entry: RemoteEntryId,
    },
}
pub(crate) fn upload_tree_path(
    parent: &RemotePath,
    relative: &str,
) -> Result<RemotePath, SftpError> {
    let mut path = parent.clone();
    let mut components = relative.split('/').peekable();
    let mut saw = false;
    while let Some(component) = components.next() {
        let name = crate::sftp::RemoteName::from_bytes(component.as_bytes())?;
        saw = true;
        path = path.join(&name)?;
        if components.peek().is_none() {
            break;
        }
    }
    saw.then_some(path).ok_or(SftpError::PathInvalid)
}
/// Only constructed by the authenticated SFTP service. A task retains this
/// client's connection lease independently of its originating shell/view.
#[derive(Clone)]
pub struct TransferContext {
    pub(crate) binding: Binding,
    pub(crate) client: Arc<SftpClient>,
    pub(crate) path: RemotePath,
    pub(crate) baseline: Option<Metadata>,
    pub(crate) target: TransferTarget,
    pub(crate) lease: capstan_ssh::ConnectionLease<capstan_ssh::PasswordConnection>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Upload,
    Download,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferState {
    Queued,
    Preparing,
    Running,
    Paused,
    Resuming,
    Retrying,
    Verifying,
    Committing,
    Completed,
    AwaitingConflict,
    Failed,
    Canceled,
    Interrupted,
}
impl TransferState {
    fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Canceled | Self::Interrupted
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Conflict {
    pub revision: ExactU64,
    pub target: Option<Metadata>,
    pub replace_allowed: bool,
    pub batch_eligible: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferSnapshot {
    pub id: Uuid,
    pub host_id: Uuid,
    pub name: String,
    pub remote_path: RemotePath,
    pub direction: Direction,
    pub state: TransferState,
    pub version: ExactU64,
    pub processed: ExactU64,
    pub total: ExactU64,
    pub bytes_per_second: ExactU64,
    pub remaining_seconds: Option<ExactU64>,
    pub error: Option<SftpError>,
    pub conflict: Option<Conflict>,
    pub cleanup_required: bool,
    pub batch: Option<BatchSummary>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_metadata_fixture_roundtrips_without_file_contents_or_lossy_integers() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../docs/agent/fixtures/M2-transfer.json"
        ))
        .unwrap();
        let snapshots: Vec<TransferSnapshot> = serde_json::from_value(fixture.clone()).unwrap();
        assert_eq!(snapshots[0].processed.0, 9_007_199_254_740_993);
        assert_eq!(snapshots[0].total.0, u64::MAX);
        assert_eq!(serde_json::to_value(snapshots).unwrap(), fixture);
    }

    fn test_task(state: TransferState) -> Arc<Task> {
        let (sender, receiver) = mpsc::channel(1);
        Arc::new(Task {
            window: "test".into(),
            target: Mutex::new(TransferTarget::Upload {
                parent: RemotePath::parse("/").unwrap(),
                name: "file".into(),
            }),
            status: Mutex::new(TransferSnapshot {
                id: Uuid::new_v4(),
                host_id: Uuid::new_v4(),
                name: "file".into(),
                remote_path: RemotePath::parse("/file").unwrap(),
                direction: Direction::Upload,
                state,
                version: ExactU64(1),
                processed: ExactU64(0),
                total: ExactU64(10),
                bytes_per_second: ExactU64(0),
                remaining_seconds: None,
                error: None,
                conflict: None,
                cleanup_required: false,
                batch: None,
            }),
            cancel: CancellationToken::new(),
            paused: AtomicBool::new(false),
            resume: Notify::new(),
            decisions: Mutex::new(sender),
            decision_receiver: Mutex::new(Some(receiver)),
            retry: Mutex::new(None),
            started: Mutex::new(None),
            desktop_timing: Mutex::new(AttemptTiming::new()),
            last_progress: Mutex::new(Instant::now()),
            batch: None,
            recovery: None,
        })
    }

    #[tokio::test]
    async fn pause_resume_is_a_cooperative_boundary_between_chunks() {
        let task = test_task(TransferState::Running);
        task.pause().unwrap();
        assert_eq!(task.status.lock().unwrap().state, TransferState::Paused);
        task.resume().unwrap();
        assert_eq!(task.status.lock().unwrap().state, TransferState::Resuming);
        task.wait_if_paused().await.unwrap();
        assert_eq!(task.status.lock().unwrap().state, TransferState::Running);
    }

    #[test]
    fn pause_rejects_final_commit_and_resume_requires_a_pause() {
        let committing = test_task(TransferState::Committing);
        assert_eq!(committing.pause(), Err(SftpError::TargetConflict));
        let running = test_task(TransferState::Running);
        assert_eq!(running.resume(), Err(SftpError::ScopeExpired));
    }

    #[test]
    fn upload_tree_target_keeps_remote_components_explicit() {
        let target: TransferTarget = serde_json::from_str(
            r#"{"kind":"upload_tree","parent":"/srv/drop","relative":"nested/file.txt"}"#,
        )
        .unwrap();
        assert!(matches!(target, TransferTarget::UploadTree { .. }));
        let parent = RemotePath::parse("/srv/drop").unwrap();
        assert!(upload_tree_path(&parent, "../escape").is_err());
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Decision {
    Replace,
    Rename { name: String },
    Skip,
    ReplaceBatch,
    SkipBatch,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSelection {
    pub token: Uuid,
    pub name: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderSelection {
    pub selection: LocalSelection,
    pub relative: String,
}

#[derive(Clone)]
enum Local {
    Source(Arc<dyn LocalSource>),
    Destination(Arc<dyn LocalDestination>),
}
struct Grant {
    binding: Binding,
    expires: Instant,
    local: Local,
    batch: Option<Arc<batch::Member>>,
    target: Option<TransferTarget>,
}
struct Task {
    window: String,
    target: Mutex<TransferTarget>,
    status: Mutex<TransferSnapshot>,
    cancel: CancellationToken,
    paused: AtomicBool,
    resume: Notify,
    decisions: Mutex<mpsc::Sender<Decision>>,
    decision_receiver: Mutex<Option<mpsc::Receiver<Decision>>>,
    retry: Mutex<Option<RetryPlan>>,
    started: Mutex<Option<Instant>>,
    desktop_timing: Mutex<AttemptTiming>,
    last_progress: Mutex<Instant>,
    batch: Option<Arc<batch::Member>>,
    recovery: Option<Arc<RecoveryStore>>,
}
#[derive(Clone)]
struct RetryPlan {
    context: Option<TransferContext>,
    policy: capstan_ssh::PoolKey,
    baseline: Option<Metadata>,
    local: Option<Local>,
    partial: Option<PartialState>,
    workers: Arc<Semaphore>,
    commits: Arc<tokio::sync::Mutex<()>>,
    budget: Arc<budget::Budget>,
}
pub(crate) fn same_retry_policy(
    original: &capstan_ssh::PoolKey,
    current: &capstan_ssh::PoolKey,
) -> bool {
    let mut original = original.clone();
    original.vault_epoch = current.vault_epoch;
    original == *current
}
pub(crate) fn retry_policy_digest(policy: &capstan_ssh::PoolKey) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(b"capstan-transfer-policy-v1");
    for value in [
        &policy.target,
        &policy.username,
        &policy.trust_fingerprint,
        &policy.proxy_path,
    ] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    digest.update(policy.port.to_be_bytes());
    digest.update(policy.credential_id.as_bytes());
    digest.update(policy.credential_revision.to_be_bytes());
    digest.update([
        u8::from(policy.agent_forwarding),
        u8::from(policy.agent_identity.is_some()),
    ]);
    if let Some(identity) = &policy.agent_identity {
        digest.update((identity.len() as u64).to_be_bytes());
        digest.update(identity.as_bytes());
    }
    // Explicit unlock changes the epoch, never the authenticated policy.
    format!("{:x}", digest.finalize())
}
#[derive(Clone)]
pub(crate) enum PartialState {
    Upload {
        path: RemotePath,
    },
    Download {
        size: u64,
        local_partial: Option<PathBuf>,
    },
}
/// Opaque, Rust-owned download descriptor; never accepted from IPC.
pub struct RecoveredDownload {
    pub(crate) host: Uuid,
    pub(crate) path: RemotePath,
    pub(crate) target: TransferTarget,
    pub(crate) total: u64,
    pub(crate) policy_sha256: String,
    pub(crate) baseline: Option<Metadata>,
}
impl Task {
    fn target(&self) -> Result<TransferTarget, SftpError> {
        self.target
            .lock()
            .map(|target| target.clone())
            .map_err(|_| SftpError::ConnectionLost)
    }
    fn retarget(&self, context: &TransferContext) -> Result<(), SftpError> {
        *self.target.lock().map_err(|_| SftpError::ConnectionLost)? = context.target.clone();
        if let Some(plan) = self
            .retry
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .as_mut()
        {
            plan.context = Some(context.clone());
        }
        Ok(())
    }
    fn take_decisions(&self) -> Option<mpsc::Receiver<Decision>> {
        self.decision_receiver.lock().ok()?.take()
    }
    fn clear_retry(&self) {
        if let Ok(mut retry) = self.retry.lock() {
            retry.take();
        }
    }
    fn retain_partial(
        &self,
        context: TransferContext,
        local: Local,
        partial: PartialState,
    ) -> Result<(), SftpError> {
        if let Local::Destination(destination) = &local {
            destination.preserve_partial()?;
        }
        let target = context.target.clone();
        let status = self
            .status
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .clone();
        let mut retry = self.retry.lock().map_err(|_| SftpError::ConnectionLost)?;
        let plan = retry.as_mut().ok_or(SftpError::ScopeExpired)?;
        plan.context = None;
        // A remote partial can be resumed only after a new picker/session
        // binding. Drop the old upload source immediately so its native file
        // handle is not retained after a disconnected transfer. A local
        // destination owns the partial itself and must remain available for
        // an explicit retry.
        plan.local = matches!(&partial, PartialState::Download { .. }).then_some(local.clone());
        plan.partial = Some(partial.clone());
        if let Some(store) = &self.recovery {
            let (remote_partial, local_partial, local_target, partial_size) = match partial {
                PartialState::Upload { path } => {
                    let previous = store
                        .record(status.id)
                        .ok()
                        .and_then(|record| record.partial_size)
                        .unwrap_or(0);
                    (
                        Some(path),
                        None,
                        None,
                        Some(previous.max(status.processed.0)),
                    )
                }
                PartialState::Download {
                    size,
                    local_partial,
                } => {
                    let target = match &local {
                        Local::Destination(destination) => destination.target_path().ok().flatten(),
                        Local::Source(_) => None,
                    };
                    (None, local_partial, target, Some(size))
                }
            };
            let record = make_record(RecoveryDraft {
                id: status.id,
                host_id: status.host_id,
                name: status.name,
                remote_path: status.remote_path,
                target,
                direction: status.direction,
                total: status.total.0,
                partial_size,
                remote_partial,
                local_partial,
                local_target,
                local_identity: match &local {
                    Local::Destination(destination) => destination.partial_identity()?,
                    Local::Source(_) => None,
                },
                source_baseline: (status.direction == Direction::Download)
                    .then(|| context.baseline.clone())
                    .flatten(),
                policy_sha256: Some(retry_policy_digest(context.lease.policy())),
            })?;
            store.upsert(record)?;
        }
        Ok(())
    }
    fn clear_recovery(&self) -> Result<(), SftpError> {
        if let Some(store) = &self.recovery {
            let id = self
                .status
                .lock()
                .map_err(|_| SftpError::ConnectionLost)?
                .id;
            store.remove(id)?;
        }
        Ok(())
    }
    pub(crate) fn checkpoint_recovery(
        &self,
        context: &TransferContext,
        remote_partial: Option<RemotePath>,
        local_partial: Option<PathBuf>,
        local_target: Option<PathBuf>,
        partial_size: Option<u64>,
    ) -> Result<(), SftpError> {
        let Some(store) = &self.recovery else {
            return Ok(());
        };
        let status = self
            .status
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .clone();
        store.upsert(make_record(RecoveryDraft {
            id: status.id,
            host_id: status.host_id,
            name: status.name,
            remote_path: status.remote_path,
            target: context.target.clone(),
            direction: status.direction,
            total: status.total.0,
            partial_size,
            remote_partial,
            local_partial,
            local_target,
            local_identity: self
                .retry
                .lock()
                .map_err(|_| SftpError::ConnectionLost)?
                .as_ref()
                .and_then(|plan| match &plan.local {
                    Some(Local::Destination(destination)) => {
                        destination.partial_identity().ok().flatten()
                    }
                    _ => None,
                }),
            source_baseline: (status.direction == Direction::Download)
                .then(|| context.baseline.clone())
                .flatten(),
            policy_sha256: Some(retry_policy_digest(context.lease.policy())),
        })?)
    }
    fn pause(&self) -> Result<(), SftpError> {
        let mut status = self.status.lock().map_err(|_| SftpError::ConnectionLost)?;
        if status.state != TransferState::Running {
            return Err(if status.state == TransferState::Committing {
                SftpError::TargetConflict
            } else {
                SftpError::ScopeExpired
            });
        }
        self.paused.store(true, Ordering::SeqCst);
        status.state = TransferState::Paused;
        status.version.0 += 1;
        Ok(())
    }
    fn resume(&self) -> Result<(), SftpError> {
        let mut status = self.status.lock().map_err(|_| SftpError::ConnectionLost)?;
        if status.state != TransferState::Paused {
            return Err(SftpError::ScopeExpired);
        }
        self.paused.store(false, Ordering::SeqCst);
        status.state = TransferState::Resuming;
        status.version.0 += 1;
        self.resume.notify_one();
        Ok(())
    }
    async fn wait_if_paused(&self) -> Result<(), SftpError> {
        loop {
            if !self.paused.load(Ordering::SeqCst) {
                let mut status = self.status.lock().map_err(|_| SftpError::ConnectionLost)?;
                if status.state == TransferState::Resuming {
                    status.state = TransferState::Running;
                    status.version.0 += 1;
                }
                return self.check();
            }
            tokio::select! {
                biased;
                () = self.cancel.cancelled() => return Err(SftpError::OperationCanceled),
                () = self.resume.notified() => {},
            }
        }
    }
}
impl Task {
    fn update(&self, operation: impl FnOnce(&mut TransferSnapshot)) -> Result<(), SftpError> {
        let mut status = self.status.lock().map_err(|_| SftpError::ConnectionLost)?;
        if !status.state.terminal() {
            operation(&mut status);
            if status.state.terminal() {
                if let Ok(mut timing) = self.desktop_timing.lock() {
                    timing.finished = Some(Instant::now());
                }
            }
            status.version.0 += 1;
        }
        Ok(())
    }
    fn check(&self) -> Result<(), SftpError> {
        if self.cancel.is_cancelled() {
            Err(SftpError::OperationCanceled)
        } else {
            Ok(())
        }
    }
    fn phase(&self, state: TransferState) -> Result<(), SftpError> {
        let mut status = self.status.lock().map_err(|_| SftpError::ConnectionLost)?;
        self.check()?;
        if state == TransferState::Running {
            *self.started.lock().map_err(|_| SftpError::ConnectionLost)? = Some(Instant::now());
        }
        if state == TransferState::Verifying {
            status.processed = status.total;
            status.remaining_seconds = Some(ExactU64(0));
        }
        status.state = state;
        status.conflict = None;
        status.version.0 += 1;
        Ok(())
    }
    fn progress(&self, processed: u64) -> Result<(), SftpError> {
        let mut last = self
            .last_progress
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?;
        if last.elapsed() < Duration::from_millis(crate::sftp::PROGRESS_INTERVAL_MS) {
            return Ok(());
        }
        *last = Instant::now();
        let millis = self
            .started
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .map_or(1, |started| started.elapsed().as_millis().max(1));
        self.update(|status| {
            status.processed = ExactU64(processed);
            let speed = (u128::from(processed) * 1000 / millis).min(u128::from(u64::MAX)) as u64;
            status.bytes_per_second = ExactU64(speed);
            status.remaining_seconds = (speed > 0)
                .then(|| ExactU64(status.total.0.saturating_sub(processed).div_ceil(speed)));
        })
    }
}
struct AttemptTiming {
    started: Instant,
    finished: Option<Instant>,
}

impl AttemptTiming {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            finished: None,
        }
    }
}

#[derive(Default)]
struct State {
    grants: HashMap<Uuid, Grant>,
    tasks: Vec<Arc<Task>>,
    closing: HashSet<String>,
}
impl State {
    fn reserve_tasks(&mut self, count: usize) -> Result<(), SftpError> {
        let mut completed = self
            .tasks
            .iter()
            .filter(|task| task.status.lock().is_ok_and(|s| s.state.terminal()))
            .count();
        self.tasks.retain(|task| {
            if completed >= crate::sftp::MAX_FINAL_TRANSFERS
                && task.status.lock().is_ok_and(|s| s.state.terminal())
            {
                completed -= 1;
                false
            } else {
                true
            }
        });
        if self.tasks.len().saturating_add(count) > MAX_QUEUED_TRANSFERS {
            return Err(SftpError::ResourceLimit);
        }
        Ok(())
    }
}
pub struct TransferService {
    connections: Mutex<Vec<(String, std::sync::Weak<capstan_ssh::PasswordConnection>)>>,
    state: Mutex<State>,
    workers: Arc<Semaphore>,
    commits: Arc<tokio::sync::Mutex<()>>,
    budget: Arc<budget::Budget>,
    recovery: Mutex<Option<Arc<RecoveryStore>>>,
    #[cfg(windows)]
    native: Mutex<native::Registry>,
    #[cfg(windows)]
    trees: Mutex<HashMap<Uuid, tree::PlanHandle>>,
    reaper_started: std::sync::atomic::AtomicBool,
}
impl Default for TransferService {
    fn default() -> Self {
        Self {
            connections: Mutex::new(Vec::new()),
            state: Mutex::new(State::default()),
            workers: Arc::new(Semaphore::new(TRANSFER_WORKERS)),
            commits: Arc::new(tokio::sync::Mutex::new(())),
            budget: Arc::new(budget::Budget::default()),
            recovery: Mutex::new(None),
            #[cfg(windows)]
            native: Mutex::new(native::Registry::default()),
            #[cfg(windows)]
            trees: Mutex::new(HashMap::new()),
            reaper_started: std::sync::atomic::AtomicBool::new(false),
        }
    }
}
impl TransferService {
    pub(crate) fn has_connection_work(&self, policy: &capstan_ssh::PoolKey) -> bool {
        self.state.lock().is_ok_and(|state| {
            state.tasks.iter().any(|task| {
                let Ok(status) = task.status.lock() else {
                    return false;
                };
                task.retry.lock().is_ok_and(|retry| {
                    retry.as_ref().is_some_and(|plan| {
                        same_retry_policy(&plan.policy, policy)
                            && (!status.state.terminal()
                                || (status.state == TransferState::Interrupted
                                    && status.error == Some(SftpError::ConnectionLost)
                                    && plan.local.is_some()
                                    && matches!(plan.partial, Some(PartialState::Download { .. }))))
                    })
                })
            })
        })
    }

    /// Restore only retained download grants after a transient disconnect.
    /// Upload grants are intentionally released by M2 and require a fresh picker.
    pub async fn recover_connection(
        &self,
        lease: capstan_ssh::ConnectionLease<capstan_ssh::PasswordConnection>,
        cancel: &CancellationToken,
    ) {
        let tasks = match self.state.lock() {
            Ok(state) => state
                .tasks
                .iter()
                .filter_map(|task| {
                    let status = task.status.lock().ok()?;
                    let retry = task.retry.lock().ok()?;
                    let plan = retry.as_ref()?;
                    (status.state == TransferState::Interrupted
                        && status.error == Some(SftpError::ConnectionLost)
                        && status.direction == Direction::Download
                        && plan.local.is_some()
                        && matches!(plan.partial, Some(PartialState::Download { .. }))
                        && same_retry_policy(&plan.policy, lease.policy()))
                    .then(|| {
                        (
                            task.clone(),
                            status.id,
                            status.host_id,
                            status.remote_path.clone(),
                            plan.baseline.clone(),
                            status.version,
                        )
                    })
                })
                .collect::<Vec<_>>(),
            Err(_) => return,
        };
        for (task, id, host, path, baseline, version) in tasks {
            if cancel.is_cancelled() || task.cancel.is_cancelled() {
                return;
            }
            let context = async {
                let client = lease
                    .open_sftp(cancel)
                    .await
                    .map_err(crate::sftp::service::map_error)?;
                let attrs = client
                    .lstat(path.as_str(), cancel)
                    .await
                    .map_err(crate::sftp::service::map_error)?;
                let current = crate::sftp::service::metadata(attrs);
                if baseline
                    .as_ref()
                    .is_none_or(|saved| !current.matches_baseline(saved))
                {
                    return Err(SftpError::ResumeMismatch);
                }
                Ok(TransferContext {
                    binding: Binding {
                        window: task.window.clone(),
                        terminal: Uuid::new_v4(),
                        connection: lease.connection().connection_id(),
                        generation: lease.connection().connection_generation(),
                        host,
                    },
                    client,
                    path,
                    baseline: Some(current),
                    target: task.target()?,
                    lease: lease.clone(),
                })
            }
            .await;
            match context.and_then(|context| self.retry_with_context(&task.window, id, context)) {
                Ok(()) => {}
                Err(error) => {
                    if let Ok(mut status) = task.status.lock() {
                        if status.state == TransferState::Interrupted && status.version == version {
                            status.error = Some(error);
                            status.version.0 += 1;
                        }
                    }
                }
            }
        }
    }
    /// Configure the Rust-selected application data directory once the Tauri
    /// setup hook has resolved portable versus roaming storage.
    pub fn configure_recovery_directory(
        &self,
        directory: std::path::PathBuf,
    ) -> Result<(), SftpError> {
        let store = Arc::new(
            RecoveryStore::open(directory.join("transfer-recovery"))
                .map_err(|_| SftpError::LocalIo)?,
        );
        *self
            .recovery
            .lock()
            .map_err(|_| SftpError::ConnectionLost)? = Some(store);
        Ok(())
    }

    pub fn recovery_snapshot(&self) -> Result<TransferRecoverySnapshot, SftpError> {
        let store = self
            .recovery
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .clone();
        store.map_or(
            Ok(TransferRecoverySnapshot {
                entries: Vec::new(),
                incomplete: false,
            }),
            |store| store.snapshot(),
        )
    }

    pub fn forget_recovery(&self, id: Uuid) -> Result<(), SftpError> {
        let store = self
            .recovery
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .clone()
            .ok_or(SftpError::ScopeExpired)?;
        store.remove(id)
    }

    pub fn recovery_target(&self, window: &str, id: Uuid) -> Result<TransferTarget, SftpError> {
        if window != "main" {
            return Err(SftpError::ScopeExpired);
        }
        let store = self
            .recovery
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .clone()
            .ok_or(SftpError::ScopeExpired)?;
        let record = store.record(id)?;
        if record.policy_sha256.is_none() || record.mode == recovery::RecoveryMode::Inspect {
            return Err(SftpError::ResumeMismatch);
        }
        Ok(record.target)
    }

    pub fn recovered_download(
        &self,
        window: &str,
        id: Uuid,
    ) -> Result<RecoveredDownload, SftpError> {
        let record = self.recovery_record(window, id)?;
        if record.direction != Direction::Download || record.mode == recovery::RecoveryMode::Inspect
        {
            return Err(SftpError::ScopeExpired);
        }
        // Older ledgers cannot prove which remote source version was read.
        // Keep them discoverable, but never invent a baseline for continuation.
        let baseline = record.source_baseline.ok_or(SftpError::ResumeMismatch)?;
        Ok(RecoveredDownload {
            host: record.host_id,
            path: record.remote_path,
            target: record.target,
            total: record.total,
            policy_sha256: record.policy_sha256.ok_or(SftpError::ResumeMismatch)?,
            baseline: Some(baseline),
        })
    }

    /// Reauthorize a live failed download from its retained Rust task rather
    /// than trying to resolve its old entry ID in a replacement browse scope.
    pub fn download_retry(&self, window: &str, id: Uuid) -> Result<RecoveredDownload, SftpError> {
        let task = self.task(window, id)?;
        let status = task.status.lock().map_err(|_| SftpError::ConnectionLost)?;
        if status.direction != Direction::Download
            || !matches!(
                status.state,
                TransferState::Failed | TransferState::Interrupted
            )
        {
            return Err(SftpError::ScopeExpired);
        }
        let retry = task.retry.lock().map_err(|_| SftpError::ConnectionLost)?;
        let retry = retry.as_ref().ok_or(SftpError::ScopeExpired)?;
        Ok(RecoveredDownload {
            host: status.host_id,
            path: status.remote_path.clone(),
            target: task.target()?,
            total: status.total.0,
            policy_sha256: retry_policy_digest(&retry.policy),
            baseline: retry.baseline.clone(),
        })
    }

    fn recovery_record(
        &self,
        window: &str,
        id: Uuid,
    ) -> Result<recovery::RecoveryRecord, SftpError> {
        if window != "main" {
            return Err(SftpError::ScopeExpired);
        }
        self.recovery
            .lock()
            .map_err(|_| SftpError::LocalIo)?
            .as_ref()
            .ok_or(SftpError::ScopeExpired)?
            .record(id)
    }

    /// A fresh native save picker authorizes only this exact destination.
    /// Ledger paths alone never grant local access or cleanup authority.
    pub async fn grant_recovered_destination(
        self: &Arc<Self>,
        window: &str,
        id: Uuid,
        context: &TransferContext,
        destination: Arc<dyn LocalDestination>,
    ) -> Result<LocalSelection, SftpError> {
        let record = self.recovery_record(window, id)?;
        if context.binding.window != window
            || record.mode == recovery::RecoveryMode::Inspect
            || record.host_id != context.binding.host
            || record.remote_path != context.path
            || record.target != context.target
            || record.direction != Direction::Download
        {
            return Err(SftpError::ScopeExpired);
        }
        if record.policy_sha256.as_ref().is_none_or(|saved| {
            !saved.eq_ignore_ascii_case(&retry_policy_digest(context.lease.policy()))
        }) {
            return Err(SftpError::ScopeExpired);
        }
        let path = record.local_partial.ok_or(SftpError::ResumeMismatch)?;
        let identity = record.local_identity.ok_or(SftpError::ResumeMismatch)?;
        let selected = destination.clone();
        local::blocking(move || {
            if selected.target_path()? != record.local_target {
                return Err(SftpError::ResumeMismatch);
            }
            selected.recover_partial(
                &path,
                identity,
                record.partial_size.unwrap_or(0),
                record.total,
            )
        })
        .await?;
        self.grant_destination(context, destination)
    }

    pub fn retry_recovered_download(
        self: &Arc<Self>,
        window: &str,
        id: Uuid,
        context: TransferContext,
        selection: Uuid,
    ) -> Result<(), SftpError> {
        self.retry_recovered(window, id, context, selection, Direction::Download)
    }

    /// Resume a discovered upload only after the caller binds a fresh
    /// authenticated SFTP context and a fresh native source selection.
    pub fn retry_recovered_upload(
        self: &Arc<Self>,
        window: &str,
        id: Uuid,
        context: TransferContext,
        selection: Uuid,
    ) -> Result<(), SftpError> {
        self.retry_recovered(window, id, context, selection, Direction::Upload)
    }

    fn retry_recovered(
        self: &Arc<Self>,
        window: &str,
        id: Uuid,
        context: TransferContext,
        selection: Uuid,
        direction: Direction,
    ) -> Result<(), SftpError> {
        if window != "main" || context.binding.window != window {
            return Err(SftpError::ScopeExpired);
        }
        let store = self
            .recovery
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .clone()
            .ok_or(SftpError::ScopeExpired)?;
        let record = store.record(id)?;
        if record.mode != recovery::RecoveryMode::Resume
            || record.direction != direction
            || record.host_id != context.binding.host
            || record.remote_path != context.path
            || record.target != context.target
        {
            return Err(SftpError::ScopeExpired);
        }
        if record.policy_sha256.as_ref().is_none_or(|saved| {
            !saved.eq_ignore_ascii_case(&retry_policy_digest(context.lease.policy()))
        }) {
            return Err(SftpError::ScopeExpired);
        }
        if direction == Direction::Download
            && record.source_baseline.as_ref().is_none_or(|original| {
                context
                    .baseline
                    .as_ref()
                    .is_none_or(|current| !current.matches_baseline(original))
            })
        {
            return Err(SftpError::ResumeMismatch);
        }
        let mut state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
        if state.closing.contains(window)
            || state
                .tasks
                .iter()
                .any(|task| task.status.lock().is_ok_and(|status| status.id == id))
        {
            return Err(SftpError::ScopeExpired);
        }
        let grant = state
            .grants
            .get(&selection)
            .filter(|grant| grant.binding == context.binding && grant.expires > Instant::now())
            .ok_or(SftpError::ScopeExpired)?;
        let partial = match (&grant.local, direction) {
            (Local::Source(source), Direction::Upload) if source.size() == record.total => {
                PartialState::Upload {
                    path: record
                        .remote_partial
                        .clone()
                        .ok_or(SftpError::ScopeExpired)?,
                }
            }
            (Local::Destination(destination), Direction::Download) => {
                if destination.target_path()? != record.local_target
                    || destination.partial_identity()? != record.local_identity
                    || record.local_identity.is_none()
                    || destination.partial_path()? != record.local_partial
                {
                    return Err(SftpError::ResumeMismatch);
                }
                let size = destination
                    .partial_size()?
                    .ok_or(SftpError::ResumeMismatch)?;
                if !valid_cleanup_size(&record, size) {
                    return Err(SftpError::ResumeMismatch);
                }
                PartialState::Download {
                    size,
                    local_partial: record.local_partial.clone(),
                }
            }
            _ => return Err(SftpError::ResumeMismatch),
        };
        if grant
            .target
            .as_ref()
            .is_some_and(|target| target != &context.target)
        {
            return Err(SftpError::ScopeExpired);
        }
        let local = grant.local.clone();
        let task = self.make_task(&context, &local, None)?;
        task.status
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .id = id;
        let retry = task.retry.lock().map_err(|_| SftpError::ConnectionLost)?;
        let mut plan = retry.clone().ok_or(SftpError::ScopeExpired)?;
        plan.partial = Some(partial.clone());
        drop(retry);
        *task.retry.lock().map_err(|_| SftpError::ConnectionLost)? = Some(plan);
        state.reserve_tasks(1)?;
        state
            .grants
            .remove(&selection)
            .ok_or(SftpError::ScopeExpired)?;
        state.tasks.push(task.clone());
        self.spawn_task_with_partial(task, context, local, Some(partial));
        Ok(())
    }

    /// Consumes only a freshly reauthorized local destination grant. Missing,
    /// changed, legacy and unselected partials remain untouched.
    pub async fn cleanup_recovered_download(
        &self,
        window: &str,
        id: Uuid,
        selection: Uuid,
    ) -> Result<(), SftpError> {
        let record = self.recovery_record(window, id)?;
        if record.mode == recovery::RecoveryMode::Inspect {
            return Err(SftpError::ResumeMismatch);
        }
        let destination = {
            let mut state = self.state.lock().map_err(|_| SftpError::LocalIo)?;
            if state.closing.contains(window)
                || state.tasks.iter().any(|task| {
                    task.status
                        .lock()
                        .is_ok_and(|status| status.id == id && !status.state.terminal())
                })
            {
                return Err(SftpError::ScopeExpired);
            }
            let grant = state
                .grants
                .get(&selection)
                .filter(|grant| {
                    grant.binding.window == window
                        && grant.binding.host == record.host_id
                        && grant.expires > Instant::now()
                })
                .ok_or(SftpError::ScopeExpired)?;
            let Local::Destination(destination) = &grant.local else {
                return Err(SftpError::ScopeExpired);
            };
            if record.direction != Direction::Download
                || record.local_identity.is_none()
                || destination.partial_identity()? != record.local_identity
                || destination.target_path()? != record.local_target
                || destination.partial_path()? != record.local_partial
            {
                return Err(SftpError::ResumeMismatch);
            }
            let destination = destination.clone();
            state.grants.remove(&selection);
            destination
        };
        local::blocking(move || destination.discard()).await?;
        self.forget_recovery(id)
    }

    /// Delete a discovered remote upload partial only after an explicit UI
    /// decision and a fresh, host-matched SFTP binding.
    pub async fn cleanup_recovered_upload(
        &self,
        window: &str,
        id: Uuid,
        context: TransferContext,
    ) -> Result<(), SftpError> {
        if window != "main" || context.binding.window != window {
            return Err(SftpError::ScopeExpired);
        }
        let store = self
            .recovery
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .clone()
            .ok_or(SftpError::ScopeExpired)?;
        let record = store.record(id)?;
        if record.mode == recovery::RecoveryMode::Inspect {
            return Err(SftpError::ResumeMismatch);
        }
        let partial = record
            .remote_partial
            .clone()
            .ok_or(SftpError::ScopeExpired)?;
        if record.direction != Direction::Upload
            || record.host_id != context.binding.host
            || record.remote_path != context.path
            || record.target != context.target
        {
            return Err(SftpError::ScopeExpired);
        }
        if record.policy_sha256.as_ref().is_none_or(|saved| {
            !saved.eq_ignore_ascii_case(&retry_policy_digest(context.lease.policy()))
        }) {
            return Err(SftpError::ScopeExpired);
        }
        if self
            .state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .tasks
            .iter()
            .any(|task| {
                task.status
                    .lock()
                    .is_ok_and(|status| status.id == id && !status.state.terminal())
            })
        {
            return Err(SftpError::ScopeExpired);
        }
        // A stale resumable checkpoint can survive a failed inspection-only
        // downgrade. It must never regain pathname deletion authority after
        // restart. Continuation still needs fresh source/prefix verification.
        let cleanup_baseline = recovery::upload_cleanup_baseline(&record)?;
        let cancel = CancellationToken::new();
        let attrs = match context.client.lstat(partial.as_str(), &cancel).await {
            Ok(attrs) => attrs,
            Err(capstan_ssh::sftp::ClientError::NotFound) => {
                store.remove(id)?;
                return Ok(());
            }
            Err(error) => return Err(crate::sftp::service::map_error(error)),
        };
        let attrs = crate::sftp::service::metadata(attrs);
        let size = attrs.size.ok_or(SftpError::ResumeMismatch)?.0;
        if attrs.kind != crate::sftp::FileKind::File
            || !valid_cleanup_size(&record, size)
            || !attrs.matches_baseline(cleanup_baseline)
        {
            return Err(SftpError::ResumeMismatch);
        }
        context
            .client
            .remove_file(partial.as_str(), &cancel)
            .await
            .map_err(crate::sftp::service::map_error)?;
        store.remove(id)
    }

    /// Editors share the final validation/publication gate with transfer jobs.
    /// No workflow holds it while waiting for a user's conflict decision.
    pub fn publication_gate(&self) -> Arc<tokio::sync::Mutex<()>> {
        self.commits.clone()
    }
    /// Trusted native-picker path only. No command accepts a filesystem path.
    pub fn grant_source(
        self: &Arc<Self>,
        context: &TransferContext,
        source: Arc<dyn LocalSource>,
    ) -> Result<LocalSelection, SftpError> {
        let name = source.name()?;
        self.grant(context, Local::Source(source), name)
    }
    pub fn grant_source_named(
        self: &Arc<Self>,
        context: &TransferContext,
        source: Arc<dyn LocalSource>,
        name: String,
    ) -> Result<LocalSelection, SftpError> {
        self.grant(context, Local::Source(source), name)
    }
    pub fn grant_destination(
        self: &Arc<Self>,
        context: &TransferContext,
        destination: Arc<dyn LocalDestination>,
    ) -> Result<LocalSelection, SftpError> {
        let name = destination.name()?;
        self.grant(context, Local::Destination(destination), name)
    }
    fn grant(
        self: &Arc<Self>,
        context: &TransferContext,
        local: Local,
        name: String,
    ) -> Result<LocalSelection, SftpError> {
        let mut state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
        if state.closing.contains(&context.binding.window) {
            return Err(SftpError::ScopeExpired);
        }
        state
            .grants
            .retain(|_, grant| grant.expires > Instant::now());
        if state.grants.len() >= crate::sftp::MAX_LOCAL_GRANTS {
            return Err(SftpError::ResourceLimit);
        }
        let token = Uuid::new_v4();
        state.grants.insert(
            token,
            Grant {
                binding: context.binding.clone(),
                expires: Instant::now()
                    + Duration::from_secs(crate::sftp::LOCAL_GRANT_LIFETIME_SECONDS),
                local,
                batch: None,
                target: None,
            },
        );
        drop(state);
        self.start_reaper();
        Ok(LocalSelection { token, name })
    }
    fn start_reaper(self: &Arc<Self>) {
        if !self
            .reaper_started
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            let weak = Arc::downgrade(self);
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    let Some(service) = weak.upgrade() else {
                        break;
                    };
                    if let Ok(mut state) = service.state.lock() {
                        state
                            .grants
                            .retain(|_, grant| grant.expires > Instant::now());
                    };
                    #[cfg(windows)]
                    {
                        if let Ok(mut trees) = service.trees.lock() {
                            trees.retain(|_, handle| handle.expires > Instant::now());
                        };
                    }
                }
            });
        }
    }
    pub fn release_selection(&self, window: &str, token: Uuid) -> Result<(), SftpError> {
        let mut state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
        if state
            .grants
            .get(&token)
            .is_some_and(|grant| grant.binding.window == window)
        {
            state.grants.remove(&token);
            Ok(())
        } else {
            Err(SftpError::ScopeExpired)
        }
    }
    pub fn enqueue(
        self: &Arc<Self>,
        context: TransferContext,
        selection: Uuid,
    ) -> Result<Uuid, SftpError> {
        let mut state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
        if state.closing.contains(&context.binding.window) {
            return Err(SftpError::ScopeExpired);
        }
        let grant = state
            .grants
            .get(&selection)
            .filter(|grant| grant.binding == context.binding && grant.expires > Instant::now())
            .ok_or(SftpError::ScopeExpired)?;
        let direction = if context.baseline.is_some() {
            Direction::Download
        } else {
            Direction::Upload
        };
        if !matches!(
            (&grant.local, direction),
            (Local::Source(_), Direction::Upload) | (Local::Destination(_), Direction::Download)
        ) {
            return Err(SftpError::ScopeExpired);
        }
        if grant
            .target
            .as_ref()
            .is_some_and(|target| target != &context.target)
        {
            return Err(SftpError::ScopeExpired);
        }
        let local = grant.local.clone();
        let task = self.make_task(&context, &local, grant.batch.clone())?;
        state.reserve_tasks(1)?;
        state
            .grants
            .remove(&selection)
            .ok_or(SftpError::ScopeExpired)?;
        let id = task
            .status
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .id;
        state.tasks.push(task.clone());
        self.spawn_task(task, context, local);
        Ok(id)
    }
    fn track_client(&self, context: &TransferContext) -> Result<(), SftpError> {
        {
            let mut connections = self
                .connections
                .lock()
                .map_err(|_| SftpError::ConnectionLost)?;
            connections.retain(|(_, connection)| connection.strong_count() > 0);
            let connection = context.lease.downgrade_connection();
            if !connections.iter().any(|(window, previous)| {
                window == &context.binding.window && previous.ptr_eq(&connection)
            }) {
                if connections.len() >= MAX_QUEUED_TRANSFERS {
                    return Err(SftpError::ResourceLimit);
                }
                connections.push((context.binding.window.clone(), connection));
            }
        }
        Ok(())
    }
    fn make_task(
        &self,
        context: &TransferContext,
        local: &Local,
        batch: Option<Arc<batch::Member>>,
    ) -> Result<Arc<Task>, SftpError> {
        self.track_client(context)?;
        let direction = match local {
            Local::Source(_) => Direction::Upload,
            Local::Destination(_) => Direction::Download,
        };
        let id = Uuid::new_v4();
        let (decisions, receiver) = mpsc::channel(1);
        let task = Arc::new(Task {
            window: context.binding.window.clone(),
            target: Mutex::new(context.target.clone()),
            cancel: CancellationToken::new(),
            paused: AtomicBool::new(false),
            resume: Notify::new(),
            decisions: Mutex::new(decisions),
            decision_receiver: Mutex::new(Some(receiver)),
            retry: Mutex::new(Some(RetryPlan {
                context: Some(context.clone()),
                policy: context.lease.policy().clone(),
                baseline: context.baseline.clone(),
                local: Some(local.clone()),
                partial: None,
                workers: self.workers.clone(),
                commits: self.commits.clone(),
                budget: self.budget.clone(),
            })),
            started: Mutex::new(None),
            desktop_timing: Mutex::new(AttemptTiming::new()),
            last_progress: Mutex::new(Instant::now()),
            status: Mutex::new(TransferSnapshot {
                id,
                host_id: context.binding.host,
                name: match &local {
                    Local::Source(_) => context
                        .path
                        .as_str()
                        .rsplit('/')
                        .next()
                        .unwrap_or("")
                        .to_owned(),
                    Local::Destination(destination) => destination.name()?,
                },
                remote_path: context.path.clone(),
                direction,
                state: TransferState::Queued,
                version: ExactU64(1),
                processed: ExactU64(0),
                total: ExactU64(match &local {
                    Local::Source(source) => source.size(),
                    Local::Destination(_) => {
                        context
                            .baseline
                            .as_ref()
                            .and_then(|m| m.size)
                            .ok_or(SftpError::UnsupportedFileType)?
                            .0
                    }
                }),
                bytes_per_second: ExactU64(0),
                remaining_seconds: None,
                error: None,
                conflict: None,
                cleanup_required: false,
                batch: batch.as_ref().map(|member| member.batch.summary.clone()),
            }),
            batch,
            recovery: self
                .recovery
                .lock()
                .map_err(|_| SftpError::ConnectionLost)?
                .clone(),
        });
        Ok(task)
    }
    fn spawn_task(&self, task: Arc<Task>, context: TransferContext, local: Local) {
        self.spawn_task_with_partial(task, context, local, None);
    }
    fn spawn_task_with_partial(
        &self,
        task: Arc<Task>,
        context: TransferContext,
        local: Local,
        partial: Option<PartialState>,
    ) {
        let workers = self.workers.clone();
        let commits = self.commits.clone();
        let budget = self.budget.clone();
        tokio::spawn(async move {
            engine::run(task, context, local, workers, commits, budget, partial).await;
        });
    }
    pub fn snapshot(&self, window: &str) -> Result<Vec<TransferSnapshot>, SftpError> {
        self.state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .tasks
            .iter()
            .filter(|task| task.window == window)
            .map(|task| {
                task.status
                    .lock()
                    .map(|status| status.clone())
                    .map_err(|_| SftpError::ConnectionLost)
            })
            .collect()
    }

    /// Rust-only desktop observation. Paths, names and credentials are excluded.
    pub(crate) fn desktop_snapshot(
        &self,
        window: &str,
        reconnecting: &[capstan_ssh::PoolKey],
    ) -> Result<Vec<crate::desktop::DesktopTransfer>, SftpError> {
        self.state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .tasks
            .iter()
            .filter(|task| task.window == window)
            .map(|task| {
                let status = task.status.lock().map_err(|_| SftpError::ConnectionLost)?;
                let timing = task
                    .desktop_timing
                    .lock()
                    .map_err(|_| SftpError::ConnectionLost)?;
                let awaiting_reconnect = status.state == TransferState::Interrupted
                    && !reconnecting.is_empty()
                    && task
                        .retry
                        .lock()
                        .map_err(|_| SftpError::ConnectionLost)?
                        .as_ref()
                        .is_some_and(|plan| {
                            reconnecting
                                .iter()
                                .any(|policy| same_retry_policy(&plan.policy, policy))
                        });
                Ok(crate::desktop::DesktopTransfer {
                    id: status.id,
                    version: status.version.0,
                    batch_id: status.batch.as_ref().map(|batch| batch.id),
                    state: status.state,
                    processed: status.processed.0,
                    total: status.total.0,
                    attempt_started: timing.started,
                    finished: timing.finished,
                    awaiting_reconnect,
                })
            })
            .collect()
    }
    /// Explicit measurement builds only; no application command exposes it.
    #[cfg(feature = "benchmarks")]
    pub fn buffer_usage(&self) -> (usize, usize) {
        self.budget.usage()
    }
    fn task(&self, window: &str, id: Uuid) -> Result<Arc<Task>, SftpError> {
        self.state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .tasks
            .iter()
            .find(|task| {
                task.window == window && task.status.lock().is_ok_and(|status| status.id == id)
            })
            .cloned()
            .ok_or(SftpError::ScopeExpired)
    }
    pub fn cancel(&self, window: &str, id: Uuid) -> Result<(), SftpError> {
        let task = self.task(window, id)?;
        let status = task.status.lock().map_err(|_| SftpError::ConnectionLost)?;
        // The final filesystem rename cannot be undone. Once it begins, report
        // its actual result instead of labeling a committed file as canceled.
        if status.state == TransferState::Committing {
            return Err(SftpError::TargetConflict);
        }
        if !status.state.terminal() {
            task.cancel.cancel();
        }
        Ok(())
    }
    pub fn pause(&self, window: &str, id: Uuid) -> Result<(), SftpError> {
        self.task(window, id)?.pause()
    }
    pub fn resume(&self, window: &str, id: Uuid) -> Result<(), SftpError> {
        self.task(window, id)?.resume()
    }
    /// Return the original target so a trusted Rust caller can bind a retry to
    /// a newly authenticated SFTP session without exposing a path capability.
    pub fn target(&self, window: &str, id: Uuid) -> Result<TransferTarget, SftpError> {
        let task = self.task(window, id)?;
        task.target()
    }
    /// Re-run a failed job using the same task identity when its original
    /// source/lease are still valid. Interrupted uploads that retained a
    /// partial require `retry_with_selection` and a fresh session so the
    /// completed prefix can be verified before appending.
    pub fn retry(&self, window: &str, id: Uuid) -> Result<(), SftpError> {
        self.retry_inner(window, id, None, None)
    }
    pub fn retry_with_context(
        &self,
        window: &str,
        id: Uuid,
        context: TransferContext,
    ) -> Result<(), SftpError> {
        self.retry_inner(window, id, Some(context), None)
    }
    pub fn retry_with_selection(
        self: &Arc<Self>,
        window: &str,
        id: Uuid,
        context: TransferContext,
        selection: Uuid,
    ) -> Result<(), SftpError> {
        let task = self.task(window, id)?;
        if task.target()? != context.target
            || context.binding.window != window
            || !matches!(
                context.target,
                TransferTarget::Upload { .. } | TransferTarget::UploadTree { .. }
            )
        {
            return Err(SftpError::ScopeExpired);
        }
        let local = {
            let mut state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
            let grant = state
                .grants
                .get(&selection)
                .filter(|grant| grant.binding == context.binding && grant.expires > Instant::now())
                .ok_or(SftpError::ScopeExpired)?;
            let expected_source = matches!(
                &context.target,
                TransferTarget::Upload { .. } | TransferTarget::UploadTree { .. }
            );
            if expected_source != matches!(grant.local, Local::Source(_)) {
                return Err(SftpError::ScopeExpired);
            }
            if grant
                .target
                .as_ref()
                .is_some_and(|target| target != &context.target)
            {
                return Err(SftpError::ScopeExpired);
            }
            state
                .grants
                .remove(&selection)
                .ok_or(SftpError::ScopeExpired)?
                .local
        };
        self.retry_inner(window, id, Some(context), Some(local))
    }
    fn retry_inner(
        &self,
        window: &str,
        id: Uuid,
        replacement: Option<TransferContext>,
        local_override: Option<Local>,
    ) -> Result<(), SftpError> {
        let state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
        if state.closing.contains(window) {
            return Err(SftpError::ScopeExpired);
        }
        let task = state
            .tasks
            .iter()
            .find(|task| {
                task.window == window && task.status.lock().is_ok_and(|status| status.id == id)
            })
            .cloned()
            .ok_or(SftpError::ScopeExpired)?;
        let (context, local, partial, workers, commits, budget) = {
            let mut status = task.status.lock().map_err(|_| SftpError::ConnectionLost)?;
            if !matches!(
                status.state,
                TransferState::Failed | TransferState::Interrupted
            ) {
                return Err(SftpError::ScopeExpired);
            }
            let mut retry = task.retry.lock().map_err(|_| SftpError::ConnectionLost)?;
            let mut plan = retry.clone().ok_or(SftpError::ScopeExpired)?;
            let context = replacement
                .or_else(|| plan.context.clone())
                .ok_or(SftpError::ScopeExpired)?;
            // A fresh unlock changes the epoch, not the authenticated endpoint,
            // credential revision, trust, user, Agent or proxy policy.
            if context.binding.window != window
                || context.binding.host != status.host_id
                || context.path != status.remote_path
                || context.target != task.target()?
                || !same_retry_policy(&plan.policy, context.lease.policy())
                || context.client.is_closed()
            {
                return Err(SftpError::ScopeExpired);
            }
            let local = local_override
                .or_else(|| plan.local.clone())
                .ok_or(SftpError::ScopeExpired)?;
            let total = match &local {
                Local::Source(source) if status.direction == Direction::Upload => source.size(),
                Local::Destination(_) if status.direction == Direction::Download => {
                    context
                        .baseline
                        .as_ref()
                        .and_then(|metadata| metadata.size)
                        .ok_or(SftpError::UnsupportedFileType)?
                        .0
                }
                _ => return Err(SftpError::ScopeExpired),
            };
            if plan.partial.is_some() && total != status.total.0 {
                return Err(SftpError::ResumeMismatch);
            }
            if status.direction == Direction::Download
                && plan.baseline.as_ref().is_some_and(|baseline| {
                    context
                        .baseline
                        .as_ref()
                        .is_none_or(|current| !current.matches_baseline(baseline))
                })
            {
                return Err(SftpError::ResumeMismatch);
            }
            self.track_client(&context)?;
            let (sender, receiver) = mpsc::channel(1);
            *task
                .decisions
                .lock()
                .map_err(|_| SftpError::ConnectionLost)? = sender;
            *task
                .decision_receiver
                .lock()
                .map_err(|_| SftpError::ConnectionLost)? = Some(receiver);
            if let Some(member) = &task.batch {
                member.active.store(false, Ordering::SeqCst);
            }
            plan.context = Some(context.clone());
            plan.local = Some(local.clone());
            *retry = Some(plan.clone());
            task.paused.store(false, Ordering::SeqCst);
            if let Ok(mut timing) = task.desktop_timing.lock() {
                *timing = AttemptTiming::new();
            }
            status.state = TransferState::Retrying;
            status.error = None;
            status.conflict = None;
            status.cleanup_required = false;
            status.processed = ExactU64(0);
            status.total = ExactU64(total);
            status.bytes_per_second = ExactU64(0);
            status.remaining_seconds = None;
            status.version.0 += 1;
            (
                context,
                local,
                plan.partial,
                plan.workers,
                plan.commits,
                plan.budget,
            )
        };
        drop(state);
        tokio::spawn(async move {
            engine::run(task, context, local, workers, commits, budget, partial).await;
        });
        Ok(())
    }
    pub fn resolve(
        &self,
        window: &str,
        id: Uuid,
        revision: ExactU64,
        decision: Decision,
    ) -> Result<(), SftpError> {
        let task = self.task(window, id)?;
        let mut status = task.status.lock().map_err(|_| SftpError::ConnectionLost)?;
        let conflict = status
            .conflict
            .as_ref()
            .filter(|c| c.revision == revision)
            .ok_or(SftpError::ScopeExpired)?;
        if status.state != TransferState::AwaitingConflict || task.cancel.is_cancelled() {
            return Err(SftpError::ScopeExpired);
        }
        if matches!(decision, Decision::Replace | Decision::ReplaceBatch)
            && !conflict.replace_allowed
        {
            return Err(SftpError::UnsupportedFileType);
        }
        if matches!(decision, Decision::ReplaceBatch | Decision::SkipBatch) {
            let member = task
                .batch
                .as_ref()
                .filter(|member| conflict.batch_eligible && member.active.load(Ordering::SeqCst))
                .ok_or(SftpError::ScopeExpired)?;
            member.batch.choose(match decision {
                Decision::ReplaceBatch => batch::BatchChoice::Replace,
                _ => batch::BatchChoice::Skip,
            })?;
            status.conflict = None;
            status.version.0 += 1;
            return Ok(());
        }
        if let Decision::Rename { name } = &decision {
            crate::sftp::RemoteName::from_bytes(name.as_bytes())?;
            #[cfg(windows)]
            if status.direction == Direction::Download
                && !capstan_native_windows::transfer::valid_name(name)
            {
                return Err(SftpError::PathInvalid);
            }
        }
        task.decisions
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .try_send(decision)
            .map_err(|_| SftpError::ScopeExpired)?;
        if let Some(member) = &task.batch {
            member.active.store(false, Ordering::SeqCst);
        }
        status.conflict = None;
        status.version.0 += 1;
        Ok(())
    }
    pub fn cancel_window(&self, window: &str) {
        #[cfg(windows)]
        if let Ok(mut registry) = self.native.lock() {
            registry.revoke(window);
        }
        #[cfg(windows)]
        if let Ok(mut trees) = self.trees.lock() {
            trees.retain(|_, handle| handle.binding.window != window);
        }
        if let Ok(mut state) = self.state.lock() {
            state
                .grants
                .retain(|_, grant| grant.binding.window != window);
            for task in &state.tasks {
                if task.window == window {
                    let _ = self.cancel_task(task);
                }
            }
        }
    }
    pub fn has_active(&self, window: &str) -> Result<bool, SftpError> {
        #[cfg(windows)]
        if self
            .trees
            .lock()
            .map_err(|_| SftpError::LocalIo)?
            .values()
            .any(|handle| handle.binding.window == window && handle.expires > Instant::now())
        {
            return Ok(true);
        }
        Ok(self
            .snapshot(window)?
            .iter()
            .any(|status| !status.state.terminal()))
    }
    /// Reject new jobs before canceling and draining the current window. A
    /// timeout keeps the window alive; it never abandons an in-flight commit.
    pub async fn prepare_exit(&self, window: &str) -> Result<(), SftpError> {
        self.state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .closing
            .insert(window.to_owned());
        self.cancel_window(window);
        let connections = self
            .connections
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .iter()
            .filter(|(owner, _)| owner == window)
            .filter_map(|(_, connection)| connection.upgrade())
            .collect::<Vec<_>>();
        let result = tokio::time::timeout(
            Duration::from_secs(crate::sftp::EXIT_DRAIN_TIMEOUT_SECONDS),
            async {
                while self.has_active(window)? {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                for connection in connections {
                    connection.wait_sftp_pending().await;
                }
                Ok(())
            },
        )
        .await
        .unwrap_or(Err(SftpError::ResourceLimit));
        if result.is_err() {
            self.state
                .lock()
                .map_err(|_| SftpError::ConnectionLost)?
                .closing
                .remove(window);
        }
        result
    }
    pub(crate) fn abort_exit(&self, window: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.closing.remove(window);
        }
    }
    fn cancel_task(&self, task: &Task) -> Result<(), SftpError> {
        let status = task.status.lock().map_err(|_| SftpError::ConnectionLost)?;
        if !status.state.terminal() && status.state != TransferState::Committing {
            task.cancel.cancel();
        }
        Ok(())
    }
}

#[cfg(test)]
mod retry_tests {
    use super::*;

    #[test]
    fn retry_only_allows_a_new_vault_epoch_in_the_authenticated_policy() {
        let original = capstan_ssh::PoolKey {
            target: "server".into(),
            port: 22,
            username: "user".into(),
            credential_id: Uuid::new_v4(),
            credential_revision: 1,
            trust_fingerprint: "SHA256:host".into(),
            vault_epoch: 1,
            agent_identity: None,
            agent_forwarding: false,
            proxy_path: String::new(),
        };
        let mut current = original.clone();
        current.vault_epoch += 1;
        assert!(same_retry_policy(&original, &current));
        assert_eq!(
            retry_policy_digest(&original),
            retry_policy_digest(&current)
        );
        for field in 0..9 {
            let mut changed = current.clone();
            match field {
                0 => changed.target.push('2'),
                1 => changed.port += 1,
                2 => changed.username.push('2'),
                3 => changed.credential_id = Uuid::new_v4(),
                4 => changed.credential_revision += 1,
                5 => changed.trust_fingerprint.push('2'),
                6 => changed.agent_identity = Some("agent".into()),
                7 => changed.agent_forwarding = true,
                _ => changed.proxy_path.push('2'),
            }
            assert!(!same_retry_policy(&original, &changed));
            assert_ne!(
                retry_policy_digest(&original),
                retry_policy_digest(&changed)
            );
        }
    }
}
