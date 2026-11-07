//! Rust-only updater commands. Remote bytes are bounded in core before the SDK sees them.
use crate::CommandError;
use capstan_core::{
    settings::Settings,
    update::{self, Release, UpdateError},
};
use serde::Serialize;
use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{Manager, State};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

mod sdk;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseView {
    id: Uuid,
    version: String,
    notes: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    available: bool,
    portable: bool,
    check_on_startup: bool,
    phase: &'static str,
    release: Option<ReleaseView>,
    downloaded: u64,
    total: u64,
    error_code: Option<String>,
}
struct Inner {
    status: UpdateStatus,
    release: Option<Release>,
    cancel: CancellationToken,
}
pub struct UpdateState {
    key: String,
    inner: Mutex<Inner>,
    operation: Arc<tokio::sync::Mutex<()>>,
}

fn command_error(error: UpdateError) -> CommandError {
    CommandError::new(error.to_string())
}
fn source(label: &str) -> Result<(), CommandError> {
    if label == "main" {
        Ok(())
    } else {
        Err(CommandError::new("window_source"))
    }
}
impl UpdateState {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>, CommandError> {
        self.inner
            .lock()
            .map_err(|_| command_error(UpdateError::Unavailable))
    }
    fn begin(&self, phase: &'static str) -> Result<CancellationToken, CommandError> {
        let mut inner = self.lock()?;
        if !inner.status.available {
            return Err(command_error(UpdateError::Unavailable));
        }
        inner.cancel = CancellationToken::new();
        inner.status.phase = phase;
        inner.status.error_code = None;
        inner.status.downloaded = 0;
        Ok(inner.cancel.clone())
    }
    fn fail(&self, error: &CommandError) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.status.phase = "error";
            inner.status.error_code = Some(error.code.clone());
        }
    }
}

pub fn install(app: &tauri::AppHandle, directory: &std::path::Path, portable: bool) {
    let key = app
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|config| config.get("pubkey"))
        .and_then(|key| key.as_str())
        .unwrap_or_default()
        .to_owned();
    let settings = app.state::<Arc<Settings>>().inner().clone();
    let check_on_startup = settings.check_updates_on_startup().unwrap_or(false);
    let available =
        cfg!(all(windows, target_arch = "x86_64")) && update::public_key_available(&key);
    let state = Arc::new(UpdateState {
        key,
        inner: Mutex::new(Inner {
            status: UpdateStatus {
                available,
                portable,
                check_on_startup,
                phase: "idle",
                release: None,
                downloaded: 0,
                total: 0,
                error_code: None,
            },
            release: None,
            cancel: CancellationToken::new(),
        }),
        operation: Arc::default(),
    });
    app.manage(state.clone());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| time.as_secs());
    if available && update::claim_startup_check(directory, check_on_startup, now).unwrap_or(false) {
        if let Ok(mut inner) = state.inner.lock() {
            inner.status.phase = "checking";
        }
        let version = app.package_info().version.to_string();
        tauri::async_runtime::spawn(async move {
            let _ = run_check(&state, &version).await;
        });
    }
}

pub fn closing(app: &tauri::AppHandle) -> bool {
    if let Some(state) = app.try_state::<Arc<UpdateState>>() {
        if let Ok(inner) = state.inner.lock() {
            if inner.status.phase == "installing" {
                return true;
            }
            inner.cancel.cancel();
        }
    }
    false
}

async fn run_check(state: &UpdateState, current: &str) -> Result<UpdateStatus, CommandError> {
    let _operation = state
        .operation
        .clone()
        .try_lock_owned()
        .map_err(|_| command_error(UpdateError::Busy))?;
    let cancel = state.begin("checking")?;
    let result = update::check(current, &cancel).await.map_err(command_error);
    match result {
        Ok(release) => {
            let mut inner = state.lock()?;
            inner.status.release = release.as_ref().map(|release| ReleaseView {
                id: Uuid::new_v4(),
                version: release.version.clone(),
                notes: release.notes.clone(),
            });
            inner.status.total = release.as_ref().map_or(0, |release| release.artifact.size);
            inner.status.phase = if release.is_some() {
                "available"
            } else {
                "idle"
            };
            inner.release = release;
            Ok(inner.status.clone())
        }
        Err(error) => {
            state.fail(&error);
            Err(error)
        }
    }
}

