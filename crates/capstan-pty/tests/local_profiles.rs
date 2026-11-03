#![cfg(windows)]

use capstan_pty::{open_local_terminal, ConPtyConfig, ConPtySource, LocalTerminalOptions};
use capstan_terminal::{BinaryFrame, FrameSink, Result, TerminalService, TerminalState};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

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

fn bundled_runtime() -> Option<ConPtyConfig> {
    let directory = PathBuf::from(std::env::var_os("CAPSTAN_TEST_CONPTY_DIR")?);
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    Some(ConPtyConfig {
        bundled_path: Some(directory.join("conpty.dll")),
        bundled_version: Some(manifest["package_version"].as_str().unwrap().to_owned()),
        expected_architecture: Some(std::env::consts::ARCH.to_owned()),
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn selected_wsl_profile_uses_linux_directory_and_packaged_runtime() {
    let Some(distribution) = std::env::var_os("CAPSTAN_TEST_WSL") else {
        eprintln!("SKIP: CAPSTAN_TEST_WSL is not set");
        return;
    };
    let runtime =
        bundled_runtime().expect("set CAPSTAN_TEST_CONPTY_DIR for the WSL compatibility run");
    let (backend, selection) = open_local_terminal(
        LocalTerminalOptions {
            shell_id: Some(format!("wsl:{}", distribution.to_string_lossy())),
            working_directory: Some("/tmp".into()),
        },
        runtime,
    )
    .expect("start the selected WSL profile in a Linux directory");
    assert_eq!(selection.source, ConPtySource::Bundled);
    assert!(selection.version.is_some());
    let service = TerminalService::default();
    let sink = Arc::new(Sink::default());
    let id = service.start(backend, sink.clone()).await.unwrap();
    let session = service.get(&id).unwrap();
    session.resize(100, 30).await.unwrap();
    session
        .write(b"unset HISTFILE; printf '\\nCAPSTAN_WSL_CWD:'; command pwd; exit\r".to_vec())
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(30), session.wait()).await;
    if result.is_err() {
        session.cancel();
    }
    let status = result.expect("WSL process and I/O workers must finish");
    assert_eq!(status.state, TerminalState::Exited);
    assert_eq!(status.exit_code, Some(0));
    assert!(sink
        .0
        .lock()
        .unwrap()
        .windows(b"CAPSTAN_WSL_CWD:/tmp".len())
        .any(|bytes| bytes == b"CAPSTAN_WSL_CWD:/tmp"));
    service.close(&id).await.unwrap();
}

#[test]
fn selected_wsl_profile_rejects_missing_and_windows_directories() {
    let Some(distribution) = std::env::var_os("CAPSTAN_TEST_WSL") else {
        eprintln!("SKIP: CAPSTAN_TEST_WSL is not set");
        return;
    };
    for directory in ["/capstan-directory-that-does-not-exist", "C:\\Windows"] {
        let result = open_local_terminal(
            LocalTerminalOptions {
                shell_id: Some(format!("wsl:{}", distribution.to_string_lossy())),
                working_directory: Some(directory.into()),
            },
            ConPtyConfig::default(),
        );
        assert!(result.is_err());
        assert_eq!(result.err().unwrap().code(), "working_directory_invalid");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn verified_bundled_runtime_starts_and_closes_powershell() {
    let Some(runtime) = bundled_runtime() else {
        eprintln!("SKIP: CAPSTAN_TEST_CONPTY_DIR is not set");
        return;
    };
    let (backend, selection) = open_local_terminal(
        LocalTerminalOptions {
            shell_id: Some("powershell51".into()),
            working_directory: None,
        },
        runtime,
    )
    .unwrap();
    assert_eq!(selection.source, ConPtySource::Bundled);
    let service = TerminalService::default();
    let id = service
        .start(backend, Arc::new(Sink::default()))
        .await
        .unwrap();
    let session = service.get(&id).unwrap();
    session.resize(90, 28).await.unwrap();
    session.write(b"exit 7\r".to_vec()).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(20), session.wait()).await;
    if result.is_err() {
        session.cancel();
    }
    let status = result.expect("bundled PowerShell must close");
    assert_eq!(status.exit_code, Some(7));
    service.close(&id).await.unwrap();
}
