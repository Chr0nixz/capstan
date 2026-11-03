//! Bounded, previewed recursive deletion. Paths never arrive from IPC.
use super::*;
use std::{collections::HashSet, sync::atomic::AtomicBool};

#[derive(Clone, PartialEq, Eq)]
struct Node {
    path: RemotePath,
    metadata: Metadata,
}
pub(super) struct DeletePlan {
    binding: Binding,
    session: SftpSessionId,
    expires: Instant,
    nodes: Vec<Node>,
    started: AtomicBool,
    cancel: CancellationToken,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteTreeSummary {
    pub token: Uuid,
    pub host_id: Uuid,
    pub path: RemotePath,
    pub files: usize,
    pub directories: usize,
    pub symlinks: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteOutcome {
    Removed,
    Failed,
    Retained,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteEntryResult {
    pub path: RemotePath,
    pub outcome: DeleteOutcome,
    pub error: Option<SftpError>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteTreeReport {
    pub entries: Vec<DeleteEntryResult>,
}
impl SftpService {
    pub async fn prepare_tree_delete(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        entry: RemoteEntryId,
    ) -> Result<DeleteTreeSummary, SftpError> {
        let view = self.view(window, terminal, session)?;
        let _operation = view.cursor.lock().await;
        self.view(window, terminal, session)?;
        let (path, baseline) = self
            .state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .scopes
            .resolve_entry(session, &view.binding, entry, Instant::now())?;
        if baseline.kind != FileKind::Directory {
            return Err(SftpError::UnsupportedFileType);
        }
        let nodes = scan(&view, &path, &view.cancel).await?;
        if nodes
            .first()
            .is_none_or(|node| !node.metadata.matches_baseline(&baseline))
        {
            return Err(SftpError::TargetConflict);
        }
        self.view(window, terminal, session)?;
        let token = Uuid::new_v4();
        let summary = DeleteTreeSummary {
            token,
            host_id: view.binding.host,
            path,
            files: nodes
                .iter()
                .filter(|node| node.metadata.kind == FileKind::File)
                .count(),
            directories: nodes
                .iter()
                .filter(|node| node.metadata.kind == FileKind::Directory)
                .count(),
            symlinks: nodes
                .iter()
                .filter(|node| node.metadata.kind == FileKind::Symlink)
                .count(),
        };
        let mut state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
        if state.deletion_closing.contains(window) {
            return Err(SftpError::ScopeExpired);
        }
        state.deletions.retain(|_, plan| {
            plan.started.load(Ordering::SeqCst)
                || (plan.expires > Instant::now() && !plan.cancel.is_cancelled())
        });
        if state.deletions.len() >= 8 {
            return Err(SftpError::ResourceLimit);
        }
        state.deletions.insert(
            token,
            Arc::new(DeletePlan {
                binding: view.binding.clone(),
                session,
                expires: Instant::now() + Duration::from_secs(300),
                nodes,
                started: AtomicBool::new(false),
                cancel: view.cancel.child_token(),
            }),
        );
        Ok(summary)
    }
    pub fn cancel_tree_delete(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        token: Uuid,
    ) -> Result<(), SftpError> {
        let view = self.view(window, terminal, session)?;
        let mut state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
        let plan = state
            .deletions
            .get(&token)
            .filter(|plan| plan.binding == view.binding && plan.session == session)
            .ok_or(SftpError::ScopeExpired)?;
        plan.cancel.cancel();
        if !plan.started.load(Ordering::SeqCst) {
            state.deletions.remove(&token);
        }
        Ok(())
    }
    pub async fn apply_tree_delete(
        &self,
        window: &str,
        terminal: &str,
        session: SftpSessionId,
        token: Uuid,
    ) -> Result<DeleteTreeReport, SftpError> {
        let view = self.view(window, terminal, session)?;
        let _operation = view.cursor.lock().await;
        self.view(window, terminal, session)?;
        let plan = {
            let state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
            if state.deletion_closing.contains(window) {
                return Err(SftpError::ScopeExpired);
            }
            let plan = state
                .deletions
                .get(&token)
                .filter(|plan| {
                    plan.binding == view.binding
                        && plan.session == session
                        && plan.expires > Instant::now()
                })
                .cloned()
                .ok_or(SftpError::ScopeExpired)?;
            if plan.started.swap(true, Ordering::SeqCst) {
                return Err(SftpError::ScopeExpired);
            }
            plan
        };
        let result = apply(&view, &plan).await;
        self.state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .deletions
            .remove(&token);
        result
    }
    pub fn has_active_tree_deletes(&self, window: &str) -> Result<bool, SftpError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| SftpError::ConnectionLost)?
            .deletions
            .values()
            .any(|plan| plan.binding.window == window && plan.started.load(Ordering::SeqCst)))
    }
    pub fn abort_tree_delete_exit(&self, window: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.deletion_closing.remove(window);
        }
    }
    pub async fn prepare_tree_delete_exit(&self, window: &str) -> Result<(), SftpError> {
        {
            let mut state = self.state.lock().map_err(|_| SftpError::ConnectionLost)?;
            state.deletion_closing.insert(window.to_owned());
            state.deletions.retain(|_, plan| {
                if plan.binding.window != window {
                    return true;
                }
                plan.cancel.cancel();
                plan.started.load(Ordering::SeqCst)
            });
        }
        let result = tokio::time::timeout(
            Duration::from_secs(super::super::EXIT_DRAIN_TIMEOUT_SECONDS),
            async {
                while self.has_active_tree_deletes(window)? {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Ok(())
            },
        )
        .await
        .unwrap_or(Err(SftpError::ResourceLimit));
        if result.is_err() {
            self.abort_tree_delete_exit(window);
        }
        result
    }
}
fn parent(path: &RemotePath) -> Result<RemotePath, SftpError> {
    let (parent, _) = path
        .as_str()
        .rsplit_once('/')
        .ok_or(SftpError::PathInvalid)?;
    RemotePath::parse(if parent.is_empty() { "/" } else { parent })
}
async fn scan(
    view: &View,
    root: &RemotePath,
    cancel: &CancellationToken,
) -> Result<Vec<Node>, SftpError> {
    let mut pending = vec![(root.clone(), 0)];
    let mut nodes = Vec::new();
    let mut seen = HashSet::new();
    let mut path_bytes = 0;
    while let Some((path, depth)) = pending.pop() {
        if depth >= 64 || !seen.insert(path.clone()) {
            return Err(SftpError::ResourceLimit);
        }
        path_bytes += path.as_str().len();
        if nodes.len() >= 4096 || path_bytes > 256 * 1024 {
            return Err(SftpError::ResourceLimit);
        }
        let current = metadata(
            view.client
                .lstat(path.as_str(), cancel)
                .await
                .map_err(map_error)?,
        );
        if !matches!(
            current.kind,
            FileKind::Directory | FileKind::File | FileKind::Symlink
        ) {
            return Err(SftpError::UnsupportedFileType);
        }
        if nodes.is_empty() && current.kind != FileKind::Directory {
            return Err(SftpError::TargetConflict);
        }
        nodes.push(Node {
            path: path.clone(),
            metadata: current.clone(),
        });
        if current.kind != FileKind::Directory {
            continue;
        }
        if view
            .client
            .realpath(path.as_str(), cancel)
            .await
            .map_err(map_error)?
            != path.as_str()
        {
            return Err(SftpError::TargetConflict);
        }
        let mut directory = view
            .client
            .open_directory(path.as_str(), cancel)
            .await
            .map_err(map_error)?;
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(super::super::REQUEST_TIMEOUT_SECONDS);
        let result = async {
            loop {
                let entries = tokio::time::timeout_at(deadline, directory.next(cancel))
                    .await
                    .map_err(|_| SftpError::ConnectionLost)?
                    .map_err(map_error)?;
                if entries.is_empty() {
                    break;
                }
                for (name, _) in entries {
                    if name == "." || name == ".." {
                        continue;
                    }
                    let child = path.join(&RemoteName::from_library_string(&name)?)?;
                    pending.push((child, depth + 1));
                    if pending.len() + nodes.len() > 4096 {
                        return Err(SftpError::ResourceLimit);
                    }
                }
            }
            Ok::<(), SftpError>(())
        }
        .await;
        let closed = directory.close(&CancellationToken::new()).await;
        result?;
        closed.map_err(map_error)?;
        if !metadata(
            view.client
                .lstat(path.as_str(), cancel)
                .await
                .map_err(map_error)?,
        )
        .matches_baseline(&current)
        {
            return Err(SftpError::TargetConflict);
        }
    }
    // Preserve the root at index zero while making comparison deterministic.
    nodes[1..].sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
    Ok(nodes)
}
async fn apply(view: &View, plan: &DeletePlan) -> Result<DeleteTreeReport, SftpError> {
    let root = &plan.nodes.first().ok_or(SftpError::ScopeExpired)?.path;
    if scan(view, root, &plan.cancel).await? != plan.nodes {
        return Err(SftpError::TargetConflict);
    }
    let directories: HashMap<_, _> = plan
        .nodes
        .iter()
        .filter(|node| node.metadata.kind == FileKind::Directory)
        .map(|node| (node.path.clone(), &node.metadata))
        .collect();
    let mut ordered: Vec<_> = plan.nodes.iter().collect();
    ordered.sort_by(|a, b| {
        b.path
            .as_str()
            .matches('/')
            .count()
            .cmp(&a.path.as_str().matches('/').count())
            .then_with(|| a.path.as_str().cmp(b.path.as_str()))
    });
    let mut blocked = HashSet::new();
    let mut entries = Vec::with_capacity(ordered.len());
    for node in ordered {
        let skipped = if plan.cancel.is_cancelled() {
            Some(SftpError::OperationCanceled)
        } else if blocked.contains(&node.path) {
            Some(SftpError::TargetConflict)
        } else {
            None
        };
        let result = if let Some(error) = skipped {
            Err(error)
        } else {
            remove(view, plan, node, &directories).await
        };
        let outcome = match &result {
            Ok(()) => DeleteOutcome::Removed,
            Err(_) if skipped.is_some() => DeleteOutcome::Retained,
            Err(_) => DeleteOutcome::Failed,
        };
        if result.is_err() {
            let mut ancestor = parent(&node.path)?;
            while ancestor.as_str() == root.as_str()
                || ancestor
                    .as_str()
                    .starts_with(&format!("{}/", root.as_str()))
            {
                blocked.insert(ancestor.clone());
                ancestor = parent(&ancestor)?;
            }
        }
        entries.push(DeleteEntryResult {
            path: node.path.clone(),
            outcome,
            error: result.err(),
        });
    }
    Ok(DeleteTreeReport { entries })
}
async fn remove(
    view: &View,
    plan: &DeletePlan,
    node: &Node,
    directories: &HashMap<RemotePath, &Metadata>,
) -> Result<(), SftpError> {
    let mut ancestor = parent(&node.path)?;
    loop {
        let current = metadata(
            view.client
                .lstat(ancestor.as_str(), &plan.cancel)
                .await
                .map_err(map_error)?,
        );
        if current.kind != FileKind::Directory
            || directories
                .get(&ancestor)
                .is_some_and(|expected| current.permissions != expected.permissions)
            || view
                .client
                .realpath(ancestor.as_str(), &plan.cancel)
                .await
                .map_err(map_error)?
                != ancestor.as_str()
        {
            return Err(SftpError::TargetConflict);
        }
        if !directories.contains_key(&ancestor) {
            break;
        }
        ancestor = parent(&ancestor)?;
    }
    let current = metadata(
        view.client
            .lstat(node.path.as_str(), &plan.cancel)
            .await
            .map_err(map_error)?,
    );
    if (node.metadata.kind == FileKind::Directory
        && (current.kind != FileKind::Directory
            || current.permissions != node.metadata.permissions))
        || (node.metadata.kind != FileKind::Directory && !current.matches_baseline(&node.metadata))
    {
        return Err(SftpError::TargetConflict);
    }
    if plan.cancel.is_cancelled() {
        return Err(SftpError::OperationCanceled);
    }
    // Once unlink/rmdir is admitted, await its real ACK. Cancellation stops the
    // next item; it never labels an already removed item as retained/replayable.
    let admitted = CancellationToken::new();
    if current.kind == FileKind::Directory {
        view.client
            .remove_empty_directory(node.path.as_str(), &admitted)
            .await
            .map_err(map_error)
    } else {
        view.client
            .remove_file(node.path.as_str(), &admitted)
            .await
            .map_err(map_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_scope_and_partial_result_contract() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../docs/agent/fixtures/M2-delete.json"
        ))
        .unwrap();
        let summary = DeleteTreeSummary {
            token: "00000000-0000-4000-8000-000000000090".parse().unwrap(),
            host_id: "00000000-0000-4000-8000-000000000002".parse().unwrap(),
            path: RemotePath::parse("/files/tree").unwrap(),
            files: 1,
            directories: 1,
            symlinks: 1,
        };
        assert_eq!(serde_json::to_value(summary).unwrap(), fixture["summary"]);
        let report = DeleteTreeReport {
            entries: vec![
                DeleteEntryResult {
                    path: RemotePath::parse("/files/tree/link").unwrap(),
                    outcome: DeleteOutcome::Removed,
                    error: None,
                },
                DeleteEntryResult {
                    path: RemotePath::parse("/files/tree/denied").unwrap(),
                    outcome: DeleteOutcome::Failed,
                    error: Some(SftpError::PermissionDenied),
                },
                DeleteEntryResult {
                    path: RemotePath::parse("/files/tree").unwrap(),
                    outcome: DeleteOutcome::Retained,
                    error: Some(SftpError::TargetConflict),
                },
            ],
        };
        assert_eq!(serde_json::to_value(report).unwrap(), fixture["report"]);
    }
}
