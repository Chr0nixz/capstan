use super::*;

#[tokio::test]
async fn concurrent_export_admission_never_leaves_an_orphan_stage() {
    let (fixture, session, transfers) = fixture().await;
    fixture.observations.transfer.put("/files/one.bin", b"one");
    let entry = fixture.list(session, "/files").await.unwrap().entries[0].id;
    let staging = tempfile::tempdir().unwrap();
    transfers
        .configure_export_directory(staging.path().to_owned())
        .unwrap();
    let prepare =
        || transfers.prepare_native_export(&fixture.service, "main", "one", session, entry);
    let (a, b, c, d, e, f, g, h, i) = tokio::join!(
        prepare(),
        prepare(),
        prepare(),
        prepare(),
        prepare(),
        prepare(),
        prepare(),
        prepare(),
        prepare()
    );
    let mut tokens = Vec::new();
    let mut rejected = 0;
    for result in [a, b, c, d, e, f, g, h, i] {
        match result {
            Ok(prepared) => tokens.push(prepared.export.token),
            Err(SftpError::ResourceLimit) => rejected += 1,
            Err(error) => panic!("unexpected export error: {error:?}"),
        }
    }
    assert_eq!(tokens.len(), 8);
    assert_eq!(rejected, 1);
    assert_eq!(std::fs::read_dir(staging.path()).unwrap().count(), 8);
    for token in tokens {
        transfers
            .release_native_export("main", token)
            .await
            .unwrap();
    }
    assert_eq!(std::fs::read_dir(staging.path()).unwrap().count(), 0);
    fixture.close().await;
}

async fn metadata_roundtrip(deny_parent: bool, replace_temporary: bool) {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    {
        let mut batch = remote.metadata_batch.lock().unwrap();
        batch.initial = true;
        batch.publication = true;
        batch.denied_final_parent = deny_parent;
        batch.replace_final_temporary = replace_temporary;
    }
    let local = tempfile::tempdir().unwrap();
    let source = local.path().join("source.bin");
    std::fs::write(&source, b"metadata snapshot fixture").unwrap();
    let id = enqueue_upload(&fixture, session, &transfers, &source, "metadata.bin");
    let completed = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let snapshot = transfers
                .snapshot("main")
                .unwrap()
                .into_iter()
                .find(|s| s.id == id)
                .unwrap();
            if matches!(
                snapshot.state,
                TransferState::Completed | TransferState::Failed
            ) {
                break snapshot;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    if completed.is_err() {
        // Closing the peer releases withheld test replies and joins the job.
        fixture.close().await;
        panic!("independent preparation/publication metadata was serialized");
    }
    let completed = completed.unwrap();
    let batches = remote.metadata_batch.lock().unwrap().observed.clone();
    assert_eq!(batches.iter().map(Vec::len).collect::<Vec<_>>(), [2, 3]);
    assert!(batches[1]
        .iter()
        .any(|(kind, path)| *kind == 16 && path == "/files"));
    assert!(batches[1]
        .iter()
        .any(|(kind, path)| *kind == 7 && path == "/files/metadata.bin"));
    assert!(batches[1]
        .iter()
        .any(|(kind, path)| *kind == 7 && path.ends_with(".partial")));
    if deny_parent || replace_temporary {
        assert_eq!(completed.state, TransferState::Failed);
        assert_eq!(remote.commits.load(Ordering::SeqCst), 0);
        assert!(!remote
            .files
            .lock()
            .unwrap()
            .contains_key("/files/metadata.bin"));
        if replace_temporary {
            assert!(remote
                .files
                .lock()
                .unwrap()
                .values()
                .any(|r| r.data == b"changed after full hash"));
        }
    } else {
        assert_eq!(completed.state, TransferState::Completed);
        assert_eq!(
            remote.bytes("/files/metadata.bin"),
            b"metadata snapshot fixture"
        );
        assert!(remote.no_partials());
    }
    fixture.close().await;
}

#[tokio::test]
async fn independent_metadata_snapshots_are_batched_with_reversed_replies() {
    metadata_roundtrip(false, false).await;
}

#[tokio::test]
async fn independent_metadata_snapshots_still_reject_a_denied_final_parent() {
    metadata_roundtrip(true, false).await;
}

#[tokio::test]
async fn independent_metadata_snapshots_still_reject_a_changed_verified_temporary() {
    metadata_roundtrip(false, true).await;
}

#[tokio::test]
async fn retry_rejects_foreign_hosts_without_changing_the_final_state_then_accepts_fresh_source() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    let local = tempfile::tempdir().unwrap();
    let source = local.path().join("source.bin");
    std::fs::write(&source, b"retry bytes").unwrap();
    remote.deny_write.store(true, Ordering::SeqCst);
    let id = enqueue_upload(&fixture, session, &transfers, &source, "retry.bin");
    let failed = finished(&transfers, id).await;
    assert_eq!(failed.state, TransferState::Failed);
    assert_eq!(transfers.retry("main", id), Err(SftpError::ScopeExpired));
    assert_eq!(finished(&transfers, id).await.version, failed.version);
    let target = transfers.target("main", id).unwrap();
    fixture
        .service
        .register(
            "main",
            "foreign",
            Uuid::new_v4(),
            fixture.shells[0].connection_id(),
            fixture.lease.clone(),
        )
        .unwrap();
    let foreign = fixture.attach("foreign").await;
    let context = fixture
        .service
        .transfer_context_for_recovery("main", "foreign", foreign, target)
        .unwrap();
    let grant = transfers
        .grant_source(
            &context,
            Arc::new(capstan_native_windows::transfer::Source::open(&source).unwrap()),
        )
        .unwrap();
    assert_eq!(
        transfers.retry_with_selection("main", id, context, grant.token),
        Err(SftpError::ScopeExpired)
    );
    assert_eq!(finished(&transfers, id).await.version, failed.version);
    remote.deny_write.store(false, Ordering::SeqCst);
    let context = upload_context(&fixture, session, "retry.bin");
    let grant = transfers
        .grant_source(
            &context,
            Arc::new(capstan_native_windows::transfer::Source::open(&source).unwrap()),
        )
        .unwrap();
    transfers
        .retry_with_selection("main", id, context, grant.token)
        .unwrap();
    assert_eq!(
        finished(&transfers, id).await.state,
        TransferState::Completed
    );
    assert_eq!(remote.bytes("/files/retry.bin"), b"retry bytes");
    fixture.service.revoke_terminal("foreign");
    fixture.close().await;
}

