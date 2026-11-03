//! Confidential cleanup checkpoints, without credentials or upload authority.
use super::{
    temporary::{LedgerPersistence, Temporary},
    EditError, TemporaryLedger,
};
use capstan_native_windows::editor::EditorStage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use uuid::Uuid;
use zeroize::Zeroizing;

const MAX_FRAME: usize = 64 * 1024;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CleanupPolicy {
    target: String,
    port: u16,
    username: String,
    credential_id: Uuid,
    credential_revision: u64,
    trust_fingerprint: String,
    agent_identity: Option<String>,
    agent_forwarding: bool,
    proxy_path: String,
}
impl From<&capstan_ssh::PoolKey> for CleanupPolicy {
    fn from(key: &capstan_ssh::PoolKey) -> Self {
        Self {
            target: key.target.clone(),
            port: key.port,
            username: key.username.clone(),
            credential_id: key.credential_id,
            credential_revision: key.credential_revision,
            trust_fingerprint: key.trust_fingerprint.clone(),
            agent_identity: key.agent_identity.clone(),
            agent_forwarding: key.agent_forwarding,
            proxy_path: key.proxy_path.clone(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CleanupScope {
    pub host: Uuid,
    policy: CleanupPolicy,
}
impl CleanupScope {
    pub fn new(host: Uuid, key: &capstan_ssh::PoolKey) -> Self {
        Self {
            host,
            policy: key.into(),
        }
    }
    pub fn matches(&self, host: Uuid, key: &capstan_ssh::PoolKey) -> bool {
        self.host == host && self.policy == CleanupPolicy::from(key)
    }
    fn valid(&self) -> bool {
        let text = |value: &str| value.len() <= 4096 && !value.chars().any(char::is_control);
        !self.host.is_nil()
            && !self.policy.credential_id.is_nil()
            && self.policy.port != 0
            && !self.policy.target.is_empty()
            && !self.policy.username.is_empty()
            && !self.policy.trust_fingerprint.is_empty()
            && [
                &self.policy.target,
                &self.policy.username,
                &self.policy.trust_fingerprint,
                &self.policy.proxy_path,
            ]
            .iter()
            .all(|value| text(value))
            && self
                .policy
                .agent_identity
                .as_ref()
                .is_none_or(|value| text(value))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    session: Uuid,
    scope: CleanupScope,
    entries: Vec<Temporary>,
}
struct Checkpoint {
    session: Uuid,
    scope: CleanupScope,
    sequence: AtomicU64,
    stage: Arc<Mutex<EditorStage>>,
}

#[async_trait::async_trait]
impl LedgerPersistence for Checkpoint {
    async fn save(&self, entries: &[Temporary]) -> Result<(), EditError> {
        let sequence = self
            .sequence
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value.checked_add(1)
            })
            .map_err(|_| EditError::ResourceLimit)?
            + 1;
        let bytes = encode(
            sequence,
            &Record {
                version: 1,
                session: self.session,
                scope: self.scope.clone(),
                entries: entries.to_vec(),
            },
        )?;
        let stage = self.stage.clone();
        tokio::task::spawn_blocking(move || {
            stage
                .lock()
                .map_err(|_| EditError::Residual)?
                .save_recovery(sequence, &bytes)
                .map_err(|_| EditError::LocalIo)
        })
        .await
        .map_err(|_| EditError::LocalIo)?
    }
}

fn encode(sequence: u64, record: &Record) -> Result<Zeroizing<Vec<u8>>, EditError> {
    if !record.scope.valid() || !TemporaryLedger::valid_entries(&record.entries) {
        return Err(EditError::Residual);
    }
    let body = Zeroizing::new(serde_json::to_vec(record).map_err(|_| EditError::Residual)?);
    if body.len() + 32 > MAX_FRAME {
        return Err(EditError::ResourceLimit);
    }
    let mut hasher = Sha256::new();
    hasher.update(sequence.to_le_bytes());
    hasher.update(&body);
    let mut frame = Zeroizing::new(hasher.finalize().to_vec());
    frame.extend_from_slice(&body);
    Ok(frame)
}
fn decode(sequence: u64, frame: &[u8], session: Uuid) -> Result<Record, EditError> {
    if frame.len() < 32 || frame.len() > MAX_FRAME {
        return Err(EditError::Residual);
    }
    let mut hasher = Sha256::new();
    hasher.update(sequence.to_le_bytes());
    hasher.update(&frame[32..]);
    if hasher.finalize().as_slice() != &frame[..32] {
        return Err(EditError::Residual);
    }
    let record: Record = serde_json::from_slice(&frame[32..]).map_err(|_| EditError::Residual)?;
    if record.version != 1
        || record.session != session
        || !record.scope.valid()
        || !TemporaryLedger::valid_entries(&record.entries)
    {
        return Err(EditError::Residual);
    }
    Ok(record)
}

pub(super) fn fresh(
    session: Uuid,
    scope: CleanupScope,
    stage: Arc<Mutex<EditorStage>>,
) -> TemporaryLedger {
    TemporaryLedger::persistent(
        Vec::new(),
        Arc::new(Checkpoint {
            session,
            scope,
            stage,
            sequence: AtomicU64::new(0),
        }),
    )
}
pub(super) struct Loaded {
    pub scope: Option<CleanupScope>,
    pub ledger: TemporaryLedger,
}
pub(super) fn load(session: Uuid, stage: Arc<Mutex<EditorStage>>) -> Result<Loaded, EditError> {
    let candidates = stage
        .lock()
        .map_err(|_| EditError::Residual)?
        .load_recovery()
        .map_err(|_| EditError::Residual)?;
    let mut newest = None;
    let mut invalid = candidates.corrupt;
    for candidate in candidates.payloads {
        match decode(candidate.sequence, &candidate.bytes, session) {
            Ok(record) => {
                if newest
                    .as_ref()
                    .is_none_or(|(sequence, _)| candidate.sequence > *sequence)
                {
                    newest = Some((candidate.sequence, record));
                }
            }
            Err(_) => invalid = true,
        }
    }
    match newest {
        Some((sequence, record)) => {
            let scope = record.scope;
            let persistence = Arc::new(Checkpoint {
                session,
                scope: scope.clone(),
                stage,
                sequence: AtomicU64::new(sequence),
            });
            Ok(Loaded {
                scope: Some(scope),
                ledger: TemporaryLedger::persistent(record.entries, persistence),
            })
        }
        None if !invalid => Ok(Loaded {
            scope: None,
            ledger: TemporaryLedger::default(),
        }),
        None => Err(EditError::Residual),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        remote_edit::{RemoteEditorService, ResidualAction},
        settings::Settings,
        sftp::RemotePath,
    };
    use capstan_native_windows::editor::EditorStore;

    fn session() -> Uuid {
        Uuid::from_u128(1)
    }
    fn scope() -> CleanupScope {
        CleanupScope::new(
            Uuid::from_u128(2),
            &capstan_ssh::PoolKey {
                target: "private-host".into(),
                port: 22,
                username: "test".into(),
                credential_id: Uuid::from_u128(3),
                credential_revision: 1,
                trust_fingerprint: "SHA256:pinned".into(),
                vault_epoch: 1,
                agent_identity: None,
                agent_forwarding: false,
                proxy_path: String::new(),
            },
        )
    }
    fn path() -> RemotePath {
        RemotePath::parse("/private/.capstan-edit-00000000-0000-4000-8000-000000000004.partial")
            .unwrap()
    }
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn frames_reject_corruption_unknown_versions_wrong_sessions_and_unbounded_records() {
        let record = Record {
            version: 1,
            session: session(),
            scope: scope(),
            entries: Vec::new(),
        };
        let frame = encode(1, &record).unwrap();
        assert!(decode(1, &frame, session()).is_ok());
        assert!(decode(2, &frame, session()).is_err());
        assert!(decode(1, &frame, Uuid::new_v4()).is_err());
        let mut corrupt = frame.to_vec();
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(decode(1, &corrupt, session()).is_err());
        assert!(decode(1, &vec![0; MAX_FRAME + 1], session()).is_err());
        let record = Record {
            version: 2,
            ..record
        };
        assert!(decode(1, &encode(1, &record).unwrap(), session()).is_err());
        let mut record = Record {
            version: 1,
            ..record
        };
        record.scope.policy.username = "bad\nuser".into();
        assert!(encode(1, &record).is_err());
    }

    #[test]
    fn valid_checksums_do_not_authorize_formal_paths_duplicate_ids_or_unbounded_lists() {
        let entry = serde_json::json!({ "id": Uuid::new_v4(), "path": path(), "evidence": null, "state": "unverified" });
        let record = serde_json::to_value(Record {
            version: 1,
            session: session(),
            scope: scope(),
            entries: Vec::new(),
        })
        .unwrap();
        let mut cases = Vec::new();
        let mut wrong_path = entry.clone();
        wrong_path["path"] = serde_json::json!("/private/notes.txt");
        cases.push(serde_json::json!([wrong_path]));
        cases.push(serde_json::json!([entry.clone(), entry.clone()]));
        cases.push(serde_json::json!((0..9)
            .map(|_| {
                let mut next = entry.clone();
                next["id"] = serde_json::json!(Uuid::new_v4());
                next["path"] =
                    serde_json::json!(format!("/private/.capstan-edit-{}.partial", Uuid::new_v4()));
                next
            })
            .collect::<Vec<_>>()));
        for entries in cases {
            let mut invalid = record.clone();
            invalid["entries"] = entries;
            let body = serde_json::to_vec(&invalid).unwrap();
            let mut digest = Sha256::new();
            digest.update(1_u64.to_le_bytes());
            digest.update(&body);
            let mut frame = digest.finalize().to_vec();
            frame.extend_from_slice(&body);
            assert!(decode(1, &frame, session()).is_err());
        }
        let mut invalid = record;
        invalid["unexpected"] = serde_json::json!(true);
        let body = serde_json::to_vec(&invalid).unwrap();
        let mut digest = Sha256::new();
        digest.update(1_u64.to_le_bytes());
        digest.update(&body);
        let mut frame = digest.finalize().to_vec();
        frame.extend_from_slice(&body);
        assert!(decode(1, &frame, session()).is_err());
    }

    #[test]
    fn mirrored_intents_survive_one_acknowledged_checkpoint_becoming_unreadable() {
        let root = tempfile::tempdir().unwrap();
        let store = EditorStore::new(root.path().join("editor"));
        let stage = Arc::new(Mutex::new(
            store
                .prepare(&session().to_string(), "notes.txt", b"local")
                .unwrap(),
        ));
        let mut ledger = fresh(session(), scope(), stage.clone());
        runtime().block_on(ledger.track(path())).unwrap();
        drop(ledger);
        drop(stage);
        // The newest acknowledged intent is damaged after OPEN could have run.
        // The other slot still carries that exact pathname, without evidence
        // that could authorize deleting an unverified remote file.
        std::fs::write(
            root.path()
                .join("editor")
                .join(session().to_string())
                .join(".capstan-recovery-a"),
            b"damaged",
        )
        .unwrap();
        let stage = Arc::new(Mutex::new(
            store.discover(8).unwrap().stages.pop().unwrap().1,
        ));
        let loaded = load(session(), stage.clone()).unwrap();
        assert_eq!(loaded.ledger.views().len(), 1);
        assert_eq!(loaded.ledger.views()[0].path, path().as_str());
        assert!(matches!(
            loaded.ledger.views()[0].state,
            super::super::TemporaryState::Unverified
        ));
        drop(loaded);
        stage.lock().unwrap().try_cleanup(None).unwrap();
    }

    #[test]
    fn recovery_checkpoint_survives_process_exit_without_destructors() {
        const VARIABLE: &str = "CAPSTAN_TEST_EDITOR_JOURNAL_ROOT";
        if let Some(root) = std::env::var_os(VARIABLE) {
            let root = std::path::PathBuf::from(root);
            let store = EditorStore::new(root.join("editor"));
            let stage = Arc::new(Mutex::new(
                store
                    .prepare(&session().to_string(), "notes.txt", b"saved but unsent")
                    .unwrap(),
            ));
            let mut ledger = fresh(session(), scope(), stage);
            runtime().block_on(ledger.track(path())).unwrap();
            // Simulate abrupt process termination: no Rust owner/stage drops.
            std::process::exit(73);
        }
        let root = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "remote_edit::journal::tests::recovery_checkpoint_survives_process_exit_without_destructors"])
            .env(VARIABLE, root.path()).output().unwrap();
        assert_eq!(output.status.code(), Some(73));
        let service = RemoteEditorService::new(
            Arc::new(Settings::new(root.path().to_owned())),
            EditorStore::new(root.path().join("editor")),
            Arc::new(tokio::sync::Mutex::new(())),
        );
        let snapshot = service.snapshot("main").unwrap();
        assert_eq!(snapshot.sessions.len(), 1);
        let recovered = &snapshot.sessions[0];
        assert!(recovered.recovered);
        assert_eq!(recovered.host_id, Some(scope().host));
        assert!(recovered.remote_path.is_none());
        assert!(!recovered.replace_allowed);
        assert_eq!(recovered.remote_temporaries.len(), 1);
        runtime().block_on(async {
            assert_eq!(
                service.retry("main", session()).await.unwrap_err(),
                EditError::ConnectionLost
            );
            assert_eq!(
                service
                    .bind_for_retry(
                        "main",
                        session(),
                        &crate::sftp::SftpService::default(),
                        "new",
                        serde_json::from_value(serde_json::json!(Uuid::new_v4())).unwrap()
                    )
                    .await
                    .unwrap_err(),
                EditError::ScopeExpired
            );
            let output = root.path().join("export");
            std::fs::create_dir(&output).unwrap();
            assert_eq!(
                service
                    .close(
                        "main",
                        session(),
                        ResidualAction::SaveLocalCopy,
                        Some(output.clone())
                    )
                    .await,
                Err(EditError::ConnectionLost)
            );
            assert_eq!(
                std::fs::read(output.join("notes.txt")).unwrap(),
                b"saved but unsent"
            );
            service
                .close("main", session(), ResidualAction::KeepForRecovery, None)
                .await
                .unwrap();
        });
        assert!(!service.has_open("main"));
        assert!(root
            .path()
            .join("editor")
            .join(session().to_string())
            .join("notes.txt")
            .exists());
        drop(service);
    }

    #[test]
    fn unreadable_checkpoints_preserve_the_document_and_allow_retaining_for_inspection() {
        let root = tempfile::tempdir().unwrap();
        let store = EditorStore::new(root.path().join("editor"));
        drop(
            store
                .prepare(&session().to_string(), "notes.txt", b"unique copy")
                .unwrap(),
        );
        for slot in [".capstan-recovery-a", ".capstan-recovery-b"] {
            std::fs::write(
                root.path()
                    .join("editor")
                    .join(session().to_string())
                    .join(slot),
                b"unreadable metadata",
            )
            .unwrap();
        }
        let service = RemoteEditorService::new(
            Arc::new(Settings::new(root.path().to_owned())),
            store,
            Arc::new(tokio::sync::Mutex::new(())),
        );
        let view = service.snapshot("main").unwrap().sessions.remove(0);
        assert!(view.recovered);
        assert_eq!(view.error, Some(EditError::Residual));
        runtime().block_on(async {
            assert_eq!(
                service
                    .close("main", session(), ResidualAction::Discard, None)
                    .await,
                Err(EditError::Residual)
            );
            service
                .close("main", session(), ResidualAction::KeepForRecovery, None)
                .await
                .unwrap();
        });
        assert_eq!(
            std::fs::read(
                root.path()
                    .join("editor")
                    .join(session().to_string())
                    .join("notes.txt")
            )
            .unwrap(),
            b"unique copy"
        );
        assert!(!service.has_open("main"));
    }
}
