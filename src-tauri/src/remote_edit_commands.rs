//! Source validation, native pickers and forwarding; editor lifecycle is core-owned.
use crate::CommandError;
use capstan_core::{
    remote_edit::{self as editor, RemoteEditorService},
    sftp::{RemoteEntryId, SftpService, SftpSessionId},
};
use capstan_terminal::TerminalSessionId;
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

fn source(window: &tauri::WebviewWindow) -> Result<&str, CommandError> {
    if window.label() == "main" {
        Ok(window.label())
    } else {
        Err(CommandError::new("window_source"))
    }
}
fn error(error: editor::EditError) -> CommandError {
    CommandError::new(error.to_string())
}

#[tauri::command]
pub fn remote_editor_configuration(
    window: tauri::WebviewWindow,
    editors: State<'_, Arc<RemoteEditorService>>,
) -> Result<Option<Uuid>, CommandError> {
    editors.configuration_id(source(&window)?).map_err(error)
}
#[tauri::command]
pub async fn select_remote_editor(
    window: tauri::WebviewWindow,
    editors: State<'_, Arc<RemoteEditorService>>,
    locale: String,
) -> Result<Option<Uuid>, CommandError> {
    let label = source(&window)?.to_owned();
    let parent = window
        .hwnd()
        .map_err(|_| CommandError::new("window_source"))?
        .0 as isize;
    let selected = tokio::task::spawn_blocking(move || {
        let (title, applications) = if locale == "zh-CN" {
            ("选择本地编辑器", "应用程序")
        } else {
            ("Select local editor", "Applications")
        };
        capstan_native_windows::transfer::pick_file(title, applications, parent, false)
    })
    .await
    .map_err(|_| CommandError::new("editor_unavailable"))?
    .map_err(|_| CommandError::new("editor_unavailable"))?;
    match selected {
        Some(path) => editors
            .configure(&label, path)
            .await
            .map(Some)
            .map_err(error),
        None => Ok(None),
    }
}
#[tauri::command]
pub async fn begin_remote_edit(
    window: tauri::WebviewWindow,
    service: State<'_, Arc<SftpService>>,
    editors: State<'_, Arc<RemoteEditorService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    entry: RemoteEntryId,
    editor_id: Uuid,
) -> Result<editor::RemoteEditView, CommandError> {
    let label = source(&window)?;
    let context = service
        .remote_file_context(label, &terminal.0, session, entry)
        .map_err(|error| CommandError::new(error.to_string()))?;
    editors
        .begin(label, context, editor_id)
        .await
        .map_err(error)
}
#[tauri::command]
pub fn remote_edit_snapshot(
    window: tauri::WebviewWindow,
    editors: State<'_, Arc<RemoteEditorService>>,
) -> Result<editor::RemoteEditSnapshot, CommandError> {
    editors.snapshot(source(&window)?).map_err(error)
}
#[tauri::command]
pub async fn discover_remote_edit_residuals(
    window: tauri::WebviewWindow,
    editors: State<'_, Arc<RemoteEditorService>>,
) -> Result<(), CommandError> {
    let label = source(&window)?.to_owned();
    let editors = editors.inner().clone();
    tokio::task::spawn_blocking(move || editors.discover(&label))
        .await
        .map_err(|_| CommandError::new("editor_residual"))?
        .map_err(error)
}
#[tauri::command]
pub async fn retry_remote_edit(
    window: tauri::WebviewWindow,
    editors: State<'_, Arc<RemoteEditorService>>,
    service: State<'_, Arc<SftpService>>,
    id: Uuid,
    terminal: Option<TerminalSessionId>,
    session: Option<SftpSessionId>,
) -> Result<editor::RemoteEditView, CommandError> {
    match (terminal, session) {
        (Some(terminal), Some(session)) => {
            let view = editors
                .bind_for_retry(source(&window)?, id, &service, &terminal.0, session)
                .await
                .map_err(error)?;
            if view.state == editor::EditState::Conflict {
                return Ok(view);
            }
        }
        (None, None) => {}
        _ => return Err(CommandError::new("scope_expired")),
    }
    editors.retry(source(&window)?, id).await.map_err(error)
}
#[tauri::command]
pub async fn cleanup_remote_edit_temporaries(
    window: tauri::WebviewWindow,
    editors: State<'_, Arc<RemoteEditorService>>,
    service: State<'_, Arc<SftpService>>,
    id: Uuid,
    terminal: Option<TerminalSessionId>,
    session: Option<SftpSessionId>,
) -> Result<editor::RemoteEditView, CommandError> {
    match (terminal, session) {
        (Some(terminal), Some(session)) => editors
            .bind_for_cleanup(source(&window)?, id, &service, &terminal.0, session)
            .await
            .map_err(error)?,
        (None, None) => {}
        _ => return Err(CommandError::new("scope_expired")),
    }
    editors
        .cleanup_temporaries(source(&window)?, id)
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn resolve_remote_edit_conflict(
    window: tauri::WebviewWindow,
    editors: State<'_, Arc<RemoteEditorService>>,
    id: Uuid,
    decision: editor::ConflictDecision,
    new_name: Option<String>,
) -> Result<editor::RemoteEditView, CommandError> {
    editors
        .resolve(source(&window)?, id, decision, new_name)
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn close_remote_edit(
    window: tauri::WebviewWindow,
    editors: State<'_, Arc<RemoteEditorService>>,
    id: Uuid,
    action: editor::ResidualAction,
    locale: String,
) -> Result<bool, CommandError> {
    let label = source(&window)?.to_owned();
    let destination = if action == editor::ResidualAction::SaveLocalCopy {
        let parent = window
            .hwnd()
            .map_err(|_| CommandError::new("window_source"))?
            .0 as isize;
        let folder = tokio::task::spawn_blocking(move || {
            let title = if locale == "zh-CN" {
                "保存编辑副本"
            } else {
                "Save edited copy"
            };
            capstan_native_windows::transfer::pick_folder(title, parent)
        })
        .await
        .map_err(|_| CommandError::new("local_io"))?
        .map_err(|_| CommandError::new("local_io"))?;
        let Some(folder) = folder else {
            return Ok(false);
        };
        Some(folder)
    } else {
        None
    };
    editors
        .close(&label, id, action, destination)
        .await
        .map_err(error)?;
    Ok(true)
}
