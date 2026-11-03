use super::*;
use capstan_core::transfer::{TreeChoice, TreeDecision};
fn directory(remote: &RemoteState, path: &str) {
    remote.put(path, b"");
    remote.files.lock().unwrap().get_mut(path).unwrap().mode = 0o40700;
}

#[tokio::test]
async fn tree_preflight_batches_peers_but_checks_ancestors_before_descendants() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    directory(remote, "/files/tree");
    remote.put("/files/tree/blocked", b"keep type conflict");
    remote.put("/files/tree/top", b"existing regular file");
    let local = tempfile::tempdir().unwrap();
    let root = local.path().join("tree");
    for child in ["allowed", "blocked"] {
        std::fs::create_dir_all(root.join(child)).unwrap();
        std::fs::write(root.join(child).join("file"), b"source").unwrap();
    }
    std::fs::write(root.join("top"), b"top").unwrap();
    let plan = transfers
        .prepare_upload_tree(
            &fixture.service,
            "main",
            "one",
            session,
            RemotePath::parse("/files").unwrap(),
            root,
        )
        .await
        .unwrap();
    assert_eq!(plan.conflicts.len(), 1);
    {
        let mut batches = remote.stat_batches.lock().unwrap();
        batches.groups = [1, 1, 3, 1].into();
        batches.observed.clear();
    }
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        transfers.resolve_tree_plan(
            &fixture.service,
            "main",
            "one",
            session,
            plan.token,
            plan.revision,
            vec![TreeDecision {
                entry: plan.conflicts[0].entry,
                decision: TreeChoice::Merge,
            }],
        ),
    )
    .await;
    let (observed, peak, drained) = {
        let batches = remote.stat_batches.lock().unwrap();
        (
            batches.observed.clone(),
            batches.peak,
            batches.groups.is_empty(),
        )
    };
    transfers.release_tree_plan("main", plan.token).unwrap();
    fixture.close().await;
    let result = result
        .expect("tree preflight serialized independent peer metadata")
        .unwrap();
    let pending = result.plan.unwrap();
    assert_eq!(result.queued, 0);
    assert_eq!(pending.conflicts.len(), 1);
    assert_eq!(pending.conflicts[0].relative, "tree/blocked");
    assert!(drained);
    assert_eq!(peak, 3);
    assert_eq!(observed.len(), 6);
    assert_eq!(&observed[..2], ["/files/tree", "/files/tree"]);
    assert!(!observed
        .iter()
        .any(|path| path.starts_with("/files/tree/blocked/")));
    assert!(transfers.snapshot("main").unwrap().is_empty());
}
#[tokio::test]
async fn tree_preflight_bounds_a_wide_level_and_never_looks_under_a_blocked_parent() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    directory(remote, "/files/tree");
    remote.put("/files/tree/node-05", b"preserve type conflict");
    let local = tempfile::tempdir().unwrap();
    let root = local.path().join("tree");
    for index in 0..17 {
        let child = root.join(format!("node-{index:02}"));
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(child.join("file"), b"source").unwrap();
    }
    let plan = transfers
        .prepare_upload_tree(
            &fixture.service,
            "main",
            "one",
            session,
            RemotePath::parse("/files").unwrap(),
            root,
        )
        .await
        .unwrap();
    assert_eq!(plan.conflicts.len(), 1);
    {
        let mut batches = remote.stat_batches.lock().unwrap();
        batches.groups = [1, 1, 16, 1, 16].into();
        batches.observed.clear();
    }
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        transfers.resolve_tree_plan(
            &fixture.service,
            "main",
            "one",
            session,
            plan.token,
            plan.revision,
            vec![TreeDecision {
                entry: plan.conflicts[0].entry,
                decision: TreeChoice::Merge,
            }],
        ),
    )
    .await;
    let (observed, peak, drained) = {
        let batches = remote.stat_batches.lock().unwrap();
        (
            batches.observed.clone(),
            batches.peak,
            batches.groups.is_empty(),
        )
    };
    transfers.release_tree_plan("main", plan.token).unwrap();
    fixture.close().await;
    let result = result.expect("wide peers were serialized").unwrap();
    let pending = result.plan.unwrap();
    assert_eq!(result.queued, 0);
    assert_eq!(pending.conflicts.len(), 1);
    assert_eq!(pending.conflicts[0].relative, "tree/node-05");
    assert!(drained);
    assert_eq!(peak, 16);
    assert_eq!(observed.len(), 35);
    assert_eq!(&observed[..2], ["/files/tree", "/files/tree"]);
    assert!(!observed
        .iter()
        .any(|path| path.starts_with("/files/tree/node-05/")));
    assert!(transfers.snapshot("main").unwrap().is_empty());
}

