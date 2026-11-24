//! Shows the local diagnostic log folder. Its path stays in Rust.
use crate::CommandError;
use std::path::PathBuf;
use tauri::State;

pub struct DiagnosticsDirectory(pub PathBuf);

/// Synchronous, so the platform shell call runs on the main thread.
#[tauri::command]
pub fn open_diagnostics_folder(
    window: tauri::WebviewWindow,
    directory: State<'_, DiagnosticsDirectory>,
) -> Result<(), CommandError> {
    if window.label() != "main" {
        return Err(CommandError::new("window_source"));
    }
    std::fs::create_dir_all(&directory.0)
        .map_err(|_| CommandError::new("diagnostics_unavailable"))?;
    open(&directory.0)
        .then_some(())
        .ok_or_else(|| CommandError::new("diagnostics_unavailable"))
}

#[cfg(windows)]
fn open(path: &std::path::Path) -> bool {
    capstan_native_windows::open_folder(path).is_ok()
}

#[cfg(target_os = "macos")]
fn open(path: &std::path::Path) -> bool {
    crate::native_dialog::open_folder(path)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn open(_: &std::path::Path) -> bool {
    false
}
