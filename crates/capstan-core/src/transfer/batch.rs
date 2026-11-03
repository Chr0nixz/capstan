//! A folder decision is confined to one native selection and its preflight versions.
use super::{engine::TargetVersion, local::blocking, *};
use crate::sftp::{service::map_error, service::metadata, RemoteName};
use capstan_ssh::sftp::ClientError;
use tokio::sync::watch;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchSummary {
    pub id: Uuid,
    pub name: String,
    pub files: u32,
    pub conflicts: u32,
}

#[derive(Clone, Copy)]
pub(super) enum BatchChoice {
    Replace,
    Skip,
}

pub(super) struct Batch {
    pub summary: BatchSummary,
    pub policy: watch::Sender<Option<BatchChoice>>,
}
impl Batch {
    pub fn choose(&self, choice: BatchChoice) -> Result<(), SftpError> {
        self.policy
            .send_if_modified(|current| {
                if current.is_some() {
                    return false;
                }
                *current = Some(choice);
                true
            })
            .then_some(())
            .ok_or(SftpError::ScopeExpired)
    }
}

pub(super) struct Member {
    pub batch: Arc<Batch>,
    pub expected: Option<TargetVersion>,
    pub active: AtomicBool,
}

type FolderItem = (TransferContext, Local);
pub(super) type PreparedFolder = Vec<(TransferContext, Local, Arc<Member>)>;

pub(super) async fn prepare(items: Vec<FolderItem>) -> Result<PreparedFolder, SftpError> {
    prepare_inner(items, false).await
}
pub(super) async fn prepare_tree(items: Vec<FolderItem>) -> Result<PreparedFolder, SftpError> {
    prepare_inner(items, true).await
}
async fn prepare_inner(
    items: Vec<FolderItem>,
    trusted_download: bool,
) -> Result<PreparedFolder, SftpError> {
    let Some((first, _)) = items.first() else {
        return Ok(Vec::new());
    };
    if items.len() > MAX_QUEUED_TRANSFERS {
        return Err(SftpError::ResourceLimit);
    }
    let binding = first.binding.clone();
    let (root, upload, target_root) = folder_root(first, trusted_download)?;
    let mut paths = HashSet::new();
    // Validate the entire selection before doing preflight I/O. Only trusted
    // native tree callers can form this group; WebView IDs cannot join it.
    for (context, local) in &items {
        if context.binding != binding
            || folder_root(context, trusted_download)?
                != (root.clone(), upload, target_root.clone())
        {
            return Err(SftpError::ScopeExpired);
        }
        if !paths.insert(context.path.clone())
            || !matches!(
                (upload, local),
                (true, Local::Source(_)) | (false, Local::Destination(_))
            )
        {
            return Err(SftpError::ScopeExpired);
        }
    }
    let mut versions = Vec::with_capacity(items.len());
    for group in items.chunks(super::preflight::MAX_SNAPSHOTS) {
        // Independent snapshots share the existing request admission. Join the
        // entire bounded group even on error: native blocking I/O is not dropped
        // and no grant is published until every preflight succeeds. Preserve
        // input order regardless of reply/completion order.
        versions
            .extend(super::preflight::join_snapshots(group.iter().map(preflight_version)).await?);
    }
    let batch = Arc::new(Batch {
        summary: BatchSummary {
            id: Uuid::new_v4(),
            name: root,
            files: items.len() as u32,
            conflicts: versions.iter().filter(|version| version.is_some()).count() as u32,
        },
        policy: watch::channel(None).0,
    });
    Ok(items
        .into_iter()
        .zip(versions)
        .map(|((context, local), expected)| {
            (
                context,
                local,
                Arc::new(Member {
                    batch: batch.clone(),
                    expected,
                    active: AtomicBool::new(true),
                }),
            )
        })
        .collect())
}

async fn preflight_version(
    (context, local): &FolderItem,
) -> Result<Option<TargetVersion>, SftpError> {
    if context.client.is_closed() {
        return Err(SftpError::ConnectionLost);
    }
    match local {
        Local::Source(_) => match context
            .client
            .lstat(context.path.as_str(), &CancellationToken::new())
            .await
        {
            Ok(attrs) => Ok(Some(TargetVersion::Remote(metadata(attrs)))),
            Err(ClientError::NotFound) => Ok(None),
            Err(error) => Err(map_error(error)),
        },
        Local::Destination(destination) => {
            let destination = destination.clone();
            blocking(move || destination.snapshot())
                .await
                .map(|version| version.map(TargetVersion::Local))
        }
    }
}

