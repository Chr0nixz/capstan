//! Folder policies use production SSH/SFTP, native files and publication checks.
use super::*;
use capstan_core::transfer::{
    FolderSelection, LocalDestination, LocalSource, TransferContext, WindowsDestination,
};
use std::path::Path;

fn directory(remote: &RemoteState, path: &str) {
    remote.files.lock().unwrap().insert(
        path.into(),
        Record {
            data: Vec::new(),
            mode: 0o40755,
            mtime: 100,
        },
    );
}

async fn selections(
    fixture: &Fixture,
    session: SftpSessionId,
    transfers: &Arc<TransferService>,
    local: &Path,
    names: &[&str],
) -> Vec<(TransferContext, FolderSelection)> {
    directory(&fixture.observations.transfer, "/files/tree");
    let mut contexts = Vec::new();
    let mut items = Vec::new();
    for name in names {
        let path = local.join(name);
        std::fs::write(&path, format!("local {name}")).unwrap();
        let context = fixture
            .service
            .transfer_context(
                "main",
                "one",
                session,
                TransferTarget::UploadTree {
                    parent: RemotePath::parse("/files").unwrap(),
                    relative: format!("tree/{name}"),
                },
            )
            .unwrap();
        contexts.push(context.clone());
        items.push((
            context,
            Arc::new(capstan_native_windows::transfer::Source::open(&path).unwrap())
                as Arc<dyn LocalSource>,
        ));
    }
    contexts
        .into_iter()
        .zip(transfers.grant_folder_sources(items).await.unwrap())
        .collect()
}

fn enqueue(
    transfers: &Arc<TransferService>,
    selected: Vec<(TransferContext, FolderSelection)>,
) -> Vec<Uuid> {
    selected
        .into_iter()
        .map(|(context, item)| transfers.enqueue(context, item.selection.token).unwrap())
        .collect()
}

#[tokio::test]
async fn folder_preflight_pipelines_sixteen_snapshots_and_keeps_reordered_versions_bound() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    let owned_names = (0..33)
        .map(|index| format!("file-{index:02}"))
        .collect::<Vec<_>>();
    let names = owned_names.iter().map(String::as_str).collect::<Vec<_>>();
    for (index, name) in names.iter().enumerate().filter(|(index, _)| index % 2 == 0) {
        remote.put(&format!("/files/tree/{name}"), &vec![b'x'; index + 1]);
    }
    {
        let mut batches = remote.stat_batches.lock().unwrap();
        batches.groups = [16, 16, 1].into();
        batches.observed.clear();
    }
    let local = tempfile::tempdir().unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        selections(&fixture, session, &transfers, local.path(), &names),
    )
    .await;
    if result.is_err() {
        fixture.close().await;
        panic!("folder preflight waited for its first reply before sending siblings");
    }
    let selected = result.unwrap();
    {
        let batches = remote.stat_batches.lock().unwrap();
        assert!(batches.groups.is_empty());
        assert_eq!(batches.observed.len(), 33);
        assert_eq!(batches.peak, 16);
    }
    let ids = enqueue(&transfers, selected);
    for (index, id) in ids.iter().enumerate() {
        if index % 2 == 0 {
            let prompt = wait(&transfers, *id, |row| row.conflict.is_some()).await;
            assert_eq!(prompt.batch.unwrap().conflicts, 17);
            let conflict = prompt.conflict.unwrap();
            assert!(conflict.batch_eligible, "preflight version changed members");
            assert_eq!(conflict.target.unwrap().size.unwrap().0, index as u64 + 1);
            transfers
                .resolve("main", *id, conflict.revision, Decision::Skip)
                .unwrap();
        }
        assert_eq!(
            finished(&transfers, *id).await.state,
            if index % 2 == 0 {
                TransferState::Canceled
            } else {
                TransferState::Completed
            }
        );
    }
    assert!(remote.no_partials());
    fixture.close().await;
}

