//! Only the selected app root's bounded, nonsecret panel preference DTO is exposed.
use crate::CommandError;
use capstan_core::settings::{PanelLayout, PanelPreferences, Settings};
use std::sync::Arc;
use tauri::State;

fn source(window: &tauri::WebviewWindow) -> Result<(), CommandError> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err(CommandError::new("window_source"))
    }
}

#[tauri::command]
pub async fn load_sftp_panel_preferences(
    window: tauri::WebviewWindow,
    settings: State<'_, Arc<Settings>>,
) -> Result<PanelPreferences, CommandError> {
    source(&window)?;
    let settings = settings.inner().clone();
    tauri::async_runtime::spawn_blocking(move || settings.snapshot())
        .await
        .map_err(|_| CommandError::new("settings_unavailable"))?
        .map_err(|error| CommandError::new(error.to_string()))
}

#[tauri::command]
pub async fn save_sftp_panel_preferences(
    window: tauri::WebviewWindow,
    settings: State<'_, Arc<Settings>>,
    layout: PanelLayout,
    session: uuid::Uuid,
    sequence: capstan_core::sftp::ExactU64,
) -> Result<(), CommandError> {
    source(&window)?;
    let settings = settings.inner().clone();
    settings
        .save(session, sequence, layout)
        .await
        .map_err(|error| CommandError::new(error.to_string()))
}
