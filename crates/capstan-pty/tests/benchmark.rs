#![cfg(all(windows, feature = "m0-benchmarks"))]
use capstan_pty::{
    benchmark::{open_fixture, Scenario},
    ConPtyConfig,
};
use capstan_terminal::{BinaryFrame, FrameSink, Result, TerminalService, TerminalState};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Default)]
struct Collect(Mutex<Vec<u8>>);
impl FrameSink for Collect {
    fn send<'a>(
        &'a self,
        frame: BinaryFrame,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.0.lock().unwrap().extend(frame.payload);
            Ok(())
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn real_fixture_exchanges_probes_resizes_and_drains_eof_with_matching_digest() {
    let (backend, (_, digest)) = open_fixture(
        Path::new(env!("CARGO_BIN_EXE_capstan-m0-fixture")),
        Scenario::Idle,
        &ConPtyConfig::default(),
    )
    .unwrap();
    let service = TerminalService::default();
    let sink = Arc::new(Collect::default());
    let id = service.start(backend, sink.clone()).await.unwrap();
    let session = service.get(&id).unwrap();
    session.resize(100, 30).await.unwrap();
    session.write(b"probe:000042\r".to_vec()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let found = sink
                .0
                .lock()
                .unwrap()
                .windows(b"CAPSTAN_M0_PROBE:000042".len())
                .any(|bytes| bytes == b"CAPSTAN_M0_PROBE:000042");
            if found {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("fixture probe output");
    session.write(b"exit\r".to_vec()).await.unwrap();
    let final_state = tokio::time::timeout(Duration::from_secs(5), session.wait())
        .await
        .unwrap();
    assert_eq!(final_state.state, TerminalState::Exited);
    assert_eq!(final_state.exit_code, Some(0));
    let output = sink.0.lock().unwrap().clone();
    let expected = output.iter().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    });
    assert_eq!(digest.lock().unwrap().bytes, output.len() as u64);
    assert_eq!(digest.lock().unwrap().checksum, expected);
    assert!(digest.lock().unwrap().eof);
    session.ack(output.len()).await.unwrap();
    assert_eq!(session.output_metrics().await.in_flight_bytes, 0);
    service.close(&id).await.unwrap();
    assert_eq!(service.registered_sessions(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn twenty_real_fixture_sessions_are_reaped_before_close_all_returns() {
    let service = TerminalService::default();
    for _ in 0..20 {
        let (backend, _) = open_fixture(
            Path::new(env!("CARGO_BIN_EXE_capstan-m0-fixture")),
            Scenario::Idle,
            &ConPtyConfig::default(),
        )
        .unwrap();
        service
            .start(backend, Arc::new(Collect::default()))
            .await
            .unwrap();
    }
    assert_eq!(service.registered_sessions(), 20);
    tokio::time::timeout(Duration::from_secs(10), service.close_all())
        .await
        .expect("all fixture tasks joined");
    assert_eq!(service.registered_sessions(), 0);
}