#[tokio::test]
async fn folder_preflight_error_returns_no_grants_and_stops_before_the_next_batch() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    let local = tempfile::tempdir().unwrap();
    let mut items = Vec::new();
    for index in 0..17 {
        let name = match index {
            1 => "denied".to_owned(),
            16 => "later".to_owned(),
            _ => format!("file-{index:02}"),
        };
        let path = local.path().join(&name);
        std::fs::write(&path, b"source").unwrap();
        let context = fixture
            .service
            .transfer_context(
                "main",
                "one",
                session,
                TransferTarget::UploadTree {
                    parent: RemotePath::parse("/files").unwrap(),
                    relative: format!("tree/{name}"),
                },
            )
            .unwrap();
        items.push((
            context,
            Arc::new(capstan_native_windows::transfer::Source::open(&path).unwrap())
                as Arc<dyn LocalSource>,
        ));
    }
    {
        let mut batches = remote.stat_batches.lock().unwrap();
        batches.groups = [16].into();
        batches.observed.clear();
        batches.denied.insert("/files/tree/denied".into());
    }
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        transfers.grant_folder_sources(items),
    )
    .await;
    let observations = remote.stat_batches.lock().unwrap().observed.clone();
    fixture.close().await;
    assert!(matches!(result, Ok(Err(SftpError::PermissionDenied))));
    assert_eq!(observations.len(), 16);
    assert!(!observations.iter().any(|path| path.ends_with("/later")));
    assert!(transfers.snapshot("main").unwrap().is_empty());
}

#[derive(Default)]
struct SnapshotHold {
    started: AtomicUsize,
    active: AtomicUsize,
    peak: AtomicUsize,
    finished: AtomicUsize,
    released: Mutex<bool>,
    changed: std::sync::Condvar,
}
struct SnapshotDestination {
    inner: WindowsDestination,
    hold: Arc<SnapshotHold>,
    denied: bool,
}
impl LocalDestination for SnapshotDestination {
    fn name(&self) -> Result<String, SftpError> {
        self.inner.name()
    }
    fn snapshot(&self) -> Result<Option<capstan_core::transfer::LocalVersion>, SftpError> {
        self.hold.started.fetch_add(1, Ordering::SeqCst);
        let active = self.hold.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.hold.peak.fetch_max(active, Ordering::SeqCst);
        let result = if self.denied {
            Err(SftpError::PermissionDenied)
        } else {
            let released = self.hold.released.lock().unwrap();
            let (released, _) = self
                .hold
                .changed
                .wait_timeout_while(released, Duration::from_secs(5), |released| !*released)
                .unwrap();
            if *released {
                self.inner.snapshot()
            } else {
                Err(SftpError::LocalIo)
            }
        };
        self.hold.active.fetch_sub(1, Ordering::SeqCst);
        self.hold.finished.fetch_add(1, Ordering::SeqCst);
        result
    }
    fn retarget(&self, name: &str) -> Result<(), SftpError> {
        self.inner.retarget(name)
    }
    fn begin(&self, name: &str) -> Result<(), SftpError> {
        self.inner.begin(name)
    }
    fn write(&self, offset: u64, data: &[u8]) -> Result<(), SftpError> {
        self.inner.write(offset, data)
    }
    fn read(&self, offset: u64, length: usize) -> Result<Vec<u8>, SftpError> {
        self.inner.read(offset, length)
    }
    fn sync(&self) -> Result<(), SftpError> {
        self.inner.sync()
    }
    fn commit(
        &self,
        expected: Option<capstan_core::transfer::LocalVersion>,
    ) -> Result<(), SftpError> {
        self.inner.commit(expected)
    }
    fn discard(&self) -> Result<(), SftpError> {
        self.inner.discard()
    }
}

