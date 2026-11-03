//! Picker/owner validation and forwarding; file bodies stay in native/core I/O.
use crate::CommandError;
use capstan_core::{
    sftp::{ExactU64, RemoteEntryId, RemotePath, SftpService, SftpSessionId},
    transfer::{
        Decision, LocalSelection, TransferService, TransferSnapshot, TransferTarget,
        WindowsDestination,
    },
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
fn error(error: capstan_core::sftp::SftpError) -> CommandError {
    CommandError::new(error.to_string())
}
fn local_error(error: std::io::Error) -> CommandError {
    CommandError::new(match error.kind() {
        std::io::ErrorKind::PermissionDenied => "permission_denied",
        std::io::ErrorKind::InvalidInput => "path_invalid",
        _ => "local_io",
    })
}

#[tauri::command]
pub fn arm_native_drop(
    window: tauri::WebviewWindow,
    sftp: State<'_, Arc<SftpService>>,
    transfers: State<'_, Arc<TransferService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    parent: RemotePath,
) -> Result<Uuid, CommandError> {
    let context = sftp
        .transfer_context(
            source(&window)?,
            &terminal.0,
            session,
            TransferTarget::Upload {
                parent,
                name: "selection".into(),
            },
        )
        .map_err(error)?;
    transfers.arm_native_drop(&context).map_err(error)
}
#[tauri::command]
pub fn disarm_native_drop(
    window: tauri::WebviewWindow,
    transfers: State<'_, Arc<TransferService>>,
    token: Uuid,
) -> Result<(), CommandError> {
    transfers
        .disarm_native_drop(source(&window)?, token)
        .map_err(error)
}

#[tauri::command]
pub async fn enqueue_native_drop(
    window: tauri::WebviewWindow,
    sftp: State<'_, Arc<SftpService>>,
    transfers: State<'_, Arc<TransferService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    parent: RemotePath,
    token: Uuid,
) -> Result<capstan_core::transfer::NativeDropResult, CommandError> {
    transfers
        .enqueue_native_drop(&sftp, source(&window)?, &terminal.0, session, parent, token)
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn prepare_native_export(
    window: tauri::WebviewWindow,
    sftp: State<'_, Arc<SftpService>>,
    transfers: State<'_, Arc<TransferService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    entry: RemoteEntryId,
) -> Result<capstan_core::transfer::PreparedExport, CommandError> {
    transfers
        .prepare_native_export(&sftp, source(&window)?, &terminal.0, session, entry)
        .await
        .map_err(error)
}
#[tauri::command]
pub fn native_export_snapshot(
    window: tauri::WebviewWindow,
    transfers: State<'_, Arc<TransferService>>,
) -> Result<Vec<capstan_core::transfer::ExportSnapshot>, CommandError> {
    transfers.export_snapshot(source(&window)?).map_err(error)
}
#[tauri::command]
pub async fn release_native_export(
    window: tauri::WebviewWindow,
    transfers: State<'_, Arc<TransferService>>,
    token: Uuid,
) -> Result<(), CommandError> {
    transfers
        .release_native_export(source(&window)?, token)
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn start_native_export(
    window: tauri::WebviewWindow,
    sftp: State<'_, Arc<SftpService>>,
    transfers: State<'_, Arc<TransferService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    token: Uuid,
) -> Result<bool, CommandError> {
    let drag = transfers
        .begin_export_drag(&sftp, source(&window)?, &terminal.0, session, token)
        .await
        .map_err(error)?;
    let parent = window
        .hwnd()
        .map_err(|_| CommandError::new("window_source"))?
        .0 as isize;
    let (send, receive) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            let _ = send.send(drag.start(parent).map_err(error));
        })
        .map_err(|_| CommandError::new("local_io"))?;
    receive.await.map_err(|_| CommandError::new("local_io"))?
}

#[tauri::command]
pub async fn select_transfer_source(
    window: tauri::WebviewWindow,
    sftp: State<'_, Arc<SftpService>>,
    transfers: State<'_, Arc<TransferService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    parent: RemotePath,
    locale: String,
) -> Result<Option<LocalSelection>, CommandError> {
    let context = || {
        sftp.transfer_context(
            source(&window)?,
            &terminal.0,
            session,
            TransferTarget::Upload {
                parent: parent.clone(),
                name: "selection".into(),
            },
        )
        .map_err(error)
    };
    context()?;
    let parent_window = window
        .hwnd()
        .map_err(|_| CommandError::new("window_source"))?
        .0 as isize;
    let file = tauri::async_runtime::spawn_blocking(move || {
        let (title, filter) = if locale == "zh-CN" {
            ("选择上传文件", "所有文件")
        } else {
            ("Select file to upload", "All files")
        };
        capstan_native_windows::transfer::pick_file(title, filter, parent_window, false)?
            .map(|path| capstan_native_windows::transfer::Source::open(&path))
            .transpose()
    })
    .await
    .map_err(|_| CommandError::new("local_io"))?
    .map_err(local_error)?;
    let Some(file) = file else {
        return Ok(None);
    };
    // Closing/navigating/rebinding while the native picker was open invalidates
    // the request. The frontend never receives the selected absolute path.
    transfers
        .grant_source(&context()?, Arc::new(file))
        .map(Some)
        .map_err(error)
}

#[tauri::command]
pub async fn select_transfer_folder_plan(
    window: tauri::WebviewWindow,
    sftp: State<'_, Arc<SftpService>>,
    transfers: State<'_, Arc<TransferService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    parent: RemotePath,
    locale: String,
) -> Result<Option<capstan_core::transfer::TreePlanSummary>, CommandError> {
    let label = source(&window)?;
    sftp.transfer_context(
        label,
        &terminal.0,
        session,
        TransferTarget::Upload {
            parent: parent.clone(),
            name: "selection".into(),
        },
    )
    .map_err(error)?;
    let parent_window = window
        .hwnd()
        .map_err(|_| CommandError::new("window_source"))?
        .0 as isize;
    let folder = tauri::async_runtime::spawn_blocking(move || {
        capstan_native_windows::transfer::pick_folder(
            if locale == "zh-CN" {
                "选择上传文件夹"
            } else {
                "Select folder to upload"
            },
            parent_window,
        )
    })
    .await
    .map_err(|_| CommandError::new("local_io"))?
    .map_err(local_error)?;
    let Some(folder) = folder else {
        return Ok(None);
    };
    transfers
        .prepare_upload_tree(&sftp, label, &terminal.0, session, parent, folder)
        .await
        .map(Some)
        .map_err(error)
}

#[tauri::command]
pub async fn select_transfer_folder_download_plan(
    window: tauri::WebviewWindow,
    sftp: State<'_, Arc<SftpService>>,
    transfers: State<'_, Arc<TransferService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    entry: RemoteEntryId,
    locale: String,
) -> Result<Option<capstan_core::transfer::TreePlanSummary>, CommandError> {
    let label = source(&window)?;
    let parent_window = window
        .hwnd()
        .map_err(|_| CommandError::new("window_source"))?
        .0 as isize;
    let folder = tauri::async_runtime::spawn_blocking(move || {
        capstan_native_windows::transfer::pick_folder(
            if locale == "zh-CN" {
                "选择下载位置"
            } else {
                "Select download folder"
            },
            parent_window,
        )
    })
    .await
    .map_err(|_| CommandError::new("local_io"))?
    .map_err(local_error)?;
    let Some(folder) = folder else {
        return Ok(None);
    };
    transfers
        .prepare_download_tree(&sftp, label, &terminal.0, session, entry, folder)
        .await
        .map(Some)
        .map_err(error)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn resolve_transfer_tree_plan(
    window: tauri::WebviewWindow,
    sftp: State<'_, Arc<SftpService>>,
    transfers: State<'_, Arc<TransferService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    token: Uuid,
    revision: ExactU64,
    decisions: Vec<capstan_core::transfer::TreeDecision>,
) -> Result<capstan_core::transfer::TreeResult, CommandError> {
    transfers
        .resolve_tree_plan(
            &sftp,
            source(&window)?,
            &terminal.0,
            session,
            token,
            revision,
            decisions,
        )
        .await
        .map_err(error)
}
#[tauri::command]
pub fn release_transfer_tree_plan(
    window: tauri::WebviewWindow,
    transfers: State<'_, Arc<TransferService>>,
    token: Uuid,
) -> Result<(), CommandError> {
    transfers
        .release_tree_plan(source(&window)?, token)
        .map_err(error)
}
#[tauri::command]
pub async fn select_transfer_destination(
    window: tauri::WebviewWindow,
    sftp: State<'_, Arc<SftpService>>,
    transfers: State<'_, Arc<TransferService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    entry: RemoteEntryId,
    locale: String,
) -> Result<Option<LocalSelection>, CommandError> {
    let context = || {
        sftp.transfer_context(
            source(&window)?,
            &terminal.0,
            session,
            TransferTarget::Download { entry },
        )
        .map_err(error)
    };
    context()?;
    let parent = window
        .hwnd()
        .map_err(|_| CommandError::new("window_source"))?
        .0 as isize;
    let file = tauri::async_runtime::spawn_blocking(move || {
        let (title, filter) = if locale == "zh-CN" {
            ("保存下载文件", "所有文件")
        } else {
            ("Save downloaded file", "All files")
        };
        capstan_native_windows::transfer::pick_file(title, filter, parent, true)?
            .map(|path| capstan_native_windows::transfer::Destination::select(&path))
            .transpose()
    })
    .await
    .map_err(|_| CommandError::new("local_io"))?
    .map_err(local_error)?;
    let Some(file) = file else {
        return Ok(None);
    };
    transfers
        .grant_destination(&context()?, Arc::new(WindowsDestination::new(file)))
        .map(Some)
        .map_err(error)
}
#[tauri::command]
pub fn enqueue_transfer(
    window: tauri::WebviewWindow,
    sftp: State<'_, Arc<SftpService>>,
    transfers: State<'_, Arc<TransferService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    target: TransferTarget,
    selection: Uuid,
) -> Result<Uuid, CommandError> {
    let context = sftp
        .transfer_context(source(&window)?, &terminal.0, session, target)
        .map_err(error)?;
    transfers.inner().enqueue(context, selection).map_err(error)
}
#[tauri::command]
pub fn release_transfer_selection(
    window: tauri::WebviewWindow,
    transfers: State<'_, Arc<TransferService>>,
    token: Uuid,
) -> Result<(), CommandError> {
    transfers
        .release_selection(source(&window)?, token)
        .map_err(error)
}
#[tauri::command]
pub fn transfer_snapshot(
    window: tauri::WebviewWindow,
    transfers: State<'_, Arc<TransferService>>,
) -> Result<Vec<TransferSnapshot>, CommandError> {
    transfers.snapshot(source(&window)?).map_err(error)
}
#[tauri::command]
pub fn transfer_recovery_snapshot(
    window: tauri::WebviewWindow,
    transfers: State<'_, Arc<TransferService>>,
) -> Result<capstan_core::transfer::TransferRecoverySnapshot, CommandError> {
    source(&window)?;
    transfers.recovery_snapshot().map_err(error)
}
#[tauri::command]
pub fn retry_recovered_transfer_upload(
    window: tauri::WebviewWindow,
    sftp: State<'_, Arc<SftpService>>,
    transfers: State<'_, Arc<TransferService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    id: Uuid,
    selection: Uuid,
) -> Result<(), CommandError> {
    let window = source(&window)?;
    let target = transfers.recovery_target(window, id).map_err(error)?;
    let context = sftp
        .transfer_context_for_recovery(window, &terminal.0, session, target)
        .map_err(error)?;
    transfers
        .retry_recovered_upload(window, id, context, selection)
        .map_err(error)
}
#[tauri::command]
pub async fn cleanup_recovered_transfer_upload(
    window: tauri::WebviewWindow,
    sftp: State<'_, Arc<SftpService>>,
    transfers: State<'_, Arc<TransferService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    id: Uuid,
) -> Result<(), CommandError> {
    let window = source(&window)?;
    let target = transfers.recovery_target(window, id).map_err(error)?;
    let context = sftp
        .transfer_context_for_recovery(window, &terminal.0, session, target)
        .map_err(error)?;
    transfers
        .cleanup_recovered_upload(window, id, context)
        .await
        .map_err(error)
}

#[tauri::command]
pub async fn select_recovered_transfer_destination(
    window: tauri::WebviewWindow,
    sftp: State<'_, Arc<SftpService>>,
    transfers: State<'_, Arc<TransferService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    id: Uuid,
    locale: String,
) -> Result<Option<LocalSelection>, CommandError> {
    let label = source(&window)?.to_owned();
    let descriptor = transfers.recovered_download(&label, id).map_err(error)?;
    sftp.transfer_context_for_recovered_download(&label, &terminal.0, session, descriptor)
        .await
        .map_err(error)?;
    let parent = window
        .hwnd()
        .map_err(|_| CommandError::new("window_source"))?
        .0 as isize;
    let destination = tauri::async_runtime::spawn_blocking(move || {
        let (title, filter) = if locale == "zh-CN" {
            ("重新选择原下载目标", "所有文件")
        } else {
            ("Select the original download destination", "All files")
        };
        capstan_native_windows::transfer::pick_file(title, filter, parent, true)?
            .map(|path| capstan_native_windows::transfer::Destination::select(&path))
            .transpose()
    })
    .await
    .map_err(|_| CommandError::new("local_io"))?
    .map_err(local_error)?;
    let Some(destination) = destination else {
        return Ok(None);
    };
    let descriptor = transfers.recovered_download(&label, id).map_err(error)?;
    let context = sftp
        .transfer_context_for_recovered_download(&label, &terminal.0, session, descriptor)
        .await
        .map_err(error)?;
    transfers
        .grant_recovered_destination(
            &label,
            id,
            &context,
            Arc::new(WindowsDestination::new(destination)),
        )
        .await
        .map(Some)
        .map_err(error)
}

#[tauri::command]
pub async fn retry_recovered_transfer_download(
    window: tauri::WebviewWindow,
    sftp: State<'_, Arc<SftpService>>,
    transfers: State<'_, Arc<TransferService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    id: Uuid,
    selection: Uuid,
) -> Result<(), CommandError> {
    let label = source(&window)?;
    let descriptor = transfers.recovered_download(label, id).map_err(error)?;
    let context = sftp
        .transfer_context_for_recovered_download(label, &terminal.0, session, descriptor)
        .await
        .map_err(error)?;
    transfers
        .retry_recovered_download(label, id, context, selection)
        .map_err(error)
}

#[tauri::command]
pub async fn cleanup_recovered_transfer_download(
    window: tauri::WebviewWindow,
    transfers: State<'_, Arc<TransferService>>,
    id: Uuid,
    selection: Uuid,
) -> Result<(), CommandError> {
    transfers
        .cleanup_recovered_download(source(&window)?, id, selection)
        .await
        .map_err(error)
}
#[tauri::command]
pub fn cancel_transfer(
    window: tauri::WebviewWindow,
    transfers: State<'_, Arc<TransferService>>,
    id: Uuid,
) -> Result<(), CommandError> {
    transfers.cancel(source(&window)?, id).map_err(error)
}
#[tauri::command]
pub fn pause_transfer(
    window: tauri::WebviewWindow,
    transfers: State<'_, Arc<TransferService>>,
    id: Uuid,
) -> Result<(), CommandError> {
    transfers.pause(source(&window)?, id).map_err(error)
}
#[tauri::command]
pub fn resume_transfer(
    window: tauri::WebviewWindow,
    transfers: State<'_, Arc<TransferService>>,
    id: Uuid,
) -> Result<(), CommandError> {
    transfers.resume(source(&window)?, id).map_err(error)
}
#[tauri::command]
pub fn retry_transfer(
    window: tauri::WebviewWindow,
    transfers: State<'_, Arc<TransferService>>,
    id: Uuid,
) -> Result<(), CommandError> {
    transfers.retry(source(&window)?, id).map_err(error)
}
#[tauri::command]
pub async fn retry_transfer_on_session(
    window: tauri::WebviewWindow,
    sftp: State<'_, Arc<SftpService>>,
    transfers: State<'_, Arc<TransferService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    id: Uuid,
) -> Result<(), CommandError> {
    let window = source(&window)?;
    let descriptor = transfers.download_retry(window, id).map_err(error)?;
    let context = sftp
        .transfer_context_for_recovered_download(window, &terminal.0, session, descriptor)
        .await
        .map_err(error)?;
    transfers
        .retry_with_context(window, id, context)
        .map_err(error)
}
#[tauri::command]
pub fn retry_transfer_with_selection(
    window: tauri::WebviewWindow,
    sftp: State<'_, Arc<SftpService>>,
    transfers: State<'_, Arc<TransferService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    id: Uuid,
    selection: Uuid,
) -> Result<(), CommandError> {
    let window = source(&window)?;
    let target = transfers.target(window, id).map_err(error)?;
    let context = sftp
        .transfer_context(window, &terminal.0, session, target)
        .map_err(error)?;
    transfers
        .retry_with_selection(window, id, context, selection)
        .map_err(error)
}
#[tauri::command]
pub fn resolve_transfer_conflict(
    window: tauri::WebviewWindow,
    transfers: State<'_, Arc<TransferService>>,
    id: Uuid,
    revision: ExactU64,
    decision: Decision,
) -> Result<(), CommandError> {
    transfers
        .resolve(source(&window)?, id, revision, decision)
        .map_err(error)
}
