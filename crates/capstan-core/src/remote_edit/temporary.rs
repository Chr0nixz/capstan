//! Bounded ownership evidence with an optional write-ahead persistence boundary.
use super::{map_client_error, metadata, read_remote_bytes, EditError};
use crate::sftp::{Metadata, RemotePath};
use capstan_ssh::sftp::{ClientError, SftpClient};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub const MAX_EDITOR_TEMPORARIES: usize = 8;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemporaryState {
    Unverified,
    Recorded,
    Changed,
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };

    #[derive(Default)]
    struct FailingCheckpoint {
        calls: AtomicUsize,
        fail: AtomicUsize,
        saved: Mutex<Vec<Vec<String>>>,
    }
    #[async_trait::async_trait]
    impl LedgerPersistence for FailingCheckpoint {
        async fn save(&self, entries: &[Temporary]) -> Result<(), EditError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail.load(Ordering::SeqCst) == call {
                return Err(EditError::LocalIo);
            }
            self.saved.lock().unwrap().push(
                entries
                    .iter()
                    .map(|entry| entry.path.as_str().to_owned())
                    .collect(),
            );
            Ok(())
        }
    }
    #[tokio::test]
    async fn failed_intent_mirroring_aborts_admission_and_failed_forgetting_keeps_the_record() {
        let checkpoint = Arc::new(FailingCheckpoint::default());
        checkpoint.fail.store(2, Ordering::SeqCst);
        let mut ledger = TemporaryLedger::persistent(Vec::new(), checkpoint.clone());
        let path =
            RemotePath::parse("/files/.capstan-edit-00000000-0000-4000-8000-000000000001.partial")
                .unwrap();
        assert_eq!(ledger.track(path.clone()).await, Err(EditError::LocalIo));
        assert_eq!(checkpoint.calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            checkpoint.saved.lock().unwrap().as_slice(),
            &[vec![path.as_str().to_owned()]]
        );
        let id = ledger.views()[0].id;
        checkpoint.fail.store(3, Ordering::SeqCst);
        assert_eq!(ledger.forget(id).await, Err(EditError::LocalIo));
        assert_eq!(ledger.views().len(), 1);
        checkpoint.fail.store(0, Ordering::SeqCst);
        ledger.forget(id).await.unwrap();
        assert!(ledger.is_empty());
        assert!(checkpoint.saved.lock().unwrap().last().unwrap().is_empty());
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTemporaryView {
    pub id: Uuid,
    pub path: String,
    pub state: TemporaryState,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Temporary {
    id: Uuid,
    path: RemotePath,
    evidence: Option<(Metadata, [u8; 32])>,
    state: TemporaryState,
}

#[derive(Default)]
pub struct TemporaryLedger {
    entries: Vec<Temporary>,
    persistence: Option<Arc<dyn LedgerPersistence>>,
}

#[async_trait::async_trait]
pub(super) trait LedgerPersistence: Send + Sync {
    async fn save(&self, entries: &[Temporary]) -> Result<(), EditError>;
}

impl TemporaryLedger {
    #[cfg(windows)]
    pub(super) fn persistent(
        entries: Vec<Temporary>,
        persistence: Arc<dyn LedgerPersistence>,
    ) -> Self {
        Self {
            entries,
            persistence: Some(persistence),
        }
    }
    pub(super) async fn checkpoint(&self) -> Result<(), EditError> {
        if let Some(persistence) = &self.persistence {
            persistence.save(&self.entries).await?;
        }
        Ok(())
    }
    #[cfg(windows)]
    pub(super) fn first_target(&self) -> Option<(RemotePath, Metadata)> {
        self.entries.first().map(|entry| {
            (
                entry.path.clone(),
                entry
                    .evidence
                    .as_ref()
                    .map(|(metadata, _)| metadata.clone())
                    .unwrap_or(Metadata {
                        kind: crate::sftp::FileKind::File,
                        size: None,
                        modified_seconds: None,
                        permissions: None,
                    }),
            )
        })
    }
    #[cfg(windows)]
    pub(super) fn valid_entries(entries: &[Temporary]) -> bool {
        let mut seen = std::collections::HashSet::new();
        let mut paths = std::collections::HashSet::new();
        entries.len() <= MAX_EDITOR_TEMPORARIES
            && entries.iter().all(|entry| {
                let name = entry.path.as_str().rsplit('/').next().unwrap_or("");
                let valid_name = name
                    .strip_prefix(".capstan-edit-")
                    .and_then(|name| name.strip_suffix(".partial"))
                    .is_some_and(|id| {
                        Uuid::parse_str(id).is_ok_and(|parsed| parsed.to_string() == id)
                    });
                valid_name
                    && seen.insert(entry.id)
                    && paths.insert(entry.path.as_str())
                    && entry.evidence.as_ref().is_none_or(|(metadata, _)| {
                        metadata.kind == crate::sftp::FileKind::File
                            && metadata.matches_baseline(metadata)
                            && metadata
                                .size
                                .is_some_and(|size| size.0 <= super::MAX_EDITOR_FILE_BYTES as u64)
                    })
            })
    }
    pub fn views(&self) -> Vec<RemoteTemporaryView> {
        self.entries
            .iter()
            .map(|entry| RemoteTemporaryView {
                id: entry.id,
                path: entry.path.as_str().into(),
                state: entry.state,
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) async fn track(&mut self, path: RemotePath) -> Result<Uuid, EditError> {
        if self.entries.len() >= MAX_EDITOR_TEMPORARIES {
            return Err(EditError::ResourceLimit);
        }
        let id = Uuid::new_v4();
        self.entries.push(Temporary {
            id,
            path,
            evidence: None,
            state: TemporaryState::Unverified,
        });
        self.checkpoint().await?;
        // Mirror every new pathname before OPEN. Either slot must retain that
        // intent if a later checkpoint is torn or one acknowledged slot is
        // damaged. Proof/removal updates may conservatively fall back to it.
        self.checkpoint().await?;
        Ok(id)
    }

    /// Evidence comes from the exclusively created handle and acknowledged
    /// source bytes, never from adopting a subsequently discovered path.
    pub(super) async fn record(
        &mut self,
        id: Uuid,
        attributes: capstan_ssh::sftp::Attributes,
        bytes: &[u8],
    ) -> Result<(), EditError> {
        let Ok(attributes) = metadata(attributes) else {
            return Ok(());
        };
        if attributes.size.map(|size| size.0) != Some(bytes.len() as u64)
            || !attributes.matches_baseline(&attributes)
        {
            return Ok(());
        }
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
            entry.evidence = Some((attributes, Sha256::digest(bytes).into()));
            entry.state = TemporaryState::Recorded;
        }
        self.checkpoint().await
    }

    pub(super) async fn forget(&mut self, id: Uuid) -> Result<(), EditError> {
        let previous = self.entries.clone();
        self.entries.retain(|entry| entry.id != id);
        if let Err(error) = self.checkpoint().await {
            self.entries = previous;
            return Err(error);
        }
        Ok(())
    }

    pub(super) async fn verify(
        &mut self,
        id: Uuid,
        client: &Arc<SftpClient>,
        cancel: &CancellationToken,
    ) -> Result<(), EditError> {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or(EditError::Residual)?;
        let Some((expected, hash)) = &entry.evidence else {
            return Err(EditError::Residual);
        };
        super::check_canonical_parent(client, &entry.path, cancel).await?;
        let verified = match read_remote_bytes(client, &entry.path, Some(expected), cancel).await {
            Ok((bytes, _)) => <[u8; 32]>::from(Sha256::digest(&bytes)) == *hash,
            Err(EditError::Conflict | EditError::UnsupportedContent | EditError::FileTooLarge) => {
                false
            }
            Err(error) => return Err(error),
        };
        if !verified {
            entry.state = TemporaryState::Changed;
            self.checkpoint().await?;
            return Err(EditError::Residual);
        }
        Ok(())
    }

    /// Never remove an unverified or replaced path. SFTP v3 has no unlink CAS;
    /// the final lstat/unlink round trip retains the documented server race.
    pub async fn cleanup(
        &mut self,
        client: &Arc<SftpClient>,
        cancel: &CancellationToken,
    ) -> Result<(), EditError> {
        let mut failure = None;
        for id in self
            .entries
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>()
        {
            if let Err(error) = self.cleanup_one(id, client, cancel).await {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }

    pub(super) async fn cleanup_one(
        &mut self,
        id: Uuid,
        client: &Arc<SftpClient>,
        cancel: &CancellationToken,
    ) -> Result<(), EditError> {
        let Some(entry) = self.entries.iter().find(|entry| entry.id == id) else {
            return Ok(());
        };
        let path = entry.path.clone();
        super::check_canonical_parent(client, &path, cancel).await?;
        match client.lstat(path.as_str(), cancel).await {
            Err(ClientError::NotFound) => {
                self.forget(id).await?;
                return Ok(());
            }
            Err(error) => return Err(map_client_error(error)),
            Ok(_) => {}
        }
        self.verify(id, client, cancel).await?;
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or(EditError::Residual)?;
        let (expected, _) = entry.evidence.as_ref().ok_or(EditError::Residual)?;
        let fresh = metadata(
            client
                .lstat(path.as_str(), cancel)
                .await
                .map_err(map_client_error)?,
        )?;
        if !fresh.matches_baseline(expected) {
            return Err(EditError::Residual);
        }
        client
            .remove_file(path.as_str(), cancel)
            .await
            .map_err(map_client_error)?;
        self.forget(id).await?;
        Ok(())
    }
}