#[tokio::test]
async fn folder_preflight_joins_native_snapshots_after_a_peer_error_without_starting_more() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    directory(remote, "/files/tree");
    for index in 0..17 {
        remote.put(&format!("/files/tree/file-{index:02}"), b"remote");
    }
    let page = fixture.list(session, "/files").await.unwrap();
    let entry = page
        .entries
        .iter()
        .find(|entry| entry.name == "tree")
        .unwrap();
    let tree = fixture
        .service
        .collect_directory_tree("main", "one", session, entry.id)
        .await
        .unwrap();
    let local = tempfile::tempdir().unwrap();
    std::fs::create_dir(local.path().join("tree")).unwrap();
    let hold = Arc::new(SnapshotHold::default());
    let items = tree
        .files()
        .iter()
        .enumerate()
        .map(|(index, file)| {
            let context = fixture
                .service
                .transfer_context_for_tree_file("main", "one", session, file)
                .unwrap();
            let inner = WindowsDestination::new(
                capstan_native_windows::transfer::Destination::select_under_folder(
                    local.path(),
                    &std::path::PathBuf::from(file.relative()),
                )
                .unwrap(),
            );
            (
                context,
                Arc::new(SnapshotDestination {
                    inner,
                    hold: hold.clone(),
                    denied: index == 0,
                }) as Arc<dyn LocalDestination>,
            )
        })
        .collect();
    let service = transfers.clone();
    let preflight = tokio::spawn(async move { service.enqueue_folder_destinations(items).await });
    let started = tokio::time::timeout(Duration::from_secs(3), async {
        while hold.started.load(Ordering::SeqCst) < 16 && !preflight.is_finished() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    let returned_early = preflight.is_finished();
    *hold.released.lock().unwrap() = true;
    hold.changed.notify_all();
    let result = tokio::time::timeout(Duration::from_secs(3), preflight).await;
    fixture.close().await;
    assert!(started.is_ok());
    assert!(
        !returned_early,
        "a peer error abandoned admitted blocking snapshots"
    );
    assert!(matches!(result, Ok(Ok(Err(SftpError::PermissionDenied)))));
    assert_eq!(hold.started.load(Ordering::SeqCst), 16);
    assert_eq!(hold.finished.load(Ordering::SeqCst), 16);
    assert_eq!(hold.active.load(Ordering::SeqCst), 0);
    assert!(hold.peak.load(Ordering::SeqCst) <= 16);
    assert!(transfers.snapshot("main").unwrap().is_empty());
}

#[tokio::test]
async fn folder_replace_covers_queued_members_but_never_another_selection_or_single_file() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    let names = ["one", "two", "three", "four", "new"];
    for name in &names[..4] {
        remote.put(&format!("/files/tree/{name}"), b"original");
    }
    let local = tempfile::tempdir().unwrap();
    let ids = enqueue(
        &transfers,
        selections(&fixture, session, &transfers, local.path(), &names).await,
    );
    let prompt = wait(&transfers, ids[0], |s| s.conflict.is_some()).await;
    let batch = prompt.batch.unwrap();
    assert_eq!((batch.files, batch.conflicts), (5, 4));
    let conflict = prompt.conflict.unwrap();
    assert!(conflict.batch_eligible);
    assert_eq!(
        transfers.resolve("other", ids[0], conflict.revision, Decision::ReplaceBatch),
        Err(SftpError::ScopeExpired)
    );
    transfers
        .resolve("main", ids[0], conflict.revision, Decision::ReplaceBatch)
        .unwrap();
    assert_eq!(
        transfers.resolve("main", ids[0], conflict.revision, Decision::SkipBatch),
        Err(SftpError::ScopeExpired)
    );
    for (id, name) in ids.into_iter().zip(names) {
        let result = finished(&transfers, id).await;
        assert_eq!(result.state, TransferState::Completed, "{result:?}");
        assert_eq!(result.batch.unwrap().id, batch.id);
        assert_eq!(
            remote.bytes(&format!("/files/tree/{name}")),
            format!("local {name}").as_bytes()
        );
    }
    let second_local = tempfile::tempdir().unwrap();
    let second = enqueue(
        &transfers,
        selections(&fixture, session, &transfers, second_local.path(), &["one"]).await,
    )[0];
    let prompt = wait(&transfers, second, |s| s.conflict.is_some()).await;
    assert_ne!(prompt.batch.unwrap().id, batch.id);
    transfers
        .resolve(
            "main",
            second,
            prompt.conflict.unwrap().revision,
            Decision::SkipBatch,
        )
        .unwrap();
    assert_eq!(
        finished(&transfers, second).await.state,
        TransferState::Canceled
    );
    remote.put("/files/outside", b"outside");
    let outside = enqueue_upload(
        &fixture,
        session,
        &transfers,
        &local.path().join("one"),
        "outside",
    );
    let prompt = wait(&transfers, outside, |s| s.conflict.is_some()).await;
    assert!(prompt.batch.is_none());
    let conflict = prompt.conflict.unwrap();
    assert!(!conflict.batch_eligible);
    assert_eq!(
        transfers.resolve("main", outside, conflict.revision, Decision::ReplaceBatch),
        Err(SftpError::ScopeExpired)
    );
    transfers
        .resolve("main", outside, conflict.revision, Decision::Skip)
        .unwrap();
    finished(&transfers, outside).await;
    assert_eq!(remote.bytes("/files/outside"), b"outside");
    assert!(remote.no_partials());
    fixture.close().await;
}

#[tokio::test]
async fn folder_skip_preserves_existing_targets_and_still_copies_absent_files() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    for name in ["one", "two", "three", "four"] {
        remote.put(&format!("/files/tree/{name}"), b"original");
    }
    let local = tempfile::tempdir().unwrap();
    let names = ["one", "two", "three", "four", "new"];
    let ids = enqueue(
        &transfers,
        selections(&fixture, session, &transfers, local.path(), &names).await,
    );
    let conflict = wait(&transfers, ids[0], |s| s.conflict.is_some())
        .await
        .conflict
        .unwrap();
    transfers
        .resolve("main", ids[0], conflict.revision, Decision::SkipBatch)
        .unwrap();
    for (id, name) in ids.into_iter().zip(names) {
        assert_eq!(
            finished(&transfers, id).await.state,
            if name == "new" {
                TransferState::Completed
            } else {
                TransferState::Canceled
            }
        );
        assert_eq!(
            remote.bytes(&format!("/files/tree/{name}")),
            if name == "new" {
                b"local new".as_slice()
            } else {
                b"original"
            }
        );
    }
    assert!(remote.no_partials());
    fixture.close().await;
}

