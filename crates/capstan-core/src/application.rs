//! Application-level exit coordination without a desktop-framework dependency.
use crate::{
    settings::{Settings, SettingsError},
    sftp::{SftpError, SftpService},
    transfer::TransferService,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExitError {
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error(transparent)]
    Transfer(#[from] SftpError),
}

pub async fn prepare_file_workflow_exit(
    settings: &Settings,
    transfers: &TransferService,
    sftp: &SftpService,
    window: &str,
) -> Result<(), ExitError> {
    let (settings_result, transfer_result, deletion_result) = tokio::join!(
        tokio::time::timeout(
            std::time::Duration::from_secs(crate::sftp::EXIT_DRAIN_TIMEOUT_SECONDS),
            settings.drain()
        ),
        transfers.prepare_exit(window),
        sftp.prepare_tree_delete_exit(window),
    );
    let result = settings_result
        .unwrap_or(Err(SettingsError::Unavailable))
        .map_err(ExitError::from)
        .and(transfer_result.map_err(ExitError::from))
        .and(deletion_result.map_err(ExitError::from));
    if result.is_err() {
        settings.resume();
        transfers.abort_exit(window);
        sftp.abort_tree_delete_exit(window);
    }
    result
}
