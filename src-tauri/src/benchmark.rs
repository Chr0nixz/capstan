//! Development-only, fixed-fixture IPC measurement adapters.
use crate::{
    tauri_channel::{forward_status, StartedTerminal, TauriBinaryChannel},
    CommandError,
};
use capstan_pty::benchmark::{ReadDigest, Scenario, SharedDigest};
use capstan_terminal::{TerminalService, TerminalSession, TerminalSessionId, TerminalSnapshot};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use tauri::{
    ipc::{Channel, InvokeResponseBody},
    State, WebviewWindow,
};

#[cfg(windows)]
pub mod transfer_load;

pub struct BenchmarkState {
    fixtures: Mutex<HashMap<TerminalSessionId, FixtureRecord>>,
    admissions: AtomicUsize,
    reported: std::sync::atomic::AtomicBool,
    report_directory: Option<PathBuf>,
    ssh_config: Option<PathBuf>,
    m2_config: Option<PathBuf>,
    #[cfg(windows)]
    m2: transfer_load::LoadState,
    ssh: tokio::sync::OnceCell<Arc<capstan_core::benchmark::SshFixture>>,
    visibility: Mutex<(u64, u64, u64)>,
    startup_failures: Arc<Mutex<Vec<&'static str>>>,
    diagnostic: bool,
    diagnostic_events: AtomicUsize,
}
impl Default for BenchmarkState {
    fn default() -> Self {
        Self {
            fixtures: Mutex::default(),
            admissions: AtomicUsize::new(0),
            reported: std::sync::atomic::AtomicBool::new(false),
            report_directory: std::env::var_os("CAPSTAN_M0_REPORT_DIR").map(PathBuf::from),
            ssh_config: std::env::var_os("CAPSTAN_M1_FIXTURE_CONFIG").map(PathBuf::from),
            m2_config: std::env::var_os("CAPSTAN_M2_LOAD_CONFIG").map(PathBuf::from),
            #[cfg(windows)]
            m2: transfer_load::LoadState::default(),
            ssh: tokio::sync::OnceCell::new(),
            visibility: Mutex::new((0, 0, 0)),
            startup_failures: Arc::default(),
            diagnostic: std::env::var("CAPSTAN_M2_DIAGNOSTIC").is_ok_and(|value| value == "1"),
            diagnostic_events: AtomicUsize::new(0),
        }
    }
}
fn trace_stage(state: &BenchmarkState, stage: &'static str) {
    if state.diagnostic && state.diagnostic_events.fetch_add(1, Ordering::Relaxed) < 4096 {
        // Fixed stage labels only: never paths, credentials or terminal bytes.
        eprintln!("M2_STAGE {stage}");
    }
}
pub(crate) fn trace_input(id: &TerminalSessionId, stage: &'static str) {
    static EVENTS: AtomicUsize = AtomicUsize::new(0);
    if std::env::var("CAPSTAN_M2_DIAGNOSTIC").is_ok_and(|value| value == "1")
        && EVENTS.fetch_add(1, Ordering::Relaxed) < 4096
    {
        // Session IDs and fixed labels only; never input or output contents.
        eprintln!("M2_INPUT {} {stage}", id.0);
    }
}
fn require_source(window: &WebviewWindow, state: &BenchmarkState) -> Result<(), CommandError> {
    if window.label() != "main" {
        return Err(CommandError::new("window_source"));
    }
    if let Ok(mut visibility) = state.visibility.lock() {
        visibility.0 += 1;
        visibility.1 += u64::from(window.is_visible().unwrap_or(false));
        visibility.2 += u64::from(window.is_focused().unwrap_or(false));
    }
    if state
        .report_directory
        .as_ref()
        .is_none_or(|path| !path.is_absolute() || !path.is_dir())
    {
        return Err(CommandError::new("benchmark_disabled"));
    }
    Ok(())
}

#[tauri::command]
pub fn m0_benchmark_enabled(window: WebviewWindow, state: State<'_, BenchmarkState>) -> bool {
    require_source(&window, &state).is_ok()
}

#[tauri::command]
pub fn m0_benchmark_mixed(
    window: WebviewWindow,
    state: State<'_, BenchmarkState>,
) -> Result<bool, CommandError> {
    require_source(&window, &state)?;
    Ok(state.ssh_config.is_some())
}

