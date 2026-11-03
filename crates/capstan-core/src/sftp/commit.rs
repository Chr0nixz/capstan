//! Shared atomic publication for transfers and remote editing.
use super::RemotePath;
use capstan_ssh::sftp::{ClientError, SftpClient};
use tokio_util::sync::CancellationToken;

/// Once admitted by the caller's publication gate, observe the actual rename
/// result instead of canceling and blindly replaying an uncertain mutation.
/// Existing targets are never removed to emulate atomic replacement.
pub(crate) async fn publish_remote_file(
    client: &std::sync::Arc<SftpClient>,
    temporary: &RemotePath,
    target: &RemotePath,
    replace: bool,
) -> Result<(), ClientError> {
    let cancel = CancellationToken::new();
    if replace {
        client
            .atomic_replace(temporary.as_str(), target.as_str(), &cancel)
            .await
    } else {
        client
            .rename(temporary.as_str(), target.as_str(), &cancel)
            .await
    }
}
