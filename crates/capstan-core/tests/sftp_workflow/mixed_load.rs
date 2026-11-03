use super::*;
use sha2::{Digest, Sha256};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saturated_shell_and_three_uploads_preserve_sibling_echo_and_complete_hashes() {
    let (mut fixture, session, transfers) = fixture().await;
    let observations = fixture.observations.clone();
    let bulk_channel = observations.shell_channels.lock().unwrap()[0];
    let server = observations.transport.get().unwrap().clone();
    let bulk = tokio::spawn(async move {
        // The first shell is deliberately never read. This exceeds its SSH
        // receive window and every bounded downstream queue before uploads.
        for _ in 0..256 {
            server.data(bulk_channel, vec![b'x'; 32768]).await.unwrap();
        }
    });
    tokio::time::timeout(Duration::from_secs(10), bulk)
        .await
        .unwrap()
        .unwrap();
    let local = tempfile::tempdir().unwrap();
    let input = local.path().join("source.bin");
    let bytes = (0..6 * 1024 * 1024)
        .map(|index| (index * 31) as u8)
        .collect::<Vec<_>>();
    let expected = Sha256::digest(&bytes);
    std::fs::write(&input, bytes).unwrap();
    let ids = (0..3)
        .map(|index| {
            enqueue_upload(
                &fixture,
                session,
                &transfers,
                &input,
                &format!("mixed-{index}.bin"),
            )
        })
        .collect::<Vec<_>>();
    wait(&transfers, ids[0], |row| {
        row.state == TransferState::Running
    })
    .await;
    for index in 0..10 {
        let probe = format!("sibling-probe-{index}").into_bytes();
        tokio::time::timeout(Duration::from_secs(2), async {
            fixture.shells[1]
                .control
                .write(probe.clone())
                .await
                .unwrap();
            let mut received = vec![0; probe.len()];
            fixture.shells[1]
                .reader
                .read_exact(&mut received)
                .await
                .unwrap();
            assert_eq!(received, probe);
        })
        .await
        .expect("a saturated sibling and SFTP uploads must not block shell echo");
    }
    for (index, id) in ids.into_iter().enumerate() {
        let result = finished(&transfers, id).await;
        assert_eq!(result.state, TransferState::Completed, "{result:?}");
        assert_eq!(
            Sha256::digest(
                observations
                    .transfer
                    .bytes(&format!("/files/mixed-{index}.bin"))
            ),
            expected
        );
    }
    assert!(observations.transfer.no_partials());
    assert_eq!(observations.auth.load(Ordering::SeqCst), 1);
    assert_eq!(observations.subsystems.load(Ordering::SeqCst), 3);
    assert_eq!(observations.peak_subsystems.load(Ordering::SeqCst), 3);
    fixture.close().await;
    assert_eq!(observations.active_subsystems.load(Ordering::SeqCst), 0);
    assert_eq!(observations.handles.load(Ordering::SeqCst), 0);
}
