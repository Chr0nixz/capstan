//! Editor publication shares the transfer commit gate and retains cleanup evidence.
use super::{
    check_remote_baseline, decode, map_client_error, metadata, EditError, TemporaryLedger,
};
use crate::sftp::{Metadata, RemoteName, RemotePath};
use capstan_ssh::sftp::SftpClient;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub struct PublicationBaseline<'a> {
    pub metadata: Option<&'a Metadata>,
    pub hash: Option<[u8; 32]>,
}

/// Single publication for probes and callers without a live editing session.
/// Production editors use the tracked form so uncertain cleanup stays visible.
pub async fn apply_remote_bytes(
    client: &Arc<SftpClient>,
    path: &RemotePath,
    source: &[u8],
    expected: Option<&Metadata>,
    expected_hash: Option<[u8; 32]>,
    commits: &tokio::sync::Mutex<()>,
    cancel: &CancellationToken,
) -> Result<Metadata, EditError> {
    apply_remote_bytes_tracked(
        client,
        path,
        source,
        PublicationBaseline {
            metadata: expected,
            hash: expected_hash,
        },
        commits,
        cancel,
        &mut TemporaryLedger::default(),
    )
    .await
}

pub async fn apply_remote_bytes_tracked(
    client: &Arc<SftpClient>,
    path: &RemotePath,
    source: &[u8],
    baseline: PublicationBaseline<'_>,
    commits: &tokio::sync::Mutex<()>,
    cancel: &CancellationToken,
    ledger: &mut TemporaryLedger,
) -> Result<Metadata, EditError> {
    let _ = decode(source)?;
    check_remote_baseline(client, path, baseline.metadata, baseline.hash, cancel).await?;
    if baseline.metadata.is_some() && !client.capabilities().atomic_replace {
        return Err(EditError::Conflict);
    }
    let parent = path
        .as_str()
        .rsplit_once('/')
        .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
        .ok_or(EditError::RemoteIo)?;
    let temporary = RemotePath::parse(parent)
        .and_then(|parent| {
            parent.join(&RemoteName::from_bytes(
                format!(".capstan-edit-{}.partial", Uuid::new_v4()).as_bytes(),
            )?)
        })
        .map_err(|_| EditError::RemoteIo)?;
    let id = ledger.track(temporary.clone()).await?;
    let result = async {
        let file = client
            .open_file(temporary.as_str(), true, cancel)
            .await
            .map_err(map_client_error)?;
        let mut offset = 0_usize;
        let write = async {
            while offset < source.len() {
                let end = (offset + crate::sftp::REQUEST_BYTES).min(source.len());
                file.write(offset as u64, &source[offset..end], cancel)
                    .await
                    .map_err(map_client_error)?;
                offset = end;
            }
            if client.capabilities().fsync {
                file.sync(cancel).await.map_err(map_client_error)?;
            }
            Ok::<_, EditError>(())
        }
        .await;
        // Capture acknowledged bytes even after a denied write/canceled wait.
        // A lost write ACK with a different size is retained as unverified.
        let cleanup = CancellationToken::new();
        if let Ok(attributes) = file.metadata(&cleanup).await {
            if let Err(error) = ledger.record(id, attributes, &source[..offset]).await {
                let _ = file.close(&cleanup).await;
                return Err(error);
            }
        }
        if let Err(error) = write {
            let _ = file.close(&cleanup).await;
            return Err(error);
        }
        ledger.verify(id, client, cancel).await?;
        if let Some(mode) = baseline.metadata.and_then(|metadata| metadata.permissions) {
            client
                .set_permissions(temporary.as_str(), mode & 0o7777, cancel)
                .await
                .map_err(map_client_error)?;
            let attributes = file.metadata(cancel).await.map_err(map_client_error)?;
            ledger.record(id, attributes, source).await?;
        }
        file.close(cancel).await.map_err(map_client_error)?;
        let _commit = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(EditError::Canceled),
            () = client.closed() => return Err(EditError::ConnectionLost),
            guard = commits.lock() => guard,
        };
        check_remote_baseline(client, path, baseline.metadata, baseline.hash, cancel).await?;
        ledger.verify(id, client, cancel).await?;
        crate::sftp::publish_remote_file(client, &temporary, path, baseline.metadata.is_some())
            .await
            .map_err(map_client_error)?;
        // A successful rename owns no temporary, even if the following stat
        // fails. A lost rename ACK leaves the record for revalidation on retry.
        ledger.forget(id).await?;
        metadata(
            client
                .lstat(path.as_str(), &cleanup)
                .await
                .map_err(map_client_error)?,
        )
    }
    .await;
    if result.is_err() {
        // A fresh token allows bounded cleanup after user cancellation, but
        // deletion still requires the original metadata and complete hash.
        let _ = ledger
            .cleanup_one(id, client, &CancellationToken::new())
            .await;
    }
    result
}