#[tokio::test]
async fn upload_tree_renames_a_conflicting_directory_without_touching_existing_data() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    remote.put("/files/tree", b"preserved file");
    let local = tempfile::tempdir().unwrap();
    let root = local.path().join("tree");
    std::fs::create_dir_all(root.join("empty")).unwrap();
    std::fs::write(root.join("file"), b"new bytes").unwrap();
    let plan = transfers
        .prepare_upload_tree(
            &fixture.service,
            "main",
            "one",
            session,
            RemotePath::parse("/files").unwrap(),
            root,
        )
        .await
        .unwrap();
    assert_eq!(plan.conflicts.len(), 1);
    assert!(transfers.snapshot("main").unwrap().is_empty());
    assert!(!remote.files.lock().unwrap().contains_key("/files/renamed"));
    let result = transfers
        .resolve_tree_plan(
            &fixture.service,
            "main",
            "one",
            session,
            plan.token,
            plan.revision,
            vec![TreeDecision {
                entry: plan.conflicts[0].entry,
                decision: TreeChoice::Rename {
                    name: "renamed".into(),
                },
            }],
        )
        .await
        .unwrap();
    assert!(result.plan.is_none());
    assert_eq!(result.queued, 1);
    for row in transfers.snapshot("main").unwrap() {
        assert_eq!(
            finished(&transfers, row.id).await.state,
            TransferState::Completed
        );
    }
    assert_eq!(remote.bytes("/files/tree"), b"preserved file");
    assert_eq!(remote.bytes("/files/renamed/file"), b"new bytes");
    assert_eq!(
        remote.files.lock().unwrap()["/files/renamed/empty"].mode & 0o170000,
        0o40000
    );
    fixture.close().await;
}
#[tokio::test]
async fn download_tree_resolves_windows_names_case_collisions_and_skips_whole_subtrees() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    for path in ["/files/tree", "/files/tree/empty", "/files/tree/skip"] {
        directory(remote, path);
    }
    for (path, bytes) in [
        ("/files/tree/bad:name", b"invalid name".as_slice()),
        ("/files/tree/A", b"upper"),
        ("/files/tree/a", b"lower"),
        ("/files/tree/skip/file", b"skip contents"),
    ] {
        remote.put(path, bytes);
    }
    let local = tempfile::tempdir().unwrap();
    std::fs::create_dir(local.path().join("tree")).unwrap();
    std::fs::write(local.path().join("tree/skip"), b"existing local file").unwrap();
    let page = fixture.list(session, "/files").await.unwrap();
    let entry = page
        .entries
        .iter()
        .find(|entry| entry.name == "tree")
        .unwrap()
        .id;
    let plan = transfers
        .prepare_download_tree(
            &fixture.service,
            "main",
            "one",
            session,
            entry,
            local.path().to_owned(),
        )
        .await
        .unwrap();
    assert_eq!(plan.conflicts.len(), 1);
    let plan = transfers
        .resolve_tree_plan(
            &fixture.service,
            "main",
            "one",
            session,
            plan.token,
            plan.revision,
            vec![TreeDecision {
                entry: plan.conflicts[0].entry,
                decision: TreeChoice::Merge,
            }],
        )
        .await
        .unwrap()
        .plan
        .unwrap();
    assert_eq!(plan.conflicts.len(), 4);
    assert!(transfers.snapshot("main").unwrap().is_empty());
    assert!(!local.path().join("tree/empty").exists());
    let decisions = plan
        .conflicts
        .iter()
        .map(|item| TreeDecision {
            entry: item.entry,
            decision: if item.relative.ends_with("bad:name") {
                TreeChoice::Rename {
                    name: "good.bin".into(),
                }
            } else if item.relative.ends_with("/a") {
                TreeChoice::Rename {
                    name: "lower.bin".into(),
                }
            } else if item.relative.ends_with("/A") {
                TreeChoice::Rename {
                    name: "upper.bin".into(),
                }
            } else {
                TreeChoice::Skip
            },
        })
        .collect();
    let result = transfers
        .resolve_tree_plan(
            &fixture.service,
            "main",
            "one",
            session,
            plan.token,
            plan.revision,
            decisions,
        )
        .await
        .unwrap();
    assert!(result.plan.is_none());
    assert_eq!(result.queued, 3);
    for row in transfers.snapshot("main").unwrap() {
        assert_eq!(
            finished(&transfers, row.id).await.state,
            TransferState::Completed
        );
    }
    assert_eq!(
        std::fs::read(local.path().join("tree/good.bin")).unwrap(),
        b"invalid name"
    );
    assert_eq!(
        std::fs::read(local.path().join("tree/upper.bin")).unwrap(),
        b"upper"
    );
    assert_eq!(
        std::fs::read(local.path().join("tree/lower.bin")).unwrap(),
        b"lower"
    );
    assert_eq!(
        std::fs::read(local.path().join("tree/skip")).unwrap(),
        b"existing local file"
    );
    assert!(local.path().join("tree/empty").is_dir());
    fixture.close().await;
}
#[tokio::test]
async fn download_tree_preserves_empty_hidden_files_in_unicode_subdirectories() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    for path in ["/files/tree", "/files/tree/中文 space", "/files/tree/empty"] {
        directory(remote, path);
    }
    for (path, bytes) in [
        ("/files/tree/中文 space/.hidden-0.bin", b"".as_slice()),
        ("/files/tree/中文 space/.hidden-16.bin", b"bytes"),
        ("/files/tree/file 1.bin", b"one"),
    ] {
        remote.put(path, bytes);
    }
    let local = tempfile::tempdir().unwrap();
    let page = fixture.list(session, "/files").await.unwrap();
    let entry = page
        .entries
        .iter()
        .find(|entry| entry.name == "tree")
        .unwrap()
        .id;
    let plan = transfers
        .prepare_download_tree(
            &fixture.service,
            "main",
            "one",
            session,
            entry,
            local.path().to_owned(),
        )
        .await
        .unwrap();
    assert!(plan.conflicts.is_empty());
    let result = transfers
        .resolve_tree_plan(
            &fixture.service,
            "main",
            "one",
            session,
            plan.token,
            plan.revision,
            vec![],
        )
        .await
        .unwrap();
    assert!(result.plan.is_none());
    assert_eq!(result.queued, 3);
    for row in transfers.snapshot("main").unwrap() {
        let status = finished(&transfers, row.id).await;
        assert_eq!(status.state, TransferState::Completed, "{status:?}");
    }
    assert_eq!(
        std::fs::read(local.path().join("tree/中文 space/.hidden-0.bin")).unwrap(),
        b""
    );
    assert_eq!(
        std::fs::read(local.path().join("tree/中文 space/.hidden-16.bin")).unwrap(),
        b"bytes"
    );
    assert_eq!(
        std::fs::read(local.path().join("tree/file 1.bin")).unwrap(),
        b"one"
    );
    assert!(local.path().join("tree/empty").is_dir());
    fixture.close().await;
}
#[tokio::test]
async fn changed_directory_reissues_the_plan_and_old_revisions_cannot_merge_it() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    directory(remote, "/files/tree");
    let local = tempfile::tempdir().unwrap();
    let root = local.path().join("tree");
    std::fs::create_dir(&root).unwrap();
    let file = root.join("file");
    std::fs::write(&file, b"source").unwrap();
    let plan = transfers
        .prepare_upload_tree(
            &fixture.service,
            "main",
            "one",
            session,
            RemotePath::parse("/files").unwrap(),
            root,
        )
        .await
        .unwrap();
    let decision = || {
        vec![TreeDecision {
            entry: plan.conflicts[0].entry,
            decision: TreeChoice::Merge,
        }]
    };
    remote
        .files
        .lock()
        .unwrap()
        .get_mut("/files/tree")
        .unwrap()
        .mtime += 1;
    let changed = transfers
        .resolve_tree_plan(
            &fixture.service,
            "main",
            "one",
            session,
            plan.token,
            plan.revision,
            decision(),
        )
        .await
        .unwrap()
        .plan
        .unwrap();
    assert!(changed.revision.0 > plan.revision.0);
    assert_eq!(changed.conflicts.len(), 1);
    assert_eq!(
        transfers
            .resolve_tree_plan(
                &fixture.service,
                "main",
                "one",
                session,
                plan.token,
                plan.revision,
                decision()
            )
            .await
            .unwrap_err(),
        SftpError::ScopeExpired
    );
    assert_eq!(
        transfers.release_tree_plan("other", plan.token),
        Err(SftpError::ScopeExpired)
    );
    transfers.release_tree_plan("main", plan.token).unwrap();
    std::fs::write(&file, b"released").unwrap();
    assert!(!remote
        .files
        .lock()
        .unwrap()
        .contains_key("/files/tree/file"));
    assert!(transfers.snapshot("main").unwrap().is_empty());
    fixture.close().await;
}

