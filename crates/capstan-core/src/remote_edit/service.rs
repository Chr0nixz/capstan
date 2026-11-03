//! Windows editor lifecycle, independent of WebView and terminal-panel mounts.
use super::{
    apply_remote_bytes_tracked, content_identity, decode, read_remote_file, ConflictDecision,
    EditError, EditSession, EditState, EditorConfig, LocalFingerprint, PublicationBaseline,
    RemoteTemporaryView, ResidualAction, TemporaryLedger, EDITOR_POLL_MS, MAX_EDITOR_SESSIONS,
};
use crate::{
    settings::Settings,
    sftp::{ExactU64, RemoteFileContext, RemoteName, RemotePath},
};
use capstan_native_windows::editor::{EditorStage, EditorStore, Snapshot};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEditView {
    pub id: Uuid,
    pub name: String,
    pub remote_path: Option<String>,
    pub host_id: Option<Uuid>,
    pub state: EditState,
    pub dirty: bool,
    pub recovered: bool,
    pub cleanup_required: bool,
    pub remote_temporaries: Vec<RemoteTemporaryView>,
    pub replace_allowed: bool,
    pub error: Option<EditError>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEditSnapshot {
    pub revision: ExactU64,
    pub sessions: Vec<RemoteEditView>,
    pub recovery_incomplete: bool,
}
struct ActiveEditor {
    edit: Option<EditSession>,
    context: Option<RemoteFileContext>,
    stage: Arc<Mutex<EditorStage>>,
    child: Option<std::process::Child>,
    conflict: Option<(crate::sftp::Metadata, [u8; 32])>,
    target: Option<RetryTarget>,
    temporaries: TemporaryLedger,
    cleanup_scope: Option<super::journal::CleanupScope>,
    recovery_error: bool,
}
#[derive(Clone)]
struct RetryTarget {
    host: Uuid,
    policy: capstan_ssh::PoolKey,
    path: RemotePath,
    metadata: crate::sftp::Metadata,
}
struct Entry {
    owner: String,
    view: Mutex<RemoteEditView>,
    active: tokio::sync::Mutex<ActiveEditor>,
    cancel: Mutex<CancellationToken>,
}
impl Entry {
    fn cancellation(&self) -> Result<CancellationToken, EditError> {
        self.cancel
            .lock()
            .map(|cancel| cancel.clone())
            .map_err(|_| EditError::Residual)
    }
}
#[derive(Default)]
struct State {
    sessions: HashMap<Uuid, Arc<Entry>>,
    opening: usize,
    exiting: bool,
    revision: u64,
    recovery_incomplete: bool,
    configuration: Option<EditorConfig>,
}
pub struct RemoteEditorService {
    state: Mutex<State>,
    store: EditorStore,
    settings: Arc<Settings>,
    commits: Arc<tokio::sync::Mutex<()>>,
}
struct Opening(Arc<RemoteEditorService>);
impl Drop for Opening {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.opening -= 1;
        }
    }
}
impl RemoteEditorService {
    pub fn new(
        settings: Arc<Settings>,
        store: EditorStore,
        commits: Arc<tokio::sync::Mutex<()>>,
    ) -> Arc<Self> {
        let configuration = settings
            .editor_executable()
            .ok()
            .flatten()
            .and_then(|path| EditorConfig::new(Uuid::new_v4(), path).ok());
        let service = Arc::new(Self {
            state: Mutex::new(State {
                configuration,
                ..State::default()
            }),
            store,
            settings,
            commits,
        });
        // Bounded local discovery does not authenticate or replay a remote write.
        let _ = service.discover("main");
        service
    }