#[tokio::test]
async fn folder_changed_appeared_and_disappeared_targets_require_individual_decisions() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    for name in ["changed", "gone", "stable"] {
        remote.put(&format!("/files/tree/{name}"), b"original");
    }
    let local = tempfile::tempdir().unwrap();
    let selected = selections(
        &fixture,
        session,
        &transfers,
        local.path(),
        &["changed", "gone", "stable", "appeared"],
    )
    .await;
    remote.put("/files/tree/changed", b"external update");
    remote.files.lock().unwrap().remove("/files/tree/gone");
    remote.put("/files/tree/appeared", b"external create");
    let ids = enqueue(&transfers, selected);
    let stable = wait(&transfers, ids[2], |s| s.conflict.is_some())
        .await
        .conflict
        .unwrap();
    transfers
        .resolve("main", ids[2], stable.revision, Decision::ReplaceBatch)
        .unwrap();
    assert_eq!(
        finished(&transfers, ids[2]).await.state,
        TransferState::Completed
    );
    for index in [0, 1, 3] {
        let conflict = wait(&transfers, ids[index], |s| s.conflict.is_some())
            .await
            .conflict
            .unwrap();
        assert!(!conflict.batch_eligible);
        assert_eq!(
            transfers.resolve(
                "main",
                ids[index],
                conflict.revision,
                Decision::ReplaceBatch
            ),
            Err(SftpError::ScopeExpired)
        );
        transfers
            .resolve("main", ids[index], conflict.revision, Decision::Skip)
            .unwrap();
        assert_eq!(
            finished(&transfers, ids[index]).await.state,
            TransferState::Canceled
        );
    }
    assert_eq!(remote.bytes("/files/tree/changed"), b"external update");
    assert_eq!(remote.bytes("/files/tree/appeared"), b"external create");
    assert!(!remote
        .files
        .lock()
        .unwrap()
        .contains_key("/files/tree/gone"));
    assert!(remote.no_partials());
    fixture.close().await;
}

#[tokio::test]
async fn folder_final_target_change_does_not_reuse_the_group_confirmation() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    remote.put("/files/tree/file", b"original");
    remote.stall_fsync.store(true, Ordering::SeqCst);
    let local = tempfile::tempdir().unwrap();
    let id = enqueue(
        &transfers,
        selections(&fixture, session, &transfers, local.path(), &["file"]).await,
    )[0];
    let conflict = wait(&transfers, id, |s| s.conflict.is_some())
        .await
        .conflict
        .unwrap();
    transfers
        .resolve("main", id, conflict.revision, Decision::ReplaceBatch)
        .unwrap();
    remote.started.notified().await;
    remote.put("/files/tree/file", b"changed after staging");
    remote.stall_fsync.store(false, Ordering::SeqCst);
    remote.release.notify_one();
    let prompt = wait(&transfers, id, |s| s.conflict.is_some()).await;
    let conflict = prompt.conflict.unwrap();
    assert!(!conflict.batch_eligible);
    assert_eq!(prompt.processed, prompt.total);
    assert_eq!(
        transfers.resolve("main", id, conflict.revision, Decision::ReplaceBatch),
        Err(SftpError::ScopeExpired)
    );
    transfers
        .resolve(
            "main",
            id,
            conflict.revision,
            Decision::Rename {
                name: "saved-copy".into(),
            },
        )
        .unwrap();
    assert_eq!(
        finished(&transfers, id).await.state,
        TransferState::Completed
    );
    assert_eq!(remote.bytes("/files/tree/file"), b"changed after staging");
    assert_eq!(remote.bytes("/files/tree/saved-copy"), b"local file");
    assert!(remote.no_partials());
    fixture.close().await;
}