#[tokio::test]
async fn native_receipts_are_owner_bound_single_use_and_upload_files_and_empty_directories() {
    let (fixture, session, transfers) = fixture().await;
    let local = tempfile::tempdir().unwrap();
    let file = local.path().join("中文 file.txt");
    std::fs::write(&file, b"native drop bytes").unwrap();
    let folder = local.path().join("tree");
    std::fs::create_dir_all(folder.join("empty")).unwrap();
    std::fs::write(folder.join("nested.txt"), b"nested bytes").unwrap();
    transfers
        .arm_native_drop(&upload_context(&fixture, session, "selection"))
        .unwrap();
    let token = transfers
        .capture_native_drop("main", vec![file, folder])
        .unwrap();
    let parent = RemotePath::parse("/files").unwrap();
    let second = fixture.attach("two").await;
    fixture
        .service
        .list("main", "two", second, parent.clone())
        .await
        .unwrap();
    assert_eq!(
        transfers
            .enqueue_native_drop(
                &fixture.service,
                "main",
                "two",
                second,
                parent.clone(),
                token
            )
            .await
            .unwrap_err(),
        SftpError::ScopeExpired
    );
    assert_eq!(
        transfers
            .enqueue_native_drop(
                &fixture.service,
                "other",
                "one",
                session,
                parent.clone(),
                token
            )
            .await
            .unwrap_err(),
        SftpError::ScopeExpired
    );
    let result = transfers
        .enqueue_native_drop(
            &fixture.service,
            "main",
            "one",
            session,
            parent.clone(),
            token,
        )
        .await
        .unwrap();
    assert_eq!(result.queued, 1);
    assert_eq!(result.plans.len(), 1);
    let plan = &result.plans[0];
    assert!(plan.conflicts.is_empty());
    assert_eq!(
        transfers
            .resolve_tree_plan(
                &fixture.service,
                "main",
                "one",
                session,
                plan.token,
                plan.revision,
                Vec::new()
            )
            .await
            .unwrap()
            .queued,
        1
    );
    assert_eq!(
        transfers
            .enqueue_native_drop(&fixture.service, "main", "one", session, parent, token)
            .await
            .unwrap_err(),
        SftpError::ScopeExpired
    );
    for task in transfers.snapshot("main").unwrap() {
        assert_eq!(
            finished(&transfers, task.id).await.state,
            TransferState::Completed
        );
    }
    let remote = &fixture.observations.transfer;
    assert_eq!(remote.bytes("/files/中文 file.txt"), b"native drop bytes");
    assert_eq!(remote.bytes("/files/tree/nested.txt"), b"nested bytes");
    assert_eq!(
        remote.files.lock().unwrap()["/files/tree/empty"].mode & 0o170000,
        0o40000
    );
    fixture.close().await;
}

