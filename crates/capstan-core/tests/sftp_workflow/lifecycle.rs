//! Connection/subsystem failures exercised through the production workflow.
use super::*;
use capstan_core::transfer::{TransferService, TransferState};

async fn eventually(predicate: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("fixture reached the expected lifecycle state");
}

async fn echo(shell: &mut capstan_ssh::SshShell) {
    let data = b"fixture-echo";
    tokio::time::timeout(Duration::from_secs(2), async {
        shell.control.write(data.to_vec()).await.unwrap();
        let mut received = vec![0; data.len()];
        shell.reader.read_exact(&mut received).await.unwrap();
        assert_eq!(received, data);
    })
    .await
    .expect("sibling shell remains usable");
}

#[tokio::test]
async fn rejected_and_invalid_handshakes_release_setup_and_explicit_retry_shares_one_subsystem() {
    let mut fixture = Fixture::new().await;
    let observations = fixture.observations.clone();
    observations.reject_subsystem.store(true, Ordering::SeqCst);
    assert!(matches!(
        fixture
            .service
            .attach("main", "one", &CancellationToken::new())
            .await,
        Err(SftpError::SftpUnavailable)
    ));
    assert_eq!(observations.subsystem_requests.load(Ordering::SeqCst), 1);
    assert_eq!(observations.subsystems.load(Ordering::SeqCst), 0);
    observations.reject_subsystem.store(false, Ordering::SeqCst);
    observations
        .invalid_sftp_version
        .store(true, Ordering::SeqCst);
    assert!(matches!(
        fixture
            .service
            .attach("main", "one", &CancellationToken::new())
            .await,
        Err(SftpError::SftpUnavailable)
    ));
    eventually(|| observations.active_subsystems.load(Ordering::SeqCst) == 0).await;
    assert_eq!(observations.subsystem_requests.load(Ordering::SeqCst), 2);
    for shell in &mut fixture.shells {
        echo(shell).await;
    }
    observations
        .invalid_sftp_version
        .store(false, Ordering::SeqCst);
    let (first, second) = tokio::join!(fixture.attach("one"), fixture.attach("two"));
    assert_ne!(first, second);
    assert_eq!(observations.auth.load(Ordering::SeqCst), 1);
    assert_eq!(observations.subsystem_requests.load(Ordering::SeqCst), 3);
    assert_eq!(observations.active_subsystems.load(Ordering::SeqCst), 1);
    fixture.close().await;
    assert_eq!(observations.active_subsystems.load(Ordering::SeqCst), 0);
    assert_eq!(observations.handles.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn failed_subsystem_reopens_explicitly_without_reusing_old_view_capabilities() {
    let mut fixture = Fixture::new().await;
    let original = fixture.attach("one").await;
    let page = fixture.list(original, "/").await.unwrap();
    let confirmation = fixture
        .service
        .prepare_mutation(
            "main",
            "one",
            original,
            page.entries[0].id,
            Mutation::Delete,
        )
        .await
        .unwrap();
    let client = fixture
        .lease
        .open_sftp(&CancellationToken::new())
        .await
        .unwrap();
    client.close().await;
    eventually(|| {
        fixture
            .observations
            .active_subsystems
            .load(Ordering::SeqCst)
            == 0
    })
    .await;
    assert_eq!(
        fixture.list(original, "/").await,
        Err(SftpError::ConnectionLost)
    );
    for shell in &mut fixture.shells {
        echo(shell).await;
    }
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 1);
    let replacement = fixture.attach("one").await;
    fixture.list(replacement, "/").await.unwrap();
    assert!(matches!(
        fixture
            .service
            .prepare_mutation(
                "main",
                "one",
                replacement,
                page.entries[0].id,
                Mutation::Delete
            )
            .await,
        Err(SftpError::ScopeExpired)
    ));
    assert_eq!(
        fixture
            .service
            .apply_mutation("main", "one", replacement, confirmation.token)
            .await,
        Err(SftpError::ScopeExpired)
    );
    assert_eq!(
        fixture
            .service
            .apply_mutation("main", "one", original, confirmation.token)
            .await,
        Err(SftpError::ConnectionLost)
    );
    assert_eq!(fixture.observations.mutations.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 2);
    fixture.close().await;
}

#[tokio::test]
async fn last_sftp_lease_releases_the_real_transport_after_thirty_idle_seconds() {
    let fixture = Fixture::new().await;
    let session = fixture.attach("one").await;
    fixture.list(session, "/many").await.unwrap();
    assert_eq!(fixture.observations.handles.load(Ordering::SeqCst), 1);
    let client = fixture
        .lease
        .open_sftp(&CancellationToken::new())
        .await
        .unwrap();
    fixture
        .service
        .detach("main", "one", session)
        .await
        .unwrap();
    fixture.service.revoke_terminal("one");
    fixture.service.revoke_terminal("two");
    for shell in &fixture.shells {
        shell.control.close().await.unwrap();
    }
    assert_eq!(fixture.observations.handles.load(Ordering::SeqCst), 0);
    let transport_closed = fixture.shells[0].control.connection_cancel().clone();
    let Fixture {
        service,
        shells,
        lease,
        pool,
        observations,
        server,
        tasks,
        cancel,
        connector: _,
    } = fixture;
    drop(lease);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(29)).await;
    assert_eq!(pool.entry_count().await, 1);
    assert!(!transport_closed.is_cancelled());
    let weak_client = Arc::downgrade(&client);
    drop(client);
    assert!(weak_client.upgrade().is_none());
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(29)).await;
    assert_eq!(pool.entry_count().await, 1);
    assert!(!transport_closed.is_cancelled());
    tokio::time::advance(Duration::from_secs(1)).await;
    // Pausing after real socket I/O can leave a fractional-millisecond
    // clock origin; Tokio rounds timer-wheel deadlines up to the next tick.
    tokio::time::advance(Duration::from_millis(1)).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(pool.entry_count().await, 0);
    assert!(transport_closed.is_cancelled());
    tokio::time::resume();
    drop(service);
    drop(shells);
    cancel.cancel();
    tasks.close();
    tasks.wait().await;
    server.abort();
    let _ = server.await;
    assert_eq!(observations.active_subsystems.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn explicit_reconnect_retires_dead_pool_aliases_and_rejects_previous_generation() {
    let mut fixture = Fixture::new().await;
    let old_session = fixture.attach("one").await;
    let page = fixture.list(old_session, "/").await.unwrap();
    let confirmation = fixture
        .service
        .prepare_mutation(
            "main",
            "one",
            old_session,
            page.entries[0].id,
            Mutation::Delete,
        )
        .await
        .unwrap();
    let mut alias = fixture.connector.pool_key.clone();
    alias.proxy_path = "fixture-alias".into();
    fixture.pool.alias(&fixture.lease, alias).await;
    let old_generation = fixture.lease.connection().connection_generation();
    let old_connection = fixture.shells[0].connection_id();
    fixture
        .observations
        .transport
        .get()
        .unwrap()
        .disconnect(
            russh::Disconnect::ByApplication,
            "fixture disconnection".into(),
            "en".into(),
        )
        .await
        .unwrap();
    eventually(|| fixture.lease.connection().is_closed()).await;
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.pool.entry_count().await, 2);
    // A new user action authorizes these acquisitions; there is no background
    // authentication or replay. Concurrent new opens still coalesce.
    let (first, second) = tokio::join!(
        fixture.connector.acquire(&fixture.pool),
        fixture.connector.acquire(&fixture.pool)
    );
    assert_eq!(
        first.connection().connection_generation(),
        second.connection().connection_generation()
    );
    assert!(first.connection().connection_generation() > old_generation);
    let new_shell = first.open_shell(80, 24).await.unwrap();
    assert_ne!(new_shell.connection_id(), old_connection);
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.pool.entry_count().await, 1);
    fixture.service.revoke_terminal("one");
    fixture.service.revoke_terminal("two");
    fixture
        .service
        .register(
            "main",
            "one",
            fixture.connector.binding.host_id,
            new_shell.connection_id(),
            first.clone(),
        )
        .unwrap();
    let new_session = fixture.attach("one").await;
    assert_eq!(
        fixture.list(old_session, "/").await,
        Err(SftpError::ScopeExpired)
    );
    assert!(matches!(
        fixture
            .service
            .prepare_mutation(
                "main",
                "one",
                new_session,
                page.entries[0].id,
                Mutation::Delete
            )
            .await,
        Err(SftpError::ScopeExpired)
    ));
    assert_eq!(
        fixture
            .service
            .apply_mutation("main", "one", new_session, confirmation.token)
            .await,
        Err(SftpError::ScopeExpired)
    );
    // Releasing old-generation tokens must leave the replacement entry live.
    fixture.lease = first;
    fixture.shells = [new_shell, second.open_shell(80, 24).await.unwrap()];
    drop(second);
    for shell in &mut fixture.shells {
        echo(shell).await;
    }
    fixture.list(new_session, "/empty").await.unwrap();
    assert_eq!(fixture.observations.mutations.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 2);
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn peer_transport_loss_interrupts_active_and_queued_jobs_without_publication() {
    let fixture = Fixture::new().await;
    fixture
        .observations
        .transfer_mode
        .store(true, Ordering::SeqCst);
    let session = fixture.attach("one").await;
    fixture.list(session, "/files").await.unwrap();
    let remote = &fixture.observations.transfer;
    remote.put("/files/untouched", b"original");
    remote.stall_write.store(true, Ordering::SeqCst);
    let transfers = Arc::new(TransferService::default());
    let local = tempfile::tempdir().unwrap();
    let mut jobs = Vec::new();
    for index in 0..5 {
        let source = local.path().join(format!("source-{index}"));
        std::fs::write(&source, b"new").unwrap();
        let id = transfers::enqueue_upload(
            &fixture,
            session,
            &transfers,
            &source,
            &format!("target-{index}"),
        );
        jobs.push((id, source));
    }
    tokio::time::timeout(Duration::from_secs(2), remote.started.notified())
        .await
        .unwrap();
    transfers::wait(&transfers, jobs[2].0, |s| s.state == TransferState::Running).await;
    fixture
        .observations
        .transport
        .get()
        .unwrap()
        .disconnect(
            russh::Disconnect::ByApplication,
            "fixture disconnection".into(),
            "en".into(),
        )
        .await
        .unwrap();
    let mut residuals = 0;
    tokio::time::timeout(Duration::from_secs(2), async {
        for (id, _) in &jobs {
            let status = transfers::finished(&transfers, *id).await;
            assert_eq!(status.state, TransferState::Interrupted, "{status:?}");
            assert_eq!(status.error, Some(SftpError::ConnectionLost));
            residuals += usize::from(status.cleanup_required);
        }
    })
    .await
    .expect("peer transport loss must drain active and queued jobs");
    assert!(residuals > 0);
    assert!(residuals <= 3);
    assert_eq!(remote.bytes("/files/untouched"), b"original");
    assert_eq!(remote.commits.load(Ordering::SeqCst), 0);
    for (_, source) in jobs {
        std::fs::write(source, b"released").unwrap();
    }
    let observations = fixture.observations.clone();
    fixture.close().await;
    assert_eq!(observations.handles.load(Ordering::SeqCst), 0);
    assert_eq!(observations.active_subsystems.load(Ordering::SeqCst), 0);
}

