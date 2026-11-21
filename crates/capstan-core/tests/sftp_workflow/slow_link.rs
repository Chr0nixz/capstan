//! Shell echo while transfers saturate a slow, high-latency link.
//!
//! On this 1 MiB/s, 80 ms link the unpaced pipelines kept up to about
//! 1.5 MiB of file data queued ahead of a keystroke: the worst echo took
//! 1.37 s during uploads and 0.97 s during downloads. Paced, it stays near
//! 0.25 s with the same transfer time. The bound leaves room for slow CI.
use super::*;
use crate::shaped_link::Link;
use capstan_core::transfer::NativeDestination;
use tokio::time::Instant;

const LINK: Link = Link {
    bytes_per_second: 1024 * 1024,
    one_way: Duration::from_millis(40),
};
const FILE_BYTES: usize = 1024 * 1024;
const WORST_ECHO: Duration = Duration::from_millis(600);

fn body(seed: usize) -> Vec<u8> {
    (0..FILE_BYTES)
        .map(|index| (index * 31 + seed) as u8)
        .collect()
}

fn active(transfers: &TransferService, ids: &[Uuid]) -> bool {
    transfers.snapshot("main").unwrap().iter().any(|row| {
        ids.contains(&row.id)
            && !matches!(
                row.state,
                TransferState::Completed
                    | TransferState::Failed
                    | TransferState::Canceled
                    | TransferState::Interrupted
            )
    })
}

/// Echo round trips on the sibling shell until every transfer has finished.
async fn probe_until_done(
    fixture: &mut Fixture,
    transfers: &TransferService,
    ids: &[Uuid],
) -> Vec<Duration> {
    let mut samples = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    while active(transfers, ids) {
        assert!(Instant::now() < deadline, "transfers did not finish");
        let probe = format!("probe-{:04}", samples.len()).into_bytes();
        let started = Instant::now();
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
        samples.push(started.elapsed());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    samples
}

fn report(direction: &str, samples: &[Duration], elapsed: Duration) -> Duration {
    let mut sorted = samples.to_vec();
    sorted.sort();
    let worst = sorted.last().copied().unwrap_or_default();
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or_default();
    eprintln!(
        "{direction}: {} probes, median {median:?}, worst {worst:?}, transfer {elapsed:?}",
        sorted.len()
    );
    worst
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shell_echo_stays_prompt_while_uploads_fill_a_slow_link() {
    let (mut fixture, session, transfers) = fixture_over(Some(LINK)).await;
    let local = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let ids = (0..3)
        .map(|index| {
            let source = local.path().join(format!("source-{index}.bin"));
            std::fs::write(&source, body(index)).unwrap();
            enqueue_upload(
                &fixture,
                session,
                &transfers,
                &source,
                &format!("slow-{index}.bin"),
            )
        })
        .collect::<Vec<_>>();
    let samples = probe_until_done(&mut fixture, &transfers, &ids).await;
    let elapsed = started.elapsed();
    for (index, id) in ids.into_iter().enumerate() {
        let result = finished(&transfers, id).await;
        assert_eq!(result.state, TransferState::Completed, "{result:?}");
        assert_eq!(
            fixture
                .observations
                .transfer
                .bytes(&format!("/files/slow-{index}.bin")),
            body(index)
        );
    }
    let worst = report("upload", &samples, elapsed);
    fixture.close().await;
    assert!(samples.len() >= 5, "too few probes: {}", samples.len());
    assert!(worst < WORST_ECHO, "upload delayed an echo by {worst:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shell_echo_stays_prompt_while_downloads_fill_a_slow_link() {
    let (mut fixture, session, transfers) = fixture_over(Some(LINK)).await;
    for index in 0..3 {
        fixture
            .observations
            .transfer
            .put(&format!("/files/remote-{index}.bin"), &body(index));
    }
    let page = fixture.list(session, "/files").await.unwrap();
    let local = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let ids = (0..3)
        .map(|index| {
            let entry = page
                .entries
                .iter()
                .find(|entry| entry.name == format!("remote-{index}.bin"))
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
            let destination = Arc::new(NativeDestination::new(
                capstan_core::native_fs::Destination::select(
                    &local.path().join(format!("local-{index}.bin")),
                )
                .unwrap(),
            ));
            let grant = transfers.grant_destination(&context, destination).unwrap();
            transfers.enqueue(context, grant.token).unwrap()
        })
        .collect::<Vec<_>>();
    let samples = probe_until_done(&mut fixture, &transfers, &ids).await;
    let elapsed = started.elapsed();
    for (index, id) in ids.into_iter().enumerate() {
        let result = finished(&transfers, id).await;
        assert_eq!(result.state, TransferState::Completed, "{result:?}");
        assert_eq!(
            std::fs::read(local.path().join(format!("local-{index}.bin"))).unwrap(),
            body(index)
        );
    }
    let worst = report("download", &samples, elapsed);
    fixture.close().await;
    assert!(samples.len() >= 5, "too few probes: {}", samples.len());
    assert!(worst < WORST_ECHO, "download delayed an echo by {worst:?}");
}