#[tokio::test]
async fn folder_retry_does_not_inherit_its_original_replace_policy() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    remote.put("/files/tree/file", b"original");
    remote.deny_write.store(true, Ordering::SeqCst);
    let local = tempfile::tempdir().unwrap();
    let selected = selections(&fixture, session, &transfers, local.path(), &["file"]).await;
    let context = selected[0].0.clone();
    let id = enqueue(&transfers, selected)[0];
    let conflict = wait(&transfers, id, |s| s.conflict.is_some())
        .await
        .conflict
        .unwrap();
    transfers
        .resolve("main", id, conflict.revision, Decision::ReplaceBatch)
        .unwrap();
    assert_eq!(finished(&transfers, id).await.state, TransferState::Failed);
    remote.deny_write.store(false, Ordering::SeqCst);
    let source = Arc::new(
        capstan_native_windows::transfer::Source::open(&local.path().join("file")).unwrap(),
    );
    let selection = transfers.grant_source(&context, source).unwrap();
    transfers
        .retry_with_selection("main", id, context, selection.token)
        .unwrap();
    let conflict = wait(&transfers, id, |s| s.conflict.is_some())
        .await
        .conflict
        .unwrap();
    assert!(!conflict.batch_eligible);
    transfers
        .resolve("main", id, conflict.revision, Decision::Skip)
        .unwrap();
    assert_eq!(
        finished(&transfers, id).await.state,
        TransferState::Canceled
    );
    assert_eq!(remote.bytes("/files/tree/file"), b"original");
    fixture.close().await;
}

#[tokio::test]
async fn folder_grants_bind_their_precise_target_and_reject_mixed_or_duplicate_groups() {
    let (fixture, session, transfers) = fixture().await;
    let local = tempfile::tempdir().unwrap();
    let mut selected = selections(&fixture, session, &transfers, local.path(), &["file"]).await;
    let (context, item) = selected.pop().unwrap();
    let other = upload_context(&fixture, session, "unrelated");
    assert_eq!(
        transfers.enqueue(other, item.selection.token),
        Err(SftpError::ScopeExpired)
    );
    let source = Arc::new(
        capstan_native_windows::transfer::Source::open(&local.path().join("file")).unwrap(),
    ) as Arc<dyn LocalSource>;
    assert!(matches!(
        transfers
            .grant_folder_sources(vec![
                (context.clone(), source.clone()),
                (context.clone(), source.clone())
            ])
            .await,
        Err(SftpError::ScopeExpired)
    ));
    assert!(matches!(
        transfers
            .grant_folder_sources(vec![(
                upload_context(&fixture, session, "plain"),
                source.clone()
            )])
            .await,
        Err(SftpError::ScopeExpired)
    ));
    let different_root = fixture
        .service
        .transfer_context(
            "main",
            "one",
            session,
            TransferTarget::UploadTree {
                parent: RemotePath::parse("/files").unwrap(),
                relative: "other/file".into(),
            },
        )
        .unwrap();
    assert!(matches!(
        transfers
            .grant_folder_sources(vec![
                (context.clone(), source.clone()),
                (different_root, source.clone())
            ])
            .await,
        Err(SftpError::ScopeExpired)
    ));
    assert!(matches!(
        transfers
            .grant_folder_sources(vec![(context.clone(), source); 129])
            .await,
        Err(SftpError::ResourceLimit)
    ));
    // The wrong-target enqueue never consumes the valid native selection.
    let id = transfers.enqueue(context, item.selection.token).unwrap();
    assert_eq!(
        finished(&transfers, id).await.state,
        TransferState::Completed
    );
    assert_eq!(
        fixture.observations.transfer.bytes("/files/tree/file"),
        b"local file"
    );
    fixture.close().await;
}

#[tokio::test]
async fn folder_server_without_atomic_replace_refuses_batch_overwrite_and_allows_skip() {
    let fixture = Fixture::new().await;
    fixture
        .observations
        .transfer_mode
        .store(true, Ordering::SeqCst);
    let remote = &fixture.observations.transfer;
    remote.without_extensions.store(true, Ordering::SeqCst);
    remote.put("/files/tree/file", b"original");
    let session = fixture.attach("one").await;
    fixture.list(session, "/files").await.unwrap();
    let transfers = Arc::new(TransferService::default());
    let local = tempfile::tempdir().unwrap();
    let id = enqueue(
        &transfers,
        selections(&fixture, session, &transfers, local.path(), &["file"]).await,
    )[0];
    let conflict = wait(&transfers, id, |s| s.conflict.is_some())
        .await
        .conflict
        .unwrap();
    assert!(!conflict.replace_allowed);
    assert!(conflict.batch_eligible);
    assert_eq!(
        transfers.resolve("main", id, conflict.revision, Decision::ReplaceBatch),
        Err(SftpError::UnsupportedFileType)
    );
    transfers
        .resolve("main", id, conflict.revision, Decision::SkipBatch)
        .unwrap();
    assert_eq!(
        finished(&transfers, id).await.state,
        TransferState::Canceled
    );
    assert_eq!(remote.bytes("/files/tree/file"), b"original");
    assert_eq!(remote.commits.load(Ordering::SeqCst), 0);
    assert!(remote.no_partials());
    fixture.close().await;
}

