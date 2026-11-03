use super::{local::blocking, *};
use crate::sftp::{
    service::{map_error, metadata},
    FileKind, RemoteName,
};
use capstan_ssh::sftp::{ClientError, RemoteFile};
use sha2::{Digest, Sha256};

enum CleanupOutcome {
    Complete,
    Retained,
    Failed(Option<Metadata>),
    Unverified,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum TargetVersion {
    Remote(Metadata),
    Local(LocalVersion),
}
impl TargetVersion {
    fn metadata(&self) -> Metadata {
        match self {
            Self::Remote(value) => value.clone(),
            Self::Local(value) => Metadata {
                kind: if value.attributes & 0x10 != 0 {
                    FileKind::Directory
                } else {
                    FileKind::File
                },
                size: Some(ExactU64(value.size)),
                // Windows FILETIME uses 100 ns ticks since 1601.
                modified_seconds: Some(ExactU64(
                    (value.modified / 10_000_000).saturating_sub(11_644_473_600),
                )),
                permissions: None,
            },
        }
    }
}

struct Job {
    task: Arc<Task>,
    context: TransferContext,
    local: Local,
    decisions: mpsc::Receiver<Decision>,
    temporary: Option<RemotePath>,
    local_partial: Option<std::path::PathBuf>,
    created: bool,
    cleanup_baseline: Option<Metadata>,
    verified: Option<(Metadata, Vec<u8>)>,
    partial: Option<PartialState>,
    commits: Arc<tokio::sync::Mutex<()>>,
    budget: Arc<budget::Budget>,
}
impl Job {
    fn check(&self) -> Result<(), SftpError> {
        self.task.check()?;
        if self.context.client.is_closed() {
            Err(SftpError::ConnectionLost)
        } else {
            Ok(())
        }
    }
    async fn target(&self) -> Result<Option<TargetVersion>, SftpError> {
        self.check()?;
        match &self.local {
            Local::Source(_) => match self
                .context
                .client
                .lstat(self.context.path.as_str(), &self.task.cancel)
                .await
            {
                Ok(attrs) => {
                    let attrs = metadata(attrs);
                    Ok(Some(TargetVersion::Remote(attrs)))
                }
                Err(ClientError::NotFound) => Ok(None),
                Err(error) => Err(map_error(error)),
            },
            Local::Destination(destination) => {
                let destination = destination.clone();
                blocking(move || destination.snapshot())
                    .await
                    .map(|value| value.map(TargetVersion::Local))
            }
        }
    }
    async fn rename(&mut self, name: &str) -> Result<(), SftpError> {
        let name = RemoteName::from_bytes(name.as_bytes())?;
        match &self.local {
            Local::Source(_) => {
                self.context.path = parent(&self.context.path)?.join(&name)?;
                match &mut self.context.target {
                    TransferTarget::Upload {
                        name: target_name, ..
                    } => {
                        *target_name = name.as_str().to_owned();
                    }
                    TransferTarget::UploadTree { relative, .. } => {
                        let (parent, _) = relative.rsplit_once('/').unwrap_or(("", relative));
                        *relative = if parent.is_empty() {
                            name.as_str().to_owned()
                        } else {
                            format!("{parent}/{}", name.as_str())
                        };
                    }
                    _ => return Err(SftpError::ScopeExpired),
                }
                self.task.retarget(&self.context)?;
                self.task.update(|status| {
                    status.name = name.as_str().to_owned();
                    status.remote_path = self.context.path.clone();
                })?;
            }
            Local::Destination(destination) => {
                let destination = destination.clone();
                let value = name.as_str().to_owned();
                blocking(move || destination.retarget(&value)).await?;
                self.task
                    .update(|status| status.name = name.as_str().to_owned())?;
            }
        }
        Ok(())
    }
    async fn approve(&mut self, force: bool) -> Result<Option<TargetVersion>, SftpError> {
        let current = self.target().await?;
        self.approve_snapshot(force, current).await
    }
    async fn approve_snapshot(
        &mut self,
        force: bool,
        mut current: Option<TargetVersion>,
    ) -> Result<Option<TargetVersion>, SftpError> {
        if force {
            if let Some(member) = &self.task.batch {
                member.active.store(false, Ordering::SeqCst);
            }
        }
        let mut batch_updates = self
            .task
            .batch
            .as_ref()
            .map(|member| member.batch.policy.subscribe());
        let mut ask = force
            || current.is_some()
            || self.task.batch.as_ref().is_some_and(|member| {
                member.active.load(Ordering::SeqCst) && current != member.expected
            });
        loop {
            if !ask {
                if let Some(member) = &self.task.batch {
                    member.active.store(false, Ordering::SeqCst);
                }
                return Ok(current);
            }
            self.check()?;
            let replace_allowed = current.as_ref().is_none_or(|target| {
                let metadata = target.metadata();
                metadata.kind == FileKind::File
                    && metadata.size.is_some()
                    && metadata.modified_seconds.is_some()
            }) && (current.is_none()
                || matches!(self.local, Local::Destination(_))
                || self.context.client.capabilities().atomic_replace);
            let batch_eligible = self.task.batch.as_ref().is_some_and(|member| {
                member.active.load(Ordering::SeqCst)
                    && current.is_some()
                    && current == member.expected
            });
            let policy = batch_updates
                .as_mut()
                .and_then(|updates| *updates.borrow_and_update());
            if batch_eligible
                && (!matches!(policy, Some(batch::BatchChoice::Replace)) || replace_allowed)
            {
                if let Some(policy) = policy {
                    // Every member rechecks its own preflight version. A policy
                    // never adopts an appeared, disappeared or changed target.
                    let latest = self.target().await?;
                    if let Some(member) = &self.task.batch {
                        member.active.store(false, Ordering::SeqCst);
                    }
                    if current == latest {
                        return match policy {
                            batch::BatchChoice::Skip => Err(SftpError::OperationCanceled),
                            batch::BatchChoice::Replace if replace_allowed => Ok(current),
                            batch::BatchChoice::Replace => Err(SftpError::UnsupportedFileType),
                        };
                    }
                    current = latest;
                    continue;
                }
            } else if let Some(member) = &self.task.batch {
                // A member cannot regain group eligibility after it changes.
                member.active.store(false, Ordering::SeqCst);
            }
            self.task.update(|status| {
                status.state = TransferState::AwaitingConflict;
                status.conflict = Some(Conflict {
                    revision: ExactU64(status.version.0 + 1),
                    target: current.as_ref().map(TargetVersion::metadata),
                    replace_allowed,
                    batch_eligible: batch_eligible && policy.is_none(),
                });
            })?;
            let decision = tokio::select! {
                biased;
                () = self.task.cancel.cancelled() => return Err(SftpError::OperationCanceled),
                () = self.context.client.closed() => return Err(SftpError::ConnectionLost),
                decision = self.decisions.recv() => decision.ok_or(SftpError::OperationCanceled)?,
                result = async {
                    match &mut batch_updates {
                        Some(updates) => updates.changed().await.map_err(|_| SftpError::ScopeExpired),
                        None => std::future::pending().await,
                    }
                } => { result?; continue; },
            };
            match decision {
                Decision::Skip => return Err(SftpError::OperationCanceled),
                Decision::Rename { name } => {
                    self.rename(&name).await?;
                    current = self.target().await?;
                    ask = current.is_some();
                }
                Decision::Replace if replace_allowed => {
                    let latest = self.target().await?;
                    if current == latest {
                        return Ok(current);
                    }
                    current = latest;
                    ask = true;
                }
                Decision::Replace => return Err(SftpError::UnsupportedFileType),
                Decision::ReplaceBatch | Decision::SkipBatch => {
                    return Err(SftpError::ScopeExpired)
                }
            }
        }
    }
    async fn check_parent(&self) -> Result<(), SftpError> {
        let parent = parent(&self.context.path)?;
        let mut ancestors = Vec::new();
        if let TransferTarget::UploadTree { parent: root, .. } = &self.context.target {
            let relative_parent = parent
                .as_str()
                .strip_prefix(root.as_str())
                .unwrap_or_default()
                .trim_start_matches('/');
            let mut current = root.clone();
            for component in relative_parent
                .split('/')
                .filter(|component| !component.is_empty())
            {
                current = current.join(&RemoteName::from_bytes(component.as_bytes())?)?;
                ancestors.push(current.clone());
            }
        }
        // Canonicalize first: never create through a redirected existing parent.
        // Every ancestor type is still checked before staging/publication; final
        // checks remain under the original global publication gate.
        match self
            .context
            .client
            .realpath(parent.as_str(), &self.task.cancel)
            .await
        {
            Ok(resolved) if resolved != parent.as_str() => Err(SftpError::TargetConflict),
            Ok(_) => {
                for group in ancestors.chunks(super::preflight::MAX_SNAPSHOTS) {
                    let present =
                        super::preflight::join_snapshots(group.iter().map(|path| async move {
                            match self
                                .context
                                .client
                                .lstat(path.as_str(), &self.task.cancel)
                                .await
                            {
                                Ok(attrs) => {
                                    if metadata(attrs).kind == FileKind::Directory {
                                        Ok(true)
                                    } else {
                                        Err(SftpError::TargetConflict)
                                    }
                                }
                                Err(ClientError::NotFound) => Ok(false),
                                Err(error) => Err(map_error(error)),
                            }
                        }))
                        .await?;
                    if present.iter().any(|exists| !exists) {
                        return self.create_missing_tree_parents(&parent, &ancestors).await;
                    }
                }
                Ok(())
            }
            Err(ClientError::NotFound) if !ancestors.is_empty() => {
                self.create_missing_tree_parents(&parent, &ancestors).await
            }
            Err(error) => Err(map_error(error)),
        }
    }
    async fn create_missing_tree_parents(
        &self,
        parent: &RemotePath,
        ancestors: &[RemotePath],
    ) -> Result<(), SftpError> {
        // Creation retains the original ancestor-ordered validation sequence.
        // Recheck all ancestors after a missing-directory/race observation.
        for path in ancestors {
            match self
                .context
                .client
                .lstat(path.as_str(), &self.task.cancel)
                .await
            {
                Ok(attrs) => {
                    if metadata(attrs).kind != FileKind::Directory {
                        return Err(SftpError::TargetConflict);
                    }
                }
                Err(ClientError::NotFound) => self
                    .context
                    .client
                    .mkdir(path.as_str(), &self.task.cancel)
                    .await
                    .map_err(map_error)?,
                Err(error) => return Err(map_error(error)),
            }
        }
        let resolved = self
            .context
            .client
            .realpath(parent.as_str(), &self.task.cancel)
            .await
            .map_err(map_error)?;
        if resolved != parent.as_str() {
            return Err(SftpError::TargetConflict);
        }
        Ok(())
    }
    async fn execute(&mut self) -> Result<(), SftpError> {
        self.task.phase(TransferState::Preparing)?;
        // Both reads are independent. Join admitted native snapshots even if
        // parent validation fails; no decision or write precedes validation.
        let (parent, target) = tokio::join!(self.check_parent(), self.target());
        parent?;
        let mut approved = self.approve_snapshot(false, target?).await?;
        self.task.phase(TransferState::Running)?;
        match &self.local {
            Local::Source(source) => {
                let source = source.clone();
                blocking(move || source.verify()).await?;
                self.upload().await?;
            }
            Local::Destination(_) => self.download().await?,
        }
        loop {
            self.check()?;
            // Stage and verify concurrently, but serialize final target checks
            // and publication across all app jobs, including local aliases.
            // Never hold this gate while waiting for a conflict decision.
            let commits = self.commits.clone();
            let commit = tokio::select! {
                biased;
                () = self.task.cancel.cancelled() => return Err(SftpError::OperationCanceled),
                () = self.context.client.closed() => return Err(SftpError::ConnectionLost),
                commit = commits.lock() => commit,
            };
            // Keep every final check under the existing global publication
            // gate. Only independent reads overlap; a failed check still
            // blocks publication and admitted native I/O is fully joined.
            let (parent, target, temporary) = tokio::join!(
                self.check_parent(),
                self.target(),
                self.temporary_snapshot(),
            );
            parent?;
            let target = target?;
            let temporary_snapshot = temporary?;
            if target != approved {
                drop(commit);
                approved = self.approve(true).await?;
                continue;
            }
            self.task.phase(TransferState::Committing)?;
            match &self.local {
                Local::Source(_) => {
                    let temporary = self.temporary.as_ref().ok_or(SftpError::RemoteIo)?;
                    let baseline = &self.verified.as_ref().ok_or(SftpError::RemoteIo)?.0;
                    let current = temporary_snapshot.ok_or(SftpError::RemoteIo)?;
                    if !current.matches_baseline(baseline) {
                        return Err(SftpError::TargetConflict);
                    }
                    // Do not cancel an admitted commit: its actual result is the
                    // only authority for whether the target was replaced.
                    let result = crate::sftp::publish_remote_file(
                        &self.context.client,
                        temporary,
                        &self.context.path,
                        approved.is_some(),
                    )
                    .await;
                    match result {
                        Ok(()) => {}
                        Err(ClientError::Uncertain) => self.reconcile_commit().await?,
                        Err(error) => return Err(map_error(error)),
                    }
                    self.temporary = None;
                    return Ok(());
                }
                Local::Destination(destination) => {
                    let expected = match approved {
                        Some(TargetVersion::Local(value)) => Some(value),
                        None => None,
                        _ => return Err(SftpError::LocalIo),
                    };
                    let destination = destination.clone();
                    match blocking(move || destination.commit(expected)).await {
                        // Native I/O rechecks the precise file identity, not
                        // only the display metadata, immediately before rename.
                        Err(SftpError::TargetConflict) => {
                            drop(commit);
                            approved = self.approve(true).await?;
                        }
                        result => return result,
                    }
                }
            }
        }
    }
    async fn temporary_snapshot(&self) -> Result<Option<Metadata>, SftpError> {
        if matches!(self.local, Local::Destination(_)) {
            return Ok(None);
        }
        let temporary = self.temporary.as_ref().ok_or(SftpError::RemoteIo)?;
        self.context
            .client
            .lstat(temporary.as_str(), &CancellationToken::new())
            .await
            .map(metadata)
            .map(Some)
            .map_err(map_error)
    }
    /// A lost rename ACK is never replayed. Open a new SFTP channel on the
    /// same authenticated transport and verify the formal file's full digest
    /// and attributes, plus disappearance of the owned temporary name.
    async fn reconcile_commit(&self) -> Result<(), SftpError> {
        let cancel = CancellationToken::new();
        let client = self
            .context
            .lease
            .open_sftp(&cancel)
            .await
            .map_err(map_error)?;
        let temporary = self.temporary.as_ref().ok_or(SftpError::RemoteIo)?;
        let (baseline, expected) = self.verified.as_ref().ok_or(SftpError::RemoteIo)?;
        match client.lstat(temporary.as_str(), &cancel).await {
            Err(ClientError::NotFound) => {}
            _ => return Err(SftpError::RemoteIo),
        }
        let current = metadata(
            client
                .lstat(self.context.path.as_str(), &cancel)
                .await
                .map_err(map_error)?,
        );
        if !current.matches_baseline(baseline) {
            return Err(SftpError::RemoteIo);
        }
        let size = current.size.ok_or(SftpError::RemoteIo)?.0;
        let file = client
            .open_file(self.context.path.as_str(), false, &cancel)
            .await
            .map_err(map_error)?;
        let actual = hash_remote(file, size, &client, &self.budget, &cancel).await;
        if actual? != *expected {
            return Err(SftpError::RemoteIo);
        }
        let after = metadata(
            client
                .lstat(self.context.path.as_str(), &cancel)
                .await
                .map_err(map_error)?,
        );
        if !after.matches_baseline(&current) {
            return Err(SftpError::RemoteIo);
        }
        Ok(())
    }
    async fn upload(&mut self) -> Result<(), SftpError> {
        let Local::Source(source) = &self.local else {
            return Err(SftpError::LocalIo);
        };
        let source = source.clone();
        let size = source.size();
        let (temporary, offset, mut digest, resuming) = match self.partial.clone() {
            Some(PartialState::Upload { path }) => {
                let expected_parent = parent(&self.context.path)?;
                if parent(&path)? != expected_parent
                    || !path
                        .as_str()
                        .rsplit('/')
                        .next()
                        .is_some_and(|name| name.starts_with(".capstan-"))
                {
                    return Err(SftpError::ResumeMismatch);
                }
                let attrs = metadata(
                    match self
                        .context
                        .client
                        .lstat(path.as_str(), &self.task.cancel)
                        .await
                    {
                        Ok(attrs) => attrs,
                        Err(ClientError::NotFound) => return Err(SftpError::ResumeMismatch),
                        Err(error) => return Err(map_error(error)),
                    },
                );
                let offset = attrs.size.ok_or(SftpError::UnsupportedFileType)?.0;
                if attrs.kind != FileKind::File || offset > size {
                    return Err(SftpError::ResumeMismatch);
                }
                let digest = verify_upload_prefix(
                    &source,
                    &self.context.client,
                    &path,
                    offset,
                    &self.budget,
                    &self.task.cancel,
                )
                .await?;
                (path, offset, digest, true)
            }
            Some(PartialState::Download { .. }) => return Err(SftpError::ScopeExpired),
            None => {
                let id = self
                    .task
                    .status
                    .lock()
                    .map_err(|_| SftpError::ConnectionLost)?
                    .id;
                let path = parent(&self.context.path)?.join(&RemoteName::from_bytes(
                    format!(".capstan-{id}.partial").as_bytes(),
                )?)?;
                (path, 0, Sha256::new(), false)
            }
        };
        self.temporary = Some(temporary.clone());
        let window = reserve_window(&self.budget, &self.context.client, &self.task.cancel).await?;
        let task = self.task.clone();
        let context = self.context.clone();
        let checkpoint_path = temporary.clone();
        // Record the deterministic name before OPEN. A canceled caller may
        // lose its HANDLE reply while the independent drainer closes it.
        blocking(move || {
            task.checkpoint_recovery(&context, Some(checkpoint_path), None, None, None)
        })
        .await?;
        let file = match if !resuming {
            self.context
                .client
                .open_file(temporary.as_str(), true, &self.task.cancel)
                .await
        } else {
            self.context
                .client
                .open_existing_file(temporary.as_str(), &self.task.cancel)
                .await
        } {
            Ok(file) => file,
            Err(error) => {
                // A rejected exclusive OPEN created nothing. Only an
                // uncertain reply can leave a residual without a handle.
                if error != ClientError::Uncertain {
                    self.temporary = None;
                }
                return Err(map_error(error));
            }
        };
        self.created = true;
        let task = self.task.clone();
        let context = self.context.clone();
        let checkpoint_path = temporary.clone();
        blocking(move || {
            task.checkpoint_recovery(&context, Some(checkpoint_path), None, None, None)
        })
        .await?;
        let mut file = file.pipeline(&self.task.cancel).await.map_err(map_error)?;
        file.retain_payload_guard(Box::new(window));
        if offset > 0 {
            file.seek_to(offset, &self.task.cancel)
                .await
                .map_err(map_error)?;
            self.task.progress(offset)?;
        }
        let result = async {
            let mut offset = offset;
            while offset < size {
                self.task.wait_if_paused().await?;
                self.check()?;
                let count = (size - offset)
                    .min(self.context.client.capabilities().request_bytes as u64)
                    as usize;
                let reservation =
                    reserve(&self.budget, &self.context.client, count, &self.task.cancel).await?;
                let source = source.clone();
                let data = blocking(move || reservation.wrap(source.read(offset, count)?)).await?;
                if data.is_empty() || data.len() > count {
                    return Err(SftpError::LocalIo);
                }
                let confirmed = file
                    .write(&data, &self.task.cancel)
                    .await
                    .map_err(map_error)?;
                digest.update(&data);
                offset += data.len() as u64;
                self.task.progress(confirmed)?;
                tokio::task::yield_now().await;
            }
            file.drain(&self.task.cancel).await.map_err(map_error)?;
            self.task.progress(size)?;
            let source = source.clone();
            blocking(move || source.verify()).await?;
            let attrs = metadata(file.metadata(&self.task.cancel).await.map_err(map_error)?);
            if attrs.kind != FileKind::File || attrs.size != Some(ExactU64(size)) {
                return Err(SftpError::RemoteIo);
            }
            if self.context.client.capabilities().fsync {
                file.sync(&self.task.cancel).await.map_err(map_error)?;
            }
            Ok::<_, SftpError>(digest.finalize().to_vec())
        }
        .await;
        let failed = result.is_err();
        let drained = if failed {
            Ok(())
        } else {
            file.drain(&CancellationToken::new())
                .await
                .map_err(map_error)
        };
        if !self.context.client.is_closed() && !self.task.cancel.is_cancelled() {
            // Capture the owned file's last attributes through its handle,
            // including after a known write failure. A pathname alone cannot
            // establish ownership when another client replaces the temporary.
            self.cleanup_baseline = file
                .metadata(&CancellationToken::new())
                .await
                .ok()
                .map(metadata);
        }
        let closed = if failed {
            Ok(())
        } else {
            file.close(&CancellationToken::new())
                .await
                .map_err(map_error)
        };
        let expected = result?;
        drained?;
        closed?;
        self.task.phase(TransferState::Verifying)?;
        let source_attrs = metadata(
            self.context
                .client
                .lstat(temporary.as_str(), &self.task.cancel)
                .await
                .map_err(map_error)?,
        );
        if source_attrs.kind != FileKind::File || source_attrs.size != Some(ExactU64(source.size()))
        {
            return Err(SftpError::RemoteIo);
        }
        let file = self
            .context
            .client
            .open_file(temporary.as_str(), false, &self.task.cancel)
            .await
            .map_err(map_error)?;
        let actual = hash_remote(
            file,
            source.size(),
            &self.context.client,
            &self.budget,
            &self.task.cancel,
        )
        .await;
        if actual? != expected {
            return Err(SftpError::RemoteIo);
        }
        let after = metadata(
            self.context
                .client
                .lstat(temporary.as_str(), &self.task.cancel)
                .await
                .map_err(map_error)?,
        );
        if !after.matches_baseline(&source_attrs) {
            return Err(SftpError::TargetConflict);
        }
        self.verified = Some((after, expected));
        Ok(())
    }
    async fn download(&mut self) -> Result<(), SftpError> {
        let Local::Destination(destination) = &self.local else {
            return Err(SftpError::LocalIo);
        };
        let destination = destination.clone();
        let baseline = self
            .context
            .baseline
            .as_ref()
            .ok_or(SftpError::UnsupportedFileType)?
            .clone();
        let current = metadata(
            self.context
                .client
                .lstat(self.context.path.as_str(), &self.task.cancel)
                .await
                .map_err(map_error)?,
        );
        if !current.matches_baseline(&baseline) {
            return Err(SftpError::TargetConflict);
        }
        let size = baseline.size.ok_or(SftpError::UnsupportedFileType)?.0;
        let existing = blocking({
            let destination = destination.clone();
            move || destination.partial_size()
        })
        .await?;
        let offset = match self.partial.clone() {
            Some(PartialState::Download { size: expected, .. }) => {
                if existing != Some(expected) || expected > size {
                    return Err(SftpError::ResumeMismatch);
                }
                expected
            }
            Some(PartialState::Upload { .. }) => return Err(SftpError::ScopeExpired),
            None => {
                if existing.is_some_and(|value| value > 0) {
                    return Err(SftpError::TargetConflict);
                }
                0
            }
        };
        let mut digest = if offset > 0 {
            verify_download_prefix(
                &destination,
                &self.context.client,
                &self.context.path,
                offset,
                &self.budget,
                &self.task.cancel,
            )
            .await?
        } else {
            Sha256::new()
        };
        let window = reserve_window(&self.budget, &self.context.client, &self.task.cancel).await?;
        let file = self
            .context
            .client
            .open_file(self.context.path.as_str(), false, &self.task.cancel)
            .await
            .map_err(map_error)?;
        let mut file = file.pipeline(&self.task.cancel).await.map_err(map_error)?;
        file.retain_payload_guard(Box::new(window));
        let result = async {
            let current = metadata(file.metadata(&self.task.cancel).await.map_err(map_error)?);
            if !current.matches_baseline(&baseline) {
                return Err(SftpError::TargetConflict);
            }
            if existing.is_none() {
                let id = self
                    .task
                    .status
                    .lock()
                    .map_err(|_| SftpError::ConnectionLost)?
                    .id;
                let name = format!(".capstan-{id}.partial");
                let output = destination.clone();
                let selected_name = name.clone();
                blocking(move || output.begin(&selected_name)).await?;
                let output = destination.clone();
                self.local_partial = blocking(move || output.partial_path()).await?;
                let output = destination.clone();
                let local_target = blocking(move || output.target_path()).await?;
                let task = self.task.clone();
                let context = self.context.clone();
                let local_partial = self.local_partial.clone();
                blocking(move || {
                    task.checkpoint_recovery(&context, None, local_partial, local_target, None)
                })
                .await?;
            } else {
                file.seek_to(offset, &self.task.cancel)
                    .await
                    .map_err(map_error)?;
                self.task.progress(offset)?;
            }
            let mut offset = offset;
            while offset < size {
                self.task.wait_if_paused().await?;
                self.check()?;
                let count = (size - offset)
                    .min(self.context.client.capabilities().request_bytes as u64)
                    as usize;
                let reservation =
                    reserve(&self.budget, &self.context.client, count, &self.task.cancel).await?;
                let data = reservation.wrap(
                    file.read(count, &self.task.cancel)
                        .await
                        .map_err(map_error)?,
                )?;
                if data.is_empty() {
                    return Err(SftpError::RemoteIo);
                }
                digest.update(&data);
                let length = data.len();
                let output = destination.clone();
                blocking(move || output.write(offset, &data)).await?;
                offset += length as u64;
                self.task.progress(offset)?;
                tokio::task::yield_now().await;
            }
            let reservation =
                reserve(&self.budget, &self.context.client, 1, &self.task.cancel).await?;
            let tail =
                reservation.wrap(file.read(1, &self.task.cancel).await.map_err(map_error)?)?;
            if !tail.is_empty() {
                return Err(SftpError::TargetConflict);
            }
            let current = metadata(file.metadata(&self.task.cancel).await.map_err(map_error)?);
            if !current.matches_baseline(&baseline) {
                return Err(SftpError::TargetConflict);
            }
            Ok::<_, SftpError>(digest.finalize().to_vec())
        }
        .await;
        let failed = result.is_err();
        let closed = if failed {
            Ok(())
        } else {
            file.close(&CancellationToken::new())
                .await
                .map_err(map_error)
        };
        let expected = result?;
        closed?;
        self.task.phase(TransferState::Verifying)?;
        let output = destination.clone();
        blocking(move || output.sync()).await?;
        let mut actual = Sha256::new();
        let mut offset = 0;
        while offset < size {
            self.task.wait_if_paused().await?;
            self.check()?;
            let output = destination.clone();
            let count = (size - offset).min(crate::sftp::REQUEST_BYTES as u64) as usize;
            let reservation =
                reserve(&self.budget, &self.context.client, count, &self.task.cancel).await?;
            let data = blocking(move || reservation.wrap(output.read(offset, count)?)).await?;
            if data.is_empty() || data.len() > count {
                return Err(SftpError::LocalIo);
            }
            offset += data.len() as u64;
            actual.update(data);
        }
        if actual.finalize().as_slice() != expected {
            return Err(SftpError::LocalIo);
        }
        let current = metadata(
            self.context
                .client
                .lstat(self.context.path.as_str(), &self.task.cancel)
                .await
                .map_err(map_error)?,
        );
        if !current.matches_baseline(&baseline) {
            return Err(SftpError::TargetConflict);
        }
        Ok(())
    }
    async fn cleanup(&mut self) -> CleanupOutcome {
        match &self.local {
            Local::Destination(destination) => {
                if (self.task.cancel.is_cancelled()
                    || self.context.client.is_closed()
                    || self.partial.is_some())
                    && blocking({
                        let destination = destination.clone();
                        move || destination.partial_size()
                    })
                    .await
                    .is_ok_and(|size| size.is_some())
                {
                    // Keep a user-owned local prefix for an explicit retry.
                    // It is verified against the remote prefix before reuse.
                    return CleanupOutcome::Retained;
                }
                let destination = destination.clone();
                match blocking(move || destination.discard()).await {
                    Ok(()) => CleanupOutcome::Complete,
                    Err(_) => CleanupOutcome::Failed(None),
                }
            }
            Local::Source(_) => {
                if let Some(temporary) = &self.temporary {
                    // An uncertain OPEN/WRITE/RENAME closes SFTP. Keep the
                    // residual for explicit recovery, never replay a mutation.
                    if !self.created {
                        return CleanupOutcome::Unverified;
                    }
                    if self.context.client.is_closed()
                        || self.task.cancel.is_cancelled()
                        || self.partial.is_some()
                    {
                        // The temporary is retained for a prefix-verified
                        // continuation. No pathname cleanup is attempted
                        // after an uncertain mutation or a canceled request.
                        return CleanupOutcome::Retained;
                    }
                    let cancel = CancellationToken::new();
                    let current = match self.context.client.lstat(temporary.as_str(), &cancel).await
                    {
                        Ok(attrs) => metadata(attrs),
                        Err(ClientError::NotFound) => return CleanupOutcome::Complete,
                        Err(_) => return CleanupOutcome::Unverified,
                    };
                    if !self
                        .cleanup_baseline
                        .as_ref()
                        .is_some_and(|baseline| current.matches_baseline(baseline))
                    {
                        return CleanupOutcome::Unverified;
                    }
                    // SFTP v3 has no compare-and-delete; the attribute check
                    // narrows, but cannot eliminate, external replacement races.
                    match self
                        .context
                        .client
                        .remove_file(temporary.as_str(), &cancel)
                        .await
                    {
                        Ok(()) | Err(ClientError::NotFound) => CleanupOutcome::Complete,
                        Err(_) => CleanupOutcome::Failed(Some(current)),
                    }
                } else {
                    if self.partial.is_some() {
                        CleanupOutcome::Retained
                    } else {
                        CleanupOutcome::Complete
                    }
                }
            }
        }
    }
    async fn record_cleanup(&self, outcome: &CleanupOutcome) -> Result<(), SftpError> {
        use super::recovery::RecoveryMode;
        if let Local::Destination(destination) = &self.local {
            let destination = destination.clone();
            blocking(move || destination.preserve_partial()).await?;
        }
        let Some(store) = self.task.recovery.clone() else {
            return Ok(());
        };
        let id = self.task.status.lock().map_err(|_| SftpError::LocalIo)?.id;
        let local = self.local.clone();
        let baseline = match outcome {
            CleanupOutcome::Failed(baseline) => baseline.clone(),
            _ => None,
        };
        let unverified = matches!(outcome, CleanupOutcome::Unverified);
        blocking(move || {
            let mut record = store.record(id)?;
            record.mode = RecoveryMode::Inspect;
            if !unverified {
                match local {
                    Local::Source(_) => {
                        if let Some(baseline) = baseline.filter(|value| {
                            value.kind == FileKind::File
                                && value.size.is_some()
                                && value.modified_seconds.is_some()
                        }) {
                            record.partial_size = baseline.size.map(|size| size.0);
                            record.remote_cleanup_baseline = Some(baseline);
                            record.mode = RecoveryMode::Cleanup;
                        }
                    }
                    Local::Destination(destination) => {
                        // Preserve the pinned file before releasing the failed job's
                        // handle. A fresh picker must prove the same identity later.
                        let size = destination.partial_size().ok().flatten();
                        if record.local_identity.is_some()
                            && destination.partial_identity().ok().flatten()
                                == record.local_identity
                            && destination.partial_path().ok().flatten() == record.local_partial
                            && size.is_some_and(|size| size <= record.total)
                        {
                            record.partial_size = size;
                            record.mode = RecoveryMode::Cleanup;
                        }
                    }
                }
            }
            store.upsert(record)
        })
        .await
    }
    fn partial_for_retry(&self) -> Option<PartialState> {
        if !(self.task.cancel.is_cancelled()
            || self.context.client.is_closed()
            || self.partial.is_some())
        {
            return None;
        }
        match &self.local {
            Local::Source(_) => self
                .temporary
                .clone()
                .map(|path| PartialState::Upload { path })
                .or_else(|| self.partial.clone()),
            Local::Destination(destination) => {
                let destination = destination.clone();
                // This is only called after cleanup has chosen to retain the
                // local temporary; a failed size query therefore disables
                // continuation rather than guessing an offset.
                destination
                    .partial_size()
                    .ok()
                    .flatten()
                    .map(|size| PartialState::Download {
                        size,
                        local_partial: self
                            .local_partial
                            .clone()
                            .or_else(|| destination.partial_path().ok().flatten()),
                    })
            }
        }
    }
}
fn parent(path: &RemotePath) -> Result<RemotePath, SftpError> {
    let (parent, _) = path
        .as_str()
        .rsplit_once('/')
        .ok_or(SftpError::PathInvalid)?;
    RemotePath::parse(if parent.is_empty() { "/" } else { parent })
}

async fn verify_upload_prefix(
    source: &Arc<dyn LocalSource>,
    client: &Arc<SftpClient>,
    path: &RemotePath,
    size: u64,
    budget: &Arc<budget::Budget>,
    cancel: &CancellationToken,
) -> Result<Sha256, SftpError> {
    let window = reserve_window(budget, client, cancel).await?;
    let remote = client
        .open_file(path.as_str(), false, cancel)
        .await
        .map_err(map_error)?;
    let mut remote = remote.pipeline(cancel).await.map_err(map_error)?;
    remote.retain_payload_guard(Box::new(window));
    let result = async {
        let mut digest = Sha256::new();
        let mut offset = 0;
        while offset < size {
            let count = (size - offset).min(client.capabilities().request_bytes as u64) as usize;
            let actual = reserve(budget, client, count, cancel)
                .await?
                .wrap(remote.read(count, cancel).await.map_err(map_error)?)?;
            if actual.is_empty() {
                return Err(SftpError::RemoteIo);
            }
            let actual_len = actual.len();
            let reservation = reserve(budget, client, actual_len, cancel).await?;
            let expected = blocking({
                let source = source.clone();
                move || {
                    reservation
                        .wrap(source.read(offset, actual_len)?)
                        .map_err(|_| SftpError::LocalIo)
                }
            })
            .await?;
            if actual.as_ref() != expected.as_ref() {
                return Err(SftpError::ResumeMismatch);
            }
            digest.update(expected);
            offset = offset.saturating_add(actual.len() as u64);
        }
        Ok(digest)
    }
    .await;
    let closed = remote
        .close(&CancellationToken::new())
        .await
        .map_err(map_error);
    let digest = result?;
    closed?;
    Ok(digest)
}

async fn verify_download_prefix(
    destination: &Arc<dyn LocalDestination>,
    client: &Arc<SftpClient>,
    path: &RemotePath,
    size: u64,
    budget: &Arc<budget::Budget>,
    cancel: &CancellationToken,
) -> Result<Sha256, SftpError> {
    let window = reserve_window(budget, client, cancel).await?;
    let remote = client
        .open_file(path.as_str(), false, cancel)
        .await
        .map_err(map_error)?;
    let mut remote = remote.pipeline(cancel).await.map_err(map_error)?;
    remote.retain_payload_guard(Box::new(window));
    let result = async {
        let mut digest = Sha256::new();
        let mut offset = 0;
        while offset < size {
            let count = (size - offset).min(client.capabilities().request_bytes as u64) as usize;
            let actual = reserve(budget, client, count, cancel)
                .await?
                .wrap(remote.read(count, cancel).await.map_err(map_error)?)?;
            if actual.is_empty() {
                return Err(SftpError::RemoteIo);
            }
            let actual_len = actual.len();
            let expected = blocking({
                let destination = destination.clone();
                move || {
                    let data = destination.read(offset, actual_len)?;
                    if data.len() != actual_len {
                        return Err(SftpError::LocalIo);
                    }
                    Ok(data)
                }
            })
            .await?;
            if actual.as_ref() != expected.as_slice() {
                return Err(SftpError::ResumeMismatch);
            }
            digest.update(expected);
            offset = offset.saturating_add(actual.len() as u64);
        }
        Ok(digest)
    }
    .await;
    let closed = remote
        .close(&CancellationToken::new())
        .await
        .map_err(map_error);
    let digest = result?;
    closed?;
    Ok(digest)
}

async fn reserve(
    budget: &Arc<budget::Budget>,
    client: &SftpClient,
    count: usize,
    cancel: &CancellationToken,
) -> Result<budget::Reservation, SftpError> {
    tokio::select! {
        biased;
        () = client.closed() => Err(SftpError::ConnectionLost),
        result = budget.reserve(count, cancel) => result,
    }
}
async fn reserve_window(
    budget: &Arc<budget::Budget>,
    client: &SftpClient,
    cancel: &CancellationToken,
) -> Result<budget::Reservation, SftpError> {
    tokio::select! {
        biased;
        () = client.closed() => Err(SftpError::ConnectionLost),
        result = budget.window(cancel) => result,
    }
}
async fn hash_remote(
    file: RemoteFile,
    size: u64,
    client: &SftpClient,
    budget: &Arc<budget::Budget>,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, SftpError> {
    let window = reserve_window(budget, client, cancel).await?;
    let mut file = file.pipeline(cancel).await.map_err(map_error)?;
    file.retain_payload_guard(Box::new(window));
    let result = async {
        let mut digest = Sha256::new();
        let mut offset = 0;
        while offset < size {
            let count = (size - offset).min(client.capabilities().request_bytes as u64) as usize;
            let reservation = reserve(budget, client, count, cancel).await?;
            let data = reservation.wrap(file.read(count, cancel).await.map_err(map_error)?)?;
            if data.is_empty() {
                return Err(SftpError::RemoteIo);
            }
            offset += data.len() as u64;
            digest.update(data);
            tokio::task::yield_now().await;
        }
        let reservation = reserve(budget, client, 1, cancel).await?;
        let tail = reservation.wrap(file.read(1, cancel).await.map_err(map_error)?)?;
        if !tail.is_empty() {
            return Err(SftpError::RemoteIo);
        }
        Ok(digest.finalize().to_vec())
    }
    .await;
    let closed = file
        .close(&CancellationToken::new())
        .await
        .map_err(map_error);
    let digest = result?;
    closed?;
    Ok(digest)
}
pub(super) async fn run(
    task: Arc<Task>,
    mut context: TransferContext,
    local: Local,
    workers: Arc<Semaphore>,
    commits: Arc<tokio::sync::Mutex<()>>,
    budget: Arc<budget::Budget>,
    partial: Option<PartialState>,
) {
    let permit = tokio::select! {
        biased;
        () = task.cancel.cancelled() => Err(SftpError::OperationCanceled),
        () = context.lease.connection().closed() => Err(SftpError::ConnectionLost),
        permit = workers.acquire_owned() => permit.map_err(|_| SftpError::ResourceLimit),
    };
    // Queued jobs retain authorization, but consume no subsystem assignment.
    // A primary browsing fault does not interrupt a sibling data subsystem.
    let data_lease = if permit.is_ok() {
        Some(context.lease.lease_sftp_transfer(&task.cancel).await)
    } else {
        None
    };
    if let Some(Ok(lease)) = &data_lease {
        context.client = lease.client().clone();
    }
    let mut job = Job {
        task: task.clone(),
        context,
        local,
        decisions: task.take_decisions().unwrap_or_else(|| mpsc::channel(1).1),
        temporary: None,
        local_partial: match &partial {
            Some(PartialState::Download { local_partial, .. }) => local_partial.clone(),
            _ => None,
        },
        created: false,
        cleanup_baseline: None,
        verified: None,
        partial,
        commits,
        budget,
    };
    let result = match (&permit, &data_lease) {
        (Ok(_), Some(Ok(_))) => job.execute().await,
        (Err(error), _) => Err(*error),
        (_, Some(Err(error))) => Err(map_error(*error)),
        _ => Err(SftpError::ConnectionLost),
    };
    let result = match result {
        // An admitted mutation can lose its ACK when the subsystem closes.
        // Keep cleanup uncertainty, but report interruption instead of a
        // generic failed job. Never replay it or interrupt an admitted commit.
        Err(SftpError::RemoteIo) if job.context.client.is_closed() => {
            Err(SftpError::ConnectionLost)
        }
        result => result,
    };
    let cleanup = if result.is_err() {
        job.cleanup().await
    } else {
        CleanupOutcome::Complete
    };
    let cleanup_required = !matches!(cleanup, CleanupOutcome::Complete);
    let partial = if matches!(cleanup, CleanupOutcome::Retained) {
        job.partial_for_retry()
    } else {
        None
    };
    if let Some(partial) = partial {
        let _ = task.retain_partial(job.context.clone(), job.local.clone(), partial);
    }
    if matches!(
        cleanup,
        CleanupOutcome::Failed(_) | CleanupOutcome::Unverified
    ) {
        if job.record_cleanup(&cleanup).await.is_err() {
            if let Some(store) = &task.recovery {
                if let Ok(status) = task.status.lock() {
                    store.restrict_to_inspection(status.id);
                }
            }
        }
        // Cleanup-only records do not retain old native handles or become
        // resumable jobs. The user can explicitly reauthorize their cleanup.
        task.clear_retry();
    }
    drop(job);
    // Finished jobs release their transport. Download capabilities remain
    // available for explicit retry; uploads require a new native source.
    if let Ok(mut retry) = task.retry.lock() {
        if let Some(plan) = retry.as_mut() {
            plan.context = None;
            if matches!(plan.local, Some(Local::Source(_))) {
                plan.local = None;
            }
        }
    }
    if result.is_ok() || task.cancel.is_cancelled() || result == Err(SftpError::OperationCanceled) {
        task.clear_retry();
    }
    if matches!(cleanup, CleanupOutcome::Complete) {
        let _ = task.clear_recovery();
    }
    let _ = task.update(|status| {
        status.state = match result {
            Ok(()) => TransferState::Completed,
            Err(_) if task.cancel.is_cancelled() => TransferState::Canceled,
            Err(SftpError::OperationCanceled) => TransferState::Canceled,
            Err(SftpError::ConnectionLost) => TransferState::Interrupted,
            Err(_) => TransferState::Failed,
        };
        if result.is_ok() {
            status.processed = status.total;
            status.remaining_seconds = Some(ExactU64(0));
        }
        status.error = result.err();
        status.conflict = None;
        status.cleanup_required = cleanup_required;
    });
    // Cancellation is visible promptly. The worker and data assignment remain
    // reserved until all owned protocol cleanup finishes; exit joins the pool.
    // Cleanup owns no task or recovery ledger, so a completed service can be
    // dropped and its encrypted recovery store reopened independently.
    drop(task);
    if let Some(Ok(lease)) = data_lease {
        lease.release().await;
    }
    drop(permit);
}
