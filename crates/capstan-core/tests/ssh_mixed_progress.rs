//! Headless progress diagnosis through production SSH, terminal and SFTP services.
//! This does not measure `WebView` input latency, rendering or desktop resources.
#![cfg(all(windows, feature = "benchmarks"))]
#![allow(clippy::pedantic)]

use capstan_core::{
    benchmark::{
        transfer_load::{Config, TransferLoad},
        SshFixture,
    },
    transfer::Direction,
};
use capstan_terminal::{BinaryFrame, FrameSink, TerminalError, TerminalService, TerminalSession};
use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_util::sync::CancellationToken;

type ProgressResult<T> = Result<T, &'static str>;
const PROBES: usize = 8;
const END: &[u8] = b"CAPSTAN_M0_END";

struct Sink(mpsc::Sender<BinaryFrame>);
impl FrameSink for Sink {
    fn send<'a>(
        &'a self,
        frame: BinaryFrame,
    ) -> Pin<Box<dyn Future<Output = capstan_terminal::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.0
                .send(frame)
                .await
                .map_err(|_| TerminalError::OutputSend("fixture sink closed".into()))
        })
    }
}

#[derive(Default)]
struct Observed {
    sequence: u64,
    bytes: usize,
    tail: Vec<u8>,
    ended: bool,
    probes: [bool; PROBES],
    held: bool,
    held_bytes: usize,
    failed: bool,
}
struct Receiver {
    observed: Arc<Mutex<Observed>>,
    session: Arc<TerminalSession>,
    task: JoinHandle<()>,
}
impl Receiver {
    async fn start(
        terminals: &TerminalService,
        shell: capstan_ssh::SshShell,
        held: bool,
        stop: CancellationToken,
    ) -> ProgressResult<Self> {
        let (sender, mut receiver) = mpsc::channel::<BinaryFrame>(32);
        let id = terminals
            .start_async(shell.into_terminal_parts(), Arc::new(Sink(sender)))
            .await
            .map_err(|_| "terminal_start")?;
        let session = terminals.get(&id).map_err(|_| "terminal_get")?;
        let observed = Arc::new(Mutex::new(Observed {
            held,
            ..Observed::default()
        }));
        let state = observed.clone();
        let terminal = session.clone();
        let markers: Vec<Vec<u8>> = (0..PROBES)
            .map(|sequence| format!("CAPSTAN_M0_PROBE:{sequence:06}").into_bytes())
            .collect();
        let task = tokio::spawn(async move {
            loop {
                let frame = tokio::select! {
                    () = stop.cancelled() => break,
                    frame = receiver.recv() => match frame { Some(frame) => frame, None => break },
                };
                let acknowledged = {
                    let mut state = state.lock().unwrap();
                    if frame.sequence != state.sequence
                        || frame.payload.len() > capstan_terminal::MAX_FRAME_SIZE
                    {
                        state.failed = true;
                        break;
                    }
                    state.sequence += 1;
                    state.bytes += frame.payload.len();
                    let mut bytes = std::mem::take(&mut state.tail);
                    bytes.extend_from_slice(&frame.payload);
                    state.ended |= bytes.windows(END.len()).any(|window| window == END);
                    // Only the idle receiver parses synthetic probe markers.
                    if !held {
                        for (sequence, marker) in markers.iter().enumerate() {
                            state.probes[sequence] |=
                                bytes.windows(marker.len()).any(|window| window == marker);
                        }
                    }
                    let start = bytes.len().saturating_sub(32);
                    state.tail.extend_from_slice(&bytes[start..]);
                    if state.held {
                        state.held_bytes += frame.payload.len();
                        0
                    } else {
                        frame.payload.len()
                    }
                };
                if acknowledged > 0 && terminal.ack(acknowledged).await.is_err() {
                    state.lock().unwrap().failed = true;
                    break;
                }
            }
        });
        Ok(Self {
            observed,
            session,
            task,
        })
    }
    async fn release(&self) -> ProgressResult<()> {
        let held = {
            let mut state = self.observed.lock().unwrap();
            state.held = false;
            std::mem::take(&mut state.held_bytes)
        };
        if held > 0 {
            self.session.ack(held).await.map_err(|_| "release_ack")?;
        }
        Ok(())
    }
}