pub(super) async fn openssh_folder_batches(
    sftp: &SftpService,
    session: SftpSessionId,
    parent: &RemotePath,
    client: &Arc<capstan_ssh::sftp::SftpClient>,
    local: &Path,
    transfers: &Arc<TransferService>,
) -> serde_json::Value {
    use sha2::{Digest, Sha256};
    let cancel = CancellationToken::new();
    let name = format!("folder-batch-{}", Uuid::new_v4());
    let directories = vec![
        name.clone(),
        format!("{name}/nested"),
        format!("{name}/empty"),
    ];
    sftp.ensure_directory_tree("main", "one", session, parent.clone(), &directories)
        .await
        .unwrap();
    let names = ["one.txt", "two.txt", "three.txt", "four.txt", "new.txt"];
    let sources = local.join("batch-sources");
    std::fs::create_dir(&sources).unwrap();
    let mut contexts = Vec::new();
    let mut items = Vec::new();
    let mut hashes = Vec::new();
    for (index, file) in names.iter().enumerate() {
        let relative = format!("{name}/nested/{file}");
        let remote_path = upload_path(parent, &relative);
        if index < 4 {
            let seed = client.open_file(&remote_path, true, &cancel).await.unwrap();
            seed.write(0, b"before batch", &cancel).await.unwrap();
            seed.close(&cancel).await.unwrap();
        }
        let bytes = format!("\u{feff}folder batch {file}\r\n").into_bytes();
        std::fs::write(sources.join(file), &bytes).unwrap();
        hashes.push(format!("{:x}", Sha256::digest(&bytes)));
        let context = sftp
            .transfer_context(
                "main",
                "one",
                session,
                TransferTarget::UploadTree {
                    parent: parent.clone(),
                    relative,
                },
            )
            .unwrap();
        contexts.push(context.clone());
        items.push((
            context,
            Arc::new(capstan_native_windows::transfer::Source::open(&sources.join(file)).unwrap())
                as Arc<dyn LocalSource>,
        ));
    }
    let selected = transfers.grant_folder_sources(items).await.unwrap();
    let ids = contexts
        .into_iter()
        .zip(selected)
        .map(|(context, item)| transfers.enqueue(context, item.selection.token).unwrap())
        .collect::<Vec<_>>();
    let prompt = wait(transfers, ids[0], |s| s.conflict.is_some()).await;
    assert_eq!(
        prompt
            .batch
            .as_ref()
            .map(|batch| (batch.files, batch.conflicts)),
        Some((5, 4))
    );
    transfers
        .resolve(
            "main",
            ids[0],
            prompt.conflict.unwrap().revision,
            Decision::ReplaceBatch,
        )
        .unwrap();
    for (index, id) in ids.into_iter().enumerate() {
        assert_eq!(
            finished(transfers, id).await.state,
            TransferState::Completed
        );
        let path = RemotePath::parse(&upload_path(
            parent,
            &format!("{name}/nested/{}", names[index]),
        ))
        .unwrap();
        let (bytes, _) = capstan_core::remote_edit::read_remote_file(client, &path, None, &cancel)
            .await
            .unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), hashes[index]);
    }
    let page = sftp
        .list("main", "one", session, parent.clone())
        .await
        .unwrap();
    let entry = page
        .entries
        .iter()
        .find(|entry| entry.name == name)
        .unwrap();
    let tree = sftp
        .collect_directory_tree("main", "one", session, entry.id)
        .await
        .unwrap();
    assert_eq!(tree.files().len(), 5);
    assert!(tree
        .files()
        .iter()
        .all(|file| file.relative().starts_with(&format!("{name}/nested/"))));
    for (round, decision) in [Decision::ReplaceBatch, Decision::SkipBatch]
        .into_iter()
        .enumerate()
    {
        let output = local.join(format!("batch-download-{round}"));
        std::fs::create_dir(&output).unwrap();
        let directories = tree
            .directories()
            .iter()
            .map(std::path::PathBuf::from)
            .collect::<Vec<_>>();
        capstan_native_windows::transfer::prepare_destination_folder(&output, &directories)
            .unwrap();
        for file in &names[..4] {
            std::fs::write(output.join(&name).join("nested").join(file), b"keep local").unwrap();
        }
        let before = transfers
            .snapshot("main")
            .unwrap()
            .into_iter()
            .map(|task| task.id)
            .collect::<std::collections::HashSet<_>>();
        let mut items = Vec::new();
        // Stable explicit order also verifies policy propagation to members
        // that have not yet acquired one of the three global workers.
        for file in names {
            let relative = format!("{name}/nested/{file}");
            let file = tree
                .files()
                .iter()
                .find(|file| file.relative() == relative)
                .unwrap();
            let context = sftp
                .transfer_context_for_tree_file("main", "one", session, file)
                .unwrap();
            let destination = capstan_native_windows::transfer::Destination::select_under_folder(
                &output,
                &std::path::PathBuf::from(file.relative()),
            )
            .unwrap();
            items.push((
                context,
                Arc::new(WindowsDestination::new(destination)) as Arc<dyn LocalDestination>,
            ));
        }
        assert_eq!(
            transfers.enqueue_folder_destinations(items).await.unwrap(),
            5
        );
        let tasks = transfers
            .snapshot("main")
            .unwrap()
            .into_iter()
            .filter(|task| !before.contains(&task.id))
            .collect::<Vec<_>>();
        let prompt = wait(transfers, tasks[0].id, |s| s.conflict.is_some()).await;
        transfers
            .resolve(
                "main",
                tasks[0].id,
                prompt.conflict.unwrap().revision,
                decision,
            )
            .unwrap();
        for task in tasks {
            let skipped = round == 1 && task.name != "new.txt";
            assert_eq!(
                finished(transfers, task.id).await.state,
                if skipped {
                    TransferState::Canceled
                } else {
                    TransferState::Completed
                }
            );
            let bytes = std::fs::read(output.join(&name).join("nested").join(&task.name)).unwrap();
            if skipped {
                assert_eq!(bytes, b"keep local");
            } else {
                let index = names.iter().position(|name| name == &task.name).unwrap();
                assert_eq!(format!("{:x}", Sha256::digest(bytes)), hashes[index]);
            }
        }
        assert_eq!(
            std::fs::read_dir(output.join(&name).join("nested"))
                .unwrap()
                .count(),
            5
        );
        assert_eq!(
            std::fs::read_dir(output.join(&name).join("empty"))
                .unwrap()
                .count(),
            0
        );
        assert!(!output.join("nested").exists());
    }
    for file in names {
        client
            .remove_file(
                &upload_path(parent, &format!("{name}/nested/{file}")),
                &cancel,
            )
            .await
            .unwrap();
    }
    for directory in [
        format!("{name}/nested"),
        format!("{name}/empty"),
        name.clone(),
    ] {
        client
            .remove_empty_directory(&upload_path(parent, &directory), &cancel)
            .await
            .unwrap();
    }
    let page = sftp
        .list("main", "one", session, parent.clone())
        .await
        .unwrap();
    assert!(!page.entries.iter().any(|entry| entry.name == name));
    serde_json::json!({ "files": 5, "uploadReplace": true, "downloadReplace": true, "downloadSkip": true,
        "selectedRootPreserved": true, "emptyDirectoryPreserved": true, "sha256": hashes, "remoteCleanup": true })
}