fn folder_root(
    context: &TransferContext,
    trusted_download: bool,
) -> Result<(String, bool, RemotePath), SftpError> {
    let (relative, upload) = match &context.target {
        TransferTarget::UploadTree { relative, .. } => (relative, true),
        TransferTarget::DownloadTree { relative, .. } => (relative, false),
        _ => return Err(SftpError::ScopeExpired),
    };
    let (root, rest) = relative.split_once('/').ok_or(SftpError::PathInvalid)?;
    RemoteName::from_bytes(root.as_bytes())?;
    if rest.is_empty() {
        return Err(SftpError::PathInvalid);
    }
    let target_root = match &context.target {
        TransferTarget::UploadTree { parent, .. } => {
            if upload_tree_path(parent, relative)? != context.path {
                return Err(SftpError::ScopeExpired);
            }
            parent.join(&RemoteName::from_bytes(root.as_bytes())?)?
        }
        TransferTarget::DownloadTree { path, .. } => {
            if path != &context.path {
                return Err(SftpError::ScopeExpired);
            }
            if trusted_download {
                return Ok((root.to_owned(), false, RemotePath::parse("/")?));
            }
            let target = path
                .as_str()
                .strip_suffix(&format!("/{rest}"))
                .ok_or(SftpError::PathInvalid)?;
            if target.rsplit('/').next() != Some(root) {
                return Err(SftpError::PathInvalid);
            }
            RemotePath::parse(target)?
        }
        _ => return Err(SftpError::ScopeExpired),
    };
    Ok((root.to_owned(), upload, target_root))
}

impl TransferService {
    /// Capture all existing upload targets before returning native selections.
    /// The usual grant count/lifetime still applies, and each grant binds its
    /// precise tree target so a caller cannot transfer a batch decision elsewhere.
    pub async fn grant_folder_sources(
        self: &Arc<Self>,
        items: Vec<(TransferContext, Arc<dyn LocalSource>)>,
    ) -> Result<Vec<FolderSelection>, SftpError> {
        if items.len() > crate::sftp::MAX_LOCAL_GRANTS {
            return Err(SftpError::ResourceLimit);
        }
        let prepared = prepare(
            items
                .into_iter()
                .map(|(context, source)| (context, Local::Source(source)))
                .collect(),
        )
        .await?;
        let mut state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
        state
            .grants
            .retain(|_, grant| grant.expires > Instant::now());
        if state.grants.len() + prepared.len() > crate::sftp::MAX_LOCAL_GRANTS {
            return Err(SftpError::ResourceLimit);
        }
        if prepared.iter().any(|(context, _, _)| {
            state.closing.contains(&context.binding.window) || context.client.is_closed()
        }) {
            return Err(SftpError::ScopeExpired);
        }
        let mut selections = Vec::with_capacity(prepared.len());
        for (context, local, member) in prepared {
            let TransferTarget::UploadTree { relative, .. } = &context.target else {
                return Err(SftpError::ScopeExpired);
            };
            let token = Uuid::new_v4();
            selections.push(FolderSelection {
                selection: LocalSelection {
                    token,
                    name: relative.clone(),
                },
                relative: relative.clone(),
            });
            state.grants.insert(
                token,
                Grant {
                    binding: context.binding,
                    expires: Instant::now()
                        + Duration::from_secs(crate::sftp::LOCAL_GRANT_LIFETIME_SECONDS),
                    local,
                    batch: Some(member),
                    target: Some(context.target),
                },
            );
        }
        drop(state);
        self.start_reaper();
        Ok(selections)
    }

    /// Download folders are admitted together without turning every file into
    /// an IPC grant. Capacity/preflight failures enqueue no members.
    pub async fn enqueue_folder_destinations(
        self: &Arc<Self>,
        items: Vec<(TransferContext, Arc<dyn LocalDestination>)>,
    ) -> Result<u32, SftpError> {
        let prepared = prepare(
            items
                .into_iter()
                .map(|(context, destination)| (context, Local::Destination(destination)))
                .collect(),
        )
        .await?;
        let tasks = prepared
            .into_iter()
            .map(|(context, local, member)| {
                self.make_task(&context, &local, Some(member))
                    .map(|task| (context, local, task))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
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
}