#[tokio::test]
async fn export_tokens_bind_generation_retain_until_explicit_cleanup_and_recover_after_restart() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    remote.put("/files/tree", b"");
    remote
        .files
        .lock()
        .unwrap()
        .get_mut("/files/tree")
        .unwrap()
        .mode = 0o40700;
    remote.put("/files/tree/empty", b"");
    remote
        .files
        .lock()
        .unwrap()
        .get_mut("/files/tree/empty")
        .unwrap()
        .mode = 0o40700;
    remote.put("/files/tree/data.bin", b"export bytes");
    let page = fixture.list(session, "/files").await.unwrap();
    let entry = page
        .entries
        .iter()
        .find(|entry| entry.name == "tree")
        .unwrap()
        .id;
    let staging = tempfile::tempdir().unwrap();
    transfers
        .configure_export_directory(staging.path().to_owned())
        .unwrap();
    let export = transfers
        .prepare_native_export(&fixture.service, "main", "one", session, entry)
        .await
        .unwrap();
    assert!(export.plan.conflicts.is_empty());
    transfers
        .resolve_tree_plan(
            &fixture.service,
            "main",
            "one",
            session,
            export.plan.token,
            export.plan.revision,
            Vec::new(),
        )
        .await
        .unwrap();
    let export = export.export;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if transfers.export_snapshot("main").unwrap()[0].state == "ready" {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let root = staging.path().join(export.token.to_string());
    assert_eq!(
        std::fs::read(root.join("tree/data.bin")).unwrap(),
        b"export bytes"
    );
    assert!(root.join("tree/empty").is_dir());
    assert!(transfers
        .begin_export_drag(&fixture.service, "other", "one", session, export.token)
        .await
        .is_err());
    let drag = transfers
        .begin_export_drag(&fixture.service, "main", "one", session, export.token)
        .await
        .unwrap();
    assert_eq!(
        transfers.export_snapshot("main").unwrap()[0].state,
        "dragging"
    );
    assert!(transfers
        .release_native_export("main", export.token)
        .await
        .is_err());
    assert!(transfers
        .capture_native_drop("main", vec![root.join("tree")])
        .is_err());
    drop(drag); // Model Escape: staging is retained, never deleted by OLE completion.
    assert_eq!(transfers.export_snapshot("main").unwrap()[0].state, "ready");
    drop(transfers);
    let restarted = TransferService::default();
    restarted
        .configure_export_directory(staging.path().to_owned())
        .unwrap();
    assert_eq!(
        restarted.export_snapshot("main").unwrap()[0].state,
        "retained"
    );
    assert!(restarted
        .begin_export_drag(&fixture.service, "main", "one", session, export.token)
        .await
        .is_err());
    restarted
        .release_native_export("main", export.token)
        .await
        .unwrap();
    assert!(!root.exists());
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 1);
    fixture.close().await;
}