#[tokio::test]
async fn missing_tree_parents_are_still_created_in_ancestor_order() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    remote.tree_parent_batch.lock().unwrap().canonical_missing = true;
    let local = tempfile::tempdir().unwrap();
    let source = local.path().join("source");
    std::fs::write(&source, b"new nested payload").unwrap();
    let context = fixture
        .service
        .transfer_context(
            "main",
            "one",
            session,
            TransferTarget::UploadTree {
                parent: RemotePath::parse("/files").unwrap(),
                relative: "new/child/file".into(),
            },
        )
        .unwrap();
    let selection = transfers
        .grant_source(
            &context,
            Arc::new(capstan_native_windows::transfer::Source::open(&source).unwrap()),
        )
        .unwrap();
    let id = transfers.enqueue(context, selection.token).unwrap();
    let row = finished(&transfers, id).await;
    assert_eq!(row.state, TransferState::Completed);
    assert_eq!(remote.bytes("/files/new/child/file"), b"new nested payload");
    assert_eq!(
        remote.tree_parent_batch.lock().unwrap().created,
        ["/files/new", "/files/new/child"]
    );
    #[cfg(feature = "benchmarks")]
    assert_eq!(transfers.buffer_usage().0, 0);
    fixture.close().await;
}

#[tokio::test]
async fn tree_publication_joins_reversed_ancestor_snapshots_after_canonical_validation() {
    tree_parent_snapshot_case(None, false).await;
}

