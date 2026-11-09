#![cfg(unix)]

use capstan_pty::{
    local_shell_profiles, open_local_terminal, ConPtyConfig, ConPtySource, LocalTerminalOptions,
    PtyCommand, PtyError, PtySession, PtySessionConfig, ShellInfo, ShellKind, ShellSource,
};
use capstan_terminal::{BinaryFrame, FrameSink, Result, TerminalService, TerminalState};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Default)]
struct Sink(Mutex<Vec<u8>>);

impl Sink {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

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

async fn wait_for(sink: &Sink, marker: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let text = sink.text();
        if text.contains(marker) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "marker {marker:?} never arrived; received {} bytes",
            text.len()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Shell startup files are skipped so a developer's rc cannot affect the result.
fn plain_shell(directory: &std::path::Path) -> PtySessionConfig {
    PtySessionConfig {
        command: PtyCommand {
            shell: ShellInfo {
                kind: ShellKind::Custom,
                executable: "/bin/sh".into(),
                args: vec!["-i".to_owned()],
                display_name: "sh".to_owned(),
                source: ShellSource::UserProvided,
            },
            working_directory: Some(directory.to_owned()),
            environment: BTreeMap::from([("CAPSTAN_PROBE".to_owned(), "42".to_owned())]),
        },
        columns: 80,
        rows: 24,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_shell_round_trips_bytes_and_honors_directory_environment_and_size() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(directory.path()).unwrap();
    let session = PtySession::spawn(&plain_shell(&canonical)).unwrap();
    assert_eq!(session.runtime().source, ConPtySource::System);
    let service = TerminalService::default();
    let sink = Arc::new(Sink::default());
    let id = service
        .start(session.into_backend().unwrap(), sink.clone())
        .await
        .unwrap();
    let terminal = service.get(&id).unwrap();
    terminal.resize(101, 33).await.unwrap();
    terminal
        .write(
            b"printf 'RESULT:%s:%s:' \"$CAPSTAN_PROBE\" \"$(pwd -P)\"; stty size; exit\n".to_vec(),
        )
        .await
        .unwrap();
    let expected = format!("RESULT:42:{}:33 101", canonical.display());
    wait_for(&sink, &expected).await;
    let finished = tokio::time::timeout(Duration::from_secs(10), terminal.wait())
        .await
        .expect("shell never exited");
    assert_eq!(finished.state, TerminalState::Exited);
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_a_running_shell_reaps_it() {
    let directory = tempfile::tempdir().unwrap();
    let session = PtySession::spawn(&plain_shell(directory.path())).unwrap();
    let service = TerminalService::default();
    let id = service
        .start(session.into_backend().unwrap(), Arc::new(Sink::default()))
        .await
        .unwrap();
    let terminal = service.get(&id).unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(10), terminal.close())
        .await
        .expect("closing must not hang");
    assert!(closed.state != TerminalState::Running);
}

#[test]
fn profiles_expose_the_login_shell_first_without_paths() {
    let profiles = local_shell_profiles();
    assert_eq!(
        profiles.first().map(|shell| shell.id.as_str()),
        Some("login")
    );
    assert!(profiles.iter().all(|shell| !shell.is_wsl));
    assert!(profiles.iter().all(|shell| !shell.name.contains('/')));
}

#[test]
fn unknown_profiles_and_missing_directories_are_refused_before_spawning() {
    let unknown = open_local_terminal(
        LocalTerminalOptions {
            shell_id: Some("powershell7".to_owned()),
            working_directory: None,
        },
        ConPtyConfig::default(),
    );
    assert!(matches!(unknown, Err(PtyError::NoShell)));
    let missing = open_local_terminal(
        LocalTerminalOptions {
            shell_id: None,
            working_directory: Some("/definitely/not/here".to_owned()),
        },
        ConPtyConfig::default(),
    );
    assert!(matches!(missing, Err(PtyError::InvalidWorkingDirectory)));
}

/// Exercises the real default path, so it reads the developer's startup files;
/// a marker in the output keeps prompts and rc noise from affecting the result.
#[tokio::test(flavor = "multi_thread")]
async fn default_profile_is_a_login_shell_with_terminal_identity() {
    let directory = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(directory.path()).unwrap();
    let (backend, selection) = open_local_terminal(
        LocalTerminalOptions {
            shell_id: None,
            working_directory: Some(canonical.to_string_lossy().into_owned()),
        },
        ConPtyConfig::default(),
    )
    .unwrap();
    assert_eq!(selection.source, ConPtySource::System);
    let service = TerminalService::default();
    let sink = Arc::new(Sink::default());
    let id = service.start(backend, sink.clone()).await.unwrap();
    let terminal = service.get(&id).unwrap();
    terminal
        .write(
            b" printf 'PROBE:%s:%s:%s:%s\\n' \"$TERM\" \"$COLORTERM\" \"$TERM_PROGRAM\" \"$(pwd -P)\"\r"
                .to_vec(),
        )
        .await
        .unwrap();
    let expected = format!(
        "PROBE:xterm-256color:truecolor:Capstan:{}",
        canonical.display()
    );
    wait_for(&sink, &expected).await;
    terminal.close().await;
}
