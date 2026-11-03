//! Source validation and DTO forwarding; tunnel ownership lives in core/ssh.
use crate::CommandError;
use capstan_core::{sftp::ExactU64, VaultManager};
use capstan_ssh::forwarding::{TunnelRuntimeSnapshot, TUNNEL_STATE_CHANGED_EVENT};
use capstan_vault::TunnelConfig;
use std::sync::Arc;
use tauri::{Emitter, Manager, State};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use uuid::Uuid;

#[derive(Default)]
struct EventTask {
    stop: CancellationToken,
    tasks: TaskTracker,
}
#[derive(serde::Serialize)]
pub struct Revision {
    revision: ExactU64,
}
fn source(label: &str) -> Result<(), CommandError> {
    if label == "main" {
        Ok(())
    } else {
        Err(CommandError::new("window_source"))
    }
}
#[tauri::command]
pub fn list_tunnels(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
) -> Result<Vec<TunnelConfig>, CommandError> {
    source(window.label())?;
    vault.tunnels().map_err(Into::into)
}
#[tauri::command]
pub fn tunnel_runtime_snapshots(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
) -> Result<Vec<TunnelRuntimeSnapshot>, CommandError> {
    source(window.label())?;
    vault.tunnel_runtime_snapshots().map_err(Into::into)
}
#[tauri::command]
pub async fn save_tunnel(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
    config: TunnelConfig,
    expected_revision: ExactU64,
) -> Result<Revision, CommandError> {
    source(window.label())?;
    vault
        .save_tunnel(config, expected_revision.0)
        .await
        .map(|revision| Revision {
            revision: ExactU64(revision),
        })
        .map_err(Into::into)
}
#[tauri::command]
pub async fn delete_tunnel(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
    id: Uuid,
    expected_revision: ExactU64,
) -> Result<Revision, CommandError> {
    source(window.label())?;
    vault
        .delete_tunnel(id, expected_revision.0)
        .await
        .map(|revision| Revision {
            revision: ExactU64(revision),
        })
        .map_err(Into::into)
}
#[tauri::command]
pub async fn start_tunnel(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
    id: Uuid,
    connection_id: Uuid,
    expected_generation: ExactU64,
) -> Result<TunnelRuntimeSnapshot, CommandError> {
    source(window.label())?;
    vault
        .start_tunnel(id, connection_id, expected_generation.0)
        .await
        .map_err(Into::into)
}
#[tauri::command]
pub async fn stop_tunnel(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
    id: Uuid,
) -> Result<TunnelRuntimeSnapshot, CommandError> {
    source(window.label())?;
    vault.stop_tunnel(id).await.map_err(Into::into)
}
pub fn install(app: &mut tauri::App) {
    let vault = app.state::<Arc<VaultManager>>().inner().clone();
    let mut events = vault.subscribe_tunnels();
    let task = Arc::new(EventTask::default());
    app.manage(task.clone());
    if let Some(window) = app.get_webview_window("main") {
        let stop = task.stop.clone();
        task.tasks.spawn(async move {
            loop {
                let event = tokio::select! {
                    biased;
                    () = stop.cancelled() => break,
                    event = events.recv() => event,
                };
                match event {
                    Ok(snapshot) => {
                        if window.emit(TUNNEL_STATE_CHANGED_EVENT, snapshot).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if let Ok(snapshots) = vault.tunnel_runtime_snapshots() {
                            for snapshot in snapshots {
                                let _ = window.emit(TUNNEL_STATE_CHANGED_EVENT, snapshot);
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }
}
pub async fn shutdown(window: &tauri::Window) {
    if let Some(task) = window.try_state::<Arc<EventTask>>() {
        task.stop.cancel();
        task.tasks.close();
        task.tasks.wait().await;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_only_accept_the_main_window_and_exact_revisions() {
        assert!(source("main").is_ok());
        assert_eq!(source("other").unwrap_err().code, "window_source");
        assert!(serde_json::from_value::<ExactU64>(serde_json::json!(42)).is_err());
        assert_eq!(
            serde_json::to_value(Revision {
                revision: ExactU64(u64::MAX)
            })
            .unwrap(),
            serde_json::json!({ "revision": "18446744073709551615" })
        );
    }
}