fn upload_path(parent: &RemotePath, relative: &str) -> String {
    format!("{}/{relative}", parent.as_str().trim_end_matches('/'))
}

#[tokio::test]
async fn folder_download_skip_preserves_nested_targets_and_the_selected_root() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    for path in ["/files/tree", "/files/tree/nested", "/files/tree/empty"] {
        directory(remote, path);
    }
    remote.put("/files/tree/nested/existing", b"remote original");
    remote.put("/files/tree/new", b"remote new");
    let page = fixture.list(session, "/files").await.unwrap();
    let entry = page.entries.iter().find(|e| e.name == "tree").unwrap();
    let tree = fixture
        .service
        .collect_directory_tree("main", "one", session, entry.id)
        .await
        .unwrap();
    assert_eq!(tree.directories(), &["tree", "tree/empty", "tree/nested"]);
    assert_eq!(
        tree.files()
            .iter()
            .map(|file| file.relative())
            .collect::<Vec<_>>(),
        vec!["tree/nested/existing", "tree/new"]
    );
    let local = tempfile::tempdir().unwrap();
    let directories = tree
        .directories()
        .iter()
        .map(std::path::PathBuf::from)
        .collect::<Vec<_>>();
    capstan_native_windows::transfer::prepare_destination_folder(local.path(), &directories)
        .unwrap();
    std::fs::write(local.path().join("tree/nested/existing"), b"keep local").unwrap();
    let items = tree
        .files()
        .iter()
        .map(|file| {
            let context = fixture
                .service
                .transfer_context_for_tree_file("main", "one", session, file)
                .unwrap();
            let destination = capstan_native_windows::transfer::Destination::select_under_folder(
                local.path(),
                &std::path::PathBuf::from(file.relative()),
            )
            .unwrap();
            (
                context,
                Arc::new(WindowsDestination::new(destination)) as Arc<dyn LocalDestination>,
            )
        })
        .collect();
    assert_eq!(
        transfers.enqueue_folder_destinations(items).await.unwrap(),
        2
    );
    let tasks = transfers.snapshot("main").unwrap();
    let existing = tasks
        .iter()
        .find(|task| task.name == "existing")
        .unwrap()
        .id;
    let conflict = wait(&transfers, existing, |s| s.conflict.is_some())
        .await
        .conflict
        .unwrap();
    transfers
        .resolve("main", existing, conflict.revision, Decision::SkipBatch)
        .unwrap();
    assert_eq!(
        finished(&transfers, existing).await.state,
        TransferState::Canceled
    );
    let new = tasks.iter().find(|task| task.name == "new").unwrap().id;
    assert_eq!(
        finished(&transfers, new).await.state,
        TransferState::Completed
    );
    assert_eq!(
        std::fs::read(local.path().join("tree/nested/existing")).unwrap(),
        b"keep local"
    );
    assert_eq!(
        std::fs::read(local.path().join("tree/new")).unwrap(),
        b"remote new"
    );
    assert_eq!(
        std::fs::read_dir(local.path().join("tree/empty"))
            .unwrap()
            .count(),
        0
    );
    assert!(!local.path().join("nested").exists());
    assert!(!local.path().join("new").exists());
    fixture.close().await;
}

