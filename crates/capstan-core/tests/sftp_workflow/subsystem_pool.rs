//! Authenticated transport, subsystem assignment and cancellation boundaries.
use super::*;
use capstan_ssh::sftp::ClientError;

async fn eventually(predicate: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !predicate() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

async fn echo(shell: &mut capstan_ssh::SshShell) {
    tokio::time::timeout(Duration::from_secs(2), async {
        shell.control.write(b"pool-echo".to_vec()).await.unwrap();
        let mut bytes = [0; 9];
        shell.reader.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"pool-echo");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn three_data_assignments_share_one_authentication_and_a_fourth_reuses_a_slot() {
    let mut fixture = Fixture::new().await;
    let (one, two) = tokio::join!(fixture.attach("one"), fixture.attach("two"));
    assert_ne!(one, two);
    let cancel = CancellationToken::new();
    let primary = fixture.lease.open_sftp(&cancel).await.unwrap();
    let (first, second, third) = tokio::join!(
        fixture.lease.lease_sftp_transfer(&cancel),
        fixture.lease.lease_sftp_transfer(&cancel),
        fixture.lease.lease_sftp_transfer(&cancel),
    );
    let (first, second, third) = (first.unwrap(), second.unwrap(), third.unwrap());
    assert!(Arc::ptr_eq(first.client(), &primary));
    assert!(!Arc::ptr_eq(first.client(), second.client()));
    assert!(!Arc::ptr_eq(first.client(), third.client()));
    assert!(!Arc::ptr_eq(second.client(), third.client()));
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 3);
    assert_eq!(
        fixture
            .observations
            .active_subsystems
            .load(Ordering::SeqCst),
        3
    );

    let fourth_connection = fixture.lease.clone();
    let fourth = fourth_connection.lease_sftp_transfer(&cancel);
    tokio::pin!(fourth);
    assert!(tokio::time::timeout(Duration::from_millis(30), &mut fourth)
        .await
        .is_err());
    second.release().await;
    let fourth = fourth.await.unwrap();
    assert!(!Arc::ptr_eq(first.client(), fourth.client()));
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 3);
    for shell in &mut fixture.shells {
        echo(shell).await;
    }
    first.release().await;
    third.release().await;
    fourth.release().await;
    // An attached browser pins reusable companions without retaining a cycle.
    let (next_one, next_two, next_three) = tokio::join!(
        fixture.lease.lease_sftp_transfer(&cancel),
        fixture.lease.lease_sftp_transfer(&cancel),
        fixture.lease.lease_sftp_transfer(&cancel),
    );
    next_one.unwrap().release().await;
    next_two.unwrap().release().await;
    next_three.unwrap().release().await;
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 3);
    assert_eq!(
        fixture.observations.peak_subsystems.load(Ordering::SeqCst),
        3
    );
    drop(primary);
    fixture.close().await;
}

#[tokio::test]
async fn failed_data_subsystem_leaves_browsing_and_siblings_alive_and_rebuilds_only_its_slot() {
    let mut fixture = Fixture::new().await;
    let session = fixture.attach("one").await;
    let cancel = CancellationToken::new();
    let first = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
    let second = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
    let third = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
    let stale = second.client().clone();
    stale.close().await;
    eventually(|| {
        fixture
            .observations
            .active_subsystems
            .load(Ordering::SeqCst)
            == 2
    })
    .await;
    second.release().await;
    fixture.list(session, "/").await.unwrap();
    third.client().lstat("/file", &cancel).await.unwrap();
    for shell in &mut fixture.shells {
        echo(shell).await;
    }
    let replacement = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
    assert!(!Arc::ptr_eq(&stale, replacement.client()));
    assert_eq!(
        stale.lstat("/file", &cancel).await,
        Err(ClientError::ConnectionLost)
    );
    replacement.client().lstat("/file", &cancel).await.unwrap();
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 4);
    assert_eq!(
        fixture.observations.peak_subsystems.load(Ordering::SeqCst),
        3
    );
    first.release().await;
    third.release().await;
    replacement.release().await;
    fixture.close().await;
}

