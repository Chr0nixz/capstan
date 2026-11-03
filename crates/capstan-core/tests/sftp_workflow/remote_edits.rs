//! Production editor publication against the same fault-injectable SSH peer.
use super::*;
use capstan_core::remote_edit::{
    apply_remote_bytes, apply_remote_bytes_tracked, read_remote_file, EditError,
    PublicationBaseline, TemporaryLedger,
};
use sha2::{Digest, Sha256};

async fn edit_context(
    fixture: &Fixture,
    session: SftpSessionId,
) -> capstan_core::sftp::RemoteFileContext {
    let page = fixture.list(session, "/files").await.unwrap();
    let entry = page
        .entries
        .iter()
        .find(|entry| entry.name == "notes.txt")
        .unwrap();
    fixture
        .service
        .remote_file_context("main", "one", session, entry.id)
        .unwrap()
}
fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[cfg(windows)]
#[tokio::test]
async fn editor_launch_failure_retains_the_copy_and_retry_does_not_fake_a_launch() {
    let (fixture, session, transfers) = fixture().await;
    fixture
        .observations
        .transfer
        .put("/files/notes.txt", b"original");
    let context = edit_context(&fixture, session).await;
    let root = tempfile::tempdir().unwrap();
    let service = capstan_core::remote_edit::RemoteEditorService::new(
        Arc::new(capstan_core::settings::Settings::new(
            root.path().to_owned(),
        )),
        capstan_native_windows::editor::EditorStore::new(root.path().join("editor")),
        transfers.publication_gate(),
    );
    let config = service
        .configure("main", root.path().join("missing-editor.exe"))
        .await
        .unwrap();
    let view = service.begin("main", context, config).await.unwrap();
    assert_eq!(view.state, capstan_core::remote_edit::EditState::Residual);
    assert_eq!(view.error, Some(EditError::EditorUnavailable));
    assert_eq!(
        service.retry("main", view.id).await.unwrap_err(),
        EditError::EditorUnavailable
    );
    assert!(service.has_open("main"));
    service
        .close(
            "main",
            view.id,
            capstan_core::remote_edit::ResidualAction::Discard,
            None,
        )
        .await
        .unwrap();
    assert!(!service.has_open("main"));
    drop(service);
    fixture.close().await;
}

#[tokio::test]
async fn editor_atomic_publication_preserves_encoding_mode_and_shared_connection() {
    let (fixture, session, transfers) = fixture().await;
    let original = b"\xEF\xBB\xBFfirst\r\n";
    let edited = b"\xEF\xBB\xBFedited\r\n";
    fixture
        .observations
        .transfer
        .put("/files/notes.txt", original);
    fixture
        .observations
        .transfer
        .files
        .lock()
        .unwrap()
        .get_mut("/files/notes.txt")
        .unwrap()
        .mode = 0o100640;
    let context = edit_context(&fixture, session).await;
    let cancel = CancellationToken::new();
    let (bytes, metadata) = read_remote_file(
        &context.client,
        &context.path,
        Some(&context.metadata),
        &cancel,
    )
    .await
    .unwrap();
    assert_eq!(bytes, original);
    let result = apply_remote_bytes(
        &context.client,
        &context.path,
        edited,
        Some(&metadata),
        Some(hash(original)),
        &transfers.publication_gate(),
        &cancel,
    )
    .await
    .unwrap();
    assert_eq!(result.permissions, Some(0o100640));
    assert_eq!(
        fixture.observations.transfer.bytes("/files/notes.txt"),
        edited
    );
    assert!(fixture.observations.transfer.no_partials());
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observations.handles.load(Ordering::SeqCst), 0);
    drop(context);
    fixture.close().await;
}

#[tokio::test]
async fn editor_remote_changes_during_staging_require_a_fresh_conflict_decision() {
    let (fixture, session, transfers) = fixture().await;
    fixture
        .observations
        .transfer
        .put("/files/notes.txt", b"original");
    let context = edit_context(&fixture, session).await;
    fixture
        .observations
        .transfer
        .stall_fsync
        .store(true, Ordering::SeqCst);
    let client = context.client.clone();
    let path = context.path.clone();
    let metadata = context.metadata.clone();
    let gate = transfers.publication_gate();
    let publication = tokio::spawn(async move {
        apply_remote_bytes(
            &client,
            &path,
            b"local changes",
            Some(&metadata),
            Some(hash(b"original")),
            &gate,
            &CancellationToken::new(),
        )
        .await
    });
    fixture.observations.transfer.started.notified().await;
    fixture
        .observations
        .transfer
        .put("/files/notes.txt", b"concurrent remote changes");
    fixture
        .observations
        .transfer
        .stall_fsync
        .store(false, Ordering::SeqCst);
    fixture.observations.transfer.release.notify_one();
    assert_eq!(publication.await.unwrap(), Err(EditError::Conflict));
    assert_eq!(
        fixture.observations.transfer.bytes("/files/notes.txt"),
        b"concurrent remote changes"
    );
    assert!(fixture.observations.transfer.no_partials());
    assert_eq!(
        fixture.observations.transfer.commits.load(Ordering::SeqCst),
        0
    );
    drop(context);
    fixture.close().await;
}

