//! Source validation and DTO forwarding only; SFTP ownership lives in core.
use crate::CommandError;
use capstan_core::sftp::{
    AttachedSftp, Confirmation, DirectoryPage, ExactU64, Mutation, RemoteEntryId, RemotePath,
    SftpService, SftpSessionId,
};
use capstan_core::shell_integration as remote_edit;
use capstan_terminal::TerminalSessionId;
use std::sync::Arc;
use tauri::State;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn source(window: &tauri::WebviewWindow) -> Result<&str, CommandError> {
    if window.label() != "main" {
        return Err(CommandError::new("window_source"));
    }
    Ok(window.label())
}
fn error(error: capstan_core::sftp::SftpError) -> CommandError {
    CommandError::new(error.to_string())
}

#[tauri::command]
pub fn update_remote_cwd(
    window: tauri::WebviewWindow,
    service: State<'_, Arc<SftpService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    generation: ExactU64,
    sequence: ExactU64,
    uri: String,
) -> Result<capstan_core::sftp::cwd::CwdSnapshot, CommandError> {
    service
        .update_cwd(
            source(&window)?,
            &terminal.0,
            session,
            generation,
            sequence,
            &uri,
        )
        .map_err(error)
}

#[tauri::command]
pub fn configure_remote_cwd_aliases(
    window: tauri::WebviewWindow,
    service: State<'_, Arc<SftpService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    generation: ExactU64,
    aliases: Option<Vec<String>>,
) -> Result<capstan_core::sftp::cwd::CwdSnapshot, CommandError> {
    service
        .configure_cwd_aliases(source(&window)?, &terminal.0, session, generation, aliases)
        .map_err(error)
}

