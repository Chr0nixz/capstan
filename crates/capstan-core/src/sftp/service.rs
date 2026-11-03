use super::{
    Binding, DirectoryPage, ExactU64, FileKind, Metadata, PageInput, RemoteEntryId, RemoteName,
    RemotePath, ScopeRegistry, SftpError, SftpSessionId,
};
use capstan_ssh::{
    sftp::{Attributes, ClientError, RemoteDirectory, SftpClient},
    ConnectionLease, PasswordConnection,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
#[path = "delete.rs"]
mod delete;
pub use delete::{DeleteEntryResult, DeleteOutcome, DeleteTreeReport, DeleteTreeSummary};

type Lease = ConnectionLease<PasswordConnection>;
#[derive(Clone)]
struct Registered {
    binding: Binding,
    lease: Lease,
    cancel: CancellationToken,
    cwd: Arc<Mutex<super::cwd::CwdState>>,
}
struct Cursor {
    directory: Option<RemoteDirectory>,
    revision: ExactU64,
    sequence: u32,
    pending: VecDeque<(RemoteName, Metadata)>,
    ended: bool,
    omitted: u32,
}
impl Cursor {
    async fn close(&mut self) {
        if let Some(directory) = self.directory.take() {
            let _ = directory.close(&CancellationToken::new()).await;
        }
    }
}
struct View {
    binding: Binding,
    terminal: String,
    client: Arc<SftpClient>,
    cursor: tokio::sync::Mutex<Option<Cursor>>,
    touched: Mutex<Instant>,
    cancel: CancellationToken,
    navigation: AtomicU64,
}
impl Drop for View {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(mut cursor) = self.cursor.get_mut().take() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    cursor.close().await;
                });
            }
        }
    }
}
#[derive(Default)]
struct State {
    terminals: HashMap<String, Registered>,
    views: HashMap<SftpSessionId, Arc<View>>,
    scopes: ScopeRegistry,
    confirmations: HashMap<Uuid, PendingConfirmation>,
    deletions: HashMap<Uuid, Arc<delete::DeletePlan>>,
    deletion_closing: std::collections::HashSet<String>,
}
#[derive(Default)]
pub struct SftpService {
    state: Mutex<State>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachedSftp {
    pub session_id: SftpSessionId,
    pub host_id: Uuid,
    pub connection_generation: ExactU64,
    pub path: RemotePath,
    pub atomic_replace: bool,
    pub fsync: bool,
    pub request_bytes: usize,
}
pub struct RemoteFileContext {
    pub binding: Binding,
    pub client: Arc<SftpClient>,
    pub lease: ConnectionLease<PasswordConnection>,
    pub path: RemotePath,
    pub metadata: Metadata,
}

/// A bounded, Rust-owned recursive selection used by the folder download
/// command. Its fields stay private so a WebView cannot manufacture a path
/// capability.
pub struct RemoteTreeFile {
    path: RemotePath,
    relative: String,
    metadata: Metadata,
}
impl RemoteTreeFile {
    pub fn relative(&self) -> &str {
        &self.relative
    }
}
pub struct RemoteTree {
    root_name: String,
    directories: Vec<String>,
    files: Vec<RemoteTreeFile>,
}
impl RemoteTree {
    pub fn root_name(&self) -> &str {
        &self.root_name
    }
    pub fn directories(&self) -> &[String] {
        &self.directories
    }
    pub fn files(&self) -> &[RemoteTreeFile] {
        &self.files
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
pub enum Mutation {
    Delete,
    Rename { name: String },
    Permissions { mode: u32 },
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Confirmation {
    pub token: Uuid,
    pub host_id: Uuid,
    pub path: RemotePath,
    pub metadata: Metadata,
    pub mutation: Mutation,
}
struct PendingConfirmation {
    session: SftpSessionId,
    binding: Binding,
    entry: RemoteEntryId,
    metadata: Metadata,
    mutation: Mutation,
    expires: Instant,
}

impl SftpService {
    /// Called only by the trusted Rust terminal-start path after authentication.
    pub fn register(
        &self,
        window: &str,
        terminal: &str,
        host: Uuid,
        connection: Uuid,
        lease: Lease,
    ) -> Result<(), SftpError> {
        let mut state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
        if state.terminals.len() >= 128 || state.terminals.contains_key(terminal) {
            return Err(SftpError::ResourceLimit);
        }
        state.terminals.insert(
            terminal.to_owned(),
            Registered {
                cwd: Arc::new(Mutex::new(super::cwd::CwdState::new(
                    lease.authenticated_target(),
                ))),
                binding: Binding {
                    window: window.to_owned(),
                    terminal: Uuid::new_v4(),
                    connection,
                    generation: lease.connection().connection_generation(),
                    host,
                },
                lease,
                cancel: CancellationToken::new(),
            },
        );
        Ok(())
    }
    pub fn revoke_terminal(&self, terminal: &str) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(registered) = state.terminals.remove(terminal) {
                registered.cancel.cancel();
                state.scopes.revoke_terminal(registered.binding.terminal);
                state
                    .views
                    .retain(|_, view| view.binding.terminal != registered.binding.terminal);
                state.confirmations.retain(|_, confirmation| {
                    confirmation.binding.terminal != registered.binding.terminal
                });
            }
        }
    }
    fn cwd_state(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        generation: ExactU64,
    ) -> Result<Arc<Mutex<super::cwd::CwdState>>, SftpError> {
        let view = self.view(window, terminal, session)?;
        let registered = self.registered(window, terminal)?;
        if registered.binding != view.binding
            || registered.binding.generation != generation.0
            || registered.cancel.is_cancelled()
        {
            return Err(SftpError::ScopeExpired);
        }
        Ok(registered.cwd)
    }

    pub fn update_cwd(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        generation: ExactU64,
        sequence: ExactU64,
        uri: &str,
    ) -> Result<super::cwd::CwdSnapshot, SftpError> {
        self.cwd_state(window, terminal, session, generation)?
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .update(sequence, uri)
    }

    pub fn configure_cwd_aliases(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        generation: ExactU64,
        aliases: Option<Vec<String>>,
    ) -> Result<super::cwd::CwdSnapshot, SftpError> {
        self.cwd_state(window, terminal, session, generation)?
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .configure(aliases)
    }
    fn registered(&self, window: &str, terminal: &str) -> Result<Registered, SftpError> {
        self.state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .terminals
            .get(terminal)
            .filter(|registered| registered.binding.window == window)
            .cloned()
            .ok_or(SftpError::ScopeExpired)
    }
    fn view(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
    ) -> Result<Arc<View>, SftpError> {
        let registered = self.registered(window, terminal)?;
        let mut state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
        let view = state
            .views
            .get(&session)
            .filter(|view| view.binding == registered.binding && view.terminal == terminal)
            .cloned()
            .ok_or(SftpError::ScopeExpired)?;
        let mut touched = view.touched.lock().map_err(|_| SftpError::ConnectionLost)?;
        if touched.elapsed() >= Duration::from_secs(1800) || view.cancel.is_cancelled() {
            return Err(SftpError::ScopeExpired);
        }
        state
            .scopes
            .authorize(session, &view.binding, Instant::now())?;
        *touched = Instant::now();
        Ok(view.clone())
    }
    pub async fn attach(
        &self,
        window: &str,
        terminal: &str,
        cancel: &CancellationToken,
    ) -> Result<AttachedSftp, SftpError> {
        let registered = self.registered(window, terminal)?;
        // Dropping an in-flight facade request quarantines only SFTP. Revoking
        // a terminal must also interrupt attachment, not wait for its timeout.
        let (client, path) = tokio::select! {
            biased;
            () = registered.cancel.cancelled() => return Err(SftpError::ScopeExpired),
            result = async {
                let client = registered.lease.open_sftp(cancel).await.map_err(map_error)?;
                let path = RemotePath::parse(&client.realpath(".", cancel).await.map_err(map_error)?)?;
                Ok::<_, SftpError>((client, path))
            } => result?,
        };
        let capabilities = client.capabilities();
        let mut state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
        if !state
            .terminals
            .get(terminal)
            .is_some_and(|current| current.binding == registered.binding)
            || registered.cancel.is_cancelled()
        {
            return Err(SftpError::ScopeExpired);
        }
        state.views.retain(|_, view| {
            view.touched
                .lock()
                .is_ok_and(|touched| touched.elapsed() < Duration::from_secs(1800))
        });
        let session_id = state
            .scopes
            .attach(registered.binding.clone(), Instant::now())?;
        state.views.insert(
            session_id,
            Arc::new(View {
                binding: registered.binding.clone(),
                terminal: terminal.to_owned(),
                client,
                cursor: tokio::sync::Mutex::new(None),
                touched: Mutex::new(Instant::now()),
                cancel: registered.cancel.child_token(),
                navigation: AtomicU64::new(0),
            }),
        );
        Ok(AttachedSftp {
            session_id,
            host_id: registered.binding.host,
            connection_generation: ExactU64(registered.binding.generation),
            path,
            atomic_replace: capabilities.atomic_replace,
            fsync: capabilities.fsync,
            request_bytes: capabilities.request_bytes,
        })
    }
    /// Capture a task's own connection lease before detaching its browser.
    /// This is a trusted Rust boundary, never deserialized from the WebView.
    pub fn transfer_context(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        target: super::TransferTarget,
    ) -> Result<super::TransferContext, SftpError> {
        let registered = self.registered(window, terminal)?;
        let view = self.view(window, terminal, session)?;
        let mut state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
        let now = Instant::now();
        let target_for_context = target.clone();
        let (path, baseline) = match target {
            super::TransferTarget::Upload { parent, name } => {
                state
                    .scopes
                    .require_directory(session, &view.binding, &parent, now)?;
                (
                    parent.join(&RemoteName::from_bytes(name.as_bytes())?)?,
                    None,
                )
            }
            super::TransferTarget::UploadTree { parent, relative } => {
                state
                    .scopes
                    .require_directory(session, &view.binding, &parent, now)?;
                (crate::transfer::upload_tree_path(&parent, &relative)?, None)
            }
            super::TransferTarget::DownloadTree { .. } => {
                return Err(SftpError::ScopeExpired);
            }
            super::TransferTarget::Download { entry } => {
                let (path, metadata) =
                    state
                        .scopes
                        .resolve_entry(session, &view.binding, entry, now)?;
                if metadata.kind != FileKind::File {
                    return Err(SftpError::UnsupportedFileType);
                }
                (path, Some(metadata))
            }
        };
        Ok(super::TransferContext {
            binding: view.binding.clone(),
            client: view.client.clone(),
            path,
            baseline,
            target: target_for_context,
            lease: registered.lease,
        })
    }

    /// Rebuild a context for a persisted transfer descriptor. The target is
    /// supplied only by the core recovery registry; callers cannot provide a
    /// filesystem path or remote path through the WebView.
    pub fn transfer_context_for_recovery(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        target: super::TransferTarget,
    ) -> Result<super::TransferContext, SftpError> {
        let registered = self.registered(window, terminal)?;
        let view = self.view(window, terminal, session)?;
        let (path, baseline) = match &target {
            super::TransferTarget::Upload { parent, name } => (
                parent.join(&RemoteName::from_bytes(name.as_bytes())?)?,
                None,
            ),
            super::TransferTarget::UploadTree { parent, relative } => {
                (crate::transfer::upload_tree_path(parent, relative)?, None)
            }
            super::TransferTarget::Download { .. } | super::TransferTarget::DownloadTree { .. } => {
                return Err(SftpError::ScopeExpired);
            }
        };
        Ok(super::TransferContext {
            binding: view.binding.clone(),
            client: view.client.clone(),
            path,
            baseline,
            target,
            lease: registered.lease,
        })
    }

    /// Reauthorize a persisted download against the current authenticated host.
    /// The opaque descriptor comes from the recovery registry, never IPC.
    pub async fn transfer_context_for_recovered_download(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        descriptor: crate::transfer::RecoveredDownload,
    ) -> Result<super::TransferContext, SftpError> {
        let registered = self.registered(window, terminal)?;
        let view = self.view(window, terminal, session)?;
        if view.binding.host != descriptor.host
            || !descriptor.policy_sha256.eq_ignore_ascii_case(
                &crate::transfer::retry_policy_digest(registered.lease.policy()),
            )
        {
            return Err(SftpError::ScopeExpired);
        }
        let baseline = metadata(
            view.client
                .lstat(descriptor.path.as_str(), &CancellationToken::new())
                .await
                .map_err(map_error)?,
        );
        if baseline.kind != FileKind::File
            || baseline.size.map(|size| size.0) != Some(descriptor.total)
            || baseline.modified_seconds.is_none()
            || descriptor
                .baseline
                .as_ref()
                .is_some_and(|original| !baseline.matches_baseline(original))
        {
            return Err(SftpError::ResumeMismatch);
        }
        self.view(window, terminal, session)?;
        Ok(super::TransferContext {
            binding: view.binding.clone(),
            client: view.client.clone(),
            path: descriptor.path,
            baseline: Some(baseline),
            target: descriptor.target,
            lease: registered.lease,
        })
    }

    #[cfg(windows)]
    pub(crate) fn export_binding(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
    ) -> Result<Binding, SftpError> {
        self.registered(window, terminal)?;
        let view = self.view(window, terminal, session)?;
        self.state
            .lock()
            .map_err(|_| SftpError::LocalIo)?
            .scopes
            .authorize(session, &view.binding, Instant::now())?;
        Ok(view.binding.clone())
    }
    #[cfg(windows)]
    pub(crate) fn tree_client(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
    ) -> Result<Arc<SftpClient>, SftpError> {
        self.export_binding(window, terminal, session)?;
        Ok(self.view(window, terminal, session)?.client.clone())
    }

    /// Bind one file from a Rust-owned recursive tree selection. The tree is
    /// collected and consumed by the native picker command in one request, so
    /// no path or file metadata is accepted from the WebView.
    pub fn transfer_context_for_tree_file(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        file: &RemoteTreeFile,
    ) -> Result<super::TransferContext, SftpError> {
        let registered = self.registered(window, terminal)?;
        let view = self.view(window, terminal, session)?;
        self.state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .scopes
            .authorize(session, &view.binding, Instant::now())?;
        Ok(super::TransferContext {
            binding: view.binding.clone(),
            client: view.client.clone(),
            path: file.path.clone(),
            baseline: Some(file.metadata.clone()),
            target: super::TransferTarget::DownloadTree {
                path: file.path.clone(),
                relative: file.relative.clone(),
            },
            lease: registered.lease,
        })
    }

    /// Collect a bounded directory tree from an entry in the live browse
    /// scope. Symlinks and unsupported entries are rejected rather than
    /// followed, and directory handles are closed before returning.
    pub async fn collect_directory_tree(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        entry: RemoteEntryId,
    ) -> Result<RemoteTree, SftpError> {
        let view = self.view(window, terminal, session)?;
        let _operation = view.cursor.lock().await;
        self.view(window, terminal, session)?;
        let (root, metadata) = self
            .state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .scopes
            .resolve_entry(session, &view.binding, entry, Instant::now())?;
        if metadata.kind != FileKind::Directory {
            return Err(SftpError::UnsupportedFileType);
        }
        let root_name = root
            .as_str()
            .rsplit('/')
            .next()
            .filter(|value| !value.is_empty())
            .ok_or(SftpError::PathInvalid)?
            .to_owned();
        let mut tree = RemoteTree {
            root_name: root_name.clone(),
            directories: vec![root_name.clone()],
            files: Vec::new(),
        };
        collect_tree_directory(&view, &root, &root_name, 0, &mut tree).await?;
        tree.directories.sort_by(|left, right| {
            left.matches('/')
                .count()
                .cmp(&right.matches('/').count())
                .then_with(|| left.cmp(right))
        });
        tree.files
            .sort_by(|left, right| left.relative.cmp(&right.relative));
        Ok(tree)
    }
    pub fn remote_file_context(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        entry: RemoteEntryId,
    ) -> Result<RemoteFileContext, SftpError> {
        let registered = self.registered(window, terminal)?;
        let view = self.view(window, terminal, session)?;
        let (path, metadata) = self
            .state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .scopes
            .resolve_entry(session, &view.binding, entry, Instant::now())?;
        if metadata.kind != FileKind::File {
            return Err(SftpError::UnsupportedFileType);
        }
        let size = metadata.size.ok_or(SftpError::UnsupportedFileType)?.0;
        if size > crate::remote_edit::MAX_EDITOR_FILE_BYTES as u64 {
            return Err(SftpError::UnsupportedFileType);
        }
        Ok(RemoteFileContext {
            binding: view.binding.clone(),
            client: view.client.clone(),
            lease: registered.lease,
            path,
            metadata,
        })
    }

    /// The editor supplies its retained Rust-owned target and baseline. No
    /// desktop command accepts these paths from the WebView as authority.
    #[cfg(windows)]
    pub(crate) fn editor_retry_context(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        path: RemotePath,
        metadata: Metadata,
    ) -> Result<RemoteFileContext, SftpError> {
        let registered = self.registered(window, terminal)?;
        let view = self.view(window, terminal, session)?;
        if registered.binding != view.binding {
            return Err(SftpError::ScopeExpired);
        }
        Ok(RemoteFileContext {
            binding: view.binding.clone(),
            client: view.client.clone(),
            lease: registered.lease,
            path,
            metadata,
        })
    }
    pub async fn detach(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
    ) -> Result<(), SftpError> {
        let view = self.view(window, terminal, session)?;
        view.cancel.cancel();
        {
            let mut state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
            state
                .scopes
                .detach(session, &view.binding, Instant::now())?;
            state.views.remove(&session);
            state
                .confirmations
                .retain(|_, pending| pending.session != session);
        }
        if let Some(mut cursor) = view.cursor.lock().await.take() {
            cursor.close().await;
        }
        Ok(())
    }
    pub async fn list(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        path: RemotePath,
    ) -> Result<DirectoryPage, SftpError> {
        let view = self.view(window, terminal, session)?;
        let navigation = view
            .navigation
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        let mut current = view.cursor.lock().await;
        self.view(window, terminal, session)?;
        if view.navigation.load(Ordering::SeqCst) != navigation {
            return Err(SftpError::OperationCanceled);
        }
        let cancel = &view.cancel;
        let canonical = RemotePath::parse(
            &view
                .client
                .realpath(path.as_str(), cancel)
                .await
                .map_err(map_error)?,
        )?;
        let directory = view
            .client
            .open_directory(canonical.as_str(), cancel)
            .await
            .map_err(map_error)?;
        let mut cursor = Cursor {
            directory: Some(directory),
            revision: ExactU64(0),
            sequence: 0,
            pending: VecDeque::new(),
            ended: false,
            omitted: 0,
        };
        let result = async {
            Self::fill_page(&view, &canonical, &mut cursor).await?;
            if view.navigation.load(Ordering::SeqCst) != navigation || cancel.is_cancelled() {
                return Err(SftpError::OperationCanceled);
            }
            self.state
                .lock()
                .map_err(|_| SftpError::ConnectionLost)?
                .scopes
                .replace_directory(
                    session,
                    &view.binding,
                    canonical,
                    Self::page_input(&cursor),
                    Instant::now(),
                )
        }
        .await;
        match result {
            Ok(page) => {
                cursor.revision = page.request_revision;
                Self::advance(&mut cursor, &page).await;
                if let Some(mut old) = current.replace(cursor) {
                    old.close().await;
                }
                Ok(page)
            }
            Err(error) => {
                cursor.close().await;
                Err(error)
            }
        }
    }
    pub async fn next_page(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        revision: ExactU64,
    ) -> Result<DirectoryPage, SftpError> {
        let view = self.view(window, terminal, session)?;
        let mut current = view.cursor.lock().await;
        self.view(window, terminal, session)?;
        let cursor = current
            .as_mut()
            .filter(|cursor| {
                cursor.revision == revision && (!cursor.ended || !cursor.pending.is_empty())
            })
            .ok_or(SftpError::ScopeExpired)?;
        self.page(&view, session, cursor).await
    }
    async fn page(
        &self,
        view: &View,
        session: SftpSessionId,
        cursor: &mut Cursor,
    ) -> Result<DirectoryPage, SftpError> {
        let path = self
            .state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .scopes
            .directory_path(session, &view.binding, Instant::now())?;
        Self::fill_page(view, &path, cursor).await?;
        let page = self
            .state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .scopes
            .accept_page(
                session,
                &view.binding,
                Self::page_input(cursor),
                Instant::now(),
            )?;
        Self::advance(cursor, &page).await;
        Ok(page)
    }

    async fn fill_page(
        view: &View,
        path: &RemotePath,
        cursor: &mut Cursor,
    ) -> Result<(), SftpError> {
        // One-entry lookahead distinguishes a full last page from a next page.
        // A hostile stream of dot/unsupported entries must not loop forever.
        let deadline =
            tokio::time::Instant::now() + Duration::from_secs(super::REQUEST_TIMEOUT_SECONDS);
        let mut packets = 0;
        while cursor.pending.len() <= super::DIRECTORY_PAGE_ENTRIES && !cursor.ended {
            if packets == 64 {
                return Err(SftpError::ResourceLimit);
            }
            packets += 1;
            let names = cursor.directory.as_mut().ok_or(SftpError::ScopeExpired)?;
            let names = tokio::time::timeout_at(deadline, names.next(&view.cancel))
                .await
                .map_err(|_| SftpError::ConnectionLost)?
                .map_err(map_error)?;
            cursor.ended = names.is_empty();
            for (name, attributes) in names {
                if name == "." || name == ".." {
                    continue;
                }
                match RemoteName::from_library_string(&name) {
                    Ok(name) if path.join(&name).is_ok() => {
                        cursor.pending.push_back((name, metadata(attributes)))
                    }
                    _ => {
                        cursor.omitted = cursor
                            .omitted
                            .checked_add(1)
                            .ok_or(SftpError::ResourceLimit)?
                    }
                }
            }
        }
        Ok(())
    }

    fn page_input(cursor: &Cursor) -> PageInput {
        PageInput {
            request_revision: cursor.revision,
            page_sequence: cursor.sequence,
            entries: cursor
                .pending
                .iter()
                .take(super::DIRECTORY_PAGE_ENTRIES)
                .cloned()
                .collect(),
            has_more: !cursor.ended || cursor.pending.len() > super::DIRECTORY_PAGE_ENTRIES,
            omitted_entries: cursor.omitted,
        }
    }

    async fn advance(cursor: &mut Cursor, page: &DirectoryPage) {
        cursor.pending.drain(..page.entries.len());
        cursor.sequence = page.page_sequence.saturating_add(1);
        if !page.has_more {
            cursor.close().await;
        }
    }
    pub fn release_entries(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        entries: &[RemoteEntryId],
    ) -> Result<(), SftpError> {
        let view = self.view(window, terminal, session)?;
        self.state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .scopes
            .release_entries(session, &view.binding, entries, Instant::now())
    }
    pub async fn create(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        parent: RemotePath,
        name: &str,
        directory: bool,
    ) -> Result<(), SftpError> {
        let view = self.view(window, terminal, session)?;
        let _operation = view.cursor.lock().await;
        self.view(window, terminal, session)?;
        self.state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .scopes
            .require_directory(session, &view.binding, &parent, Instant::now())?;
        let path = parent.join(&RemoteName::from_bytes(name.as_bytes())?)?;
        let cancel = view.cancel.clone();
        if directory {
            view.client
                .mkdir(path.as_str(), &cancel)
                .await
                .map_err(map_error)
        } else {
            view.client
                .open_file(path.as_str(), true, &cancel)
                .await
                .map_err(map_error)?
                .close(&cancel)
                .await
                .map_err(map_error)
        }
    }
    /// Create the bounded directory portion of a native folder selection.
    ///
    /// The caller must have a live browse scope for `parent`; relative names
    /// are parsed as remote components and are never treated as arbitrary
    /// paths. Existing directories are accepted, while an existing file or a
    /// failed mkdir stops the operation before any file task is queued.
    pub async fn ensure_directory_tree(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        parent: RemotePath,
        relative_paths: &[String],
    ) -> Result<(), SftpError> {
        if relative_paths.len() > 4096 {
            return Err(SftpError::ResourceLimit);
        }
        let view = self.view(window, terminal, session)?;
        let _operation = view.cursor.lock().await;
        self.view(window, terminal, session)?;
        self.state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .scopes
            .require_directory(session, &view.binding, &parent, Instant::now())?;
        let mut paths = Vec::new();
        for relative in relative_paths {
            let mut current = parent.clone();
            let components = relative.split('/').collect::<Vec<_>>();
            if components.is_empty() || components.iter().any(|component| component.is_empty()) {
                return Err(SftpError::PathInvalid);
            }
            for component in components {
                current = current.join(&RemoteName::from_bytes(component.as_bytes())?)?;
                paths.push(current.clone());
            }
        }
        paths.sort_by(|left, right| {
            left.as_str()
                .matches('/')
                .count()
                .cmp(&right.as_str().matches('/').count())
                .then_with(|| left.as_str().cmp(right.as_str()))
        });
        paths.dedup();
        for path in paths {
            match view.client.lstat(path.as_str(), &view.cancel).await {
                Ok(attributes) => {
                    if metadata(attributes).kind != FileKind::Directory {
                        return Err(SftpError::TargetConflict);
                    }
                }
                Err(ClientError::NotFound) => {
                    view.client
                        .mkdir(path.as_str(), &view.cancel)
                        .await
                        .map_err(map_error)?;
                }
                Err(error) => return Err(map_error(error)),
            }
        }
        Ok(())
    }

    pub async fn prepare_mutation(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        entry: RemoteEntryId,
        mutation: Mutation,
    ) -> Result<Confirmation, SftpError> {
        let view = self.view(window, terminal, session)?;
        let _operation = view.cursor.lock().await;
        self.view(window, terminal, session)?;
        let (path, baseline) = self
            .state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .scopes
            .resolve_entry(session, &view.binding, entry, Instant::now())?;
        if let Mutation::Rename { name } = &mutation {
            RemoteName::from_bytes(name.as_bytes())?;
        }
        if matches!(mutation, Mutation::Permissions { mode } if mode > 0o7777) {
            return Err(SftpError::PathInvalid);
        }
        if baseline.kind == FileKind::Unknown
            || baseline.kind == FileKind::Other
            || (baseline.kind == FileKind::Symlink
                && matches!(mutation, Mutation::Permissions { .. }))
        {
            return Err(SftpError::UnsupportedFileType);
        }
        let current = metadata(
            view.client
                .lstat(path.as_str(), &view.cancel)
                .await
                .map_err(map_error)?,
        );
        if !current.matches_baseline(&baseline) {
            return Err(SftpError::TargetConflict);
        }
        let token = Uuid::new_v4();
        let mut state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
        state
            .confirmations
            .retain(|_, confirmation| confirmation.expires > Instant::now());
        if state.confirmations.len() >= 128 {
            return Err(SftpError::ResourceLimit);
        }
        state.confirmations.insert(
            token,
            PendingConfirmation {
                session,
                binding: view.binding.clone(),
                entry,
                metadata: current.clone(),
                mutation: mutation.clone(),
                expires: Instant::now() + Duration::from_secs(300),
            },
        );
        Ok(Confirmation {
            token,
            host_id: view.binding.host,
            path,
            metadata: current,
            mutation,
        })
    }
    pub async fn apply_mutation(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        token: Uuid,
    ) -> Result<(), SftpError> {
        let view = self.view(window, terminal, session)?;
        let _operation = view.cursor.lock().await;
        self.view(window, terminal, session)?;
        let pending = {
            let mut state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
            let pending = state
                .confirmations
                .get(&token)
                .ok_or(SftpError::ScopeExpired)?;
            if pending.binding != view.binding
                || pending.session != session
                || pending.expires <= Instant::now()
            {
                return Err(SftpError::ScopeExpired);
            }
            state
                .confirmations
                .remove(&token)
                .ok_or(SftpError::ScopeExpired)?
        };
        let (path, _) = self
            .state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .scopes
            .resolve_entry(session, &view.binding, pending.entry, Instant::now())?;
        let cancel = view.cancel.clone();
        let current = metadata(
            view.client
                .lstat(path.as_str(), &cancel)
                .await
                .map_err(map_error)?,
        );
        if !current.matches_baseline(&pending.metadata) {
            return Err(SftpError::TargetConflict);
        }
        match pending.mutation {
            Mutation::Delete => {
                if current.kind == FileKind::Directory {
                    view.client
                        .remove_empty_directory(path.as_str(), &cancel)
                        .await
                        .map_err(map_error)
                } else {
                    view.client
                        .remove_file(path.as_str(), &cancel)
                        .await
                        .map_err(map_error)
                }
            }
            Mutation::Permissions { mode } => {
                // SFTP v3 SETSTAT follows symlinks. Never chmod a link target
                // through a confirmation describing only the link itself.
                if current.kind == FileKind::Symlink {
                    return Err(SftpError::UnsupportedFileType);
                }
                view.client
                    .set_permissions(path.as_str(), mode, &cancel)
                    .await
                    .map_err(map_error)
            }
            Mutation::Rename { name } => {
                let parent = path
                    .as_str()
                    .rsplit_once('/')
                    .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
                    .ok_or(SftpError::PathInvalid)?;
                let target =
                    RemotePath::parse(parent)?.join(&RemoteName::from_bytes(name.as_bytes())?)?;
                match view.client.lstat(target.as_str(), &cancel).await {
                    Err(ClientError::NotFound) => {}
                    Ok(_) => return Err(SftpError::TargetConflict),
                    Err(error) => return Err(map_error(error)),
                }
                view.client
                    .rename(path.as_str(), target.as_str(), &cancel)
                    .await
                    .map_err(map_error)
            }
        }
    }
}

const MAX_REMOTE_TREE_ENTRIES: usize = 4096;

async fn collect_tree_directory(
    view: &View,
    path: &RemotePath,
    relative_prefix: &str,
    depth: usize,
    tree: &mut RemoteTree,
) -> Result<(), SftpError> {
    let mut pending = vec![(path.clone(), relative_prefix.to_owned(), depth)];
    while let Some((directory_path, prefix, depth)) = pending.pop() {
        if depth >= 64 {
            return Err(SftpError::ResourceLimit);
        }
        let mut directory = view
            .client
            .open_directory(directory_path.as_str(), &view.cancel)
            .await
            .map_err(map_error)?;
        let mut children = Vec::new();
        let deadline =
            tokio::time::Instant::now() + Duration::from_secs(super::REQUEST_TIMEOUT_SECONDS);
        let result = async {
            loop {
                let packet = tokio::time::timeout_at(deadline, directory.next(&view.cancel))
                    .await
                    .map_err(|_| SftpError::ConnectionLost)?
                    .map_err(map_error)?;
                if packet.is_empty() {
                    break;
                }
                children.extend(packet);
                if children.len() > MAX_REMOTE_TREE_ENTRIES {
                    return Err(SftpError::ResourceLimit);
                }
            }
            Ok::<(), SftpError>(())
        }
        .await;
        let closed = directory.close(&CancellationToken::new()).await;
        result?;
        closed.map_err(map_error)?;
        for (name, attributes) in children {
            if name == "." || name == ".." {
                continue;
            }
            let name = RemoteName::from_library_string(&name)?;
            let child_path = directory_path.join(&name)?;
            let relative = if prefix.is_empty() {
                name.as_str().to_owned()
            } else {
                format!("{}/{}", prefix, name.as_str())
            };
            if relative.len() > 4096 {
                return Err(SftpError::ResourceLimit);
            }
            let metadata = metadata(attributes);
            match metadata.kind {
                FileKind::Directory => {
                    tree.directories.push(relative.clone());
                    if tree.directories.len() + tree.files.len() > MAX_REMOTE_TREE_ENTRIES {
                        return Err(SftpError::ResourceLimit);
                    }
                    pending.push((child_path, relative, depth + 1));
                }
                FileKind::File => {
                    tree.files.push(RemoteTreeFile {
                        path: child_path,
                        relative,
                        metadata,
                    });
                    if tree.directories.len() + tree.files.len() > MAX_REMOTE_TREE_ENTRIES {
                        return Err(SftpError::ResourceLimit);
                    }
                }
                FileKind::Symlink | FileKind::Other | FileKind::Unknown => {
                    return Err(SftpError::UnsupportedFileType);
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn metadata(attributes: Attributes) -> Metadata {
    Metadata {
        kind: FileKind::from_permissions(attributes.permissions),
        size: attributes.size.map(ExactU64),
        modified_seconds: attributes
            .modified_seconds
            .map(|value| ExactU64(u64::from(value))),
        permissions: attributes.permissions,
    }
}
pub(crate) fn map_error(error: ClientError) -> SftpError {
    match error {
        ClientError::Unavailable => SftpError::SftpUnavailable,
        ClientError::PermissionDenied => SftpError::PermissionDenied,
        ClientError::InvalidPath => SftpError::PathInvalid,
        ClientError::NotFound => SftpError::NotFound,
        ClientError::Unsupported => SftpError::UnsupportedFileType,
        ClientError::ConnectionLost => SftpError::ConnectionLost,
        ClientError::Canceled => SftpError::OperationCanceled,
        ClientError::ResourceLimit => SftpError::ResourceLimit,
        ClientError::Uncertain | ClientError::RemoteIo => SftpError::RemoteIo,
    }
}