#[tauri::command]
pub fn update_status(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<UpdateState>>,
) -> Result<UpdateStatus, CommandError> {
    source(window.label())?;
    Ok(state.lock()?.status.clone())
}
#[tauri::command]
pub async fn check_update(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<UpdateState>>,
) -> Result<UpdateStatus, CommandError> {
    source(window.label())?;
    run_check(
        state.inner(),
        &window.app_handle().package_info().version.to_string(),
    )
    .await
}
#[tauri::command]
pub fn cancel_update(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<UpdateState>>,
) -> Result<(), CommandError> {
    source(window.label())?;
    let inner = state.lock()?;
    if inner.status.phase == "installing" {
        return Err(command_error(UpdateError::Busy));
    }
    inner.cancel.cancel();
    Ok(())
}
#[tauri::command]
pub async fn save_update_preference(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<UpdateState>>,
    settings: State<'_, Arc<Settings>>,
    enabled: bool,
) -> Result<(), CommandError> {
    source(window.label())?;
    settings
        .inner()
        .clone()
        .save_update_preference(enabled)
        .await
        .map_err(|error| CommandError::new(error.to_string()))?;
    state.lock()?.status.check_on_startup = enabled;
    Ok(())
}
#[tauri::command]
pub fn update_activity(window: tauri::WebviewWindow) -> Result<bool, CommandError> {
    source(window.label())?;
    Ok(window
        .state::<Arc<capstan_terminal::TerminalService>>()
        .registered_sessions()
        > 0
        || window
            .state::<Arc<capstan_core::transfer::TransferService>>()
            .has_active("main")
            .unwrap_or(true)
        || window
            .state::<Arc<capstan_core::sftp::SftpService>>()
            .has_active_tree_deletes("main")
            .unwrap_or(true))
}
#[tauri::command]
pub async fn install_update(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<UpdateState>>,
    id: Uuid,
    confirmed: bool,
    confirm_activity: bool,
) -> Result<(), CommandError> {
    source(window.label())?;
    let _operation = state
        .operation
        .clone()
        .try_lock_owned()
        .map_err(|_| command_error(UpdateError::Busy))?;
    if !confirmed || (update_activity(window.clone())? && !confirm_activity) {
        return Err(command_error(UpdateError::Confirmation));
    }
    let release = {
        let inner = state.lock()?;
        if inner.status.portable || !inner.status.available {
            return Err(command_error(UpdateError::Unavailable));
        }
        if inner
            .status
            .release
            .as_ref()
            .is_none_or(|release| release.id != id)
        {
            return Err(command_error(UpdateError::Manifest));
        }
        inner
            .release
            .clone()
            .ok_or_else(|| command_error(UpdateError::Manifest))?
    };
    let cancel = state.begin("downloading")?;
    let result = run_install(&window, state.inner(), release, &cancel, confirm_activity).await;
    if let Err(error) = &result {
        state.fail(error);
    }
    result
}

async fn run_install(
    window: &tauri::WebviewWindow,
    state: &UpdateState,
    release: Release,
    cancel: &CancellationToken,
    confirm_activity: bool,
) -> Result<(), CommandError> {
    let bytes = update::download(&release, &state.key, cancel, |received, total| {
        if let Ok(mut inner) = state.inner.lock() {
            inner.status.downloaded = received;
            inner.status.total = total;
        }
    })
    .await
    .map_err(command_error)?;
    let sdk_update = sdk_update(window.app_handle(), &release, &state.key)?;
    if update_activity(window.clone())? && !confirm_activity {
        return Err(command_error(UpdateError::Confirmation));
    }
    {
        let mut inner = state.lock()?;
        if cancel.is_cancelled() {
            return Err(command_error(UpdateError::Canceled));
        }
        inner.status.phase = "installing";
    }
    // The SDK cannot fail the before-exit callback. Prepare fallibly first, then let
    // its NSIS launcher exit only after all application workflows have drained.
    crate::tauri_channel::prepare_update_exit(window).await?;
    let result = tauri::async_runtime::spawn_blocking(move || sdk_update.install(bytes)).await;
    update::abort_prepared_exit(
        &window.state::<Arc<Settings>>(),
        &window.state::<Arc<capstan_core::transfer::TransferService>>(),
        &window.state::<Arc<capstan_core::sftp::SftpService>>(),
    );
    #[cfg(windows)]
    window
        .state::<Arc<capstan_core::remote_edit::RemoteEditorService>>()
        .abort_exit();
    result
        .map_err(|_| command_error(UpdateError::Install))?
        .map_err(|_| command_error(UpdateError::Install))
}

/// Restore the policy-checked metadata without a second, unbounded network read.
fn sdk_update<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    release: &Release,
    key: &str,
) -> Result<tauri_plugin_updater::Update, CommandError> {
    let manifest = serde_json::from_slice(&release.sdk_manifest().map_err(command_error)?)
        .map_err(|_| command_error(UpdateError::Manifest))?;
    sdk::builder(app, key, update::TARGET)
        .build()
        .map_err(|_| command_error(UpdateError::Unavailable))?
        .restore_update(manifest)
        .map_err(|_| command_error(UpdateError::Manifest))?
        .ok_or_else(|| command_error(UpdateError::Version))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_reject_other_window_sources() {
        assert!(source("main").is_ok());
        for label in ["", "Main", "child", "main-child"] {
            assert_eq!(source(label).unwrap_err().code, "window_source");
        }
    }
}