#[cfg(not(windows))]
#[tauri::command]
pub fn m2_benchmark_round(
    window: WebviewWindow,
    state: State<'_, BenchmarkState>,
) -> Result<Option<u8>, CommandError> {
    require_source(&window, &state)?;
    if state.m2_config.is_some() {
        return Err(CommandError::new("benchmark_unsupported"));
    }
    Ok(None)
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureBackend {
    #[default]
    Local,
    Ssh,
    SshAlternate,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum StartedFixture {
    Local(StartedTerminal),
    Ssh(StartedSshFixture),
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartedSshFixture {
    kind: &'static str,
    id: TerminalSessionId,
    host_id: uuid::Uuid,
    connection_id: uuid::Uuid,
}

#[tauri::command]
pub async fn m0_start_fixture(
    window: WebviewWindow,
    state: State<'_, BenchmarkState>,
    manager: State<'_, Arc<TerminalService>>,
    scenario: Scenario,
    backend: Option<FixtureBackend>,
    on_output: Channel<InvokeResponseBody>,
    on_status: Channel<TerminalSnapshot>,
) -> Result<StartedFixture, CommandError> {
    require_source(&window, &state)?;
    state
        .admissions
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            (value < 128).then_some(value + 1)
        })
        .map_err(|_| CommandError::new("benchmark_limit"))?;
    let backend = backend.unwrap_or_default();
    trace_stage(
        &state,
        if matches!(backend, FixtureBackend::Local) {
            "fixture.local.begin"
        } else {
            "fixture.ssh.begin"
        },
    );
    if !matches!(backend, FixtureBackend::Local) {
        let config = state
            .ssh_config
            .clone()
            .ok_or_else(|| CommandError::new("benchmark_disabled"))?;
        let ssh = state
            .ssh
            .get_or_try_init(|| async {
                capstan_core::benchmark::SshFixture::load(config)
                    .await
                    .map(Arc::new)
                    .map_err(|error| {
                        state
                            .startup_failures
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(error.code());
                        CommandError::new(error.code())
                    })
            })
            .await?
            .clone();
        trace_stage(&state, "fixture.ssh.configured");
        let failures = state.startup_failures.clone();
        let (id, (host_id, connection_id, digest, algorithms, _lease)) = manager
            .inner()
            .clone()
            .open_async(
                move |cancel| async move {
                    let (host, shell, lease) = ssh
                        .open_with_lease(
                            scenario == Scenario::Bulk,
                            matches!(backend, FixtureBackend::SshAlternate),
                            cancel,
                        )
                        .await
                        .map_err(|error| {
                            failures
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .push(error.code());
                            crate::ssh_start_error(error)
                        })?;
                    let connection_id = shell.connection_id();
                    let algorithms = shell.negotiated_algorithms();
                    let (parts, digest) =
                        capstan_pty::benchmark::measure_async_backend(shell.into_terminal_parts());
                    Ok((parts, (host, connection_id, digest, algorithms, lease)))
                },
                tokio_util::sync::CancellationToken::new(),
                Arc::new(TauriBinaryChannel::new(on_output)),
            )
            .await?;
        let session = manager.get(&id)?;
        trace_stage(&state, "fixture.ssh.started");
        forward_status(session.clone(), on_status);
        state
            .fixtures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                id.clone(),
                FixtureRecord {
                    digest,
                    session,
                    source: None,
                    version: None,
                    architecture: std::env::consts::ARCH.to_owned(),
                    scenario,
                    backend,
                    connection_id: Some(connection_id),
                    #[cfg(windows)]
                    host_id: Some(host_id),
                    #[cfg(windows)]
                    lease: state.m2_config.is_some().then_some(_lease),
                    algorithms,
                },
            );
        return Ok(StartedFixture::Ssh(StartedSshFixture {
            kind: "ssh",
            id,
            host_id,
            connection_id,
        }));
    }
    let application = std::env::current_exe().map_err(|_| CommandError::new("runtime_path"))?;
    let fixture = application.with_file_name(if cfg!(windows) {
        "capstan-m0-fixture.exe"
    } else {
        "capstan-m0-fixture"
    });
    let (id, (runtime, digest)) = manager
        .open(
            move || {
                capstan_pty::benchmark::open_fixture(
                    &fixture,
                    scenario,
                    &capstan_pty::ConPtyConfig::for_executable(&application),
                )
                .map_err(|_| {
                    capstan_terminal::TerminalError::Backend("benchmark_fixture".to_owned())
                })
            },
            Arc::new(TauriBinaryChannel::new(on_output)),
        )
        .await?;
    let session = manager.get(&id)?;
    forward_status(session.clone(), on_status);
    state
        .fixtures
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            id.clone(),
            FixtureRecord {
                digest,
                session,
                source: Some(runtime.source),
                version: runtime.version.clone(),
                architecture: runtime.architecture.clone(),
                scenario,
                backend,
                connection_id: None,
                #[cfg(windows)]
                host_id: None,
                #[cfg(windows)]
                lease: None,
                algorithms: None,
            },
        );
    Ok(StartedFixture::Local(StartedTerminal {
        kind: "local",
        id,
        runtime,
    }))
}