#[cfg(windows)]
#[tokio::test]
async fn transport_loss_during_publication_preserves_admitted_commit_and_drains_exit() {
    use capstan_core::transfer::{TransferTarget, WindowsDestination};
    let fixture = Fixture::new().await;
    fixture
        .observations
        .transfer_mode
        .store(true, Ordering::SeqCst);
    fixture
        .observations
        .transfer
        .put("/files/source", b"verified content");
    let session = fixture.attach("one").await;
    let entry = fixture.list(session, "/files").await.unwrap().entries[0].id;
    let transfers = Arc::new(TransferService::default());
    let local = tempfile::tempdir().unwrap();
    let hold = Arc::new(transfers::CommitHold::default());
    let mut jobs = Vec::new();
    for index in 0..2 {
        let context = fixture
            .service
            .transfer_context("main", "one", session, TransferTarget::Download { entry })
            .unwrap();
        let output = local.path().join(format!("output-{index}"));
        let destination = transfers::FaultDestination {
            inner: WindowsDestination::new(
                capstan_native_windows::transfer::Destination::select(&output).unwrap(),
            ),
            fault: 3,
            hold: Some(hold.clone()),
        };
        let selection = transfers
            .grant_destination(&context, Arc::new(destination))
            .unwrap();
        jobs.push((transfers.enqueue(context, selection.token).unwrap(), output));
        if index == 0 {
            tokio::time::timeout(Duration::from_secs(2), hold.started.notified())
                .await
                .unwrap();
        }
    }
    transfers::wait(&transfers, jobs[1].0, |s| {
        s.state == TransferState::Verifying
    })
    .await;
    fixture.lease.connection().disconnect().await;
    let waiting = transfers::finished(&transfers, jobs[1].0).await;
    assert_eq!(waiting.state, TransferState::Interrupted, "{waiting:?}");
    assert!(!waiting.cleanup_required);
    assert!(!jobs[1].1.exists());
    assert_eq!(
        transfers.cancel("main", jobs[0].0),
        Err(SftpError::TargetConflict)
    );
    let draining = transfers.clone();
    let exit = tokio::spawn(async move { draining.prepare_exit("main").await });
    tokio::task::yield_now().await;
    assert!(!exit.is_finished());
    assert!(transfers.has_active("main").unwrap());
    *hold.released.lock().unwrap() = true;
    hold.changed.notify_all();
    let committed = transfers::finished(&transfers, jobs[0].0).await;
    assert_eq!(committed.state, TransferState::Completed, "{committed:?}");
    assert_eq!(std::fs::read(&jobs[0].1).unwrap(), b"verified content");
    assert_eq!(exit.await.unwrap(), Ok(()));
    assert!(!transfers.has_active("main").unwrap());
    assert!(std::fs::read_dir(local.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains(".partial")));
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn cancel_window_clears_unused_native_grants_and_drains_waiting_jobs() {
    let fixture = Fixture::new().await;
    fixture
        .observations
        .transfer_mode
        .store(true, Ordering::SeqCst);
    fixture
        .observations
        .transfer
        .put("/files/target", b"original");
    let session = fixture.attach("one").await;
    fixture.list(session, "/files").await.unwrap();
    let transfers = Arc::new(TransferService::default());
    let local = tempfile::tempdir().unwrap();
    let active_source = local.path().join("active");
    let unused_source = local.path().join("unused");
    std::fs::write(&active_source, b"new").unwrap();
    std::fs::write(&unused_source, b"unused").unwrap();
    let id = transfers::enqueue_upload(&fixture, session, &transfers, &active_source, "target");
    transfers::wait(&transfers, id, |s| s.conflict.is_some()).await;
    let context = transfers::upload_context(&fixture, session, "unused");
    let selection = transfers
        .grant_source(
            &context,
            Arc::new(capstan_native_windows::transfer::Source::open(&unused_source).unwrap()),
        )
        .unwrap();
    assert!(std::fs::write(&active_source, b"blocked").is_err());
    assert!(std::fs::write(&unused_source, b"blocked").is_err());
    // This is the core primitive used by disconnect-on-lock; native lock UI
    // and credentials/policy routing remain a separate desktop acceptance.
    transfers.cancel_window("main");
    assert_eq!(
        transfers::finished(&transfers, id).await.state,
        TransferState::Canceled
    );
    assert_eq!(
        transfers.enqueue(context, selection.token),
        Err(SftpError::ScopeExpired)
    );
    std::fs::write(&active_source, b"released").unwrap();
    std::fs::write(&unused_source, b"released").unwrap();
    assert_eq!(
        fixture.observations.transfer.bytes("/files/target"),
        b"original"
    );
    assert!(fixture.observations.transfer.no_partials());
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn abrupt_tcp_eof_interrupts_upload_and_releases_handles_without_authentication_replay() {
    use capstan_core::transfer::TransferTarget;
    let mut fixture = Fixture::new().await;
    fixture
        .observations
        .transfer_mode
        .store(true, Ordering::SeqCst);
    let endpoint = fixture.connector.binding.port;
    let relay = super::relay::Relay::start(endpoint).await;
    let mut connector = fixture.connector.clone();
    connector.binding.port = relay.port;
    connector.pool_key.port = relay.port;
    let lease = connector.acquire(&fixture.pool).await;
    let shell = lease.open_shell(80, 24).await.unwrap();
    fixture
        .service
        .register(
            "main",
            "abrupt",
            connector.binding.host_id,
            shell.connection_id(),
            lease.clone(),
        )
        .unwrap();
    let session = fixture
        .service
        .attach("main", "abrupt", &CancellationToken::new())
        .await
        .unwrap()
        .session_id;
    fixture
        .service
        .list(
            "main",
            "abrupt",
            session,
            RemotePath::parse("/files").unwrap(),
        )
        .await
        .unwrap();
    let remote = &fixture.observations.transfer;
    remote.put("/files/untouched", b"original");
    remote.stall_write.store(true, Ordering::SeqCst);
    let local = tempfile::tempdir().unwrap();
    let source = local.path().join("source");
    let source_bytes: Vec<u8> = (0..(128 * 1024)).map(|value| value as u8).collect();
    std::fs::write(&source, &source_bytes).unwrap();
    let context = fixture
        .service
        .transfer_context(
            "main",
            "abrupt",
            session,
            TransferTarget::Upload {
                parent: RemotePath::parse("/files").unwrap(),
                name: "target".into(),
            },
        )
        .unwrap();
    let recovery_directory = tempfile::tempdir().unwrap();
    let transfers = Arc::new(TransferService::default());
    transfers
        .configure_recovery_directory(recovery_directory.path().to_owned())
        .unwrap();
    let selection = transfers
        .grant_source(
            &context,
            Arc::new(capstan_native_windows::transfer::Source::open(&source).unwrap()),
        )
        .unwrap();
    let id = transfers.enqueue(context, selection.token).unwrap();
    tokio::time::timeout(Duration::from_secs(2), remote.started.notified())
        .await
        .unwrap();
    // Let one write converge, then stop on a later write so the retained
    // temporary contains a real prefix rather than an empty residual.
    remote.release.notify_one();
    tokio::time::timeout(Duration::from_secs(2), remote.started.notified())
        .await
        .unwrap();
    relay.break_connections();
    let status = tokio::time::timeout(Duration::from_secs(2), transfers::finished(&transfers, id))
        .await
        .unwrap();
    assert_eq!(status.state, TransferState::Interrupted, "{status:?}");
    assert_eq!(status.error, Some(SftpError::ConnectionLost));
    assert!(status.cleanup_required);
    let recovered = transfers.recovery_snapshot().unwrap();
    assert_eq!(recovered.entries.len(), 1);
    assert_eq!(recovered.entries[0].id, id);
    assert_eq!(
        recovered.entries[0].state,
        capstan_core::transfer::RecoveryState::AwaitingReconnect
    );
    eventually(|| lease.connection().is_closed()).await;

    // The interrupted job retained an owned remote prefix but released its
    // original native source handle. Rebind it to a fresh picker and a new
    // authenticated SFTP view; the retry must verify the prefix before
    // appending and must publish the final target without leaving a partial.
    remote.stall_write.store(false, Ordering::SeqCst);
    fixture.service.revoke_terminal("abrupt");
    shell.control.close().await.unwrap();
    // Keep the same authenticated endpoint policy across the explicit retry.
    // The relay listener survives dropping only the first TCP stream.
    let retry_lease = connector.acquire(&fixture.pool).await;
    let retry_shell = retry_lease.open_shell(80, 24).await.unwrap();
    fixture
        .service
        .register(
            "main",
            "abrupt-retry",
            fixture.connector.binding.host_id,
            retry_shell.connection_id(),
            retry_lease.clone(),
        )
        .unwrap();
    let retry_session = fixture
        .service
        .attach("main", "abrupt-retry", &CancellationToken::new())
        .await
        .unwrap()
        .session_id;
    fixture
        .service
        .list(
            "main",
            "abrupt-retry",
            retry_session,
            RemotePath::parse("/files").unwrap(),
        )
        .await
        .unwrap();
    let retry_context = fixture
        .service
        .transfer_context(
            "main",
            "abrupt-retry",
            retry_session,
            TransferTarget::Upload {
                parent: RemotePath::parse("/files").unwrap(),
                name: "target".into(),
            },
        )
        .unwrap();
    let unrelated_context = fixture
        .service
        .transfer_context(
            "main",
            "abrupt-retry",
            retry_session,
            TransferTarget::Upload {
                parent: RemotePath::parse("/files").unwrap(),
                name: "other-target".into(),
            },
        )
        .unwrap();
    assert_eq!(
        transfers
            .cleanup_recovered_upload("main", id, unrelated_context)
            .await,
        Err(SftpError::ScopeExpired)
    );
    assert_eq!(transfers.recovery_snapshot().unwrap().entries.len(), 1);
    assert!(!remote.no_partials());
    let partial_path = format!("/files/.capstan-{id}.partial");
    let original_partial = remote.bytes(&partial_path);
    remote.put(
        &partial_path,
        &vec![0; source_bytes.len().saturating_add(1)],
    );
    let oversized_partial_context = fixture
        .service
        .transfer_context(
            "main",
            "abrupt-retry",
            retry_session,
            TransferTarget::Upload {
                parent: RemotePath::parse("/files").unwrap(),
                name: "target".into(),
            },
        )
        .unwrap();
    assert_eq!(
        transfers
            .cleanup_recovered_upload("main", id, oversized_partial_context)
            .await,
        Err(SftpError::ResumeMismatch)
    );
    assert_eq!(transfers.recovery_snapshot().unwrap().entries.len(), 1);
    remote.put(&partial_path, &original_partial);
    drop(transfers);
    let restarted_transfers = Arc::new(TransferService::default());
    restarted_transfers
        .configure_recovery_directory(recovery_directory.path().to_owned())
        .unwrap();
    assert_eq!(
        restarted_transfers
            .recovery_snapshot()
            .unwrap()
            .entries
            .len(),
        1
    );
    // A durable Resume checkpoint is still eligible for source-verified
    // continuation, but cannot authorize deletion of a same-size replacement.
    let replacement = vec![b'x'; original_partial.len().max(1)];
    remote.put(&partial_path, &replacement);
    let cleanup_context = fixture
        .service
        .transfer_context(
            "main",
            "abrupt-retry",
            retry_session,
            TransferTarget::Upload {
                parent: RemotePath::parse("/files").unwrap(),
                name: "target".into(),
            },
        )
        .unwrap();
    assert_eq!(
        restarted_transfers
            .cleanup_recovered_upload("main", id, cleanup_context)
            .await,
        Err(SftpError::ResumeMismatch)
    );
    assert_eq!(remote.bytes(&partial_path), replacement);
    remote.put(&partial_path, &original_partial);
    let retry_selection = restarted_transfers
        .grant_source(
            &retry_context,
            Arc::new(capstan_native_windows::transfer::Source::open(&source).unwrap()),
        )
        .unwrap();
    restarted_transfers
        .retry_recovered_upload("main", id, retry_context, retry_selection.token)
        .unwrap();
    let retry_status = tokio::time::timeout(
        Duration::from_secs(2),
        transfers::finished(&restarted_transfers, id),
    )
    .await
    .unwrap();
    assert_eq!(
        retry_status.state,
        TransferState::Completed,
        "{retry_status:?}"
    );
    assert_eq!(remote.bytes("/files/target"), source_bytes);
    assert!(remote.no_partials());
    assert!(restarted_transfers
        .recovery_snapshot()
        .unwrap()
        .entries
        .is_empty());
    fixture.service.revoke_terminal("abrupt-retry");
    retry_shell.control.close().await.unwrap();
    std::fs::write(&source, b"released").unwrap();
    assert_eq!(remote.bytes("/files/untouched"), b"original");
    assert_eq!(remote.commits.load(Ordering::SeqCst), 1);
    // Initial sibling connection, first proxy connection, explicit replacement.
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 3);
    assert_eq!(relay.opened(), 2);
    relay.close().await;
    for shell in &mut fixture.shells {
        echo(shell).await;
    }
    let observations = fixture.observations.clone();
    fixture.close().await;
    assert_eq!(observations.handles.load(Ordering::SeqCst), 0);
    assert_eq!(observations.active_subsystems.load(Ordering::SeqCst), 0);
}

#[cfg(windows)]
#[tokio::test]
async fn primary_loss_interrupts_its_conflict_while_siblings_and_queued_authority_survive() {
    let fixture = Fixture::new().await;
    fixture
        .observations
        .transfer_mode
        .store(true, Ordering::SeqCst);
    let session = fixture.attach("one").await;
    fixture.list(session, "/files").await.unwrap();
    let transfers = Arc::new(TransferService::default());
    let local = tempfile::tempdir().unwrap();
    let mut jobs = Vec::new();
    for index in 0..5 {
        let name = format!("target-{index}");
        fixture
            .observations
            .transfer
            .put(&format!("/files/{name}"), b"original");
        let source = local.path().join(&name);
        std::fs::write(&source, b"new").unwrap();
        let id = transfers::enqueue_upload(&fixture, session, &transfers, &source, &name);
        jobs.push((id, source));
        if index < 3 {
            transfers::wait(&transfers, id, |status| status.conflict.is_some()).await;
        }
    }
    let before = transfers.snapshot("main").unwrap();
    assert_eq!(
        before
            .iter()
            .filter(|s| s.state == TransferState::AwaitingConflict)
            .count(),
        3
    );
    assert_eq!(
        before
            .iter()
            .filter(|s| s.state == TransferState::Queued)
            .count(),
        2
    );
    let client = fixture
        .lease
        .open_sftp(&CancellationToken::new())
        .await
        .unwrap();
    client.close().await;
    let interrupted = transfers::finished(&transfers, jobs[0].0).await;
    assert_eq!(
        interrupted.state,
        TransferState::Interrupted,
        "{interrupted:?}"
    );
    assert_eq!(interrupted.error, Some(SftpError::ConnectionLost));
    assert!(interrupted.conflict.is_none());
    assert!(!interrupted.cleanup_required);
    // Already authorized queued work may acquire a replacement primary on
    // the same generation. It neither reauthenticates nor reuses browse IDs.
    transfers::wait(&transfers, jobs[3].0, |s| s.conflict.is_some()).await;
    for (id, _) in &jobs[1..4] {
        let current = transfers
            .snapshot("main")
            .unwrap()
            .into_iter()
            .find(|s| s.id == *id)
            .unwrap();
        assert_eq!(current.state, TransferState::AwaitingConflict);
    }
    transfers.cancel_window("main");
    tokio::time::timeout(Duration::from_secs(2), async {
        for (id, _) in &jobs[1..] {
            let status = transfers::finished(&transfers, *id).await;
            assert_eq!(status.state, TransferState::Canceled, "{status:?}");
            assert!(status.conflict.is_none());
            assert!(!status.cleanup_required);
        }
    })
    .await
    .expect("cancellation releases every remaining queued/conflict waiter promptly");
    for (_, source) in jobs {
        std::fs::write(source, b"released").unwrap();
    }
    assert!(fixture.observations.transfer.no_partials());
    assert_eq!(
        fixture.observations.transfer.commits.load(Ordering::SeqCst),
        0
    );
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 1);
    fixture.close().await;
}
