//! Folder plans defer every mutation until incompatible names/types and existing
//! directories have explicit, version-bound decisions. Skips cover a subtree.
use super::*;
use crate::sftp::{
    service::{map_error, metadata},
    FileKind, SftpService, SftpSessionId,
};
use capstan_native_windows::transfer::{Destination, SelectedDirectory};
use capstan_ssh::sftp::ClientError;

const MAX_PLANS: usize = 8;
pub(super) struct PlanHandle {
    pub(super) binding: Binding,
    pub(super) expires: Instant,
    plan: Arc<tokio::sync::Mutex<Plan>>,
    cancel: CancellationToken,
    pub(super) export: Option<Uuid>,
}
pub(super) struct ExportInput {
    pub token: Uuid,
    pub binding: Binding,
    pub client: Arc<SftpClient>,
    pub root: PathBuf,
    pub directories: Vec<String>,
    pub files: Vec<(String, TransferContext)>,
}
impl Drop for PlanHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ConflictKind {
    DirectoryExists,
    TypeConflict,
    InvalidName,
    NameCollision,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TreeConflict {
    pub entry: u32,
    pub source: String,
    pub relative: String,
    pub directory: bool,
    kind: ConflictKind,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TreePlanSummary {
    pub token: Uuid,
    pub revision: ExactU64,
    pub name: String,
    pub files: u32,
    pub conflicts: Vec<TreeConflict>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeResult {
    pub plan: Option<TreePlanSummary>,
    pub queued: u32,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TreeDecision {
    pub entry: u32,
    pub decision: TreeChoice,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TreeChoice {
    Merge,
    Rename { name: String },
    Skip,
}
#[derive(Clone, PartialEq, Eq)]
enum Version {
    Remote(Metadata),
    Local(LocalVersion),
    Blocked,
}
impl Version {
    fn kind(&self) -> FileKind {
        match self {
            Self::Remote(value) => value.kind,
            Self::Local(value) if value.attributes & 0x10 != 0 => FileKind::Directory,
            Self::Local(_) => FileKind::File,
            Self::Blocked => FileKind::Other,
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
struct Pending {
    view: TreeConflict,
    version: Option<Version>,
}
struct Inspection {
    index: usize,
    kind: Option<ConflictKind>,
    version: Option<Version>,
}
struct Node {
    source: String,
    relative: String,
    directory: bool,
    local: Option<Arc<dyn LocalSource>>,
    context: Option<TransferContext>,
    skipped: bool,
    merged: Option<Version>,
}
struct Plan {
    token: Uuid,
    revision: u64,
    binding: Binding,
    client: Arc<SftpClient>,
    parent: Option<RemotePath>,
    destination: Option<Arc<SelectedDirectory>>,
    nodes: Vec<Node>,
    pending: Vec<Pending>,
    cancel: CancellationToken,
    closed: bool,
    export: Option<Uuid>,
}
impl Plan {
    fn summary(&self) -> TreePlanSummary {
        TreePlanSummary {
            token: self.token,
            revision: ExactU64(self.revision),
            name: self
                .nodes
                .first()
                .map_or_else(String::new, |node| node.source.clone()),
            files: self
                .nodes
                .iter()
                .filter(|node| !node.directory && !node.skipped)
                .count() as u32,
            conflicts: self
                .pending
                .iter()
                .map(|pending| pending.view.clone())
                .collect(),
        }
    }
    async fn scan(&mut self) -> Result<(), SftpError> {
        if self.cancel.is_cancelled() {
            return Err(SftpError::OperationCanceled);
        }
        let mut names = HashMap::<String, usize>::new();
        for node in &self.nodes {
            if !node.skipped {
                *names
                    .entry(if self.destination.is_some() {
                        node.relative.to_uppercase()
                    } else {
                        node.relative.clone()
                    })
                    .or_default() += 1;
            }
        }
        let mut blocked = Vec::<String>::new();
        let mut pending = Vec::new();
        let mut order = self
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.relative.split('/').count(), index))
            .collect::<Vec<_>>();
        order.sort_unstable();
        // Inspect ancestors before issuing any descendant lookup. Only peers
        // at the same depth can overlap, so a conflicting/invalid directory
        // still blocks its entire subtree before any I/O beneath it.
        for level in order.chunk_by(|left, right| left.0 == right.0) {
            let candidates = level
                .iter()
                .map(|&(_, index)| index)
                .filter(|&index| {
                    let node = &self.nodes[index];
                    !node.skipped
                        && !blocked
                            .iter()
                            .any(|parent| node.relative.starts_with(&format!("{parent}/")))
                })
                .collect::<Vec<_>>();
            for group in candidates.chunks(super::preflight::MAX_SNAPSHOTS) {
                // Finish every admitted snapshot even when a peer fails.
                // Remote admission and bounded native I/O remain unchanged.
                let inspections = super::preflight::join_snapshots(
                    group
                        .iter()
                        .map(|index| self.inspect_node(Some(*index), &names)),
                )
                .await?;
                for inspection in inspections {
                    let Some(inspection) = inspection else {
                        continue;
                    };
                    if let Some(kind) = inspection.kind {
                        let node = &mut self.nodes[inspection.index];
                        node.merged = None;
                        if node.directory {
                            blocked.push(node.relative.clone());
                        }
                        pending.push(Pending {
                            view: TreeConflict {
                                entry: inspection.index as u32,
                                source: node.source.clone(),
                                relative: node.relative.clone(),
                                directory: node.directory,
                                kind,
                            },
                            version: inspection.version,
                        });
                    }
                }
            }
        }
        // Depth scheduling must not change stable UI entry/conflict ordering.
        pending.sort_unstable_by_key(|item| item.view.entry);
        self.pending = pending;
        Ok(())
    }

    async fn inspect_node(
        &self,
        index: Option<usize>,
        names: &HashMap<String, usize>,
    ) -> Result<Option<Inspection>, SftpError> {
        let Some(index) = index else {
            return Ok(None);
        };
        if self.cancel.is_cancelled() {
            return Err(SftpError::OperationCanceled);
        }
        let node = &self.nodes[index];
        let valid = node.relative.len() <= 4096
            && node.relative.split('/').count() <= 64
            && (self.export.is_none()
                || node.relative.split('/').next() != Some(".capstan-export"))
            && node.relative.split('/').all(|name| {
                if self.destination.is_some() {
                    capstan_native_windows::transfer::valid_name(name)
                } else {
                    crate::sftp::RemoteName::from_bytes(name.as_bytes()).is_ok()
                }
            });
        let key = if self.destination.is_some() {
            node.relative.to_uppercase()
        } else {
            node.relative.clone()
        };
        let (kind, version) = if !valid {
            (Some(ConflictKind::InvalidName), None)
        } else if names[&key] > 1 {
            (Some(ConflictKind::NameCollision), None)
        } else {
            let version = if let Some(destination) = &self.destination {
                let destination = destination.clone();
                let relative = PathBuf::from(&node.relative);
                local::blocking(move || match destination.snapshot(&relative) {
                    Ok(version) => Ok(version.map(|version| Version::Local(version.into()))),
                    Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {
                        Ok(Some(Version::Blocked))
                    }
                    Err(error) => Err(local::local_error(error)),
                })
                .await?
            } else {
                let path = upload_tree_path(
                    self.parent.as_ref().ok_or(SftpError::ScopeExpired)?,
                    &node.relative,
                )?;
                match self.client.lstat(path.as_str(), &self.cancel).await {
                    Ok(attributes) => Some(Version::Remote(metadata(attributes))),
                    Err(ClientError::NotFound) => None,
                    Err(error) => return Err(map_error(error)),
                }
            };
            let kind = match version.as_ref().map(Version::kind) {
                None => None,
                Some(FileKind::Directory)
                    if node.directory && node.merged.as_ref() == version.as_ref() =>
                {
                    None
                }
                Some(FileKind::Directory) if node.directory => Some(ConflictKind::DirectoryExists),
                Some(FileKind::File) if !node.directory => None,
                Some(_) => Some(ConflictKind::TypeConflict),
            };
            (kind, version)
        };
        Ok(Some(Inspection {
            index,
            kind,
            version,
        }))
    }
}
impl TransferService {
    pub(super) async fn prepare_export_plan(
        self: &Arc<Self>,
        input: ExportInput,
    ) -> Result<TreePlanSummary, SftpError> {
        let destination = local::blocking(move || {
            SelectedDirectory::open(&input.root).map_err(local::local_error)
        })
        .await?;
        let mut nodes = input
            .directories
            .into_iter()
            .map(|relative| node(relative, true, None, None))
            .collect::<Vec<_>>();
        nodes.extend(
            input
                .files
                .into_iter()
                .map(|(relative, context)| node(relative, false, None, Some(context))),
        );
        self.register_tree(Plan {
            token: Uuid::new_v4(),
            revision: 1,
            binding: input.binding,
            client: input.client,
            parent: None,
            destination: Some(Arc::new(destination)),
            nodes,
            pending: Vec::new(),
            cancel: CancellationToken::new(),
            closed: false,
            export: Some(input.token),
        })
        .await
    }
    /// `path` comes only from the native picker/drop adapter, never an IPC field.
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_upload_tree(
        self: &Arc<Self>,
        sftp: &SftpService,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        parent: RemotePath,
        path: PathBuf,
    ) -> Result<TreePlanSummary, SftpError> {
        let context = sftp.transfer_context(
            window,
            terminal,
            session,
            TransferTarget::Upload {
                parent: parent.clone(),
                name: "selection".into(),
            },
        )?;
        let nodes = local::blocking(move || {
            let root = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or(SftpError::PathInvalid)?
                .to_owned();
            let directories = capstan_native_windows::transfer::enumerate_folder_directories(&path)
                .map_err(local::local_error)?;
            let files = capstan_native_windows::transfer::enumerate_folder(&path)
                .map_err(local::local_error)?;
            let mut nodes = vec![node(root.clone(), true, None, None)];
            nodes.extend(
                directories
                    .into_iter()
                    .map(|relative| node(format!("{root}/{}", slash(&relative)), true, None, None)),
            );
            nodes.extend(files.into_iter().map(|(relative, source)| {
                node(
                    format!("{root}/{}", slash(&relative)),
                    false,
                    Some(Arc::new(source)),
                    None,
                )
            }));
            Ok(nodes)
        })
        .await?;
        let binding = sftp.export_binding(window, terminal, session)?;
        if binding != context.binding {
            return Err(SftpError::ScopeExpired);
        }
        self.register_tree(Plan {
            token: Uuid::new_v4(),
            revision: 1,
            binding,
            client: context.client,
            parent: Some(parent),
            destination: None,
            nodes,
            pending: Vec::new(),
            cancel: CancellationToken::new(),
            closed: false,
            export: None,
        })
        .await
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_download_tree(
        self: &Arc<Self>,
        sftp: &SftpService,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        entry: RemoteEntryId,
        path: PathBuf,
    ) -> Result<TreePlanSummary, SftpError> {
        let tree = sftp
            .collect_directory_tree(window, terminal, session, entry)
            .await?;
        let destination =
            local::blocking(move || SelectedDirectory::open(&path).map_err(local::local_error))
                .await?;
        let mut nodes = tree
            .directories()
            .iter()
            .map(|relative| node(relative.clone(), true, None, None))
            .collect::<Vec<_>>();
        for file in tree.files() {
            nodes.push(node(
                file.relative().to_owned(),
                false,
                None,
                Some(sftp.transfer_context_for_tree_file(window, terminal, session, file)?),
            ));
        }
        self.register_tree(Plan {
            token: Uuid::new_v4(),
            revision: 1,
            binding: sftp.export_binding(window, terminal, session)?,
            client: sftp.tree_client(window, terminal, session)?,
            parent: None,
            destination: Some(Arc::new(destination)),
            nodes,
            pending: Vec::new(),
            cancel: CancellationToken::new(),
            closed: false,
            export: None,
        })
        .await
    }
    async fn register_tree(self: &Arc<Self>, mut plan: Plan) -> Result<TreePlanSummary, SftpError> {
        if plan.nodes.len() > 4096
            || plan.nodes.iter().filter(|node| !node.directory).count() > MAX_QUEUED_TRANSFERS
        {
            return Err(SftpError::ResourceLimit);
        }
        plan.nodes.sort_by(|left, right| {
            left.relative
                .matches('/')
                .count()
                .cmp(&right.relative.matches('/').count())
                .then_with(|| left.relative.cmp(&right.relative))
        });
        plan.scan().await?;
        let summary = plan.summary();
        let mut plans = self.trees.lock().map_err(|_| SftpError::LocalIo)?;
        plans.retain(|_, handle| handle.expires > Instant::now());
        if plans.len() >= MAX_PLANS {
            return Err(SftpError::ResourceLimit);
        }
        plans.insert(
            plan.token,
            PlanHandle {
                binding: plan.binding.clone(),
                expires: Instant::now()
                    + Duration::from_secs(crate::sftp::LOCAL_GRANT_LIFETIME_SECONDS),
                cancel: plan.cancel.clone(),
                export: plan.export,
                plan: Arc::new(tokio::sync::Mutex::new(plan)),
            },
        );
        drop(plans);
        self.start_reaper();
        Ok(summary)
    }
    pub fn release_tree_plan(&self, window: &str, token: Uuid) -> Result<(), SftpError> {
        let mut plans = self.trees.lock().map_err(|_| SftpError::LocalIo)?;
        if plans
            .get(&token)
            .is_none_or(|handle| handle.binding.window != window)
        {
            return Err(SftpError::ScopeExpired);
        }
        let export = plans.remove(&token).and_then(|handle| handle.export);
        drop(plans);
        if let Some(export) = export {
            self.retain_native_export(export)?;
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn resolve_tree_plan(
        self: &Arc<Self>,
        sftp: &SftpService,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        token: Uuid,
        revision: ExactU64,
        decisions: Vec<TreeDecision>,
    ) -> Result<TreeResult, SftpError> {
        let binding = sftp.export_binding(window, terminal, session)?;
        let handle = {
            let plans = self.trees.lock().map_err(|_| SftpError::LocalIo)?;
            plans
                .get(&token)
                .filter(|handle| handle.binding == binding && handle.expires > Instant::now())
                .map(|handle| handle.plan.clone())
                .ok_or(SftpError::ScopeExpired)?
        };
        let mut plan = handle.lock().await;
        if plan.closed || plan.revision != revision.0 {
            return Err(SftpError::ScopeExpired);
        }
        let previous = plan.pending.clone();
        plan.scan().await?;
        if plan.pending != previous {
            plan.revision += 1;
            return Ok(TreeResult {
                plan: Some(plan.summary()),
                queued: 0,
            });
        }
        let mut ids = HashSet::new();
        for decision in &decisions {
            let pending = plan
                .pending
                .iter()
                .find(|pending| pending.view.entry == decision.entry)
                .ok_or(SftpError::ScopeExpired)?;
            if !ids.insert(decision.entry) {
                return Err(SftpError::ScopeExpired);
            }
            match &decision.decision {
                TreeChoice::Merge if pending.view.kind != ConflictKind::DirectoryExists => {
                    return Err(SftpError::ScopeExpired)
                }
                TreeChoice::Rename { name }
                    if (plan.destination.is_some()
                        && !capstan_native_windows::transfer::valid_name(name))
                        || crate::sftp::RemoteName::from_bytes(name.as_bytes()).is_err() =>
                {
                    return Err(SftpError::PathInvalid)
                }
                _ => {}
            }
        }
        for decision in decisions {
            let pending = plan
                .pending
                .iter()
                .find(|pending| pending.view.entry == decision.entry)
                .cloned()
                .ok_or(SftpError::ScopeExpired)?;
            let index = decision.entry as usize;
            match decision.decision {
                TreeChoice::Merge => plan.nodes[index].merged = pending.version,
                TreeChoice::Skip => {
                    let prefix = plan.nodes[index].relative.clone();
                    for node in &mut plan.nodes {
                        if node.relative == prefix
                            || node.relative.starts_with(&format!("{prefix}/"))
                        {
                            node.skipped = true;
                            node.local = None;
                            node.context = None;
                        }
                    }
                }
                TreeChoice::Rename { name } => {
                    let prefix = plan.nodes[index].relative.clone();
                    let renamed = prefix
                        .rsplit_once('/')
                        .map_or_else(|| name.clone(), |(parent, _)| format!("{parent}/{name}"));
                    for node in &mut plan.nodes {
                        if node.relative == prefix
                            || node.relative.starts_with(&format!("{prefix}/"))
                        {
                            node.relative = format!("{renamed}{}", &node.relative[prefix.len()..]);
                            node.merged = None;
                        }
                    }
                }
            }
        }
        plan.scan().await?;
        plan.revision += 1;
        if !plan.pending.is_empty() {
            return Ok(TreeResult {
                plan: Some(plan.summary()),
                queued: 0,
            });
        }
        if sftp.export_binding(window, terminal, session)? != plan.binding
            || plan.cancel.is_cancelled()
        {
            return Err(SftpError::ScopeExpired);
        }
        let directories = plan
            .nodes
            .iter()
            .filter(|node| node.directory && !node.skipped)
            .map(|node| node.relative.clone())
            .collect::<Vec<_>>();
        if let Some(token) = plan.export {
            let files = plan
                .nodes
                .iter()
                .filter(|node| !node.directory && !node.skipped)
                .map(|node| node.relative.clone())
                .collect::<Vec<_>>();
            self.configure_native_export(token, &directories, &files)
                .await?;
        }
        if let Some(parent) = &plan.parent {
            sftp.ensure_directory_tree(window, terminal, session, parent.clone(), &directories)
                .await?;
        } else {
            let destination = plan.destination.clone().ok_or(SftpError::ScopeExpired)?;
            local::blocking(move || {
                capstan_native_windows::transfer::prepare_destination_folder(
                    destination.root(),
                    &directories.iter().map(PathBuf::from).collect::<Vec<_>>(),
                )
                .map_err(local::local_error)
            })
            .await?;
        }
        let mut items = Vec::new();
        for node in plan
            .nodes
            .iter()
            .filter(|node| !node.directory && !node.skipped)
        {
            if let Some(parent) = &plan.parent {
                let context = sftp.transfer_context(
                    window,
                    terminal,
                    session,
                    TransferTarget::UploadTree {
                        parent: parent.clone(),
                        relative: node.relative.clone(),
                    },
                )?;
                items.push((
                    context,
                    Local::Source(node.local.clone().ok_or(SftpError::ScopeExpired)?),
                ));
            } else {
                let mut context = node.context.clone().ok_or(SftpError::ScopeExpired)?;
                if node.relative.contains('/') {
                    context.target = TransferTarget::DownloadTree {
                        path: context.path.clone(),
                        relative: node.relative.clone(),
                    };
                }
                let destination = plan.destination.clone().ok_or(SftpError::ScopeExpired)?;
                let relative = PathBuf::from(&node.relative);
                let local = local::blocking(move || {
                    Destination::select_under_folder(destination.root(), &relative)
                        .map_err(local::local_error)
                })
                .await?;
                items.push((
                    context,
                    Local::Destination(Arc::new(WindowsDestination::new(local))),
                ));
            }
        }
        if plan.cancel.is_cancelled() {
            return Err(SftpError::OperationCanceled);
        }
        let queued = if let Some(token) = plan.export {
            self.complete_native_export(token, &plan.binding, items)
                .await?
        } else {
            let prepared = batch::prepare_tree(items).await?;
            if plan.cancel.is_cancelled() {
                return Err(SftpError::OperationCanceled);
            }
            self.enqueue_prepared(prepared)?
        };
        plan.closed = true;
        drop(plan);
        self.trees
            .lock()
            .map_err(|_| SftpError::LocalIo)?
            .remove(&token);
        Ok(TreeResult { plan: None, queued })
    }
}
fn node(
    relative: String,
    directory: bool,
    local: Option<Arc<dyn LocalSource>>,
    context: Option<TransferContext>,
) -> Node {
    Node {
        source: relative.clone(),
        relative,
        directory,
        local,
        context,
        skipped: false,
        merged: None,
    }
}
fn slash(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_tree_fixture_preserves_exact_revision_and_only_metadata_fields() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../../docs/agent/fixtures/M2-tree.json"))
                .unwrap();
        let plan: TreePlanSummary = serde_json::from_value(fixture.clone()).unwrap();
        assert_eq!(plan.revision.0, 9_007_199_254_740_995);
        assert_eq!(plan.conflicts[0].kind, ConflictKind::NameCollision);
        assert_eq!(serde_json::to_value(plan).unwrap(), fixture);
    }
}
