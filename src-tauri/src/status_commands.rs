use crate::{tauri_channel::SshSessions, CommandError};
use capstan_core::{server_status::SERVER_STATUS_EVENT, sftp::ExactU64, VaultManager};
use capstan_terminal::TerminalSessionId;
use std::sync::Arc;
use tauri::{Emitter, Manager, State, WebviewWindow, Window, Wry};
use uuid::Uuid;

fn source(label: &str) -> Result<(), CommandError> {
    if label == "main" {
        Ok(())
    } else {
        Err(CommandError::new("window_source"))
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusBinding {
    connection_id: Uuid,
    generation: ExactU64,
}

#[tauri::command]
pub fn server_status_binding(
    window: WebviewWindow<Wry>,
    vault: State<'_, Arc<VaultManager>>,
    sessions: State<'_, SshSessions>,
    terminal: TerminalSessionId,
) -> Result<Option<StatusBinding>, CommandError> {
    source(window.label())?;
    Ok(sessions
        .owner(&terminal)
        .and_then(|owner| vault.connection_for_owner(owner).ok())
        .map(|connection| StatusBinding {
            connection_id: connection.connection_id,
            generation: ExactU64(connection.generation),
        }))
}

#[tauri::command]
pub fn select_server_status_connection(
    window: WebviewWindow<Wry>,
    vault: State<'_, Arc<VaultManager>>,
    connection_id: Option<Uuid>,
) -> Result<(), CommandError> {
    source(window.label())?;
    vault.select_server_status_connection(connection_id);
    Ok(())
}

#[tauri::command]
pub fn retry_server_status(
    window: WebviewWindow<Wry>,
    vault: State<'_, Arc<VaultManager>>,
    connection_id: Uuid,
    generation: ExactU64,
) -> Result<(), CommandError> {
    source(window.label())?;
    if vault.retry_server_status(connection_id, generation) {
        Ok(())
    } else {
        Err(CommandError::new("status_closed"))
    }
}

pub fn sync_visibility(window: &Window<Wry>) {
    if window.label() != "main" {
        return;
    }
    if let Some(vault) = window.try_state::<Arc<VaultManager>>() {
        vault.set_server_status_window_visible(
            window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(true),
        );
    }
}

pub fn install(window: WebviewWindow<Wry>, vault: Arc<VaultManager>) {
    let mut updates = vault.subscribe_server_status();
    let mut connections = vault.subscribe_connections();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                update = updates.changed() => {
                    if update.is_err() { return; }
                    let _ = window.emit(SERVER_STATUS_EVENT, vault.server_status_snapshot());
                }
                update = connections.recv() => {
                    if matches!(update, Err(tokio::sync::broadcast::error::RecvError::Closed)) { return; }
                    vault.refresh_server_status_target();
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn status_commands_require_main_window() {
        assert!(super::source("main").is_ok());
        assert_eq!(super::source("other").unwrap_err().code, "window_source");
    }
}