#[tokio::test]
async fn tree_publication_joins_denied_ancestor_snapshots_without_publishing() {
    tree_parent_snapshot_case(Some(SftpError::PermissionDenied), false).await;
}

#[tokio::test]
async fn tree_publication_rejects_an_ancestor_replaced_after_staging() {
    tree_parent_snapshot_case(Some(SftpError::TargetConflict), true).await;
}

async fn tree_parent_snapshot_case(expected_error: Option<SftpError>, replaced: bool) {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    for path in ["/files/tree", "/files/tree/sub"] {
        directory(remote, path);
    }
    let local = tempfile::tempdir().unwrap();
    let root = local.path().join("tree");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/file"), b"verified tree payload").unwrap();
    {
        let mut batch = remote.tree_parent_batch.lock().unwrap();
        batch.paths = ["/files/tree".to_owned(), "/files/tree/sub".to_owned()].into();
        if replaced {
            batch.replaced = Some("/files/tree/sub".to_owned());
        } else if expected_error.is_some() {
            batch.denied = Some("/files/tree/sub".to_owned());
        }
    }
    let mut plan = transfers
        .prepare_upload_tree(
            &fixture.service,
            "main",
            "one",
            session,
            RemotePath::parse("/files").unwrap(),
            root,
        )
        .await
        .unwrap();
    let mut decisions = 0;
    loop {
        let choices = plan
            .conflicts
            .iter()
            .map(|conflict| {
                assert!(conflict.directory);
                TreeDecision {
                    entry: conflict.entry,
                    decision: TreeChoice::Merge,
                }
            })
            .collect();
        let result = transfers
            .resolve_tree_plan(
                &fixture.service,
                "main",
                "one",
                session,
                plan.token,
                plan.revision,
                choices,
            )
            .await
            .unwrap();
        if let Some(next) = result.plan {
            decisions += 1;
            assert!(decisions < 4);
            plan = next;
        } else {
            assert_eq!(result.queued, 1);
            break;
        }
    }
    let id = transfers.snapshot("main").unwrap()[0].id;
    let result = tokio::time::timeout(Duration::from_secs(3), finished(&transfers, id)).await;
    let (observed, canonical, peak) = {
        let batch = remote.tree_parent_batch.lock().unwrap();
        (batch.observed.clone(), batch.canonical.clone(), batch.peak)
    };
    let published = remote
        .files
        .lock()
        .unwrap()
        .get("/files/tree/sub/file")
        .map(|item| item.data.clone());
    let commits = remote.commits.load(Ordering::SeqCst);
    fixture.close().await;
    let row = result.expect("ancestor snapshots serialized instead of joining the admitted group");
    assert_eq!(observed, ["/files/tree", "/files/tree/sub"]);
    assert_eq!(
        canonical.first().map(String::as_str),
        Some("/files/tree/sub")
    );
    assert_eq!(peak, 2);
    if let Some(error) = expected_error {
        assert_eq!(row.state, TransferState::Failed);
        assert_eq!(row.error, Some(error));
        assert!(published.is_none());
        assert_eq!(commits, 0);
    } else {
        assert_eq!(row.state, TransferState::Completed);
        assert_eq!(
            published.as_deref(),
            Some(b"verified tree payload".as_slice())
        );
        assert_eq!(commits, 1);
    }
    #[cfg(feature = "benchmarks")]
    assert_eq!(transfers.buffer_usage().0, 0);
}