#[derive(Serialize)]
pub struct SourceSample {
    digest: ReadDigest,
    snapshot: TerminalSnapshot,
    runtime_source: Option<capstan_pty::ConPtySource>,
    runtime_version: Option<String>,
    runtime_architecture: String,
    fixture_generated_bytes: u64,
    backend: FixtureBackend,
    connection_id: Option<uuid::Uuid>,
    algorithms: Option<capstan_ssh::NegotiatedAlgorithms>,
}
#[derive(Clone)]
struct FixtureRecord {
    digest: SharedDigest,
    session: Arc<TerminalSession>,
    source: Option<capstan_pty::ConPtySource>,
    version: Option<String>,
    architecture: String,
    scenario: Scenario,
    backend: FixtureBackend,
    connection_id: Option<uuid::Uuid>,
    #[cfg(windows)]
    host_id: Option<uuid::Uuid>,
    #[cfg(windows)]
    lease: Option<capstan_ssh::ConnectionLease<capstan_ssh::PasswordConnection>>,
    algorithms: Option<capstan_ssh::NegotiatedAlgorithms>,
}
impl FixtureRecord {
    async fn sample(&self) -> SourceSample {
        let mut snapshot = self.session.snapshot();
        snapshot.metrics = self.session.output_metrics().await;
        SourceSample {
            digest: self
                .digest
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
            snapshot,
            runtime_source: self.source,
            runtime_version: self.version.clone(),
            runtime_architecture: self.architecture.clone(),
            backend: self.backend,
            connection_id: self.connection_id,
            algorithms: self.algorithms.clone(),
            fixture_generated_bytes: match self.scenario {
                Scenario::Bulk => capstan_pty::benchmark::BULK_BYTES,
                Scenario::Idle => 0,
            },
        }
    }
}
#[tauri::command]
pub async fn m0_source_sample(
    window: WebviewWindow,
    state: State<'_, BenchmarkState>,
    id: TerminalSessionId,
) -> Result<SourceSample, CommandError> {
    require_source(&window, &state)?;
    trace_stage(&state, "source.begin");
    let fixture = state
        .fixtures
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&id)
        .cloned()
        .ok_or_else(|| CommandError::new("session_closed"))?;
    let sample = fixture.sample().await;
    trace_stage(&state, "source.ready");
    Ok(sample)
}