#[tokio::test]
async fn file_directory_collisions_allow_save_as_and_never_replace_a_directory() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    remote.put("/files/collision", b"");
    remote
        .files
        .lock()
        .unwrap()
        .get_mut("/files/collision")
        .unwrap()
        .mode = 0o40700;
    let local = tempfile::tempdir().unwrap();
    let source = local.path().join("source");
    std::fs::write(&source, b"new file").unwrap();
    let id = enqueue_upload(&fixture, session, &transfers, &source, "collision");
    let prompt = wait(&transfers, id, |status| status.conflict.is_some())
        .await
        .conflict
        .unwrap();
    assert!(!prompt.replace_allowed);
    assert_eq!(
        prompt.target.unwrap().kind,
        capstan_core::sftp::FileKind::Directory
    );
    assert!(transfers
        .resolve("main", id, prompt.revision, Decision::Replace)
        .is_err());
    transfers
        .resolve(
            "main",
            id,
            prompt.revision,
            Decision::Rename {
                name: "safe.bin".into(),
            },
        )
        .unwrap();
    assert_eq!(
        finished(&transfers, id).await.state,
        TransferState::Completed
    );
    assert_eq!(remote.bytes("/files/safe.bin"), b"new file");
    assert_eq!(
        remote.files.lock().unwrap()["/files/collision"].mode & 0o170000,
        0o40000
    );
    fixture.close().await;
}

#[tokio::test]
async fn explorer_export_uses_the_same_name_plan_for_unrepresentable_files_and_directories() {
    use capstan_core::transfer::{TreeChoice, TreeDecision};
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    let staging = tempfile::tempdir().unwrap();
    transfers
        .configure_export_directory(staging.path().to_owned())
        .unwrap();
    for (source, replacement, directory) in [
        ("bad:name", "renamed.bin", false),
        ("CON", "renamed-dir", true),
    ] {
        let path = format!("/files/{source}");
        remote.put(&path, b"exported");
        if directory {
            remote.files.lock().unwrap().get_mut(&path).unwrap().mode = 0o40700;
            remote.put(&format!("{path}/file"), b"exported");
        }
        let page = fixture.list(session, "/files").await.unwrap();
        let entry = page
            .entries
            .iter()
            .find(|entry| entry.name == source)
            .unwrap()
            .id;
        let prepared = transfers
            .prepare_native_export(&fixture.service, "main", "one", session, entry)
            .await
            .unwrap();
        assert_eq!(prepared.plan.conflicts.len(), 1);
        let result = transfers
            .resolve_tree_plan(
                &fixture.service,
                "main",
                "one",
                session,
                prepared.plan.token,
                prepared.plan.revision,
                vec![TreeDecision {
                    entry: prepared.plan.conflicts[0].entry,
                    decision: TreeChoice::Rename {
                        name: replacement.into(),
                    },
                }],
            )
            .await
            .unwrap();
        assert!(result.plan.is_none());
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if transfers
                    .export_snapshot("main")
                    .unwrap()
                    .iter()
                    .any(|item| item.token == prepared.export.token && item.state == "ready")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let root = staging.path().join(prepared.export.token.to_string());
        let file = if directory {
            root.join(replacement).join("file")
        } else {
            root.join(replacement)
        };
        assert_eq!(std::fs::read(file).unwrap(), b"exported");
        transfers
            .release_native_export("main", prepared.export.token)
            .await
            .unwrap();
        assert!(!root.exists());
    }
    fixture.close().await;
}

#[tokio::test]
async fn canceled_pipeline_keeps_payload_admission_until_exit_drains_the_owned_requests() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    let local = tempfile::tempdir().unwrap();
    let source = local.path().join("source.bin");
    std::fs::write(&source, vec![0x5a; 3 * 1024 * 1024]).unwrap();
    remote.stall_write.store(true, Ordering::SeqCst);
    let id = enqueue_upload(&fixture, session, &transfers, &source, "pending.bin");
    remote.started.notified().await;
    transfers.cancel("main", id).unwrap();
    assert_eq!(
        finished(&transfers, id).await.state,
        TransferState::Canceled
    );
    std::fs::write(&source, b"source handle released").unwrap();
    #[cfg(feature = "benchmarks")]
    assert!(transfers.buffer_usage().0 > 0);
    let service = transfers.clone();
    let exit = tokio::spawn(async move { service.prepare_exit("main").await });
    tokio::task::yield_now().await;
    assert!(!exit.is_finished());
    remote.stall_write.store(false, Ordering::SeqCst);
    remote.release.notify_one();
    tokio::time::timeout(Duration::from_secs(3), exit)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    #[cfg(feature = "benchmarks")]
    assert_eq!(transfers.buffer_usage().0, 0);
    assert!(!remote
        .files
        .lock()
        .unwrap()
        .contains_key("/files/pending.bin"));
    fixture.close().await;
}