    pub fn configuration_id(&self, window: &str) -> Result<Option<Uuid>, EditError> {
        validate_window(window)?;
        Ok(self
            .state
            .lock()
            .map_err(|_| EditError::Residual)?
            .configuration
            .as_ref()
            .map(|config| config.id))
    }
    pub async fn configure(&self, window: &str, executable: PathBuf) -> Result<Uuid, EditError> {
        validate_window(window)?;
        let config = EditorConfig::new(Uuid::new_v4(), executable)?;
        self.settings
            .clone()
            .save_editor_executable(config.executable.clone())
            .await
            .map_err(|_| EditError::LocalIo)?;
        let id = config.id;
        self.state
            .lock()
            .map_err(|_| EditError::Residual)?
            .configuration = Some(config);
        Ok(id)
    }
    pub fn snapshot(&self, window: &str) -> Result<RemoteEditSnapshot, EditError> {
        validate_window(window)?;
        let state = self.state.lock().map_err(|_| EditError::Residual)?;
        let mut sessions = Vec::new();
        for entry in state
            .sessions
            .values()
            .filter(|entry| entry.owner == window)
        {
            sessions.push(entry.view.lock().map_err(|_| EditError::Residual)?.clone());
        }
        sessions.sort_by_key(|session| session.id);
        Ok(RemoteEditSnapshot {
            revision: ExactU64(state.revision),
            sessions,
            recovery_incomplete: state.recovery_incomplete,
        })
    }
    pub fn discover(&self, window: &str) -> Result<(), EditError> {
        validate_window(window)?;
        let mut state = self.state.lock().map_err(|_| EditError::Residual)?;
        let remaining = MAX_EDITOR_SESSIONS.saturating_sub(state.sessions.len() + state.opening);
        match self.store.discover(remaining) {
            Ok(discovery) => {
                state.recovery_incomplete = discovery.incomplete;
                for (id, stage) in discovery.stages {
                    let Ok(id) = Uuid::parse_str(&id) else {
                        continue;
                    };
                    let name = stage.name().to_owned();
                    let stage = Arc::new(Mutex::new(stage));
                    let (scope, temporaries, error) = match super::journal::load(id, stage.clone())
                    {
                        Ok(loaded) => (loaded.scope, loaded.ledger, None),
                        Err(error) => (None, TemporaryLedger::default(), Some(error)),
                    };
                    let host_id = if temporaries.is_empty() {
                        None
                    } else {
                        scope.as_ref().map(|scope| scope.host)
                    };
                    let view = RemoteEditView {
                        id,
                        name,
                        remote_path: None,
                        host_id,
                        state: EditState::Residual,
                        dirty: true,
                        recovered: true,
                        cleanup_required: true,
                        remote_temporaries: temporaries.views(),
                        replace_allowed: false,
                        error,
                    };
                    state.sessions.insert(
                        id,
                        Arc::new(Entry {
                            owner: window.into(),
                            view: Mutex::new(view),
                            active: tokio::sync::Mutex::new(ActiveEditor {
                                edit: None,
                                context: None,
                                stage,
                                child: None,
                                conflict: None,
                                target: None,
                                temporaries,
                                cleanup_scope: scope,
                                recovery_error: error.is_some(),
                            }),
                            cancel: Mutex::new(CancellationToken::new()),
                        }),
                    );
                }
            }
            Err(_) => state.recovery_incomplete = true,
        }
        state.revision = state.revision.wrapping_add(1);
        Ok(())
    }
    fn entry(&self, window: &str, id: Uuid) -> Result<Arc<Entry>, EditError> {
        validate_window(window)?;
        self.state
            .lock()
            .map_err(|_| EditError::Residual)?
            .sessions
            .get(&id)
            .filter(|entry| entry.owner == window)
            .cloned()
            .ok_or(EditError::ScopeExpired)
    }
    fn update(
        &self,
        entry: &Entry,
        active: &ActiveEditor,
        dirty: bool,
        error: Option<EditError>,
    ) -> Result<RemoteEditView, EditError> {
        let mut state = self.state.lock().map_err(|_| EditError::Residual)?;
        let mut view = entry.view.lock().map_err(|_| EditError::Residual)?;
        if let Some(edit) = &active.edit {
            view.state = edit.state;
            view.remote_path = Some(edit.remote_path.clone());
            view.cleanup_required = edit.cleanup_required || !active.temporaries.is_empty();
        }
        view.remote_temporaries = active.temporaries.views();
        view.dirty = dirty;
        view.error = error;
        view.replace_allowed = active.conflict.is_some()
            && active
                .context
                .as_ref()
                .is_some_and(|context| context.client.capabilities().atomic_replace);
        state.revision = state.revision.wrapping_add(1);
        Ok(view.clone())
    }
    pub async fn begin(
        self: &Arc<Self>,
        window: &str,
        context: RemoteFileContext,
        editor_id: Uuid,
    ) -> Result<RemoteEditView, EditError> {
        validate_window(window)?;
        if context.binding.window != window {
            return Err(EditError::ScopeExpired);
        }
        let config = {
            let mut state = self.state.lock().map_err(|_| EditError::Residual)?;
            if state.exiting || state.sessions.len() + state.opening >= MAX_EDITOR_SESSIONS {
                return Err(EditError::ResourceLimit);
            }
            let config = state
                .configuration
                .as_ref()
                .filter(|config| config.id == editor_id)
                .cloned()
                .ok_or(EditError::EditorUnavailable)?;
            state.opening += 1;
            config
        };
        let _opening = Opening(self.clone());
        let cancel = CancellationToken::new();
        let (bytes, metadata) = read_remote_file(
            &context.client,
            &context.path,
            Some(&context.metadata),
            &cancel,
        )
        .await?;
        let id = Uuid::new_v4();
        let name = context
            .path
            .as_str()
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .ok_or(EditError::UnsupportedContent)?
            .to_owned();
        let mut edit = EditSession::begin(
            id,
            context.path.as_str().into(),
            &bytes,
            format!("{metadata:?}"),
        )?;
        let store = self.store.clone();
        let initial = bytes.clone();
        let stage_name = name.clone();
        let (stage, launched) = tokio::task::spawn_blocking(move || {
            let stage = store
                .prepare(&id.to_string(), &stage_name, &initial)
                .map_err(|_| EditError::LocalIo)?;
            let launched = stage.launch(&config.executable);
            Ok::<_, EditError>((stage, launched))
        })
        .await
        .map_err(|_| EditError::LocalIo)??;
        let (child, error) = match launched {
            Ok(child) => {
                edit.mark_editor_started();
                (Some(child), None)
            }
            Err(_) => {
                edit.mark_residual();
                (None, Some(EditError::EditorUnavailable))
            }
        };
        let view = RemoteEditView {
            id,
            name,
            remote_path: Some(edit.remote_path.clone()),
            host_id: Some(context.binding.host),
            state: edit.state,
            dirty: false,
            recovered: false,
            cleanup_required: edit.cleanup_required,
            remote_temporaries: Vec::new(),
            replace_allowed: false,
            error,
        };
        let stage = Arc::new(Mutex::new(stage));
        let scope = super::journal::CleanupScope::new(context.binding.host, context.lease.policy());
        let temporaries = super::journal::fresh(id, scope.clone(), stage.clone());
        let entry = Arc::new(Entry {
            owner: window.into(),
            view: Mutex::new(view.clone()),
            active: tokio::sync::Mutex::new(ActiveEditor {
                target: Some(RetryTarget {
                    host: context.binding.host,
                    policy: context.lease.policy().clone(),
                    path: context.path.clone(),
                    metadata: metadata.clone(),
                }),
                temporaries,
                cleanup_scope: Some(scope),
                recovery_error: false,
                edit: Some(edit),
                context: Some(context),
                stage,
                child,
                conflict: None,
            }),
            cancel: Mutex::new(cancel),
        });
        {
            let mut state = self.state.lock().map_err(|_| EditError::Residual)?;
            state.sessions.insert(id, entry.clone());
            state.revision = state.revision.wrapping_add(1);
        }
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            watch(weak, entry, id).await;
        });
        Ok(view)
    }
    async fn observe(&self, entry: &Arc<Entry>) -> Result<bool, EditError> {
        let Ok(mut active) = entry.active.try_lock() else {
            return Ok(true);
        };
        let cancel = entry.cancellation()?;
        if active
            .edit
            .as_ref()
            .is_none_or(|edit| edit.state == EditState::Closed)
        {
            return Ok(false);
        }
        if let Some(child) = &mut active.child {
            let _ = child.try_wait();
        }
        if active.context.is_some()
            && (cancel.is_cancelled()
                || active.context.as_ref().is_some_and(|context| {
                    context.client.is_closed() || context.lease.connection().is_closed()
                }))
        {
            if let Some(edit) = &mut active.edit {
                edit.mark_residual();
            }
            active.context.take();
            self.update(entry, &active, true, Some(EditError::ConnectionLost))?;
            return Ok(true);
        }
        if active
            .edit
            .as_ref()
            .is_some_and(|edit| matches!(edit.state, EditState::Residual | EditState::Conflict))
        {
            return Ok(true);
        }
        let snapshot = match snapshot(active.stage.clone()).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                if let Some(edit) = &mut active.edit {
                    edit.invalidate_pending();
                }
                self.update(entry, &active, true, Some(error))?;
                return Ok(true);
            }
        };
        let fingerprint = fingerprint(&snapshot);
        let edit = active.edit.as_mut().ok_or(EditError::Residual)?;
        let dirty = edit.is_dirty(&snapshot.bytes);
        if let Err(error) = decode(&snapshot.bytes) {
            edit.invalidate_pending();
            self.update(entry, &active, dirty, Some(error))?;
            return Ok(true);
        }
        let ready = edit.observe_content(fingerprint, &snapshot.bytes, Instant::now())?;
        self.update(entry, &active, dirty, None)?;
        if ready {
            if let Err(error) = publish(&mut active, &snapshot.bytes, &self.commits, &cancel).await
            {
                if let Some(edit) = &mut active.edit {
                    if error == EditError::Conflict {
                        edit.mark_conflict()?;
                    } else {
                        edit.mark_residual();
                    }
                }
                if error == EditError::Conflict {
                    capture_conflict(&mut active, &cancel).await;
                }
                self.update(entry, &active, true, Some(error))?;
            } else {
                self.update(entry, &active, false, None)?;
            }
        }
        Ok(true)
    }
    pub async fn retry(&self, window: &str, id: Uuid) -> Result<RemoteEditView, EditError> {
        let configuration = self
            .state
            .lock()
            .map_err(|_| EditError::Residual)?
            .configuration
            .clone();
        let entry = self.entry(window, id)?;
        let mut active = entry.active.lock().await;
        let cancel = entry.cancellation()?;
        if cancel.is_cancelled() || active.context.is_none() {
            return Err(EditError::ConnectionLost);
        }
        let edit = active.edit.as_mut().ok_or(EditError::Residual)?;
        if edit.state == EditState::Conflict {
            return Err(EditError::Conflict);
        }
        if let Err(error) = cleanup_remote(&mut active, &cancel).await {
            self.update(&entry, &active, true, Some(error))?;
            return Err(error);
        }
        if active.child.is_none() {
            let executable = configuration
                .ok_or(EditError::EditorUnavailable)?
                .executable;
            let stage = active.stage.clone();
            let child = tokio::task::spawn_blocking(move || {
                stage
                    .lock()
                    .map_err(|_| EditError::Residual)?
                    .launch(&executable)
                    .map_err(|_| EditError::EditorUnavailable)
            })
            .await
            .map_err(|_| EditError::EditorUnavailable)??;
            active.child = Some(child);
        }
        let edit = active.edit.as_mut().ok_or(EditError::Residual)?;
        edit.mark_editor_started();
        edit.invalidate_pending();
        self.update(&entry, &active, true, None)
    }
    /// Rebind only an explicit edit ID to an already authenticated SFTP view.
    /// Preserve the old remote baseline; the next save must still verify it.
    pub async fn bind_for_retry(
        &self,
        window: &str,
        id: Uuid,
        sftp: &crate::sftp::SftpService,
        terminal: &str,
        session: crate::sftp::SftpSessionId,
    ) -> Result<RemoteEditView, EditError> {
        let entry = self.entry(window, id)?;
        let mut active = entry.active.lock().await;
        if active
            .edit
            .as_ref()
            .is_none_or(|edit| !matches!(edit.state, EditState::Residual | EditState::Conflict))
        {
            return Err(EditError::ScopeExpired);
        }
        let target = active.target.as_ref().ok_or(EditError::ScopeExpired)?;
        let context = sftp
            .editor_retry_context(
                window,
                terminal,
                session,
                target.path.clone(),
                target.metadata.clone(),
            )
            .map_err(|_| EditError::ScopeExpired)?;
        if context.binding.host != target.host
            || !same_policy(&target.policy, context.lease.policy())
        {
            return Err(EditError::ScopeExpired);
        }
        if context.client.is_closed() || context.lease.connection().is_closed() {
            return Err(EditError::ConnectionLost);
        }
        active.context = Some(context);
        *entry.cancel.lock().map_err(|_| EditError::Residual)? = CancellationToken::new();
        self.update(&entry, &active, true, None)
    }
    pub async fn resolve(
        &self,
        window: &str,
        id: Uuid,
        decision: ConflictDecision,
        new_name: Option<String>,
    ) -> Result<RemoteEditView, EditError> {
        let entry = self.entry(window, id)?;
        if entry
            .active
            .lock()
            .await
            .edit
            .as_ref()
            .is_none_or(|edit| edit.state != EditState::Conflict)
        {
            return Err(EditError::Conflict);
        }
        if decision == ConflictDecision::KeepRemote {
            // Clean first; a failure must retain a visible, recoverable session.
            self.close(window, id, ResidualAction::Discard, None)
                .await?;
            let mut view = entry.view.lock().map_err(|_| EditError::Residual)?.clone();
            view.state = EditState::Closed;
            return Ok(view);
        }
        let mut active = entry.active.lock().await;
        if active
            .edit
            .as_ref()
            .is_none_or(|edit| edit.state != EditState::Conflict)
        {
            return Err(EditError::Conflict);
        }
        let cancel = entry.cancellation()?;
        let snapshot = stable_snapshot(active.stage.clone()).await?;
        let context = active.context.as_ref().ok_or(EditError::ConnectionLost)?;
        let (target, expected, expected_hash) = if decision == ConflictDecision::KeepLocalAsNew {
            let name =
                RemoteName::from_bytes(new_name.as_deref().ok_or(EditError::Conflict)?.as_bytes())
                    .map_err(|_| EditError::Conflict)?;
            let parent = context
                .path
                .as_str()
                .rsplit_once('/')
                .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
                .ok_or(EditError::Conflict)?;
            (
                RemotePath::parse(parent)
                    .and_then(|parent| parent.join(&name))
                    .map_err(|_| EditError::Conflict)?,
                None,
                None,
            )
        } else {
            let (metadata, hash) = active.conflict.clone().ok_or(EditError::Conflict)?;
            (context.path.clone(), Some(metadata), Some(hash))
        };
        let client = context.client.clone();
        let result = apply_remote_bytes_tracked(
            &client,
            &target,
            &snapshot.bytes,
            PublicationBaseline {
                metadata: expected.as_ref(),
                hash: expected_hash,
            },
            &self.commits,
            &cancel,
            &mut active.temporaries,
        )
        .await;
        let metadata = match result {
            Ok(metadata) => metadata,
            Err(error) => {
                if error == EditError::Conflict {
                    capture_conflict(&mut active, &cancel).await;
                }
                self.update(&entry, &active, true, Some(error))?;
                return Err(error);
            }
        };
        if let Some(context) = &mut active.context {
            context.path = target.clone();
            context.metadata = metadata.clone();
        }
        if let Some(retained) = &mut active.target {
            retained.path = target.clone();
            retained.metadata = metadata.clone();
        }
        let edit = active.edit.as_mut().ok_or(EditError::Residual)?;
        edit.resolve_conflict(decision)?;
        edit.remote_path = target.as_str().into();
        edit.mark_uploaded(&snapshot.bytes, format!("{metadata:?}"))?;
        active.conflict = None;
        self.update(&entry, &active, false, None)
    }
    /// `destination` comes only from a native picker. Cancellation is handled
    /// before this call and never removes the entry from the UI.
    pub async fn close(
        &self,
        window: &str,
        id: Uuid,
        action: ResidualAction,
        destination: Option<PathBuf>,
    ) -> Result<(), EditError> {
        let entry = self.entry(window, id)?;
        let mut active = entry.active.lock().await;
        if action == ResidualAction::RetryUpload {
            return Err(EditError::Residual);
        }
        if action == ResidualAction::KeepForRecovery {
            // Preserve every local file and checkpoint, including unreadable
            // metadata. Release remote authority; a later start only discovers
            // the copy and never resumes an upload automatically.
            if let Some(edit) = &mut active.edit {
                edit.close();
            }
            active.context.take();
            entry.cancellation()?.cancel();
            drop(active);
            let mut state = self.state.lock().map_err(|_| EditError::Residual)?;
            state.sessions.remove(&id);
            state.revision = state.revision.wrapping_add(1);
            return Ok(());
        }
        let expected = if action == ResidualAction::SaveLocalCopy {
            let root = destination.ok_or(EditError::Canceled)?;
            let snapshot = snapshot(active.stage.clone()).await?;
            let bytes = snapshot.bytes;
            let name = entry
                .view
                .lock()
                .map_err(|_| EditError::Residual)?
                .name
                .clone();
            tokio::task::spawn_blocking(move || {
                capstan_native_windows::editor::copy_bytes_to_folder(&root, &name, &bytes)
            })
            .await
            .map_err(|_| EditError::LocalIo)?
            .map_err(|_| EditError::LocalIo)?;
            Some(snapshot.fingerprint)
        } else {
            None
        };
        if let Err(error) = cleanup_remote(&mut active, &entry.cancellation()?).await {
            if let Some(edit) = &mut active.edit {
                edit.mark_residual();
            }
            self.update(&entry, &active, true, Some(error))?;
            return Err(error);
        }
        let stage = active.stage.clone();
        let result = tokio::task::spawn_blocking(move || {
            stage
                .lock()
                .map_err(|_| EditError::Residual)?
                .try_cleanup(expected)
                .map_err(|_| EditError::Residual)
        })
        .await
        .map_err(|_| EditError::Residual)?;
        if let Err(error) = result {
            if let Some(edit) = &mut active.edit {
                edit.mark_residual();
            }
            self.update(&entry, &active, true, Some(error))?;
            return Err(error);
        }
        if let Some(edit) = &mut active.edit {
            edit.close();
        }
        active.context.take();
        entry.cancellation()?.cancel();
        drop(active);
        let mut state = self.state.lock().map_err(|_| EditError::Residual)?;
        state.sessions.remove(&id);
        state.revision = state.revision.wrapping_add(1);
        Ok(())
    }
    pub async fn cleanup_temporaries(
        &self,
        window: &str,
        id: Uuid,
    ) -> Result<RemoteEditView, EditError> {
        let entry = self.entry(window, id)?;
        let mut active = entry.active.lock().await;
        let result = cleanup_remote(&mut active, &entry.cancellation()?).await;
        let dirty = entry.view.lock().map_err(|_| EditError::Residual)?.dirty;
        let view = self.update(&entry, &active, dirty, result.as_ref().err().copied())?;
        result.map(|()| view)
    }
    /// Recovered documents never regain upload authority. This explicit binding
    /// authorizes only checking/removing the recorded remote temporaries.
    pub async fn bind_for_cleanup(
        &self,
        window: &str,
        id: Uuid,
        sftp: &crate::sftp::SftpService,
        terminal: &str,
        session: crate::sftp::SftpSessionId,
    ) -> Result<(), EditError> {
        let entry = self.entry(window, id)?;
        let mut active = entry.active.lock().await;
        if active.recovery_error {
            return Err(EditError::Residual);
        }
        let scope = active
            .cleanup_scope
            .as_ref()
            .ok_or(EditError::ScopeExpired)?;
        let (path, metadata) = if let Some(target) = &active.target {
            (target.path.clone(), target.metadata.clone())
        } else {
            active
                .temporaries
                .first_target()
                .ok_or(EditError::ScopeExpired)?
        };
        let context = sftp
            .editor_retry_context(window, terminal, session, path, metadata)
            .map_err(|_| EditError::ScopeExpired)?;
        if !scope.matches(context.binding.host, context.lease.policy()) {
            return Err(EditError::ScopeExpired);
        }
        if context.client.is_closed() || context.lease.connection().is_closed() {
            return Err(EditError::ConnectionLost);
        }
        active.context = Some(context);
        *entry.cancel.lock().map_err(|_| EditError::Residual)? = CancellationToken::new();
        Ok(())
    }
    pub fn has_open(&self, window: &str) -> bool {
        self.state.lock().map_or(true, |state| {
            state.opening > 0 || state.sessions.values().any(|entry| entry.owner == window)
        })
    }
    pub fn prepare_exit(&self, window: &str) -> Result<(), EditError> {
        validate_window(window)?;
        let mut state = self.state.lock().map_err(|_| EditError::Residual)?;
        if state.opening > 0 || state.sessions.values().any(|entry| entry.owner == window) {
            return Err(EditError::Residual);
        }
        state.exiting = true;
        Ok(())
    }
    pub fn abort_exit(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.exiting = false;
        }
    }
    pub fn cancel_window(&self, window: &str) {
        if let Ok(state) = self.state.lock() {
            for entry in state
                .sessions
                .values()
                .filter(|entry| entry.owner == window)
            {
                if let Ok(cancel) = entry.cancellation() {
                    cancel.cancel();
                }
            }
        }
    }
}

