//! Only the selected app root's bounded, nonsecret panel preference DTO is exposed.
use crate::CommandError;
use capstan_core::settings::{
    DesktopPreference, DesktopPreferences, PanelLayout, PanelPreferences, Settings,
};
use std::sync::Arc;
use tauri::{Manager, State};

fn source(window: &tauri::WebviewWindow) -> Result<(), CommandError> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err(CommandError::new("window_source"))
    }
}

#[tauri::command]
pub async fn load_desktop_preferences(
    window: tauri::WebviewWindow,
    settings: State<'_, Arc<Settings>>,
    desktop: State<'_, Arc<crate::desktop_commands::DesktopState>>,
    locale: String,
) -> Result<DesktopPreferences, CommandError> {
    source(&window)?;
    desktop.set_locale(&locale);
    let settings = settings.inner().clone();
    tauri::async_runtime::spawn_blocking(move || settings.desktop_preferences())
        .await
        .map_err(|_| CommandError::new("settings_unavailable"))?
        .map_err(|error| CommandError::new(error.to_string()))
}

#[tauri::command]
pub async fn save_desktop_preference(
    window: tauri::WebviewWindow,
    settings: State<'_, Arc<Settings>>,
    preference: DesktopPreference,
    value: bool,
) -> Result<DesktopPreferences, CommandError> {
    source(&window)?;
    if matches!(preference, DesktopPreference::JumpListRecentHosts) {
        let app = window.app_handle().clone();
        return crate::launch_commands::save_recent_hosts_preference(window, app, value).await;
    }
    settings
        .inner()
        .clone()
        .save_desktop_preference(preference, value)
        .await
        .map_err(|error| CommandError::new(error.to_string()))
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