#[tokio::test]
async fn primary_failure_does_not_close_data_siblings_or_reauthorize_an_old_view() {
    let fixture = Fixture::new().await;
    let session = fixture.attach("one").await;
    let page = fixture.list(session, "/").await.unwrap();
    let cancel = CancellationToken::new();
    let first = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
    let second = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
    let third = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
    first.client().close().await;
    eventually(|| {
        fixture
            .observations
            .active_subsystems
            .load(Ordering::SeqCst)
            == 2
    })
    .await;
    first.release().await;
    assert_eq!(
        fixture.list(session, "/").await,
        Err(SftpError::ConnectionLost)
    );
    second.client().lstat("/file", &cancel).await.unwrap();
    third.client().lstat("/file", &cancel).await.unwrap();
    let replacement = fixture.attach("one").await;
    assert!(matches!(
        fixture
            .service
            .prepare_mutation(
                "main",
                "one",
                replacement,
                page.entries[0].id,
                Mutation::Delete,
            )
            .await,
        Err(SftpError::ScopeExpired)
    ));
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 4);
    second.release().await;
    third.release().await;
    fixture.close().await;
}

#[tokio::test]
async fn rejected_extra_subsystems_fall_back_to_bounded_cancelable_serial_assignments() {
    let mut fixture = Fixture::new().await;
    fixture.attach("one").await;
    let cancel = CancellationToken::new();
    let first = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
    fixture
        .observations
        .reject_subsystem
        .store(true, Ordering::SeqCst);
    let stopped = CancellationToken::new();
    let pending_connection = fixture.lease.clone();
    let pending = pending_connection.lease_sftp_transfer(&stopped);
    tokio::pin!(pending);
    tokio::select! {
        biased;
        _ = &mut pending => panic!("primary assignment is still held"),
        () = eventually(|| fixture.observations.subsystem_requests.load(Ordering::SeqCst) >= 2) => {}
    }
    stopped.cancel();
    assert!(matches!(pending.await, Err(ClientError::Canceled)));
    first.release().await;
    let next = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.observations.peak_subsystems.load(Ordering::SeqCst),
        1
    );
    for shell in &mut fixture.shells {
        echo(shell).await;
    }
    next.release().await;
    fixture.close().await;
}

#[tokio::test]
async fn session_channel_limit_reuses_two_assignments_without_interrupting_shells() {
    let mut fixture = Fixture::new().await;
    let session = fixture.attach("one").await;
    let cancel = CancellationToken::new();
    // Two existing shells leave room for only two SFTP session channels.
    fixture
        .observations
        .session_limit
        .store(4, Ordering::SeqCst);
    let first = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
    let second = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
    let reusable = second.client().clone();
    let stopped = CancellationToken::new();
    let pending_connection = fixture.lease.clone();
    let pending = pending_connection.lease_sftp_transfer(&stopped);
    tokio::pin!(pending);
    tokio::select! {
        biased;
        _ = &mut pending => panic!("both usable assignments are still held"),
        () = eventually(|| fixture.observations.rejected_sessions.load(Ordering::SeqCst) == 1) => {}
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut pending)
            .await
            .is_err()
    );
    assert_eq!(
        fixture
            .observations
            .rejected_sessions
            .load(Ordering::SeqCst),
        1
    );
    stopped.cancel();
    assert!(matches!(pending.await, Err(ClientError::Canceled)));
    second.release().await;
    let next = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
    assert!(Arc::ptr_eq(next.client(), &reusable));
    next.client().lstat("/file", &cancel).await.unwrap();
    fixture.list(session, "/").await.unwrap();
    for shell in &mut fixture.shells {
        echo(shell).await;
    }
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 2);
    assert_eq!(
        fixture
            .observations
            .rejected_sessions
            .load(Ordering::SeqCst),
        1
    );
    assert!(!fixture.lease.connection().is_closed());
    first.release().await;
    next.release().await;
    fixture.close().await;
}

#[tokio::test]
async fn dropped_data_assignment_waits_for_a_late_reply_before_reuse() {
    let mut fixture = Fixture::new().await;
    fixture.attach("one").await;
    let cancel = CancellationToken::new();
    let first = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
    let second = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
    let third = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
    let data_client = second.client().clone();
    fixture
        .observations
        .stall_stat
        .store(true, Ordering::SeqCst);
    let stopped = CancellationToken::new();
    let request = {
        let client = data_client.clone();
        let stopped = stopped.clone();
        tokio::spawn(async move { client.lstat("/file", &stopped).await })
    };
    fixture.observations.stat_started.notified().await;
    stopped.cancel();
    assert_eq!(request.await.unwrap(), Err(ClientError::Canceled));
    drop(second);
    let next_connection = fixture.lease.clone();
    let next = next_connection.lease_sftp_transfer(&cancel);
    tokio::pin!(next);
    assert!(tokio::time::timeout(Duration::from_millis(30), &mut next)
        .await
        .is_err());
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 3);
    for shell in &mut fixture.shells {
        echo(shell).await;
    }
    fixture
        .observations
        .stall_stat
        .store(false, Ordering::SeqCst);
    fixture.observations.stat_release.notify_one();
    let next = tokio::time::timeout(Duration::from_secs(2), next)
        .await
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(next.client(), &data_client));
    assert!(!data_client.is_closed());
    first.release().await;
    third.release().await;
    next.release().await;
    fixture.close().await;
}