async fn diagnostic(stage: &str, receivers: &[Receiver], load: Option<&TransferLoad>) {
    let mut terminals = Vec::new();
    for receiver in receivers {
        let observed = {
            let state = receiver.observed.lock().unwrap();
            serde_json::json!({"frames":state.sequence,"bytes":state.bytes,
                "ended":state.ended,"probes":state.probes.iter().filter(|seen| **seen).count(),
                "held":state.held,"heldBytes":state.held_bytes,"failed":state.failed,
                "receiverFinished":receiver.task.is_finished()})
        };
        let metrics =
            tokio::time::timeout(Duration::from_secs(1), receiver.session.output_metrics())
                .await
                .ok();
        terminals.push(serde_json::json!({"observed":observed,"metrics":metrics}));
    }
    let transfers = load.and_then(|load| load.diagnostic_progress().ok());
    eprintln!(
        "HEADLESS_DIAGNOSTIC {}",
        serde_json::json!({"stage":stage,"terminals":terminals,"transfers":transfers})
    );
}

async fn until(mut ready: impl FnMut() -> bool) {
    while !ready() {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn exercise(
    fixture: &SshFixture,
    config: Config,
    direction: Direction,
) -> ProgressResult<serde_json::Value> {
    let terminals = TerminalService::default();
    let stop = CancellationToken::new();
    let mut receivers: Vec<Receiver> = Vec::new();
    let mut load = None;
    let mut probe_samples = Vec::new();
    // Only the diagnostic lifetime scales; probe/pause/drain SLAs are unchanged.
    let lifetime = if config.smoke { 150 } else { 900 };
    let mut outcome = tokio::time::timeout(Duration::from_secs(lifetime), async {
        let (host, bulk, lease) = fixture
            .open_with_lease(true, false, CancellationToken::new())
            .await
            .map_err(|_| "bulk_open")?;
        let connection = bulk.connection_id();
        let (_, idle) = fixture
            .open(false, false, CancellationToken::new())
            .await
            .map_err(|_| "idle_open")?;
        if idle.connection_id() != connection || fixture.pool_entries().await != 1 {
            return Err("connection_reuse");
        }
        receivers.push(Receiver::start(&terminals, bulk, true, stop.clone()).await?);
        receivers.push(Receiver::start(&terminals, idle, false, stop.clone()).await?);
        tokio::time::timeout(Duration::from_secs(15), async {
            while receivers[0].session.output_metrics().await.pause_count == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .map_err(|_| "bulk_pause_deadline")?;
        eprintln!("HEADLESS_STAGE bulk_paused");
        load = Some(
            TransferLoad::start(config, host, connection, lease, direction)
                .await
                .map_err(|_| "transfer_start")?,
        );
        let active = load.as_ref().ok_or("transfer_missing")?;
        tokio::time::timeout(Duration::from_secs(20), async {
            while !active.all_running().map_err(|_| "transfer_snapshot")? {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Ok::<_, &'static str>(())
        })
        .await
        .map_err(|_| "all_running_deadline")??;
        for sequence in 0..PROBES {
            if !active.all_running().map_err(|_| "transfer_snapshot")? {
                return Err("transfer_finished_before_probe");
            }
            let started = Instant::now();
            let mut submitted_ms = 0.0;
            tokio::time::timeout(Duration::from_secs(5), async {
                receivers[1]
                    .session
                    .write(format!("probe:{sequence:06}\r").into_bytes())
                    .await
                    .map_err(|_| "probe_write")?;
                submitted_ms = started.elapsed().as_secs_f64() * 1000.0;
                until(|| {
                    let state = receivers[1].observed.lock().unwrap();
                    state.probes[sequence] || state.failed
                })
                .await;
                if receivers[1].observed.lock().unwrap().failed {
                    return Err("probe_receiver");
                }
                Ok(())
            })
            .await
            .map_err(|_| "probe_deadline")??;
            let response_ms = started.elapsed().as_secs_f64() * 1000.0;
            let sample = serde_json::json!({"direction":direction,"sequence":sequence,
                "submissionMs":submitted_ms,"echoWaitMs":response_ms-submitted_ms,
                "responseMs":response_ms});
            eprintln!("HEADLESS_PROBE {sample}");
            probe_samples.push(sample);
            if !active.all_running().map_err(|_| "transfer_snapshot")? {
                return Err("transfer_finished_during_probe");
            }
        }
        eprintln!("HEADLESS_STAGE sibling_probes_complete");
        diagnostic("before_release", &receivers, Some(active)).await;
        receivers[0].release().await?;
        tokio::time::timeout(Duration::from_secs(30), async {
            while !receivers[0].observed.lock().unwrap().ended {
                tokio::time::sleep(Duration::from_secs(5)).await;
                diagnostic("bulk_drain", &receivers, Some(active)).await;
            }
        })
        .await
        .map_err(|_| "bulk_drain_deadline")?;
        for receiver in &receivers {
            receiver
                .session
                .write(b"exit\r".to_vec())
                .await
                .map_err(|_| "exit_write")?;
            let exited = tokio::time::timeout(Duration::from_secs(10), receiver.session.wait())
                .await
                .map_err(|_| "terminal_exit_deadline")?;
            if exited.exit_code != Some(0) {
                return Err("terminal_exit");
            }
            if receiver.observed.lock().unwrap().failed {
                return Err("receiver_failed");
            }
        }
        eprintln!("HEADLESS_STAGE terminals_drained");
        let result = active.finish().await.map_err(|_| "transfer_finish")?;
        if result.connection_id != connection
            || result.hashes.len() != 3
            || result.final_owned_payload_admission_bytes != 0
            || result.active_jobs_after != 0
        {
            return Err("transfer_evidence");
        }
        let bulk = receivers[0].session.output_metrics().await;
        let idle = receivers[1].session.output_metrics().await;
        if bulk.pause_count == 0
            || bulk.resume_count == 0
            || bulk.in_flight_bytes != 0
            || idle.in_flight_bytes != 0
            || receivers[0].observed.lock().unwrap().bytes < 100 * 1024 * 1024
        {
            return Err("terminal_evidence");
        }
        Ok(
            serde_json::json!({"headless":true,"sshConnections":1,"poolEntriesDuring":1,
            "inputProbes":PROBES,"inputProbeSamples":probe_samples,
            "bulkMetrics":bulk,"idleMetrics":idle,"transfer":result,
            "performanceAcceptance":false,"desktopAcceptance":false}),
        )
    })
    .await
    .map_err(|_| "mixed_progress_deadline")
    .and_then(|result| result);

    if outcome.is_err() {
        diagnostic("before_failure_cleanup", &receivers, load.as_ref()).await;
    }
    let mut cleanup_errors = Vec::new();
    if let Some(load) = &load {
        if !matches!(
            tokio::time::timeout(Duration::from_secs(35), load.cancel()).await,
            Ok(Ok(()))
        ) {
            cleanup_errors.push("load_cleanup");
        }
    }
    if tokio::time::timeout(Duration::from_secs(10), terminals.close_all())
        .await
        .is_err()
    {
        cleanup_errors.push("terminal_cleanup_deadline");
    }
    stop.cancel();
    for mut receiver in receivers {
        match tokio::time::timeout(Duration::from_secs(3), &mut receiver.task).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => cleanup_errors.push("receiver_join"),
            Err(_) => {
                receiver.task.abort();
                let _ = receiver.task.await;
                cleanup_errors.push("receiver_cleanup_deadline");
            }
        }
    }
    if tokio::time::timeout(Duration::from_secs(10), fixture.disconnect())
        .await
        .is_err()
    {
        cleanup_errors.push("fixture_disconnect_deadline");
    }
    if terminals.registered_sessions() != 0 || fixture.pool_entries().await != 0 {
        cleanup_errors.push("registration_cleanup");
    }
    eprintln!(
        "HEADLESS_CLEANUP {}",
        serde_json::json!({"primaryFailure":outcome.as_ref().err(),"cleanupErrors":cleanup_errors,
            "registeredSessionsAfter":terminals.registered_sessions(),
            "poolEntriesAfter":fixture.pool_entries().await})
    );
    if outcome.is_ok() {
        if let Some(error) = cleanup_errors.first() {
            outcome = Err(*error);
        }
    }
    let mut sample = outcome?;
    sample["poolEntriesAfter"] = serde_json::json!(0);
    sample["registeredSessionsAfter"] = serde_json::json!(0);
    Ok(sample)
}

#[tokio::test]
async fn configured_openssh_mixed_progress_without_webview() {
    if std::env::var("CAPSTAN_TEST_HEADLESS_MIXED").as_deref() != Ok("1") {
        eprintln!("SKIP: configure the isolated headless mixed fixture explicitly");
        return;
    }
    let fixture = SshFixture::load(PathBuf::from(
        std::env::var_os("CAPSTAN_M1_FIXTURE_CONFIG").unwrap(),
    ))
    .await
    .unwrap();
    let config = Config::load(&PathBuf::from(
        std::env::var_os("CAPSTAN_M2_LOAD_CONFIG").unwrap(),
    ))
    .unwrap();
    // Config::load admits only exact 64 MiB smoke or 1 GiB full data.
    // Headless correctness/progress still does not measure visible latency.
    for direction in [Direction::Upload, Direction::Download] {
        let sample = exercise(&fixture, config.clone(), direction)
            .await
            .unwrap_or_else(|code| panic!("Headless mixed progress failed: {code}"));
        println!("HEADLESS_MIXED_SAMPLE {sample}");
    }
}