#[tauri::command]
pub fn m0_registered_sessions(
    window: WebviewWindow,
    state: State<'_, BenchmarkState>,
    manager: State<'_, Arc<TerminalService>>,
) -> Result<usize, CommandError> {
    require_source(&window, &state)?;
    Ok(manager.registered_sessions())
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: u8,
    pub viewport: Viewport,
    pub runs: Vec<Run>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Viewport {
    width: u32,
    height: u32,
    scale: f64,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    Bulk,
    DelayedAck,
    TabLifecycle,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    Timeout,
    Integrity,
    Input,
    Backend,
    Renderer,
    Cleanup,
    Cancelled,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub kind: RunKind,
    pub round: u8,
    pub elapsed_ms: f64,
    pub renderer_peak: u8,
    pub renderers_after: u8,
    pub registered_after: u32,
    pub input_response_ms: Vec<f64>,
    #[serde(default)]
    pub isolation_input_response_ms: Vec<f64>,
    pub terminals: Vec<ReceiverSample>,
    pub failure: Option<Failure>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiverSample {
    pub id: TerminalSessionId,
    pub bytes: u64,
    pub checksum: u32,
    pub frames: u32,
    pub raw_binary: bool,
    pub parsed_bytes: u64,
    pub acked_bytes: u64,
    pub max_pending_bytes: u64,
    #[serde(default)]
    pub bulk_elapsed_ms: f64,
    #[serde(default)]
    pub input_response_ms: Vec<f64>,
}

impl Report {
    fn validate(&self) -> bool {
        self.schema == 1
            && self.viewport.width > 0
            && self.viewport.height > 0
            && (0.5..=8.0).contains(&self.viewport.scale)
            && !self.runs.is_empty()
            && self.runs.len() <= 5
            && self.runs.iter().all(|run| {
                run.round <= 3
                    && run.renderer_peak <= 20
                    && (0.0..=3_600_000.0).contains(&run.elapsed_ms)
                    && run.terminals.len() <= 20
                    && run.input_response_ms.len() <= 1000
                    && run.isolation_input_response_ms.len() <= 1000
                    && run
                        .isolation_input_response_ms
                        .iter()
                        .all(|value| (0.0..=3_600_000.0).contains(value))
                    && run.terminals.iter().all(|value| {
                        (0.0..=3_600_000.0).contains(&value.bulk_elapsed_ms)
                            && value.input_response_ms.len() <= 1000
                            && value
                                .input_response_ms
                                .iter()
                                .all(|value| (0.0..=3_600_000.0).contains(value))
                    })
                    && run
                        .input_response_ms
                        .iter()
                        .all(|value| (0.0..=3_600_000.0).contains(value))
            })
    }
}

#[tauri::command]
pub async fn m0_finish_report(
    window: WebviewWindow,
    state: State<'_, BenchmarkState>,
    manager: State<'_, Arc<TerminalService>>,
    report: Report,
) -> Result<(), CommandError> {
    require_source(&window, &state)?;
    if !report.validate() || manager.registered_sessions() != 0 {
        return Err(CommandError::new("benchmark_report"));
    }
    #[cfg(windows)]
    if state.m2.active.lock().await.is_some() {
        return Err(CommandError::new("benchmark_load_active"));
    }
    let directory = state
        .report_directory
        .clone()
        .ok_or_else(|| CommandError::new("benchmark_disabled"))?;
    // Source evidence is supplied by Rust, never accepted from a WebView payload.
    let fixtures: Vec<_> = state
        .fixtures
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .map(|(id, fixture)| (id.0.clone(), fixture.clone()))
        .collect();
    let mut sources = HashMap::new();
    for (id, fixture) in fixtures {
        sources.insert(id, fixture.sample().await);
    }
    if report
        .runs
        .iter()
        .flat_map(|run| &run.terminals)
        .any(|sample| !sources.contains_key(&sample.id.0))
    {
        return Err(CommandError::new("benchmark_report"));
    }
    let window_visible = window
        .is_visible()
        .map_err(|_| CommandError::new("window_operation"))?;
    let (pool_before_disconnect, pool_after_disconnect) = if let Some(ssh) = state.ssh.get() {
        let before = ssh.pool_entries().await;
        ssh.disconnect().await;
        (before, ssh.pool_entries().await)
    } else {
        (0, 0)
    };
    let visibility = *state
        .visibility
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let startup_failures = state
        .startup_failures
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    #[cfg(windows)]
    let transfer_loads = state
        .m2
        .completed
        .lock()
        .map_err(|_| CommandError::new("benchmark_report"))?
        .clone();
    #[cfg(not(windows))]
    let transfer_loads: Vec<()> = Vec::new();
    let value = serde_json::to_vec_pretty(&serde_json::json!({
        "receiver": report, "sources": sources, "window_visible": window_visible,
        "measurement_mode": if state.m2_config.is_some() { "m2_mixed" } else if state.ssh_config.is_some() { "m1_mixed" } else { "m0_local" },
        "transfer_loads": transfer_loads, "diagnostic_mode": state.diagnostic,
        "visibility_samples": { "total": visibility.0, "visible": visibility.1, "focused": visibility.2 },
        "ssh_pool_before_disconnect": pool_before_disconnect, "ssh_pool_after_disconnect": pool_after_disconnect,
        "startup_failure_codes": startup_failures,
    }))
        .map_err(|_| CommandError::new("benchmark_report"))?;
    if state.reported.swap(true, Ordering::AcqRel) {
        return Err(CommandError::new("benchmark_report_exists"));
    }
    tauri::async_runtime::spawn_blocking(move || -> Result<(), CommandError> {
        use std::io::Write;
        // Never overwrite an earlier run's evidence; only this fixed filename is exposed.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("terminal-report.pending"))
            .map_err(|_| CommandError::new("benchmark_report_exists"))?;
        file.write_all(&value)
            .and_then(|()| file.sync_all())
            .map_err(|_| CommandError::new("benchmark_report_write"))?;
        drop(file);
        if directory.join("terminal-report.json").exists() {
            return Err(CommandError::new("benchmark_report_exists"));
        }
        std::fs::rename(
            directory.join("terminal-report.pending"),
            directory.join("terminal-report.json"),
        )
        .map_err(|_| CommandError::new("benchmark_report_write"))
    })
    .await
    .map_err(|_| CommandError::new("task_failed"))??;
    state
        .fixtures
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_contract_rejects_secret_fields_and_unbounded_samples() {
        let report = include_str!("../../docs/agent/fixtures/m0-benchmark-contract.json");
        let mut parsed: Report = serde_json::from_str(report).unwrap();
        assert!(parsed.validate());
        parsed.runs[0].input_response_ms = vec![1.0; 1001];
        assert!(!parsed.validate());
        parsed.runs[0].input_response_ms.clear();
        parsed.runs[0].terminals[0].input_response_ms = vec![1.0; 1001];
        assert!(!parsed.validate());
        parsed.runs[0].terminals[0].input_response_ms = vec![f64::NAN];
        assert!(!parsed.validate());
        parsed.runs[0].terminals[0].input_response_ms.clear();
        parsed.viewport.scale = f64::NAN;
        assert!(!parsed.validate());
        let mut payload: serde_json::Value = serde_json::from_str(report).unwrap();
        payload["terminal_content"] = serde_json::json!("must not be accepted");
        assert!(serde_json::from_value::<Report>(payload).is_err());
    }
}