async fn snapshot(stage: Arc<Mutex<EditorStage>>) -> Result<Snapshot, EditError> {
    tokio::task::spawn_blocking(move || {
        stage
            .lock()
            .map_err(|_| EditError::Residual)?
            .snapshot()
            .map_err(|_| EditError::LocalIo)
    })
    .await
    .map_err(|_| EditError::LocalIo)?
}
async fn stable_snapshot(stage: Arc<Mutex<EditorStage>>) -> Result<Snapshot, EditError> {
    let first = snapshot(stage.clone()).await?;
    tokio::time::sleep(std::time::Duration::from_millis(
        super::STABLE_SAVE_INTERVAL_MS,
    ))
    .await;
    let second = snapshot(stage).await?;
    if first.fingerprint != second.fingerprint || first.bytes != second.bytes {
        return Err(EditError::StaleSave);
    }
    Ok(second)
}
async fn capture_conflict(active: &mut ActiveEditor, cancel: &CancellationToken) {
    active.conflict = if let Some(context) = &active.context {
        super::read_remote_bytes(&context.client, &context.path, None, cancel)
            .await
            .ok()
            .map(|(bytes, metadata)| (metadata, Sha256::digest(&bytes).into()))
    } else {
        None
    };
}
fn fingerprint(snapshot: &Snapshot) -> LocalFingerprint {
    LocalFingerprint {
        identity: content_identity(&snapshot.bytes) ^ snapshot.fingerprint.identity,
        size: snapshot.fingerprint.size,
        modified: snapshot.fingerprint.modified,
    }
}
async fn publish(
    active: &mut ActiveEditor,
    bytes: &[u8],
    commits: &tokio::sync::Mutex<()>,
    cancel: &CancellationToken,
) -> Result<(), EditError> {
    let context = active.context.as_ref().ok_or(EditError::ConnectionLost)?;
    let edit = active.edit.as_ref().ok_or(EditError::Residual)?;
    let metadata = apply_remote_bytes_tracked(
        &context.client,
        &context.path,
        bytes,
        PublicationBaseline {
            metadata: Some(&context.metadata),
            hash: Some(edit.baseline_hash()),
        },
        commits,
        cancel,
        &mut active.temporaries,
    )
    .await?;
    if let Some(context) = &mut active.context {
        context.metadata = metadata.clone();
    }
    if let Some(target) = &mut active.target {
        target.metadata = metadata.clone();
    }
    if let Some(edit) = &mut active.edit {
        edit.mark_uploaded(bytes, format!("{metadata:?}"))?;
    }
    Ok(())
}

