//! Native theme picker and Rust-owned preview tokens.
use crate::{
    native_dialog::{pick, Picker},
    CommandError,
};
use capstan_core::themes::{TerminalThemeDefinition, ThemeCatalog, ThemeError, ThemePreview};
use std::sync::Arc;
use tauri::{State, WebviewWindow};
use uuid::Uuid;

fn source(window: &WebviewWindow) -> Result<(), CommandError> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err(CommandError::new("window_source"))
    }
}

async fn run<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, ThemeError> + Send + 'static,
) -> Result<T, CommandError> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|_| CommandError::new("theme_unavailable"))?
        .map_err(|error| CommandError::new(error.to_string()))
}

#[tauri::command]
pub async fn list_terminal_themes(
    window: WebviewWindow,
    catalog: State<'_, Arc<ThemeCatalog>>,
) -> Result<Vec<TerminalThemeDefinition>, CommandError> {
    source(&window)?;
    let catalog = catalog.inner().clone();
    run(move || catalog.list()).await
}

#[derive(Default)]
pub struct ThemePicker(tokio::sync::Mutex<()>);

#[tauri::command]
pub async fn preview_terminal_themes(
    window: WebviewWindow,
    catalog: State<'_, Arc<ThemeCatalog>>,
    picker: State<'_, ThemePicker>,
    locale: String,
) -> Result<Option<ThemePreview>, CommandError> {
    source(&window)?;
    let _picker = picker
        .0
        .try_lock()
        .map_err(|_| CommandError::new("theme_unavailable"))?;
    let catalog = catalog.inner().clone();
    let title = if locale.starts_with("zh") {
        "选择终端配色文件"
    } else {
        "Select a terminal color scheme"
    };
    let path = pick(&window, title, &locale, Picker::Theme)
        .await
        .map_err(|_| CommandError::new("theme_unavailable"))?;
    run(move || path.map(|path| catalog.preview(&path)).transpose()).await
}

#[tauri::command]
pub async fn save_terminal_theme_preview(
    window: WebviewWindow,
    catalog: State<'_, Arc<ThemeCatalog>>,
    token: Uuid,
) -> Result<Vec<TerminalThemeDefinition>, CommandError> {
    source(&window)?;
    let catalog = catalog.inner().clone();
    run(move || catalog.save_preview(token)).await
}

#[tauri::command]
pub async fn discard_terminal_theme_preview(
    window: WebviewWindow,
    catalog: State<'_, Arc<ThemeCatalog>>,
    token: Uuid,
) -> Result<(), CommandError> {
    source(&window)?;
    let catalog = catalog.inner().clone();
    run(move || catalog.discard_preview(token)).await
}

#[tauri::command]
pub async fn rename_terminal_theme(
    window: WebviewWindow,
    catalog: State<'_, Arc<ThemeCatalog>>,
    id: String,
    name: String,
) -> Result<Vec<TerminalThemeDefinition>, CommandError> {
    source(&window)?;
    let catalog = catalog.inner().clone();
    run(move || catalog.rename(&id, &name)).await
}

#[tauri::command]
pub async fn delete_terminal_theme(
    window: WebviewWindow,
    catalog: State<'_, Arc<ThemeCatalog>>,
    id: String,
) -> Result<Vec<TerminalThemeDefinition>, CommandError> {
    source(&window)?;
    let catalog = catalog.inner().clone();
    run(move || catalog.delete(&id)).await
}