#[tokio::test]
async fn editor_hash_catches_same_size_and_timestamp_remote_changes() {
    let (fixture, session, transfers) = fixture().await;
    fixture
        .observations
        .transfer
        .put("/files/notes.txt", b"original");
    let context = edit_context(&fixture, session).await;
    for external in [&b"external"[..], &b"\0bad\0raw"[..]] {
        fixture
            .observations
            .transfer
            .files
            .lock()
            .unwrap()
            .get_mut("/files/notes.txt")
            .unwrap()
            .data = external.to_vec();
        let result = apply_remote_bytes(
            &context.client,
            &context.path,
            b"local",
            Some(&context.metadata),
            Some(hash(b"original")),
            &transfers.publication_gate(),
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(result, Err(EditError::Conflict));
        assert_eq!(
            fixture.observations.transfer.bytes("/files/notes.txt"),
            external
        );
    }
    assert!(fixture.observations.transfer.no_partials());
    drop(context);
    fixture.close().await;
}

#[tokio::test]
async fn editor_failed_write_commit_and_readback_preserve_the_original() {
    for fault in [0, 1, 2] {
        let (fixture, session, transfers) = fixture().await;
        fixture
            .observations
            .transfer
            .put("/files/notes.txt", b"original");
        let context = edit_context(&fixture, session).await;
        match fault {
            0 => fixture
                .observations
                .transfer
                .deny_write
                .store(true, Ordering::SeqCst),
            1 => fixture
                .observations
                .transfer
                .deny_commit
                .store(true, Ordering::SeqCst),
            _ => fixture
                .observations
                .transfer
                .corrupt_read
                .store(true, Ordering::SeqCst),
        }
        let mut ledger = TemporaryLedger::default();
        assert!(apply_remote_bytes_tracked(
            &context.client,
            &context.path,
            b"local",
            PublicationBaseline {
                metadata: Some(&context.metadata),
                hash: Some(hash(b"original"))
            },
            &transfers.publication_gate(),
            &CancellationToken::new(),
            &mut ledger,
        )
        .await
        .is_err());
        assert_eq!(
            fixture.observations.transfer.bytes("/files/notes.txt"),
            b"original"
        );
        if fault == 2 {
            assert_eq!(ledger.views().len(), 1);
            assert!(!fixture.observations.transfer.no_partials());
            fixture
                .observations
                .transfer
                .corrupt_read
                .store(false, Ordering::SeqCst);
            ledger
                .cleanup(&context.client, &CancellationToken::new())
                .await
                .unwrap();
        }
        assert!(ledger.is_empty());
        assert!(fixture.observations.transfer.no_partials());
        drop(context);
        fixture.close().await;
    }
}

#[tokio::test]
async fn editor_without_atomic_replace_can_only_publish_to_an_absent_new_name() {
    let fixture = Fixture::new().await;
    fixture
        .observations
        .transfer_mode
        .store(true, Ordering::SeqCst);
    fixture
        .observations
        .transfer
        .without_extensions
        .store(true, Ordering::SeqCst);
    fixture
        .observations
        .transfer
        .put("/files/notes.txt", b"original");
    let session = fixture.attach("one").await;
    let context = edit_context(&fixture, session).await;
    let gate = tokio::sync::Mutex::new(());
    let cancel = CancellationToken::new();
    assert_eq!(
        apply_remote_bytes(
            &context.client,
            &context.path,
            b"local",
            Some(&context.metadata),
            Some(hash(b"original")),
            &gate,
            &cancel
        )
        .await,
        Err(EditError::Conflict)
    );
    let new_path = RemotePath::parse("/files/new.txt").unwrap();
    apply_remote_bytes(
        &context.client,
        &new_path,
        b"local",
        None,
        None,
        &gate,
        &cancel,
    )
    .await
    .unwrap();
    assert_eq!(
        fixture.observations.transfer.bytes("/files/new.txt"),
        b"local"
    );
    assert_eq!(
        fixture.observations.transfer.bytes("/files/notes.txt"),
        b"original"
    );
    assert!(fixture.observations.transfer.no_partials());
    drop(context);
    fixture.close().await;
}

#[tokio::test]
async fn editor_sessions_share_the_final_commit_gate_and_reject_stale_baselines() {
    let (fixture, session, transfers) = fixture().await;
    fixture
        .observations
        .transfer
        .put("/files/notes.txt", b"original");
    let context = edit_context(&fixture, session).await;
    let gate = transfers.publication_gate();
    let cancel = CancellationToken::new();
    let (first, second) = tokio::join!(
        apply_remote_bytes(
            &context.client,
            &context.path,
            b"first local",
            Some(&context.metadata),
            Some(hash(b"original")),
            &gate,
            &cancel
        ),
        apply_remote_bytes(
            &context.client,
            &context.path,
            b"second local",
            Some(&context.metadata),
            Some(hash(b"original")),
            &gate,
            &cancel
        ),
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert!(first == Err(EditError::Conflict) || second == Err(EditError::Conflict));
    assert_eq!(
        fixture.observations.transfer.commits.load(Ordering::SeqCst),
        1
    );
    assert!(fixture.observations.transfer.no_partials());
    drop(context);
    fixture.close().await;
}

#[tokio::test]
async fn editor_cleanup_retains_replaced_temporaries_and_bounds_the_ledger() {
    let (fixture, session, transfers) = fixture().await;
    let state = &fixture.observations.transfer;
    state.put("/files/notes.txt", b"original");
    let context = edit_context(&fixture, session).await;
    state.deny_commit.store(true, Ordering::SeqCst);
    state.replace_denied_temporary.store(true, Ordering::SeqCst);
    let mut ledger = TemporaryLedger::default();
    for _ in 0..capstan_core::remote_edit::MAX_EDITOR_TEMPORARIES {
        assert_eq!(
            apply_remote_bytes_tracked(
                &context.client,
                &context.path,
                b"local changes",
                PublicationBaseline {
                    metadata: Some(&context.metadata),
                    hash: Some(hash(b"original"))
                },
                &transfers.publication_gate(),
                &CancellationToken::new(),
                &mut ledger,
            )
            .await,
            Err(EditError::PermissionDenied)
        );
    }
    assert_eq!(ledger.views().len(), 8);
    for entry in ledger.views() {
        assert_eq!(state.bytes(&entry.path), b"external replacement");
    }
    assert_eq!(
        ledger
            .cleanup(&context.client, &CancellationToken::new())
            .await,
        Err(EditError::Residual)
    );
    assert_eq!(
        apply_remote_bytes_tracked(
            &context.client,
            &context.path,
            b"local changes",
            PublicationBaseline {
                metadata: Some(&context.metadata),
                hash: Some(hash(b"original"))
            },
            &transfers.publication_gate(),
            &CancellationToken::new(),
            &mut ledger,
        )
        .await,
        Err(EditError::ResourceLimit)
    );
    assert_eq!(state.bytes("/files/notes.txt"), b"original");
    // Inspection/removal is an external user action. Missing paths can then be
    // retired without adopting the replacements as a new deletion baseline.
    for entry in ledger.views() {
        state.files.lock().unwrap().remove(&entry.path);
    }
    ledger
        .cleanup(&context.client, &CancellationToken::new())
        .await
        .unwrap();
    assert!(ledger.is_empty());
    assert!(state.no_partials());
    drop(context);
    fixture.close().await;
}

#[tokio::test]
async fn editor_detects_temporary_content_changes_before_rename_even_with_equal_attributes() {
    let (fixture, session, transfers) = fixture().await;
    let state = &fixture.observations.transfer;
    state.put("/files/notes.txt", b"original");
    let context = edit_context(&fixture, session).await;
    state.stall_fsync.store(true, Ordering::SeqCst);
    let client = context.client.clone();
    let path = context.path.clone();
    let expected = context.metadata.clone();
    let commits = transfers.publication_gate();
    let write = tokio::spawn(async move {
        let mut ledger = TemporaryLedger::default();
        let result = apply_remote_bytes_tracked(
            &client,
            &path,
            b"local",
            PublicationBaseline {
                metadata: Some(&expected),
                hash: Some(hash(b"original")),
            },
            &commits,
            &CancellationToken::new(),
            &mut ledger,
        )
        .await;
        (result, ledger)
    });
    state.started.notified().await;
    {
        let mut files = state.files.lock().unwrap();
        let (_, temporary) = files
            .iter_mut()
            .find(|(path, _)| path.contains(".partial"))
            .unwrap();
        temporary.data.copy_from_slice(b"other");
    }
    state.stall_fsync.store(false, Ordering::SeqCst);
    state.release.notify_one();
    let (result, mut ledger) = write.await.unwrap();
    assert_eq!(result, Err(EditError::Residual));
    assert_eq!(ledger.views().len(), 1);
    assert_eq!(state.bytes(&ledger.views()[0].path), b"other");
    assert_eq!(state.bytes("/files/notes.txt"), b"original");
    assert_eq!(state.commits.load(Ordering::SeqCst), 0);
    assert_eq!(
        ledger
            .cleanup(&context.client, &CancellationToken::new())
            .await,
        Err(EditError::Residual)
    );
    drop(context);
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn editor_explicit_retry_uses_a_new_verified_session_but_keeps_the_old_baseline() {
    use capstan_core::remote_edit::{
        ConflictDecision, EditState, RemoteEditorService, ResidualAction,
    };
    let (fixture, session, transfers) = fixture().await;
    fixture
        .observations
        .transfer
        .put("/files/notes.txt", b"original");
    let root = tempfile::tempdir().unwrap();
    let service = RemoteEditorService::new(
        Arc::new(capstan_core::settings::Settings::new(
            root.path().to_owned(),
        )),
        capstan_native_windows::editor::EditorStore::new(root.path().join("editor")),
        transfers.publication_gate(),
    );
    // The test harness receives a nonmatching test filter and exits. No real
    // editor or user document is launched by this integration test.
    let config = service
        .configure("main", std::env::current_exe().unwrap())
        .await
        .unwrap();
    let edit = service
        .begin("main", edit_context(&fixture, session).await, config)
        .await
        .unwrap();
    service.cancel_window("main");
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if service.snapshot("main").unwrap().sessions[0].error
                == Some(EditError::ConnectionLost)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let local = root
        .path()
        .join("editor")
        .join(edit.id.to_string())
        .join("notes.txt");
    std::fs::write(&local, b"local after reconnect").unwrap();
    fixture
        .observations
        .transfer
        .put("/files/notes.txt", b"\0\xFFremote binary changes");
    let mut connector = fixture.connector.clone();
    connector.pool_key.vault_epoch += 1;
    connector.binding.vault_epoch += 1;
    let lease = connector.acquire(&fixture.pool).await;
    let shell = lease.open_shell(80, 24).await.unwrap();
    fixture
        .service
        .register(
            "main",
            "new-editor",
            connector.binding.host_id,
            shell.connection_id(),
            lease.clone(),
        )
        .unwrap();
    let new_session = fixture.attach("new-editor").await;
    assert_eq!(
        service
            .bind_for_retry("main", edit.id, &fixture.service, "one", new_session)
            .await
            .unwrap_err(),
        EditError::ScopeExpired
    );
    service
        .bind_for_retry("main", edit.id, &fixture.service, "new-editor", new_session)
        .await
        .unwrap();
    service.retry("main", edit.id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if service.snapshot("main").unwrap().sessions[0].state == EditState::Conflict {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        fixture.observations.transfer.bytes("/files/notes.txt"),
        b"\0\xFFremote binary changes"
    );
    assert_eq!(std::fs::read(local).unwrap(), b"local after reconnect");
    service
        .resolve("main", edit.id, ConflictDecision::ReplaceRemote, None)
        .await
        .unwrap();
    assert_eq!(
        fixture.observations.transfer.bytes("/files/notes.txt"),
        b"local after reconnect"
    );
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 2);
    service
        .close("main", edit.id, ResidualAction::Discard, None)
        .await
        .unwrap();
    fixture.service.revoke_terminal("new-editor");
    shell.control.close().await.unwrap();
    drop(service);
    drop(lease);
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn editor_rebind_rejects_a_changed_credential_revision_on_the_same_host() {
    let (fixture, session, transfers) = fixture().await;
    fixture
        .observations
        .transfer
        .put("/files/notes.txt", b"original");
    let root = tempfile::tempdir().unwrap();
    let service = capstan_core::remote_edit::RemoteEditorService::new(
        Arc::new(capstan_core::settings::Settings::new(
            root.path().to_owned(),
        )),
        capstan_native_windows::editor::EditorStore::new(root.path().join("editor")),
        transfers.publication_gate(),
    );
    let config = service
        .configure("main", root.path().join("missing.exe"))
        .await
        .unwrap();
    let edit = service
        .begin("main", edit_context(&fixture, session).await, config)
        .await
        .unwrap();
    let mut connector = fixture.connector.clone();
    connector.pool_key.credential_revision += 1;
    let lease = connector.acquire(&fixture.pool).await;
    let shell = lease.open_shell(80, 24).await.unwrap();
    fixture
        .service
        .register(
            "main",
            "new-editor",
            connector.binding.host_id,
            shell.connection_id(),
            lease.clone(),
        )
        .unwrap();
    let new_session = fixture.attach("new-editor").await;
    assert_eq!(
        service
            .bind_for_retry("main", edit.id, &fixture.service, "new-editor", new_session)
            .await
            .unwrap_err(),
        EditError::ScopeExpired
    );
    assert_eq!(
        fixture.observations.transfer.bytes("/files/notes.txt"),
        b"original"
    );
    service
        .close(
            "main",
            edit.id,
            capstan_core::remote_edit::ResidualAction::Discard,
            None,
        )
        .await
        .unwrap();
    fixture.service.revoke_terminal("new-editor");
    shell.control.close().await.unwrap();
    drop(service);
    drop(lease);
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn editor_retains_visible_cleanup_evidence_across_rebind_and_blocks_destructive_close() {
    use capstan_core::remote_edit::{EditState, RemoteEditorService, ResidualAction};
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    remote.put("/files/notes.txt", b"original");
    let root = tempfile::tempdir().unwrap();
    let service = RemoteEditorService::new(
        Arc::new(capstan_core::settings::Settings::new(
            root.path().to_owned(),
        )),
        capstan_native_windows::editor::EditorStore::new(root.path().join("editor")),
        transfers.publication_gate(),
    );
    let config = service
        .configure("main", std::env::current_exe().unwrap())
        .await
        .unwrap();
    let edit = service
        .begin("main", edit_context(&fixture, session).await, config)
        .await
        .unwrap();
    remote.deny_commit.store(true, Ordering::SeqCst);
    remote
        .replace_denied_temporary
        .store(true, Ordering::SeqCst);
    let local = root
        .path()
        .join("editor")
        .join(edit.id.to_string())
        .join("notes.txt");
    std::fs::write(&local, b"unsent local changes").unwrap();
    let retained = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let view = service.snapshot("main").unwrap().sessions.remove(0);
            if view.state == EditState::Residual && !view.remote_temporaries.is_empty() {
                break view;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(retained.cleanup_required);
    let temporary = retained.remote_temporaries[0].path.clone();
    assert_eq!(remote.bytes(&temporary), b"external replacement");
    assert_eq!(
        service
            .close("main", edit.id, ResidualAction::Discard, None)
            .await,
        Err(EditError::Residual)
    );
    assert_eq!(std::fs::read(&local).unwrap(), b"unsent local changes");
    assert!(service.has_open("main"));
    drop(service);
    let service = RemoteEditorService::new(
        Arc::new(capstan_core::settings::Settings::new(
            root.path().to_owned(),
        )),
        capstan_native_windows::editor::EditorStore::new(root.path().join("editor")),
        transfers.publication_gate(),
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            service.discover("main").unwrap();
            if service
                .snapshot("main")
                .unwrap()
                .sessions
                .iter()
                .any(|view| view.id == edit.id)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let recovered = service.snapshot("main").unwrap().sessions.remove(0);
    assert!(recovered.recovered);
    assert_eq!(recovered.remote_temporaries[0].path, temporary);
    assert!(recovered.remote_path.is_none());
    let mut foreign = fixture.connector.clone();
    foreign.pool_key.credential_revision += 1;
    let foreign_lease = foreign.acquire(&fixture.pool).await;
    let foreign_shell = foreign_lease.open_shell(80, 24).await.unwrap();
    fixture
        .service
        .register(
            "main",
            "foreign-cleanup",
            foreign.binding.host_id,
            foreign_shell.connection_id(),
            foreign_lease.clone(),
        )
        .unwrap();
    let foreign_session = fixture.attach("foreign-cleanup").await;
    assert_eq!(
        service
            .bind_for_cleanup(
                "main",
                edit.id,
                &fixture.service,
                "foreign-cleanup",
                foreign_session
            )
            .await,
        Err(EditError::ScopeExpired)
    );
    assert_eq!(remote.bytes(&temporary), b"external replacement");
    fixture.service.revoke_terminal("foreign-cleanup");
    foreign_shell.control.close().await.unwrap();
    drop(foreign_lease);
    let mut connector = fixture.connector.clone();
    connector.pool_key.vault_epoch += 1;
    let lease = connector.acquire(&fixture.pool).await;
    let shell = lease.open_shell(80, 24).await.unwrap();
    fixture
        .service
        .register(
            "main",
            "cleanup-editor",
            connector.binding.host_id,
            shell.connection_id(),
            lease.clone(),
        )
        .unwrap();
    let new_session = fixture.attach("cleanup-editor").await;
    assert_eq!(
        service
            .bind_for_retry(
                "main",
                edit.id,
                &fixture.service,
                "cleanup-editor",
                new_session
            )
            .await
            .unwrap_err(),
        EditError::ScopeExpired
    );
    service
        .bind_for_cleanup(
            "main",
            edit.id,
            &fixture.service,
            "cleanup-editor",
            new_session,
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .cleanup_temporaries("main", edit.id)
            .await
            .unwrap_err(),
        EditError::Residual
    );
    assert_eq!(remote.bytes(&temporary), b"external replacement");
    remote.files.lock().unwrap().remove(&temporary);
    assert!(service
        .cleanup_temporaries("main", edit.id)
        .await
        .unwrap()
        .remote_temporaries
        .is_empty());
    service
        .close("main", edit.id, ResidualAction::Discard, None)
        .await
        .unwrap();
    assert!(!local.exists());
    assert!(!service.has_open("main"));
    fixture.service.revoke_terminal("cleanup-editor");
    shell.control.close().await.unwrap();
    drop(service);
    drop(lease);
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn recovered_lost_commit_ack_is_cleaned_without_replaying_the_upload() {
    use capstan_core::remote_edit::{EditState, RemoteEditorService, ResidualAction};
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    remote.put("/files/notes.txt", b"original");
    let root = tempfile::tempdir().unwrap();
    let service = RemoteEditorService::new(
        Arc::new(capstan_core::settings::Settings::new(
            root.path().to_owned(),
        )),
        capstan_native_windows::editor::EditorStore::new(root.path().join("editor")),
        transfers.publication_gate(),
    );
    let config = service
        .configure("main", std::env::current_exe().unwrap())
        .await
        .unwrap();
    let edit = service
        .begin("main", edit_context(&fixture, session).await, config)
        .await
        .unwrap();
    remote.lose_commit_ack.store(true, Ordering::SeqCst);
    let local = root
        .path()
        .join("editor")
        .join(edit.id.to_string())
        .join("notes.txt");
    std::fs::write(&local, b"committed without acknowledgement").unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if service.snapshot("main").unwrap().sessions[0].state == EditState::Residual {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        remote.bytes("/files/notes.txt"),
        b"committed without acknowledgement"
    );
    assert_eq!(remote.commits.load(Ordering::SeqCst), 1);
    service
        .close("main", edit.id, ResidualAction::KeepForRecovery, None)
        .await
        .unwrap();
    drop(service);
    let service = RemoteEditorService::new(
        Arc::new(capstan_core::settings::Settings::new(
            root.path().to_owned(),
        )),
        capstan_native_windows::editor::EditorStore::new(root.path().join("editor")),
        transfers.publication_gate(),
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            service.discover("main").unwrap();
            if !service.snapshot("main").unwrap().sessions.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let recovered = service.snapshot("main").unwrap().sessions.remove(0);
    assert!(recovered.recovered);
    assert_eq!(recovered.remote_temporaries.len(), 1);
    // Starting the service only discovers local records; no network replay.
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 1);
    let new_session = fixture.attach("two").await;
    service
        .bind_for_cleanup("main", edit.id, &fixture.service, "two", new_session)
        .await
        .unwrap();
    assert!(service
        .cleanup_temporaries("main", edit.id)
        .await
        .unwrap()
        .remote_temporaries
        .is_empty());
    assert_eq!(remote.commits.load(Ordering::SeqCst), 1);
    assert_eq!(
        remote.bytes("/files/notes.txt"),
        b"committed without acknowledgement"
    );
    service
        .close("main", edit.id, ResidualAction::Discard, None)
        .await
        .unwrap();
    assert!(!local.exists());
    drop(service);
    fixture.close().await;
}