async fn cleanup_remote(
    active: &mut ActiveEditor,
    cancel: &CancellationToken,
) -> Result<(), EditError> {
    if active.recovery_error {
        return Err(EditError::Residual);
    }
    if active.temporaries.is_empty() {
        return Ok(());
    }
    let context = active.context.as_ref().ok_or(EditError::ConnectionLost)?;
    active.temporaries.cleanup(&context.client, cancel).await
}

fn same_policy(original: &capstan_ssh::PoolKey, current: &capstan_ssh::PoolKey) -> bool {
    let mut original = original.clone();
    // An explicitly authenticated session after unlocking may have a new Vault
    // epoch; trust, credential revision, user and Agent policy must still match.
    original.vault_epoch = current.vault_epoch;
    original == *current
}
async fn watch(service: Weak<RemoteEditorService>, entry: Arc<Entry>, _id: Uuid) {
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(EDITOR_POLL_MS));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        let Some(service) = service.upgrade() else {
            break;
        };
        if !service.observe(&entry).await.unwrap_or(true) {
            break;
        }
    }
}
fn validate_window(window: &str) -> Result<(), EditError> {
    if window == "main" {
        Ok(())
    } else {
        Err(EditError::ScopeExpired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_preserves_the_authenticated_policy_except_for_a_new_vault_epoch() {
        let original = capstan_ssh::PoolKey {
            target: "host".into(),
            port: 22,
            username: "user".into(),
            credential_id: Uuid::new_v4(),
            credential_revision: 1,
            trust_fingerprint: "SHA256:pinned".into(),
            vault_epoch: 1,
            agent_identity: Some("identity".into()),
            agent_forwarding: false,
            proxy_path: String::new(),
        };
        let mut unlocked = original.clone();
        unlocked.vault_epoch += 1;
        assert!(same_policy(&original, &unlocked));
        for field in 0..10 {
            let mut changed = unlocked.clone();
            match field {
                0 => changed.target = "other-host".into(),
                1 => changed.port += 1,
                2 => changed.username = "other-user".into(),
                3 => changed.credential_id = Uuid::new_v4(),
                4 => changed.credential_revision += 1,
                5 => changed.trust_fingerprint = "SHA256:changed".into(),
                6 => changed.agent_identity = None,
                7 => changed.agent_forwarding = true,
                8 => changed.proxy_path = "proxy".into(),
                _ => changed.target = "HOST".into(),
            }
            assert!(!same_policy(&original, &changed));
        }
    }

    #[test]
    fn editor_snapshot_matches_the_rust_typescript_fixture() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../docs/agent/fixtures/M2-editor.json"
        ))
        .unwrap();
        let snapshot = RemoteEditSnapshot {
            revision: ExactU64(9_007_199_254_740_993),
            recovery_incomplete: false,
            sessions: vec![
                RemoteEditView {
                    id: Uuid::parse_str("00000000-0000-4000-8000-000000000031").unwrap(),
                    name: "notes 中文.txt".into(),
                    remote_path: Some("/home/test/notes 中文.txt".into()),
                    host_id: Some(Uuid::parse_str("00000000-0000-4000-8000-000000000032").unwrap()),
                    state: EditState::Conflict,
                    dirty: true,
                    recovered: false,
                    cleanup_required: true,
                    remote_temporaries: vec![RemoteTemporaryView {
                        id: Uuid::parse_str("00000000-0000-4000-8000-000000000034").unwrap(),
                        path: "/home/test/.capstan-edit-example.partial".into(),
                        state: super::super::TemporaryState::Changed,
                    }],
                    replace_allowed: true,
                    error: Some(EditError::Conflict),
                },
                RemoteEditView {
                    id: Uuid::parse_str("00000000-0000-4000-8000-000000000033").unwrap(),
                    name: "recovered.txt".into(),
                    remote_path: None,
                    host_id: None,
                    state: EditState::Residual,
                    dirty: true,
                    recovered: true,
                    cleanup_required: true,
                    remote_temporaries: Vec::new(),
                    replace_allowed: false,
                    error: None,
                },
            ],
        };
        assert_eq!(serde_json::to_value(snapshot).unwrap(), fixture);
    }

    fn recovered(root: &std::path::Path) -> (Arc<RemoteEditorService>, Uuid) {
        let id = Uuid::new_v4();
        let store = EditorStore::new(root.join("editor"));
        drop(
            store
                .prepare(&id.to_string(), "notes.txt", b"unsent local changes")
                .unwrap(),
        );
        let service = RemoteEditorService::new(
            Arc::new(Settings::new(root.to_owned())),
            store,
            Arc::new(tokio::sync::Mutex::new(())),
        );
        (service, id)
    }

    #[tokio::test]
    async fn recovered_copy_requires_a_local_decision_and_export_closes_only_after_publication() {
        let root = tempfile::tempdir().unwrap();
        let (service, id) = recovered(root.path());
        let snapshot = service.snapshot("main").unwrap();
        assert_eq!(snapshot.sessions.len(), 1);
        assert!(snapshot.sessions[0].recovered);
        assert!(snapshot.sessions[0].remote_path.is_none());
        assert!(snapshot.sessions[0].host_id.is_none());
        assert_eq!(
            service
                .bind_for_retry(
                    "main",
                    id,
                    &crate::sftp::SftpService::default(),
                    "new",
                    serde_json::from_value(serde_json::json!(Uuid::new_v4())).unwrap()
                )
                .await
                .unwrap_err(),
            EditError::ScopeExpired
        );
        assert_eq!(service.prepare_exit("main"), Err(EditError::Residual));
        assert_eq!(
            service.retry("main", id).await.unwrap_err(),
            EditError::ConnectionLost
        );
        assert_eq!(
            service
                .close("main", id, ResidualAction::SaveLocalCopy, None)
                .await,
            Err(EditError::Canceled)
        );
        assert!(service.has_open("main"));
        let output = root.path().join("export");
        std::fs::create_dir(&output).unwrap();
        service
            .close(
                "main",
                id,
                ResidualAction::SaveLocalCopy,
                Some(output.clone()),
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(output.join("notes.txt")).unwrap(),
            b"unsent local changes"
        );
        assert!(!service.has_open("main"));
        service.prepare_exit("main").unwrap();
        service.abort_exit();
        assert!(service.snapshot("secondary").is_err());
        assert!(service
            .close("main", id, ResidualAction::Discard, None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn failed_cleanup_keeps_the_session_and_unknown_editor_files() {
        let root = tempfile::tempdir().unwrap();
        let (service, id) = recovered(root.path());
        let extra = root
            .path()
            .join("editor")
            .join(id.to_string())
            .join("backup.txt");
        std::fs::write(&extra, b"editor backup").unwrap();
        assert_eq!(
            service
                .close("main", id, ResidualAction::Discard, None)
                .await,
            Err(EditError::Residual)
        );
        assert_eq!(std::fs::read(&extra).unwrap(), b"editor backup");
        assert!(service.has_open("main"));
        let other = RemoteEditorService::new(
            Arc::new(Settings::new(root.path().to_owned())),
            EditorStore::new(root.path().join("editor")),
            Arc::new(tokio::sync::Mutex::new(())),
        );
        assert!(other.snapshot("main").unwrap().sessions.is_empty());
        std::fs::remove_file(extra).unwrap();
        service
            .close("main", id, ResidualAction::Discard, None)
            .await
            .unwrap();
        assert!(!service.has_open("main"));
    }

    #[tokio::test]
    async fn keep_remote_releases_registry_only_after_owned_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let (service, id) = recovered(root.path());
        let entry = service.entry("main", id).unwrap();
        let mut edit = EditSession::begin(id, "/notes.txt".into(), b"remote", "v1".into()).unwrap();
        edit.state = EditState::Conflict;
        entry.active.lock().await.edit = Some(edit);
        let extra = root
            .path()
            .join("editor")
            .join(id.to_string())
            .join("backup");
        std::fs::write(&extra, b"retain").unwrap();
        assert_eq!(
            service
                .resolve("main", id, ConflictDecision::KeepRemote, None)
                .await
                .unwrap_err(),
            EditError::Residual
        );
        assert!(service.has_open("main"));
        assert_eq!(
            service.snapshot("main").unwrap().sessions[0].state,
            EditState::Residual
        );
        std::fs::remove_file(extra).unwrap();
        entry.active.lock().await.edit.as_mut().unwrap().state = EditState::Conflict;
        let view = service
            .resolve("main", id, ConflictDecision::KeepRemote, None)
            .await
            .unwrap();
        assert_eq!(view.state, EditState::Closed);
        assert!(service.snapshot("main").unwrap().sessions.is_empty());
        assert!(!root.path().join("editor").join(id.to_string()).exists());
    }

    #[tokio::test]
    async fn native_editor_preference_survives_service_restart_without_exposing_the_path() {
        let root = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::new(root.path().to_owned()));
        let store = EditorStore::new(root.path().join("editor"));
        let service = RemoteEditorService::new(
            settings.clone(),
            store.clone(),
            Arc::new(tokio::sync::Mutex::new(())),
        );
        let executable = root.path().join("Editor 中文.exe");
        service.configure("main", executable.clone()).await.unwrap();
        assert_eq!(settings.editor_executable().unwrap(), Some(executable));
        let restarted =
            RemoteEditorService::new(settings, store, Arc::new(tokio::sync::Mutex::new(())));
        assert!(restarted.configuration_id("main").unwrap().is_some());
        assert!(!serde_json::to_string(&restarted.snapshot("main").unwrap())
            .unwrap()
            .contains("Editor 中文.exe"));
    }

    #[tokio::test]
    async fn discovery_admits_eight_and_rescans_after_a_copy_is_resolved() {
        let root = tempfile::tempdir().unwrap();
        let store = EditorStore::new(root.path().join("editor"));
        for _ in 0..10 {
            drop(
                store
                    .prepare(&Uuid::new_v4().to_string(), "notes.txt", b"retained")
                    .unwrap(),
            );
        }
        let service = RemoteEditorService::new(
            Arc::new(Settings::new(root.path().to_owned())),
            store,
            Arc::new(tokio::sync::Mutex::new(())),
        );
        let first = service.snapshot("main").unwrap();
        assert_eq!(first.sessions.len(), MAX_EDITOR_SESSIONS);
        assert!(first.recovery_incomplete);
        for session in first.sessions {
            service
                .close("main", session.id, ResidualAction::Discard, None)
                .await
                .unwrap();
        }
        service.discover("main").unwrap();
        let rest = service.snapshot("main").unwrap();
        assert_eq!(rest.sessions.len(), 2);
        assert!(!rest.recovery_incomplete);
        for session in rest.sessions {
            service
                .close("main", session.id, ResidualAction::Discard, None)
                .await
                .unwrap();
        }
    }
}
