use super::*;
use capstan_core::transfer::WindowsDestination;

async fn recover(mode: &str) {
    let (fixture, session, transfers) = fixture().await;
    let body = (0..1024 * 1024)
        .map(|value| (value % 251) as u8)
        .collect::<Vec<_>>();
    let remote = &fixture.observations.transfer;
    remote.put("/files/source.bin", &body);
    let local = tempfile::tempdir().unwrap();
    let target = local.path().join("returned.bin");
    std::fs::write(&target, b"original destination").unwrap();
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
    let destination = Arc::new(WindowsDestination::new(
        capstan_native_windows::transfer::Destination::select(&target).unwrap(),
    ));
    let grant = transfers.grant_destination(&context, destination).unwrap();
    remote.stall_read.store(true, Ordering::SeqCst);
    let id = transfers.enqueue(context, grant.token).unwrap();
    wait(&transfers, id, |status| {
        status.state == TransferState::AwaitingConflict
    })
    .await;
    let conflict = transfers
        .snapshot("main")
        .unwrap()
        .into_iter()
        .find(|status| status.id == id)
        .unwrap()
        .conflict
        .unwrap();
    transfers
        .resolve("main", id, conflict.revision, Decision::Replace)
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), remote.started.notified())
        .await
        .unwrap();
    let partial = local.path().join(format!(".capstan-{id}.partial"));
    tokio::time::timeout(Duration::from_secs(3), async {
        while !std::fs::metadata(&partial).is_ok_and(|metadata| metadata.len() > 0) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    fixture.lease.connection().disconnect().await;
    assert_eq!(
        finished(&transfers, id).await.state,
        TransferState::Interrupted
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"original destination");
    let size = std::fs::metadata(&partial).unwrap().len();
    if mode == "remote" {
        remote.put("/files/source.bin", &body);
    }
    if mode == "prefix" {
        // Same size and metadata, different completed prefix.
        remote
            .files
            .lock()
            .unwrap()
            .get_mut("/files/source.bin")
            .unwrap()
            .data[0] = 255;
    }
    remote.stall_read.store(false, Ordering::SeqCst);
    remote.release.notify_waiters();
    let mut connector = fixture.connector.clone();
    if mode == "policy" {
        connector.pool_key.credential_revision += 1;
    }
    let lease = connector.acquire(&fixture.pool).await;
    transfers
        .recover_connection(lease.clone(), &CancellationToken::new())
        .await;
    if matches!(mode, "resume" | "prefix") {
        let waiting = wait(&transfers, id, |status| {
            status.state == TransferState::AwaitingConflict
        })
        .await;
        assert_eq!(std::fs::read(&target).unwrap(), b"original destination");
        transfers
            .resolve(
                "main",
                id,
                waiting.conflict.unwrap().revision,
                Decision::Replace,
            )
            .unwrap();
    }
    if mode == "resume" {
        let result = finished(&transfers, id).await;
        assert_eq!(result.state, TransferState::Completed, "{result:?}");
        assert_eq!(std::fs::read(&target).unwrap(), body);
        assert!(!partial.exists());
    } else {
        let result = finished(&transfers, id).await;
        assert_ne!(result.state, TransferState::Completed);
        assert_eq!(
            result.error,
            Some(if mode == "policy" {
                SftpError::ConnectionLost
            } else {
                SftpError::ResumeMismatch
            })
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"original destination");
        assert_eq!(std::fs::metadata(&partial).unwrap().len(), size);
    }
    lease.connection().disconnect().await;
    fixture.close().await;
}

#[tokio::test]
async fn reconnect_resumes_only_verified_retained_downloads() {
    recover("resume").await;
}
#[tokio::test]
async fn reconnect_preserves_original_and_partial_when_remote_metadata_changes() {
    recover("remote").await;
}
#[tokio::test]
async fn reconnect_checks_the_completed_prefix_before_writing() {
    recover("prefix").await;
}
#[tokio::test]
async fn reconnect_does_not_rebind_to_a_changed_authentication_policy() {
    recover("policy").await;
}
