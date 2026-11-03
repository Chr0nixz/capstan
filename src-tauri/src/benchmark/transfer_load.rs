//! Benchmark-only adapters for Rust-owned, disposable transfer fixtures.
use super::*;
use capstan_core::{
    benchmark::transfer_load::{Config, LoadResult, TransferLoad},
    transfer::Direction,
};

#[derive(Default)]
pub(super) struct LoadState {
    config: tokio::sync::OnceCell<Config>,
    pub(super) active: tokio::sync::Mutex<Option<Arc<TransferLoad>>>,
    pub(super) completed: Mutex<Vec<LoadResult>>,
}
async fn config(state: &BenchmarkState) -> Result<Option<Config>, CommandError> {
    let Some(path) = state.m2_config.clone() else {
        return Ok(None);
    };
    state
        .m2
        .config
        .get_or_try_init(|| async {
            tokio::task::spawn_blocking(move || Config::load(&path))
                .await
                .map_err(|_| CommandError::new("benchmark_load_config"))?
                .map_err(|error| CommandError::new(error.to_string()))
        })
        .await
        .cloned()
        .map(Some)
}
#[tauri::command]
pub async fn m2_benchmark_round(
    window: WebviewWindow,
    state: State<'_, BenchmarkState>,
) -> Result<Option<u8>, CommandError> {
    require_source(&window, &state)?;
    trace_stage(&state, "round.begin");
    Ok(config(&state).await?.map(|config| config.round))
}
#[tauri::command]
pub async fn m2_start_load(
    window: WebviewWindow,
    state: State<'_, BenchmarkState>,
    terminal: TerminalSessionId,
    direction: Direction,
) -> Result<uuid::Uuid, CommandError> {
    require_source(&window, &state)?;
    trace_stage(&state, "load.start.begin");
    let config = config(&state)
        .await?
        .ok_or_else(|| CommandError::new("benchmark_disabled"))?;
    if !window
        .is_visible()
        .map_err(|_| CommandError::new("window_operation"))?
    {
        return Err(CommandError::new("benchmark_requires_visible"));
    }
    let record = state
        .fixtures
        .lock()
        .map_err(|_| CommandError::new("benchmark_load"))?
        .get(&terminal)
        .cloned()
        .ok_or_else(|| CommandError::new("benchmark_load"))?;
    if !matches!(record.backend, FixtureBackend::Ssh)
        || record.session.snapshot().state != capstan_terminal::TerminalState::Running
    {
        return Err(CommandError::new("benchmark_load"));
    }
    let mut active = state.m2.active.lock().await;
    let completed = state
        .m2
        .completed
        .lock()
        .map_err(|_| CommandError::new("benchmark_load"))?
        .len();
    if active.is_some() || completed >= 2 {
        return Err(CommandError::new("benchmark_load_limit"));
    }
    let load = TransferLoad::start(
        config,
        record
            .host_id
            .ok_or_else(|| CommandError::new("benchmark_load"))?,
        record
            .connection_id
            .ok_or_else(|| CommandError::new("benchmark_load"))?,
        record
            .lease
            .ok_or_else(|| CommandError::new("benchmark_load"))?,
        direction,
    )
    .await
    .map_err(|error| CommandError::new(error.to_string()))?;
    trace_stage(&state, "load.start.ready");
    // Each direction starts a new visible input phase after potentially long
    // file verification. Restore this measurement window's focus first;
    // actual focus observations still have to satisfy the unchanged gate.
    let focus = window
        .is_focused()
        .and_then(|focused| if focused { Ok(()) } else { window.set_focus() });
    if focus.is_err() {
        load.cancel()
            .await
            .map_err(|error| CommandError::new(error.to_string()))?;
        return Err(CommandError::new("window_operation"));
    }
    let id = load.id();
    let load = Arc::new(load);
    if state.diagnostic {
        let weak = Arc::downgrade(&load);
        let fixtures: Vec<_> = state
            .fixtures
            .lock()
            .map_err(|_| CommandError::new("benchmark_load"))?
            .values()
            .cloned()
            .collect();
        tokio::spawn(async move {
            // Diagnostic smoke only; no background sampler during formal runs.
            for tick in 0..150 {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let Some(load) = weak.upgrade() else { break };
                eprintln!("M2_LOAD {tick} {:?}", load.diagnostic_progress());
                drop(load);
                for fixture in &fixtures {
                    let snapshot = fixture.session.snapshot();
                    let metrics = fixture.session.output_metrics().await;
                    eprintln!(
                        "M2_TERMINAL {} {:?} {} {} {} {} {}",
                        snapshot.id.0,
                        snapshot.state,
                        metrics.input_bytes,
                        metrics.output_bytes,
                        metrics.in_flight_bytes,
                        metrics.pause_count,
                        metrics.resume_count
                    );
                }
            }
        });
    }
    *active = Some(load);
    Ok(id)
}
async fn current(
    state: &BenchmarkState,
    id: uuid::Uuid,
) -> Result<Arc<TransferLoad>, CommandError> {
    state
        .m2
        .active
        .lock()
        .await
        .as_ref()
        .filter(|load| load.id() == id)
        .cloned()
        .ok_or_else(|| CommandError::new("benchmark_load"))
}
#[tauri::command]
pub async fn m2_load_active(
    window: WebviewWindow,
    state: State<'_, BenchmarkState>,
    id: uuid::Uuid,
) -> Result<bool, CommandError> {
    require_source(&window, &state)?;
    trace_stage(&state, "load.active.begin");
    let active = current(&state, id)
        .await?
        .all_running()
        .map_err(|error| CommandError::new(error.to_string()))?;
    trace_stage(
        &state,
        if active {
            "load.active.true"
        } else {
            "load.active.false"
        },
    );
    Ok(active)
}
#[tauri::command]
pub async fn m2_finish_load(
    window: WebviewWindow,
    state: State<'_, BenchmarkState>,
    id: uuid::Uuid,
) -> Result<(), CommandError> {
    require_source(&window, &state)?;
    trace_stage(&state, "load.finish.begin");
    let load = current(&state, id).await?;
    let result = load.finish().await;
    trace_stage(&state, "load.finish.returned");
    if result.is_err() {
        load.cancel()
            .await
            .map_err(|error| CommandError::new(error.to_string()))?;
    }
    let mut active = state.m2.active.lock().await;
    if active.as_ref().is_none_or(|load| load.id() != id) {
        return Err(CommandError::new("benchmark_load"));
    }
    active.take();
    state
        .m2
        .completed
        .lock()
        .map_err(|_| CommandError::new("benchmark_load"))?
        .push(result.map_err(|error| CommandError::new(error.to_string()))?);
    Ok(())
}
#[tauri::command]
pub async fn m2_cancel_load(
    window: WebviewWindow,
    state: State<'_, BenchmarkState>,
    id: uuid::Uuid,
) -> Result<(), CommandError> {
    require_source(&window, &state)?;
    trace_stage(&state, "load.cancel.begin");
    let load = current(&state, id).await?;
    load.cancel()
        .await
        .map_err(|error| CommandError::new(error.to_string()))?;
    trace_stage(&state, "load.cancel.ready");
    let mut active = state.m2.active.lock().await;
    if active.as_ref().is_none_or(|load| load.id() != id) {
        return Err(CommandError::new("benchmark_load"));
    }
    active.take();
    Ok(())
}