#[tokio::test]
async fn folder_download_rechecks_native_identity_for_queued_members() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    directory(remote, "/files/tree");
    let names = ["one", "two", "three", "four", "new"];
    for name in names {
        remote.put(
            &format!("/files/tree/{name}"),
            format!("remote {name}").as_bytes(),
        );
    }
    let page = fixture.list(session, "/files").await.unwrap();
    let entry = page.entries.iter().find(|e| e.name == "tree").unwrap();
    let tree = fixture
        .service
        .collect_directory_tree("main", "one", session, entry.id)
        .await
        .unwrap();
    let local = tempfile::tempdir().unwrap();
    std::fs::create_dir(local.path().join("tree")).unwrap();
    for name in &names[..4] {
        std::fs::write(local.path().join("tree").join(name), b"original").unwrap();
    }
    // Use a deterministic file order so the changed file waits behind all
    // three workers in the first conflict phase.
    let mut items = Vec::new();
    for name in names {
        let file = tree
            .files()
            .iter()
            .find(|file| file.relative() == format!("tree/{name}"))
            .unwrap();
        let context = fixture
            .service
            .transfer_context_for_tree_file("main", "one", session, file)
            .unwrap();
        let destination = WindowsDestination::new(
            capstan_native_windows::transfer::Destination::select(
                &local.path().join("tree").join(name),
            )
            .unwrap(),
        );
        items.push((context, Arc::new(destination) as Arc<dyn LocalDestination>));
    }
    assert_eq!(
        transfers.enqueue_folder_destinations(items).await.unwrap(),
        5
    );
    let tasks = transfers.snapshot("main").unwrap();
    let first = tasks[0].id;
    let conflict = wait(&transfers, first, |s| s.conflict.is_some())
        .await
        .conflict
        .unwrap();
    std::fs::write(local.path().join("tree/four"), b"external local update").unwrap();
    transfers
        .resolve("main", first, conflict.revision, Decision::ReplaceBatch)
        .unwrap();
    for task in tasks {
        if task.name == "four" {
            let conflict = wait(&transfers, task.id, |s| s.conflict.is_some())
                .await
                .conflict
                .unwrap();
            assert!(!conflict.batch_eligible);
            transfers
                .resolve("main", task.id, conflict.revision, Decision::Skip)
                .unwrap();
            assert_eq!(
                finished(&transfers, task.id).await.state,
                TransferState::Canceled
            );
            assert_eq!(
                std::fs::read(local.path().join("tree/four")).unwrap(),
                b"external local update"
            );
        } else {
            let result = finished(&transfers, task.id).await;
            assert_eq!(result.state, TransferState::Completed, "{result:?}");
            assert_eq!(
                std::fs::read(local.path().join("tree").join(&task.name)).unwrap(),
                format!("remote {}", task.name).as_bytes()
            );
        }
    }
    assert_eq!(
        std::fs::read_dir(local.path().join("tree"))
            .unwrap()
            .count(),
        5
    );
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 3);
    assert_eq!(
        fixture.observations.peak_subsystems.load(Ordering::SeqCst),
        3
    );
    fixture.close().await;
}
