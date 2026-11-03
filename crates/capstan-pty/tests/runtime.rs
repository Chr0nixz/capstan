#![cfg(windows)]

use capstan_pty::{
    ConPtyConfig, ConPtySource, PowerShellPreference, PtyCommand, PtySession, PtySessionConfig,
    ShellDetector,
};
use capstan_terminal::{BinaryFrame, FrameSink, Result, TerminalService, TerminalState};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Sink(Mutex<Vec<u8>>);
impl FrameSink for Sink {
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
async fn invalid_bundle_falls_back_and_real_powershell_output_survives_exit() {
    let mut shell = ShellDetector::new()
        .choose_powershell(PowerShellPreference::Prefer51)
        .expect("Windows PowerShell");
    shell.args = vec![
        "-NoLogo".into(),
        "-NoProfile".into(),
        "-Command".into(),
        "[Console]::Out.Write('capstan-eof-marker'); exit 7".into(),
    ];
    let config = PtySessionConfig {
        command: PtyCommand {
            shell,
            working_directory: None,
            environment: BTreeMap::default(),
        },
        columns: 80,
        rows: 24,
    };
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("conpty.dll");
    let mut invalid_library = vec![0; 128];
    invalid_library[..2].copy_from_slice(b"MZ");
    invalid_library[60..64].copy_from_slice(&64_u32.to_le_bytes());
    invalid_library[64..68].copy_from_slice(b"PE\0\0");
    let machine: u16 = if cfg!(target_arch = "aarch64") {
        0xaa64
    } else if cfg!(target_arch = "x86") {
        0x14c
    } else {
        0x8664
    };
    invalid_library[68..70].copy_from_slice(&machine.to_le_bytes());
    std::fs::write(&path, invalid_library).expect("write incomplete PE");
    let session = PtySession::spawn_with_runtime(
        &config,
        &ConPtyConfig {
            bundled_path: Some(path),
            ..Default::default()
        },
    )
    .expect("system fallback");
    assert_eq!(session.runtime().source, ConPtySource::System);
    assert!(session.runtime().reason.contains("failed to load or start"));
    let service = TerminalService::default();
    let sink = Arc::new(Sink::default());
    let id = service
        .start(session.into_backend().expect("backend"), sink.clone())
        .await
        .expect("start service");
    let handle = service.get(&id).expect("session handle");
    let result = tokio::time::timeout(Duration::from_secs(10), handle.wait()).await;
    if result.is_err() {
        handle.cancel();
    }
    let status = result.expect("PTY I/O workers must exit");
    assert_eq!(
        status.state,
        TerminalState::Exited,
        "{:?}",
        status.error_code
    );
    assert_eq!(status.exit_code, Some(7));
    assert!(sink
        .0
        .lock()
        .unwrap()
        .windows(b"capstan-eof-marker".len())
        .any(|window| window == b"capstan-eof-marker"));
    service.close(&id).await.expect("close exited session");
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_running_powershell_joins_reader_and_reaps_process() {
    let shell = ShellDetector::new()
        .choose_powershell(PowerShellPreference::Prefer51)
        .expect("Windows PowerShell");
    let config = PtySessionConfig {
        command: PtyCommand {
            shell,
            working_directory: None,
            environment: BTreeMap::default(),
        },
        columns: 80,
        rows: 24,
    };
    let session = PtySession::spawn(&config).expect("spawn system PTY");
    let service = TerminalService::default();
    let id = service
        .start(
            session.into_backend().expect("backend"),
            Arc::new(Sink::default()),
        )
        .await
        .expect("service");
    let handle = service.get(&id).expect("handle");
    handle.resize(100, 30).await.expect("resize");
    tokio::time::timeout(Duration::from_secs(10), service.close(&id))
        .await
        .expect("bounded close")
        .expect("close");
    assert_eq!(handle.snapshot().state, TerminalState::Cancelled);
}

#[test]
fn spawn_failure_omits_command_arguments_and_directory() {
    let mut shell = ShellDetector::new()
        .choose_powershell(PowerShellPreference::Prefer51)
        .expect("Windows PowerShell");
    let sensitive_argument = "capstan-test-sensitive-argument";
    let directory = tempfile::tempdir().expect("tempdir");
    let private_directory = directory.path().join("capstan-test-private-directory");
    std::fs::create_dir(&private_directory).expect("working directory");
    shell.executable = directory.path().join("missing-shell.exe");
    let mut config = PtySessionConfig {
        command: PtyCommand {
            shell,
            working_directory: Some(private_directory),
            environment: BTreeMap::default(),
        },
        columns: 80,
        rows: 24,
    };
    for (argument, expected_error) in [
        (sensitive_argument.to_owned(), "CreateProcessW failed"),
        (
            format!("{sensitive_argument}\0"),
            "invalid encoding for command line argument",
        ),
    ] {
        config.command.shell.args = vec![argument];
        let error = PtySession::spawn(&config)
            .expect_err("invalid command must fail")
            .to_string();
        assert!(error.contains(expected_error), "{error}");
        assert!(!error.contains(sensitive_argument));
        assert!(!error.contains("capstan-test-private-directory"));
    }
}
