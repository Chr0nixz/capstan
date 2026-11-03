//! OS drop receipts and staged export tokens. IPC never grants local paths.
use super::*;
use crate::sftp::{SftpService, SftpSessionId};
use capstan_native_windows::{
    export::ExportStage,
    staged_drag::{DragOutcome, StagedDrag},
    transfer::Source,
};

const MAX_EXPORTS: usize = 8;
struct DropReceipt {
    window: String,
    paths: Vec<PathBuf>,
    expires: Instant,
    binding: Binding,
    parent: RemotePath,
    target: Uuid,
}
struct DropTarget {
    token: Uuid,
    binding: Binding,
    parent: RemotePath,
}
struct Export {
    window: String,
    name: String,
    binding: Option<Binding>,
    expires: Instant,
    tasks: Vec<Arc<Task>>,
    stage: Arc<Mutex<ExportStage>>,
    dragging: Arc<AtomicBool>,
    planning: bool,
    has_items: bool,
    ready_at: Option<Instant>,
}
#[derive(Default)]
pub(super) struct Registry {
    drops: HashMap<Uuid, DropReceipt>,
    exports: HashMap<Uuid, Export>,
    directory: Option<PathBuf>,
    targets: HashMap<String, DropTarget>,
    pending_exports: std::collections::HashSet<Uuid>,
}
struct ExportReservation<'a> {
    registry: &'a Mutex<Registry>,
    token: Uuid,
}
impl Drop for ExportReservation<'_> {
    fn drop(&mut self) {
        if let Ok(mut registry) = self.registry.lock() {
            registry.pending_exports.remove(&self.token);
        }
    }
}
struct PendingExportStage(Option<ExportStage>);
impl Drop for PendingExportStage {
    fn drop(&mut self) {
        // Before registry ownership, the stage contains only its marker.
        // Also clean it if the awaiting prepare future is canceled.
        if let Some(stage) = &mut self.0 {
            let _ = stage.cleanup();
        }
    }
}
impl Registry {
    pub(super) fn revoke(&mut self, window: &str) {
        self.targets.remove(window);
        self.drops.retain(|_, receipt| receipt.window != window);
        for export in self
            .exports
            .values_mut()
            .filter(|export| export.window == window)
        {
            export.binding = None;
            export.planning = false;
            export.expires = Instant::now();
        }
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeDropResult {
    pub queued: u32,
    pub plans: Vec<TreePlanSummary>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedExport {
    pub export: ExportSnapshot,
    pub plan: TreePlanSummary,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSnapshot {
    pub token: Uuid,
    pub name: String,
    pub state: &'static str,
}
pub struct ExportDrag {
    stage: Arc<Mutex<ExportStage>>,
    dragging: Arc<AtomicBool>,
}
impl ExportDrag {
    /// The desktop adapter must call this on its native OLE/UI thread.
    pub fn start(&self, parent: isize) -> Result<bool, SftpError> {
        let stage = self.stage.lock().map_err(|_| SftpError::LocalIo)?;
        let drag = StagedDrag::prepare(
            stage.root(),
            &stage.drag_paths().map_err(local::local_error)?,
        )
        .map_err(local::local_error)?;
        drag.start(parent)
            .map(|outcome| outcome == DragOutcome::Copied)
            .map_err(local::local_error)
    }
}
impl Drop for ExportDrag {
    fn drop(&mut self) {
        self.dragging.store(false, Ordering::SeqCst);
    }
}
impl TransferService {
    pub fn arm_native_drop(&self, context: &TransferContext) -> Result<Uuid, SftpError> {
        let TransferTarget::Upload { parent, .. } = &context.target else {
            return Err(SftpError::ScopeExpired);
        };
        if context.binding.window != "main"
            || context.client.is_closed()
            || self
                .state
                .lock()
                .map_err(|_| SftpError::LocalIo)?
                .closing
                .contains(&context.binding.window)
        {
            return Err(SftpError::ScopeExpired);
        }
        let token = Uuid::new_v4();
        self.native
            .lock()
            .map_err(|_| SftpError::LocalIo)?
            .targets
            .insert(
                context.binding.window.clone(),
                DropTarget {
                    token,
                    binding: context.binding.clone(),
                    parent: parent.clone(),
                },
            );
        Ok(token)
    }
    pub fn disarm_native_drop(&self, window: &str, token: Uuid) -> Result<(), SftpError> {
        let mut registry = self.native.lock().map_err(|_| SftpError::LocalIo)?;
        if registry
            .targets
            .get(window)
            .is_some_and(|target| target.token == token)
        {
            registry.targets.remove(window);
        }
        Ok(())
    }
    /// Only a trusted native DragDropEvent handler calls this method. There is
    /// deliberately no IPC command accepting these paths.
    pub fn capture_native_drop(
        &self,
        window: &str,
        paths: Vec<PathBuf>,
    ) -> Result<Uuid, SftpError> {
        if window != "main"
            || paths.is_empty()
            || paths.len() > 256
            || paths
                .iter()
                .map(|path| path.as_os_str().len())
                .sum::<usize>()
                > 65_536
            || paths.iter().any(|path| !path.is_absolute())
        {
            return Err(SftpError::ScopeExpired);
        }
        let mut registry = self.native.lock().map_err(|_| SftpError::LocalIo)?;
        registry
            .drops
            .retain(|_, receipt| receipt.expires > Instant::now());
        if registry
            .exports
            .values()
            .any(|export| export.dragging.load(Ordering::SeqCst))
        {
            return Err(SftpError::ScopeExpired);
        }
        if registry.drops.len() >= crate::sftp::MAX_LOCAL_GRANTS {
            return Err(SftpError::ResourceLimit);
        }
        let token = Uuid::new_v4();
        let target = registry
            .targets
            .get(window)
            .ok_or(SftpError::ScopeExpired)?;
        let binding = target.binding.clone();
        let parent = target.parent.clone();
        let target = target.token;
        registry.drops.insert(
            token,
            DropReceipt {
                window: window.into(),
                paths,
                expires: Instant::now() + Duration::from_secs(60),
                binding,
                parent,
                target,
            },
        );
        Ok(token)
    }
    pub async fn enqueue_native_drop(
        self: &Arc<Self>,
        sftp: &SftpService,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        parent: RemotePath,
        token: Uuid,
    ) -> Result<NativeDropResult, SftpError> {
        let initial = sftp.transfer_context(
            window,
            terminal,
            session,
            TransferTarget::Upload {
                parent: parent.clone(),
                name: "selection".into(),
            },
        )?;
        let paths = {
            let mut registry = self.native.lock().map_err(|_| SftpError::LocalIo)?;
            let receipt = registry
                .drops
                .get(&token)
                .filter(|receipt| receipt.window == window && receipt.expires > Instant::now())
                .ok_or(SftpError::ScopeExpired)?;
            if receipt.binding != initial.binding
                || receipt.parent != parent
                || registry
                    .targets
                    .get(window)
                    .is_none_or(|target| target.token != receipt.target)
            {
                return Err(SftpError::ScopeExpired);
            }
            let paths = receipt.paths.clone();
            registry.drops.remove(&token);
            paths
        };
        let selected = local::blocking(move || {
            let mut files = Vec::new();
            let mut folders = Vec::new();
            let mut names = HashSet::new();
            for path in paths {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .filter(|name| capstan_native_windows::transfer::valid_name(name))
                    .ok_or(SftpError::PathInvalid)?
                    .to_owned();
                if !names.insert(name.clone()) {
                    return Err(SftpError::TargetConflict);
                }
                if std::fs::symlink_metadata(&path)
                    .map_err(local::local_error)?
                    .is_dir()
                {
                    folders.push(path);
                } else {
                    files.push((name, Source::open(&path).map_err(local::local_error)?));
                }
            }
            if folders.len() > 8 {
                return Err(SftpError::ResourceLimit);
            }
            Ok((files, folders))
        })
        .await?;
        if initial.binding != sftp.export_binding(window, terminal, session)? {
            return Err(SftpError::ScopeExpired);
        }
        let mut plans = Vec::new();
        let mut queued = 0;
        let result = async {
            for folder in selected.1 {
                plans.push(
                    self.prepare_upload_tree(
                        sftp,
                        window,
                        terminal,
                        session,
                        parent.clone(),
                        folder,
                    )
                    .await?,
                );
            }
            for (name, file) in selected.0 {
                let context = sftp.transfer_context(
                    window,
                    terminal,
                    session,
                    TransferTarget::Upload {
                        parent: parent.clone(),
                        name,
                    },
                )?;
                let grant = self.grant_source(&context, Arc::new(file))?;
                if let Err(error) = self.enqueue(context, grant.token) {
                    let _ = self.release_selection(window, grant.token);
                    return Err(error);
                }
                queued += 1;
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            for plan in &plans {
                let _ = self.release_tree_plan(window, plan.token);
            }
            return Err(error);
        }
        Ok(NativeDropResult { queued, plans })
    }

    pub fn configure_export_directory(&self, directory: PathBuf) -> Result<(), SftpError> {
        let discovered = ExportStage::discover(&directory).map_err(local::local_error)?;
        let mut registry = self.native.lock().map_err(|_| SftpError::LocalIo)?;
        for (id, stage) in discovered.into_iter().take(MAX_EXPORTS) {
            let Ok(token) = Uuid::parse_str(&id) else {
                continue;
            };
            registry.exports.insert(
                token,
                Export {
                    window: "main".into(),
                    name: stage.display_name().unwrap_or(id),
                    binding: None,
                    expires: Instant::now(),
                    tasks: Vec::new(),
                    stage: Arc::new(Mutex::new(stage)),
                    dragging: Arc::new(AtomicBool::new(false)),
                    planning: false,
                    has_items: false,
                    ready_at: None,
                },
            );
        }
        registry.directory = Some(directory);
        Ok(())
    }
    pub async fn prepare_native_export(
        self: &Arc<Self>,
        sftp: &SftpService,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        entry: RemoteEntryId,
    ) -> Result<PreparedExport, SftpError> {
        let (name, directories, files) = match sftp.transfer_context(
            window,
            terminal,
            session,
            TransferTarget::Download { entry },
        ) {
            Ok(context) => {
                let name = context
                    .path
                    .as_str()
                    .rsplit('/')
                    .next()
                    .ok_or(SftpError::PathInvalid)?
                    .to_owned();
                (name.clone(), Vec::new(), vec![(name, context)])
            }
            Err(SftpError::UnsupportedFileType) => {
                let tree = sftp
                    .collect_directory_tree(window, terminal, session, entry)
                    .await?;
                let files = tree
                    .files()
                    .iter()
                    .map(|file| {
                        Ok((
                            file.relative().to_owned(),
                            sftp.transfer_context_for_tree_file(window, terminal, session, file)?,
                        ))
                    })
                    .collect::<Result<Vec<_>, SftpError>>()?;
                (
                    tree.root_name().to_owned(),
                    tree.directories().to_vec(),
                    files,
                )
            }
            Err(error) => return Err(error),
        };
        // A directory export can contain no files; derive its binding from the
        // authorized tree through the current service context instead.
        let binding = match files.first() {
            Some((_, context)) => context.binding.clone(),
            None => sftp.export_binding(window, terminal, session)?,
        };
        let token = Uuid::new_v4();
        let directory = {
            let mut registry = self.native.lock().map_err(|_| SftpError::LocalIo)?;
            if registry.exports.len() + registry.pending_exports.len() >= MAX_EXPORTS {
                return Err(SftpError::ResourceLimit);
            }
            let directory = registry.directory.clone().ok_or(SftpError::LocalIo)?;
            registry.pending_exports.insert(token);
            directory
        };
        let reservation = ExportReservation {
            registry: &self.native,
            token,
        };
        let mut stage = local::blocking(move || {
            ExportStage::prepare(&directory, &token.to_string(), &[], &[])
                .map(|stage| PendingExportStage(Some(stage)))
                .map_err(local::local_error)
        })
        .await?;
        let root = stage
            .0
            .as_ref()
            .ok_or(SftpError::LocalIo)?
            .root()
            .to_owned();
        {
            let mut registry = self.native.lock().map_err(|_| SftpError::LocalIo)?;
            registry.pending_exports.remove(&token);
            registry.exports.insert(
                token,
                Export {
                    window: window.into(),
                    name: name.clone(),
                    binding: Some(binding.clone()),
                    expires: Instant::now()
                        + Duration::from_secs(crate::sftp::LOCAL_GRANT_LIFETIME_SECONDS),
                    tasks: Vec::new(),
                    stage: Arc::new(Mutex::new(stage.0.take().ok_or(SftpError::LocalIo)?)),
                    dragging: Arc::new(AtomicBool::new(false)),
                    planning: true,
                    has_items: false,
                    ready_at: None,
                },
            );
        }
        drop(reservation);
        let plan = self
            .prepare_export_plan(tree::ExportInput {
                token,
                binding,
                client: sftp.tree_client(window, terminal, session)?,
                root,
                directories,
                files,
            })
            .await;
        match plan {
            Ok(plan) => Ok(PreparedExport {
                export: ExportSnapshot {
                    token,
                    name,
                    state: "planning",
                },
                plan,
            }),
            Err(error) => {
                let _ = self.release_native_export(window, token).await;
                Err(error)
            }
        }
    }
    pub(super) fn retain_native_export(&self, token: Uuid) -> Result<(), SftpError> {
        if let Some(export) = self
            .native
            .lock()
            .map_err(|_| SftpError::LocalIo)?
            .exports
            .get_mut(&token)
        {
            export.expires = Instant::now();
            export.planning = false;
            export.binding = None;
        }
        Ok(())
    }
    pub(super) async fn configure_native_export(
        &self,
        token: Uuid,
        directories: &[String],
        files: &[String],
    ) -> Result<(), SftpError> {
        let stage = {
            let registry = self.native.lock().map_err(|_| SftpError::LocalIo)?;
            let export = registry
                .exports
                .get(&token)
                .ok_or(SftpError::ScopeExpired)?;
            if !export.planning || export.dragging.load(Ordering::SeqCst) {
                return Err(SftpError::ScopeExpired);
            }
            export.stage.clone()
        };
        let owned_directories = directories.to_vec();
        let owned_files = files.to_vec();
        local::blocking(move || {
            stage
                .lock()
                .map_err(|_| SftpError::LocalIo)?
                .configure(&owned_directories, &owned_files)
                .map_err(local::local_error)
        })
        .await?;
        let mut registry = self.native.lock().map_err(|_| SftpError::LocalIo)?;
        let export = registry
            .exports
            .get_mut(&token)
            .filter(|export| export.planning && !export.dragging.load(Ordering::SeqCst))
            .ok_or(SftpError::ScopeExpired)?;
        export.has_items = !directories.is_empty() || !files.is_empty();
        if let Some(name) = directories
            .first()
            .or_else(|| files.first())
            .and_then(|path| path.split('/').next())
        {
            export.name = name.to_owned();
        }
        Ok(())
    }
    pub(super) async fn complete_native_export(
        self: &Arc<Self>,
        token: Uuid,
        binding: &Binding,
        items: Vec<(TransferContext, Local)>,
    ) -> Result<u32, SftpError> {
        let tasks =
            if items.len() == 1 && matches!(items[0].0.target, TransferTarget::Download { .. }) {
                items
                    .into_iter()
                    .map(|(context, local)| {
                        self.make_task(&context, &local, None)
                            .map(|task| (context, local, task))
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                batch::prepare_tree(items)
                    .await?
                    .into_iter()
                    .map(|(context, local, member)| {
                        self.make_task(&context, &local, Some(member))
                            .map(|task| (context, local, task))
                    })
                    .collect::<Result<Vec<_>, _>>()?
            };
        let stage = {
            let registry = self.native.lock().map_err(|_| SftpError::LocalIo)?;
            registry
                .exports
                .get(&token)
                .filter(|export| {
                    export.binding.as_ref() == Some(binding)
                        && export.planning
                        && !export.dragging.load(Ordering::SeqCst)
                })
                .map(|export| export.stage.clone())
                .ok_or(SftpError::ScopeExpired)?
        };
        let partials = {
            let stage = stage.lock().map_err(|_| SftpError::LocalIo)?;
            tasks
                .iter()
                .map(|(_, local, task)| {
                    let Local::Destination(destination) = local else {
                        return Err(SftpError::ScopeExpired);
                    };
                    let target = destination.target_path()?.ok_or(SftpError::LocalIo)?;
                    let id = task.status.lock().map_err(|_| SftpError::LocalIo)?.id;
                    let partial = target.with_file_name(format!(".capstan-{id}.partial"));
                    Ok(partial
                        .strip_prefix(stage.root())
                        .map_err(|_| SftpError::ScopeExpired)?
                        .to_string_lossy()
                        .replace('\\', "/"))
                })
                .collect::<Result<Vec<_>, SftpError>>()?
        };
        local::blocking(move || {
            stage
                .lock()
                .map_err(|_| SftpError::LocalIo)?
                .register_partials(&partials)
                .map_err(local::local_error)
        })
        .await?;
        let mut registry = self.native.lock().map_err(|_| SftpError::LocalIo)?;
        let export = registry
            .exports
            .get_mut(&token)
            .filter(|export| {
                export.binding.as_ref() == Some(binding)
                    && export.planning
                    && !export.dragging.load(Ordering::SeqCst)
            })
            .ok_or(SftpError::ScopeExpired)?;
        let mut state = self.state.lock().map_err(|_| SftpError::LocalIo)?;
        if state.closing.contains(&binding.window)
            || tasks
                .iter()
                .any(|(context, _, _)| context.binding != *binding || context.client.is_closed())
        {
            return Err(SftpError::ScopeExpired);
        }
        state.reserve_tasks(tasks.len())?;
        export.tasks = tasks.iter().map(|(_, _, task)| task.clone()).collect();
        export.planning = false;
        export.expires =
            Instant::now() + Duration::from_secs(crate::sftp::LOCAL_GRANT_LIFETIME_SECONDS);
        let count = tasks.len() as u32;
        for (context, local, task) in tasks {
            state.tasks.push(task.clone());
            self.spawn_task(task, context, local);
        }
        Ok(count)
    }
    pub(super) fn enqueue_prepared(
        self: &Arc<Self>,
        prepared: batch::PreparedFolder,
    ) -> Result<u32, SftpError> {
        let tasks = prepared
            .into_iter()
            .map(|(context, local, member)| {
                self.make_task(&context, &local, Some(member))
                    .map(|task| (context, local, task))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut state = self.state.lock().map_err(|_| SftpError::LocalIo)?;
        if tasks.iter().any(|(context, _, _)| {
            state.closing.contains(&context.binding.window) || context.client.is_closed()
        }) {
            return Err(SftpError::ScopeExpired);
        }
        state.reserve_tasks(tasks.len())?;
        let count = tasks.len() as u32;
        for (context, local, task) in tasks {
            state.tasks.push(task.clone());
            self.spawn_task(task, context, local);
        }
        Ok(count)
    }
    pub fn export_snapshot(&self, window: &str) -> Result<Vec<ExportSnapshot>, SftpError> {
        let mut registry = self.native.lock().map_err(|_| SftpError::LocalIo)?;
        registry
            .exports
            .iter_mut()
            .filter(|(_, export)| export.window == window)
            .map(|(token, export)| {
                Ok(ExportSnapshot {
                    token: *token,
                    name: export.name.clone(),
                    state: update_export_state(export)?,
                })
            })
            .collect()
    }
    pub async fn begin_export_drag(
        &self,
        sftp: &SftpService,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        token: Uuid,
    ) -> Result<ExportDrag, SftpError> {
        let binding = sftp.export_binding(window, terminal, session)?;
        let guard = {
            let mut registry = self.native.lock().map_err(|_| SftpError::LocalIo)?;
            let export = registry
                .exports
                .get_mut(&token)
                .filter(|export| export.binding.as_ref() == Some(&binding))
                .ok_or(SftpError::ScopeExpired)?;
            if update_export_state(export)? != "ready"
                || export.dragging.swap(true, Ordering::SeqCst)
            {
                return Err(SftpError::ScopeExpired);
            }
            ExportDrag {
                stage: export.stage.clone(),
                dragging: export.dragging.clone(),
            }
        };
        local::blocking(move || {
            guard
                .stage
                .lock()
                .map_err(|_| SftpError::LocalIo)?
                .pin()
                .map_err(local::local_error)?;
            Ok(guard)
        })
        .await
    }
    pub async fn release_native_export(&self, window: &str, token: Uuid) -> Result<(), SftpError> {
        let (tasks, stage, dragging) = {
            let registry = self.native.lock().map_err(|_| SftpError::LocalIo)?;
            let export = registry
                .exports
                .get(&token)
                .filter(|export| export.window == window)
                .ok_or(SftpError::ScopeExpired)?;
            if export.dragging.swap(true, Ordering::SeqCst) {
                return Err(SftpError::ScopeExpired);
            }
            (
                export.tasks.clone(),
                export.stage.clone(),
                export.dragging.clone(),
            )
        };
        self.trees
            .lock()
            .map_err(|_| SftpError::LocalIo)?
            .retain(|_, handle| handle.export != Some(token));
        let result = async {
            for task in &tasks {
                task.cancel.cancel();
            }
            tokio::time::timeout(
                Duration::from_secs(crate::sftp::CANCEL_TIMEOUT_SECONDS),
                async {
                    loop {
                        if tasks.iter().all(|task| {
                            task.status
                                .lock()
                                .is_ok_and(|status| status.state.terminal())
                        }) {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                },
            )
            .await
            .map_err(|_| SftpError::LocalIo)?;
            for task in &tasks {
                let local = task
                    .retry
                    .lock()
                    .map_err(|_| SftpError::LocalIo)?
                    .as_ref()
                    .and_then(|plan| plan.local.clone());
                if let Some(Local::Destination(destination)) = local {
                    local::blocking(move || destination.discard()).await?;
                }
                task.clear_retry();
                task.clear_recovery()?;
            }
            let partials = local::blocking(move || {
                let mut stage = stage.lock().map_err(|_| SftpError::LocalIo)?;
                let partials = stage.partial_paths();
                stage.cleanup().map_err(local::local_error)?;
                Ok(partials)
            })
            .await?;
            let store = self
                .recovery
                .lock()
                .map_err(|_| SftpError::LocalIo)?
                .clone();
            if let Some(store) = store {
                for partial in partials {
                    let id = partial
                        .file_name()
                        .and_then(|name| name.to_str())
                        .and_then(|name| name.strip_prefix(".capstan-"))
                        .and_then(|name| name.strip_suffix(".partial"))
                        .and_then(|name| Uuid::parse_str(name).ok());
                    if let Some(id) = id {
                        if store
                            .record(id)
                            .is_ok_and(|record| record.local_partial.as_ref() == Some(&partial))
                        {
                            store.remove(id)?;
                        }
                    }
                }
            }
            self.native
                .lock()
                .map_err(|_| SftpError::LocalIo)?
                .exports
                .remove(&token);
            Ok(())
        }
        .await;
        dragging.store(false, Ordering::SeqCst);
        result
    }
}
fn update_export_state(export: &mut Export) -> Result<&'static str, SftpError> {
    if export.dragging.load(Ordering::SeqCst) {
        return Ok("dragging");
    }
    if export.binding.is_none() {
        return Ok("retained");
    }
    if export.planning {
        return Ok(if export.expires > Instant::now() {
            "planning"
        } else {
            "retained"
        });
    }
    if !export.has_items {
        return Ok("retained");
    }
    let statuses = export
        .tasks
        .iter()
        .map(|task| {
            task.status
                .lock()
                .map(|status| status.state)
                .map_err(|_| SftpError::LocalIo)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if statuses
        .iter()
        .all(|state| *state == TransferState::Completed)
    {
        if export.ready_at.is_none() {
            export.ready_at = Some(Instant::now());
            export.expires =
                Instant::now() + Duration::from_secs(crate::sftp::LOCAL_GRANT_LIFETIME_SECONDS);
        }
        Ok(if export.expires > Instant::now() {
            "ready"
        } else {
            "retained"
        })
    } else if statuses
        .iter()
        .any(|state| state.terminal() && *state != TransferState::Completed)
    {
        Ok("failed")
    } else {
        Ok("preparing")
    }
}