#[tauri::command]
pub async fn attach_sftp(
    window: tauri::WebviewWindow,
    service: State<'_, Arc<SftpService>>,
    terminal: TerminalSessionId,
) -> Result<AttachedSftp, CommandError> {
    service
        .attach(source(&window)?, &terminal.0, &CancellationToken::new())
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn detach_sftp(
    window: tauri::WebviewWindow,
    service: State<'_, Arc<SftpService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
) -> Result<(), CommandError> {
    service
        .detach(source(&window)?, &terminal.0, session)
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn list_remote_directory(
    window: tauri::WebviewWindow,
    service: State<'_, Arc<SftpService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    path: RemotePath,
) -> Result<DirectoryPage, CommandError> {
    service
        .list(source(&window)?, &terminal.0, session, path)
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn next_remote_directory_page(
    window: tauri::WebviewWindow,
    service: State<'_, Arc<SftpService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    revision: ExactU64,
) -> Result<DirectoryPage, CommandError> {
    service
        .next_page(source(&window)?, &terminal.0, session, revision)
        .await
        .map_err(error)
}
#[tauri::command]
pub fn release_remote_entries(
    window: tauri::WebviewWindow,
    service: State<'_, Arc<SftpService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    entries: Vec<RemoteEntryId>,
) -> Result<(), CommandError> {
    service
        .release_entries(source(&window)?, &terminal.0, session, &entries)
        .map_err(error)
}
#[tauri::command]
pub async fn create_remote_entry(
    window: tauri::WebviewWindow,
    service: State<'_, Arc<SftpService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    parent: RemotePath,
    name: String,
    directory: bool,
) -> Result<(), CommandError> {
    service
        .create(
            source(&window)?,
            &terminal.0,
            session,
            parent,
            &name,
            directory,
        )
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn prepare_remote_mutation(
    window: tauri::WebviewWindow,
    service: State<'_, Arc<SftpService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    entry: RemoteEntryId,
    mutation: Mutation,
) -> Result<Confirmation, CommandError> {
    service
        .prepare_mutation(source(&window)?, &terminal.0, session, entry, mutation)
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn apply_remote_mutation(
    window: tauri::WebviewWindow,
    service: State<'_, Arc<SftpService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    token: Uuid,
) -> Result<(), CommandError> {
    service
        .apply_mutation(source(&window)?, &terminal.0, session, token)
        .await
        .map_err(error)
}

#[tauri::command]
pub async fn prepare_remote_tree_delete(
    window: tauri::WebviewWindow,
    service: State<'_, Arc<SftpService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    entry: RemoteEntryId,
) -> Result<capstan_core::sftp::DeleteTreeSummary, CommandError> {
    service
        .prepare_tree_delete(source(&window)?, &terminal.0, session, entry)
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn apply_remote_tree_delete(
    window: tauri::WebviewWindow,
    service: State<'_, Arc<SftpService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    token: Uuid,
) -> Result<capstan_core::sftp::DeleteTreeReport, CommandError> {
    service
        .apply_tree_delete(source(&window)?, &terminal.0, session, token)
        .await
        .map_err(error)
}
#[tauri::command]
pub fn cancel_remote_tree_delete(
    window: tauri::WebviewWindow,
    service: State<'_, Arc<SftpService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    token: Uuid,
) -> Result<(), CommandError> {
    service
        .cancel_tree_delete(source(&window)?, &terminal.0, session, token)
        .map_err(error)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn preview_shell_integration(
    window: tauri::WebviewWindow,
    service: State<'_, Arc<SftpService>>,
    previews: State<'_, Arc<remote_edit::PreviewRegistry>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    entry: RemoteEntryId,
    shell: remote_edit::Shell,
    action: remote_edit::Action,
) -> Result<remote_edit::PreviewSummary, CommandError> {
    let window_label = source(&window)?.to_owned();
    let context = service
        .remote_file_context(&window_label, &terminal.0, session, entry)
        .map_err(error)?;
    let cancel = CancellationToken::new();
    let bytes = remote_edit::read_remote_config_checked(
        &context.client,
        &context.path,
        Some(&context.metadata),
        &cancel,
    )
    .await
    .map_err(|cause| CommandError::new(cause.to_string()))?;
    previews
        .create(
            remote_edit::PreviewBinding {
                window: window_label,
                terminal: terminal.0.clone(),
                generation: context.binding.generation,
                path: context.path.as_str().to_owned(),
            },
            &bytes,
            shell,
            action,
            context.lease.authenticated_target(),
        )
        .map_err(|cause| CommandError::new(cause.to_string()))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn apply_shell_integration(
    window: tauri::WebviewWindow,
    service: State<'_, Arc<SftpService>>,
    previews: State<'_, Arc<remote_edit::PreviewRegistry>>,
    transfers: State<'_, Arc<capstan_core::transfer::TransferService>>,
    terminal: TerminalSessionId,
    session: SftpSessionId,
    entry: RemoteEntryId,
    token: Uuid,
) -> Result<remote_edit::RemoteApplyResult, CommandError> {
    let window_label = source(&window)?.to_owned();
    let context = service
        .remote_file_context(&window_label, &terminal.0, session, entry)
        .map_err(error)?;
    let cancel = CancellationToken::new();
    let bytes = remote_edit::read_remote_config_checked(
        &context.client,
        &context.path,
        Some(&context.metadata),
        &cancel,
    )
    .await
    .map_err(|cause| CommandError::new(cause.to_string()))?;
    let binding = remote_edit::PreviewBinding {
        window: window_label,
        terminal: terminal.0,
        generation: context.binding.generation,
        path: context.path.as_str().to_owned(),
    };
    let (_, _, host, plan) = previews
        .consume(&binding, token, &bytes)
        .map_err(|cause| CommandError::new(cause.to_string()))?;
    if host != context.lease.authenticated_target() {
        return Err(CommandError::new("scope_expired"));
    }
    remote_edit::apply_remote_plan(
        &context.client,
        &context.path,
        &bytes,
        &plan,
        &context.metadata,
        &transfers.publication_gate(),
        &cancel,
    )
    .await
    .map_err(|cause| CommandError::new(cause.to_string()))
}
