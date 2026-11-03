use super::*;
use capstan_core::transfer::{LocalDestination, WindowsDestination};

#[tokio::test]
async fn live_download_retry_reauthorizes_the_original_path_on_a_fresh_scope() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    let body = (0..1024 * 1024)
        .map(|n| (n % 251) as u8)
        .collect::<Vec<_>>();
    remote.put("/files/source.bin", &body);
    let local = tempfile::tempdir().unwrap();
    let page = fixture.list(session, "/files").await.unwrap();
    let entry = page
        .entries
        .iter()
        .find(|entry| entry.name == "source.bin")
        .unwrap();
    let context = fixture
        .service
        .transfer_context(
            "main",
            "one",
            session,
            TransferTarget::Download { entry: entry.id },
        )
        .unwrap();
    let target = local.path().join("returned.bin");
    let destination = Arc::new(WindowsDestination::new(
        capstan_native_windows::transfer::Destination::select(&target).unwrap(),
    ));
    let grant = transfers.grant_destination(&context, destination).unwrap();
    remote.stall_read.store(true, Ordering::SeqCst);
    let id = transfers.enqueue(context, grant.token).unwrap();
    tokio::time::timeout(Duration::from_secs(3), remote.started.notified())
        .await
        .unwrap();
    let partial = local.path().join(format!(".capstan-{id}.partial"));
    tokio::time::timeout(Duration::from_secs(3), async {
        // Progress publication is coalesced at 100 ms, so an early stall can
        // precede the first published progress even after native writes.
        while !std::fs::metadata(&partial).is_ok_and(|metadata| metadata.len() > 0) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    fixture
        .lease
        .open_sftp(&CancellationToken::new())
        .await
        .unwrap()
        .close()
        .await;
    let interrupted = finished(&transfers, id).await;
    assert_eq!(interrupted.state, TransferState::Interrupted);
    let retained = std::fs::metadata(&partial).unwrap().len();
    assert!(retained > 0 && retained < body.len() as u64);
    remote.stall_read.store(false, Ordering::SeqCst);
    remote.release.notify_waiters();
    let fresh = fixture.attach("one").await;
    let old_target = transfers.target("main", id).unwrap();
    assert!(matches!(
        fixture
            .service
            .transfer_context("main", "one", fresh, old_target),
        Err(SftpError::ScopeExpired)
    ));
    assert!(matches!(
        transfers.download_retry("other", id),
        Err(SftpError::ScopeExpired)
    ));
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
    assert!(matches!(
        fixture
            .service
            .transfer_context_for_recovered_download(
                "main",
                "foreign",
                foreign,
                transfers.download_retry("main", id).unwrap(),
            )
            .await,
        Err(SftpError::ScopeExpired)
    ));
    let mut changed = fixture.connector.clone();
    changed.pool_key.credential_revision += 1;
    let changed_lease = changed.acquire(&fixture.pool).await;
    let changed_shell = changed_lease.open_shell(80, 24).await.unwrap();
    fixture
        .service
        .register(
            "main",
            "changed",
            changed.binding.host_id,
            changed_shell.connection_id(),
            changed_lease,
        )
        .unwrap();
    let changed_session = fixture.attach("changed").await;
    assert!(matches!(
        fixture
            .service
            .transfer_context_for_recovered_download(
                "main",
                "changed",
                changed_session,
                transfers.download_retry("main", id).unwrap(),
            )
            .await,
        Err(SftpError::ScopeExpired)
    ));
    assert_eq!(finished(&transfers, id).await.version, interrupted.version);
    assert_eq!(std::fs::metadata(&partial).unwrap().len(), retained);
    let original_mtime = remote.files.lock().unwrap()["/files/source.bin"].mtime;
    remote.put("/files/source.bin", &body);
    assert!(matches!(
        fixture
            .service
            .transfer_context_for_recovered_download(
                "main",
                "one",
                fresh,
                transfers.download_retry("main", id).unwrap(),
            )
            .await,
        Err(SftpError::ResumeMismatch)
    ));
    assert_eq!(finished(&transfers, id).await.version, interrupted.version);
    assert_eq!(std::fs::metadata(&partial).unwrap().len(), retained);
    remote
        .files
        .lock()
        .unwrap()
        .get_mut("/files/source.bin")
        .unwrap()
        .mtime = original_mtime;
    let context = fixture
        .service
        .transfer_context_for_recovered_download(
            "main",
            "one",
            fresh,
            transfers.download_retry("main", id).unwrap(),
        )
        .await
        .unwrap();
    transfers.retry_with_context("main", id, context).unwrap();
    let completed = finished(&transfers, id).await;
    assert_eq!(completed.state, TransferState::Completed, "{completed:?}");
    assert_eq!(completed.processed.0, body.len() as u64);
    assert_eq!(std::fs::read(&target).unwrap(), body);
    assert!(!partial.exists());
    assert!(matches!(
        transfers.download_retry("main", id),
        Err(SftpError::ScopeExpired)
    ));
    fixture.service.revoke_terminal("foreign");
    fixture.service.revoke_terminal("changed");
    changed_shell.control.close().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn download_recovery_requires_fresh_exact_picker_and_identity_then_resumes_or_cleans() {
    for mode in ["resume", "cleanup", "corrupt"] {
        let (fixture, session, transfers) = fixture().await;
        let remote = &fixture.observations.transfer;
        let body = (0..1024 * 1024)
            .map(|n| (n % 251) as u8)
            .collect::<Vec<_>>();
        remote.put("/files/source.bin", &body);
        let local = tempfile::tempdir().unwrap();
        let ledger = tempfile::tempdir().unwrap();
        transfers
            .configure_recovery_directory(ledger.path().to_owned())
            .unwrap();
        let page = fixture.list(session, "/files").await.unwrap();
        let entry = page
            .entries
            .iter()
            .find(|entry| entry.name == "source.bin")
            .unwrap();
        let context = fixture
            .service
            .transfer_context(
                "main",
                "one",
                session,
                TransferTarget::Download { entry: entry.id },
            )
            .unwrap();
        let target = local.path().join("returned.bin");
        let destination = Arc::new(WindowsDestination::new(
            capstan_native_windows::transfer::Destination::select(&target).unwrap(),
        ));
        let grant = transfers.grant_destination(&context, destination).unwrap();
        remote.stall_read.store(true, Ordering::SeqCst);
        let id = transfers.enqueue(context, grant.token).unwrap();
        remote.started.notified().await;
        transfers.cancel("main", id).unwrap();
        assert_eq!(
            finished(&transfers, id).await.state,
            TransferState::Canceled
        );
        let partial = local.path().join(format!(".capstan-{id}.partial"));
        drop(transfers);
        remote.stall_read.store(false, Ordering::SeqCst);
        remote.release.notify_one();
        let size = std::fs::metadata(&partial).unwrap().len();
        assert!(size > 0 && size < body.len() as u64);
        if mode == "corrupt" {
            use std::io::{Seek, SeekFrom, Write};
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .open(&partial)
                .unwrap();
            file.seek(SeekFrom::Start(0)).unwrap();
            file.write_all(&[body[0] ^ 0xff]).unwrap();
        }
        assert!(!target.exists());
        let restarted = Arc::new(TransferService::default());
        restarted
            .configure_recovery_directory(ledger.path().to_owned())
            .unwrap();
        if mode == "resume" {
            let mut changed = fixture.connector.clone();
            changed.pool_key.credential_revision += 1;
            let lease = changed.acquire(&fixture.pool).await;
            let shell = lease.open_shell(80, 24).await.unwrap();
            fixture
                .service
                .register(
                    "main",
                    "changed-recovery",
                    changed.binding.host_id,
                    shell.connection_id(),
                    lease,
                )
                .unwrap();
            let changed_session = fixture.attach("changed-recovery").await;
            assert!(matches!(
                fixture
                    .service
                    .transfer_context_for_recovered_download(
                        "main",
                        "changed-recovery",
                        changed_session,
                        restarted.recovered_download("main", id).unwrap()
                    )
                    .await,
                Err(SftpError::ScopeExpired)
            ));
            assert_eq!(std::fs::metadata(&partial).unwrap().len(), size);
            assert!(!target.exists());
            fixture.service.revoke_terminal("changed-recovery");
            shell.control.close().await.unwrap();
        }
        let descriptor = restarted.recovered_download("main", id).unwrap();
        let original_mtime = remote.files.lock().unwrap()["/files/source.bin"].mtime;
        remote.put("/files/source.bin", &body);
        assert!(matches!(
            fixture
                .service
                .transfer_context_for_recovered_download("main", "one", session, descriptor,)
                .await,
            Err(SftpError::ResumeMismatch)
        ));
        assert_eq!(std::fs::metadata(&partial).unwrap().len(), size);
        remote
            .files
            .lock()
            .unwrap()
            .get_mut("/files/source.bin")
            .unwrap()
            .mtime = original_mtime;
        let descriptor = restarted.recovered_download("main", id).unwrap();
        let context = fixture
            .service
            .transfer_context_for_recovered_download("main", "one", session, descriptor)
            .await
            .unwrap();
        let wrong = Arc::new(WindowsDestination::new(
            capstan_native_windows::transfer::Destination::select(&local.path().join("wrong.bin"))
                .unwrap(),
        ));
        assert_eq!(
            restarted
                .grant_recovered_destination("main", id, &context, wrong)
                .await
                .unwrap_err(),
            SftpError::ResumeMismatch
        );
        assert_eq!(std::fs::metadata(&partial).unwrap().len(), size);
        let destination: Arc<dyn LocalDestination> = Arc::new(WindowsDestination::new(
            capstan_native_windows::transfer::Destination::select(&target).unwrap(),
        ));
        let grant = restarted
            .grant_recovered_destination("main", id, &context, destination)
            .await
            .unwrap();
        // Abandoning an authorized picker must retain the old partial.
        restarted.release_selection("main", grant.token).unwrap();
        assert!(partial.exists());
        let destination = Arc::new(WindowsDestination::new(
            capstan_native_windows::transfer::Destination::select(&target).unwrap(),
        ));
        let grant = restarted
            .grant_recovered_destination("main", id, &context, destination)
            .await
            .unwrap();
        if mode == "cleanup" {
            std::fs::write(&target, b"user final file").unwrap();
            restarted
                .cleanup_recovered_download("main", id, grant.token)
                .await
                .unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"user final file");
        } else {
            restarted
                .retry_recovered_download("main", id, context, grant.token)
                .unwrap();
            let result = finished(&restarted, id).await;
            if mode == "corrupt" {
                assert_eq!(result.error, Some(SftpError::ResumeMismatch));
                assert!(result.cleanup_required);
                assert_eq!(std::fs::metadata(&partial).unwrap().len(), size);
                assert!(!target.exists());
                assert_eq!(restarted.recovery_snapshot().unwrap().entries.len(), 1);
                drop(restarted);
                assert!(partial.exists());
                fixture.close().await;
                continue;
            }
            assert_eq!(result.state, TransferState::Completed);
            assert_eq!(std::fs::read(&target).unwrap(), body);
        }
        assert!(!partial.exists());
        assert!(restarted.recovery_snapshot().unwrap().entries.is_empty());
        // The rejection probe explicitly authenticated one different policy;
        // the actual continuation still reuses the original connection.
        assert_eq!(
            fixture.observations.auth.load(Ordering::SeqCst),
            if mode == "resume" { 2 } else { 1 }
        );
        fixture.close().await;
    }
}