#[tokio::test]
async fn canceling_or_aborting_extra_initialization_reaps_it_before_the_next_open() {
    for abort in [false, true] {
        let fixture = Fixture::new().await;
        fixture.attach("one").await;
        let cancel = CancellationToken::new();
        let first = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
        fixture
            .observations
            .stall_init
            .store(true, Ordering::SeqCst);
        let stopped = CancellationToken::new();
        let pending = {
            let connection = fixture.lease.clone();
            let stopped = stopped.clone();
            tokio::spawn(async move { connection.lease_sftp_transfer(&stopped).await })
        };
        tokio::time::timeout(
            Duration::from_secs(2),
            fixture.observations.init_started.notified(),
        )
        .await
        .unwrap();
        if abort {
            pending.abort();
            let Err(error) = pending.await else {
                panic!("aborted initialization returned normally")
            };
            assert!(error.is_cancelled());
        } else {
            stopped.cancel();
            assert!(matches!(pending.await.unwrap(), Err(ClientError::Canceled)));
        }
        eventually(|| {
            fixture
                .observations
                .active_subsystems
                .load(Ordering::SeqCst)
                == 1
        })
        .await;
        fixture
            .observations
            .stall_init
            .store(false, Ordering::SeqCst);
        let second = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
        let third = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
        assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 4);
        assert_eq!(
            fixture
                .observations
                .active_subsystems
                .load(Ordering::SeqCst),
            3
        );
        assert_eq!(
            fixture.observations.peak_subsystems.load(Ordering::SeqCst),
            3
        );
        first.release().await;
        second.release().await;
        third.release().await;
        fixture.close().await;
    }
}

#[tokio::test]
async fn dropping_the_last_browser_and_data_users_releases_all_subsystems_before_pool_expiry() {
    let fixture = Fixture::new().await;
    let session = fixture.attach("one").await;
    let cancel = CancellationToken::new();
    let first = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
    let second = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
    let third = fixture.lease.lease_sftp_transfer(&cancel).await.unwrap();
    fixture
        .service
        .detach("main", "one", session)
        .await
        .unwrap();
    first.release().await;
    second.release().await;
    third.release().await;
    eventually(|| {
        fixture
            .observations
            .active_subsystems
            .load(Ordering::SeqCst)
            == 0
    })
    .await;
    assert_eq!(fixture.pool.entry_count().await, 1);
    assert!(!fixture.lease.connection().is_closed());
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn exit_drains_a_canceled_data_subsystem_even_when_the_primary_has_no_pending_work() {
    use capstan_core::transfer::{TransferService, TransferState};
    let fixture = Fixture::new().await;
    fixture
        .observations
        .transfer_mode
        .store(true, Ordering::SeqCst);
    let session = fixture.attach("one").await;
    fixture.list(session, "/files").await.unwrap();
    let primary = fixture
        .lease
        .lease_sftp_transfer(&CancellationToken::new())
        .await
        .unwrap();
    let transfers = Arc::new(TransferService::default());
    let local = tempfile::tempdir().unwrap();
    let source = local.path().join("source");
    std::fs::write(&source, b"new").unwrap();
    let remote = &fixture.observations.transfer;
    remote.stall_write.store(true, Ordering::SeqCst);
    let id = transfers::enqueue_upload(&fixture, session, &transfers, &source, "target");
    tokio::time::timeout(Duration::from_secs(2), remote.started.notified())
        .await
        .unwrap();
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 2);
    transfers.cancel("main", id).unwrap();
    assert_eq!(
        transfers::finished(&transfers, id).await.state,
        TransferState::Canceled
    );
    // Task/source ownership ends before the separate protocol drainer.
    std::fs::write(&source, b"released").unwrap();
    let exit = {
        let transfers = transfers.clone();
        tokio::spawn(async move { transfers.prepare_exit("main").await })
    };
    tokio::pin!(exit);
    assert!(tokio::time::timeout(Duration::from_millis(30), &mut exit)
        .await
        .is_err());
    remote.stall_write.store(false, Ordering::SeqCst);
    remote.release.notify_one();
    tokio::time::timeout(Duration::from_secs(2), exit)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(fixture.observations.handles.load(Ordering::SeqCst), 0);
    #[cfg(feature = "benchmarks")]
    assert_eq!(transfers.buffer_usage().0, 0);
    primary.release().await;
    fixture.close().await;
}
