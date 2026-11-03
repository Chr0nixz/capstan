//! Durable, profile-encrypted records for transfer partials on Windows.
//!
//! The records are deliberately weaker than a transfer capability.  They can
//! tell the application what a previous process left behind, but they never
//! contain credentials, an SSH session, or permission to perform a remote
//! mutation.  Resuming always needs a fresh authenticated session and a new
//! native picker grant.

use super::{Direction, TransferTarget};
use crate::sftp::{ExactU64, FileKind, Metadata, RemotePath, SftpError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;
use zeroize::Zeroizing;

mod storage;
#[cfg(all(test, windows))]
use std::path::Path;
use storage::Storage;

const VERSION: u32 = 1;
const MAX_RECORDS: usize = 128;
const MAX_FRAME: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferRecoveryView {
    pub id: Uuid,
    pub host_id: Uuid,
    pub name: String,
    pub remote_path: RemotePath,
    pub direction: Direction,
    pub total: ExactU64,
    pub partial_size: Option<ExactU64>,
    pub local_partial: bool,
    pub state: RecoveryState,
    pub updated_seconds: ExactU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryState {
    AwaitingReconnect,
    Available,
    Missing,
    Changed,
    ManualInspection,
    CleanupRequired,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecoveryMode {
    #[default]
    Resume,
    Cleanup,
    Inspect,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferRecoverySnapshot {
    pub entries: Vec<TransferRecoveryView>,
    pub incomplete: bool,
}

/// Internal descriptor.  Paths are never returned by the public view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecoveryRecord {
    pub id: Uuid,
    pub host_id: Uuid,
    pub name: String,
    pub remote_path: RemotePath,
    pub target: TransferTarget,
    pub direction: Direction,
    pub total: u64,
    pub partial_size: Option<u64>,
    pub remote_partial: Option<RemotePath>,
    pub local_partial: Option<PathBuf>,
    pub local_target: Option<PathBuf>,
    #[serde(default)]
    pub local_identity: Option<u128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_baseline: Option<Metadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_sha256: Option<String>,
    #[serde(default)]
    pub mode: RecoveryMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_cleanup_baseline: Option<Metadata>,
    pub updated_seconds: u64,
}

pub(crate) struct RecoveryDraft {
    pub id: Uuid,
    pub host_id: Uuid,
    pub name: String,
    pub remote_path: RemotePath,
    pub target: TransferTarget,
    pub direction: Direction,
    pub total: u64,
    pub partial_size: Option<u64>,
    pub remote_partial: Option<RemotePath>,
    pub local_partial: Option<PathBuf>,
    pub local_target: Option<PathBuf>,
    pub local_identity: Option<u128>,
    pub source_baseline: Option<Metadata>,
    pub policy_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordFile {
    version: u32,
    sequence: u64,
    records: Vec<RecoveryRecord>,
    #[serde(default)]
    incomplete: bool,
}

#[derive(Debug, Clone)]
struct Loaded {
    sequence: u64,
    records: Vec<RecoveryRecord>,
    incomplete: bool,
}

#[derive(Debug)]
pub(crate) struct RecoveryStore {
    storage: Mutex<Storage>,
    state: Mutex<Loaded>,
}

impl RecoveryStore {
    pub(crate) fn open(directory: PathBuf) -> io::Result<Self> {
        let mut storage = Storage::open(&directory)?;
        let (mut loaded, legacy) = load_slots(&mut storage);
        if legacy && !loaded.records.is_empty() {
            // Plain checksums never establish original authorization. The
            // decoded legacy records are manual-only; mirror their encrypted
            // replacement before returning a usable store. Partial data stays
            // untouched if migration fails.
            for _ in 0..2 {
                loaded.sequence = loaded
                    .sequence
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("recovery sequence exhausted"))?;
                persist(&mut storage, &loaded)
                    .map_err(|_| io::Error::other("recovery migration failed"))?;
            }
        }
        Ok(Self {
            storage: Mutex::new(storage),
            state: Mutex::new(loaded),
        })
    }

    pub(crate) fn snapshot(&self) -> Result<TransferRecoverySnapshot, SftpError> {
        let state = self.state.lock().map_err(|_| SftpError::LocalIo)?;
        let entries = state.records.iter().map(view).collect();
        Ok(TransferRecoverySnapshot {
            entries,
            incomplete: state.incomplete,
        })
    }

    pub(crate) fn upsert(&self, record: RecoveryRecord) -> Result<(), SftpError> {
        validate_record(&record)?;
        let mut state = self.state.lock().map_err(|_| SftpError::LocalIo)?;
        let mut next = state.clone();
        if let Some(existing) = next.records.iter_mut().find(|item| item.id == record.id) {
            *existing = record;
        } else {
            if next.records.len() >= MAX_RECORDS {
                return Err(SftpError::ResourceLimit);
            }
            next.records.push(record);
        }
        next.records.sort_by_key(|item| item.updated_seconds);
        next.sequence = next
            .sequence
            .checked_add(1)
            .ok_or(SftpError::ResourceLimit)?;
        let mut storage = self.storage.lock().map_err(|_| SftpError::LocalIo)?;
        persist(&mut storage, &next)?;
        next.incomplete |= load_slots(&mut storage).0.incomplete;
        *state = next;
        Ok(())
    }

    pub(crate) fn remove(&self, id: Uuid) -> Result<(), SftpError> {
        let mut state = self.state.lock().map_err(|_| SftpError::LocalIo)?;
        let mut next = state.clone();
        let before = next.records.len();
        next.records.retain(|record| record.id != id);
        if next.records.len() != before {
            next.sequence = next
                .sequence
                .checked_add(1)
                .ok_or(SftpError::ResourceLimit)?;
            let mut storage = self.storage.lock().map_err(|_| SftpError::LocalIo)?;
            persist(&mut storage, &next)?;
            next.incomplete |= load_slots(&mut storage).0.incomplete;
            *state = next;
        }
        Ok(())
    }

    pub(crate) fn record(&self, id: Uuid) -> Result<RecoveryRecord, SftpError> {
        self.state
            .lock()
            .map_err(|_| SftpError::LocalIo)?
            .records
            .iter()
            .find(|record| record.id == id)
            .cloned()
            .ok_or(SftpError::ScopeExpired)
    }
    pub(crate) fn restrict_to_inspection(&self, id: Uuid) {
        // A failed checkpoint must not leave an executable old capability in
        // the running process. The next successful store write also persists
        // this restriction; an unwritable store is explicitly incomplete.
        if let Ok(mut state) = self.state.lock() {
            state.incomplete = true;
            if let Some(record) = state.records.iter_mut().find(|record| record.id == id) {
                record.mode = RecoveryMode::Inspect;
                record.remote_cleanup_baseline = None;
            }
        }
    }
}

fn view(record: &RecoveryRecord) -> TransferRecoveryView {
    let (state, local_partial) = if record.remote_partial.is_some() {
        (RecoveryState::AwaitingReconnect, false)
    } else if let Some(path) = &record.local_partial {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_file() => {
                let current = metadata.len();
                let expected = record.partial_size.unwrap_or(current);
                if current == expected {
                    (RecoveryState::Available, true)
                } else {
                    (RecoveryState::Changed, true)
                }
            }
            Ok(_) => (RecoveryState::Changed, true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (RecoveryState::Missing, true),
            Err(_) => (RecoveryState::Changed, true),
        }
    } else {
        (RecoveryState::Missing, false)
    };
    let insufficient_proof = record.mode == RecoveryMode::Inspect
        || record.policy_sha256.is_none()
        || (record.direction == Direction::Download
            && (record.source_baseline.is_none() || record.local_identity.is_none()));
    let state = if record.mode == RecoveryMode::Inspect
        || insufficient_proof
            && matches!(
                state,
                RecoveryState::Available | RecoveryState::AwaitingReconnect
            ) {
        RecoveryState::ManualInspection
    } else if record.mode == RecoveryMode::Cleanup
        && matches!(
            state,
            RecoveryState::Available | RecoveryState::AwaitingReconnect
        )
    {
        RecoveryState::CleanupRequired
    } else {
        state
    };
    TransferRecoveryView {
        id: record.id,
        host_id: record.host_id,
        name: record.name.clone(),
        remote_path: record.remote_path.clone(),
        direction: record.direction,
        total: ExactU64(record.total),
        partial_size: record.partial_size.map(ExactU64),
        local_partial,
        state,
        updated_seconds: ExactU64(record.updated_seconds),
    }
}

fn validate_record(record: &RecoveryRecord) -> Result<(), SftpError> {
    if record.id.is_nil()
        || record.host_id.is_nil()
        || record.name.is_empty()
        || record.name.len() > 255
        || record.remote_path.as_str().len() > crate::sftp::MAX_REMOTE_PATH_BYTES
        || record.partial_size.is_some_and(|size| size > record.total)
        || (record.remote_partial.is_none() == record.local_partial.is_none())
        || record.policy_sha256.as_ref().is_some_and(|digest| {
            digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        || record.source_baseline.as_ref().is_some_and(|baseline| {
            record.direction != Direction::Download
                || baseline.kind != FileKind::File
                || baseline.size.map(|size| size.0) != Some(record.total)
                || baseline.modified_seconds.is_none()
        })
        || record
            .remote_cleanup_baseline
            .as_ref()
            .is_some_and(|baseline| {
                record.direction != Direction::Upload
                    || record.mode != RecoveryMode::Cleanup
                    || baseline.kind != FileKind::File
                    || baseline.size.map(|size| size.0) != record.partial_size
                    || baseline.modified_seconds.is_none()
            })
        || (record.mode == RecoveryMode::Cleanup
            && record.direction == Direction::Upload
            && record.remote_cleanup_baseline.is_none())
    {
        return Err(SftpError::PathInvalid);
    }
    let expected_partial = format!(".capstan-{}.partial", record.id);
    if let Some(path) = &record.local_partial {
        if !path.is_absolute() || path.as_os_str().len() > 32_767 {
            return Err(SftpError::PathInvalid);
        }
        let valid_target = record.local_target.as_ref().is_some_and(|target| {
            target.is_absolute()
                && path.parent() == target.parent()
                && path.file_name().and_then(|name| name.to_str())
                    == Some(expected_partial.as_str())
        });
        if !valid_target
            || record.direction != Direction::Download
            || record.remote_partial.is_some()
        {
            return Err(SftpError::PathInvalid);
        }
    }
    if let Some(path) = &record.remote_partial {
        if path.as_str().len() > crate::sftp::MAX_REMOTE_PATH_BYTES
            || record.direction != Direction::Upload
            || record.local_partial.is_some()
            || remote_parent(path) != remote_parent(&record.remote_path)
            || path.as_str().rsplit('/').next() != Some(expected_partial.as_str())
        {
            return Err(SftpError::PathInvalid);
        }
    }
    if record.direction == Direction::Upload && record.remote_partial.is_none()
        || record.direction == Direction::Download && record.local_partial.is_none()
    {
        return Err(SftpError::PathInvalid);
    }
    let expected_remote = match &record.target {
        TransferTarget::Upload { parent, name } if record.direction == Direction::Upload => {
            Some(parent.join(&crate::sftp::RemoteName::from_bytes(name.as_bytes())?)?)
        }
        TransferTarget::UploadTree { parent, relative }
            if record.direction == Direction::Upload =>
        {
            Some(super::upload_tree_path(parent, relative)?)
        }
        TransferTarget::Download { .. } if record.direction == Direction::Download => {
            Some(record.remote_path.clone())
        }
        TransferTarget::DownloadTree { path, relative }
            if record.direction == Direction::Download =>
        {
            if !valid_tree_relative(relative) {
                return Err(SftpError::PathInvalid);
            }
            Some(path.clone())
        }
        _ => None,
    };
    if expected_remote.as_ref() != Some(&record.remote_path) {
        return Err(SftpError::PathInvalid);
    }
    Ok(())
}

fn valid_tree_relative(relative: &str) -> bool {
    let mut count = 0;
    for component in relative.split('/') {
        if crate::sftp::RemoteName::from_bytes(component.as_bytes()).is_err() {
            return false;
        }
        count += 1;
    }
    count >= 2
}

pub(crate) fn valid_cleanup_size(record: &RecoveryRecord, size: u64) -> bool {
    size <= record.total
        && record
            .partial_size
            .is_none_or(|confirmed| size >= confirmed)
}

pub(crate) fn upload_cleanup_baseline(record: &RecoveryRecord) -> Result<&Metadata, SftpError> {
    if record.direction != Direction::Upload || record.mode != RecoveryMode::Cleanup {
        return Err(SftpError::ResumeMismatch);
    }
    record
        .remote_cleanup_baseline
        .as_ref()
        .ok_or(SftpError::ResumeMismatch)
}

fn remote_parent(path: &RemotePath) -> Option<&str> {
    path.as_str()
        .rsplit_once('/')
        .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(all(test, windows))]
fn slot_path(directory: &Path, slot: u8) -> PathBuf {
    directory.join(format!("transfer-recovery.{slot}.json"))
}

fn load_slots(storage: &mut Storage) -> (Loaded, bool) {
    let mut newest: Option<Loaded> = None;
    let mut incomplete = false;
    let mut legacy = false;
    for slot in 0..=1 {
        match storage.read(slot) {
            Ok(Some(frame)) => match decode(&frame.bytes) {
                Ok(mut record) => {
                    if record.records.len() > MAX_RECORDS
                        || record
                            .records
                            .iter()
                            .any(|item| validate_record(item).is_err())
                    {
                        incomplete = true;
                        continue;
                    }
                    if frame.legacy {
                        legacy = true;
                        for item in &mut record.records {
                            item.policy_sha256 = None;
                        }
                    }
                    if newest
                        .as_ref()
                        .is_none_or(|current| record.sequence > current.sequence)
                    {
                        newest = Some(Loaded {
                            sequence: record.sequence,
                            records: record.records,
                            incomplete: record.incomplete,
                        });
                    }
                }
                Err(_) => incomplete = true,
            },
            Ok(None) => {}
            Err(_) => incomplete = true,
        }
    }
    let mut loaded = newest.unwrap_or(Loaded {
        sequence: 0,
        records: Vec::new(),
        incomplete,
    });
    loaded.incomplete |= incomplete;
    (loaded, legacy)
}

fn encode(record: &RecordFile) -> io::Result<Zeroizing<Vec<u8>>> {
    let body = Zeroizing::new(serde_json::to_vec(record).map_err(io::Error::other)?);
    if body.len() + 32 > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery frame too large",
        ));
    }
    let mut digest = Sha256::new();
    digest.update(&body);
    let mut frame = Zeroizing::new(digest.finalize().to_vec());
    frame.extend_from_slice(&body);
    Ok(frame)
}

fn decode(frame: &[u8]) -> io::Result<RecordFile> {
    if frame.len() < 32 || frame.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid recovery frame",
        ));
    }
    let mut digest = Sha256::new();
    digest.update(&frame[32..]);
    if digest.finalize().as_slice() != &frame[..32] {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery checksum",
        ));
    }
    let record: RecordFile = serde_json::from_slice(&frame[32..])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "recovery json"))?;
    if record.version != VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery version",
        ));
    }
    Ok(record)
}

fn persist(storage: &mut Storage, state: &Loaded) -> Result<(), SftpError> {
    let record = RecordFile {
        version: VERSION,
        sequence: state.sequence,
        records: state.records.clone(),
        incomplete: state.incomplete,
    };
    let frame = encode(&record).map_err(|_| SftpError::LocalIo)?;
    storage
        .write((state.sequence & 1) as usize, &frame)
        .map_err(|_| SftpError::LocalIo)
}

pub(crate) fn make_record(draft: RecoveryDraft) -> Result<RecoveryRecord, SftpError> {
    let record = RecoveryRecord {
        id: draft.id,
        host_id: draft.host_id,
        name: draft.name,
        remote_path: draft.remote_path,
        target: draft.target,
        direction: draft.direction,
        total: draft.total,
        partial_size: draft.partial_size,
        remote_partial: draft.remote_partial,
        local_partial: draft.local_partial,
        local_target: draft.local_target,
        local_identity: draft.local_identity,
        source_baseline: draft.source_baseline,
        policy_sha256: draft.policy_sha256,
        mode: RecoveryMode::Resume,
        remote_cleanup_baseline: None,
        updated_seconds: now_seconds(),
    };
    validate_record(&record)?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn failed_inspection_checkpoint_cannot_restore_remote_cleanup_authority() {
        let root = tempfile::tempdir().unwrap();
        let store = RecoveryStore::open(root.path().to_owned()).unwrap();
        let id = Uuid::new_v4();
        let mut original = record(id, false);
        original.policy_sha256 = Some("a".repeat(64));
        store.upsert(original).unwrap();
        let mut restricted = store.record(id).unwrap();
        restricted.mode = RecoveryMode::Inspect;
        // Exhaustion is a deterministic prewrite failure: the preceding
        // resumable frame remains valid on disk, just as with failed encoding
        // or encryption before Storage::write.
        store.state.lock().unwrap().sequence = u64::MAX;
        assert_eq!(store.upsert(restricted), Err(SftpError::ResourceLimit));
        store.restrict_to_inspection(id);
        assert_eq!(store.record(id).unwrap().mode, RecoveryMode::Inspect);
        drop(store);
        let restarted = RecoveryStore::open(root.path().to_owned()).unwrap();
        let stale = restarted.record(id).unwrap();
        assert_eq!(stale.mode, RecoveryMode::Resume);
        assert_eq!(
            upload_cleanup_baseline(&stale),
            Err(SftpError::ResumeMismatch)
        );
    }

    fn record(id: Uuid, local: bool) -> RecoveryRecord {
        let target = if local {
            TransferTarget::Download {
                entry: crate::sftp::RemoteEntryId::new(),
            }
        } else {
            TransferTarget::Upload {
                parent: RemotePath::parse("/drop").unwrap(),
                name: "report.bin".into(),
            }
        };
        RecoveryRecord {
            id,
            host_id: Uuid::from_u128(2),
            name: "report.bin".into(),
            remote_path: RemotePath::parse("/drop/report.bin").unwrap(),
            target,
            direction: if local {
                Direction::Download
            } else {
                Direction::Upload
            },
            total: 10,
            partial_size: Some(4),
            remote_partial: (!local)
                .then(|| RemotePath::parse(&format!("/drop/.capstan-{id}.partial")).unwrap()),
            local_partial: local
                .then(|| std::env::temp_dir().join(format!("capstan-{id}.partial"))),
            local_target: local.then(|| std::env::temp_dir().join(format!("capstan-{id}.bin"))),
            local_identity: None,
            source_baseline: None,
            policy_sha256: None,
            mode: RecoveryMode::Resume,
            remote_cleanup_baseline: None,
            updated_seconds: 1,
        }
    }

    #[test]
    #[cfg(windows)]
    fn slots_recover_newest_valid_frame_and_mark_corruption() {
        let root = tempfile::tempdir().unwrap();
        let store = RecoveryStore::open(root.path().to_owned()).unwrap();
        let id = Uuid::from_u128(1);
        store.upsert(record(id, false)).unwrap();
        store.upsert(record(id, false)).unwrap();
        let snapshot = store.snapshot().unwrap();
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].state, RecoveryState::ManualInspection);

        drop(store);
        let mut bytes = fs::read(slot_path(root.path(), 0)).unwrap();
        bytes[0] ^= 0xff;
        fs::write(slot_path(root.path(), 0), bytes).unwrap();
        let reopened = RecoveryStore::open(root.path().to_owned()).unwrap();
        assert_eq!(reopened.snapshot().unwrap().entries.len(), 1);
        assert!(reopened.snapshot().unwrap().incomplete);
    }

    #[test]
    #[cfg(windows)]
    fn plaintext_legacy_slots_migrate_without_promoting_their_authorization() {
        let root = tempfile::tempdir().unwrap();
        let mut item = record(Uuid::from_u128(16), false);
        item.policy_sha256 = Some("a".repeat(64));
        let legacy = RecordFile {
            version: VERSION,
            sequence: 10,
            records: vec![item.clone()],
            incomplete: false,
        };
        fs::write(
            slot_path(root.path(), 0),
            encode(&legacy).unwrap().as_slice(),
        )
        .unwrap();
        let store = RecoveryStore::open(root.path().to_owned()).unwrap();
        assert_eq!(
            store.snapshot().unwrap().entries[0].state,
            RecoveryState::ManualInspection
        );
        assert!(store.record(item.id).unwrap().policy_sha256.is_none());
        drop(store);
        for slot in 0..=1 {
            let bytes = fs::read(slot_path(root.path(), slot)).unwrap();
            assert!(bytes.starts_with(b"CAPSTAN_TRANSFER_DPAPI_V2\0"));
            assert!(!bytes
                .windows(item.remote_path.as_str().len())
                .any(|part| part == item.remote_path.as_str().as_bytes()));
        }
        let store = RecoveryStore::open(root.path().to_owned()).unwrap();
        assert_eq!(store.snapshot().unwrap().entries.len(), 1);
        assert_eq!(
            store.snapshot().unwrap().entries[0].state,
            RecoveryState::ManualInspection
        );
        assert!(!store.snapshot().unwrap().incomplete);
        let service = super::super::TransferService::default();
        *service.recovery.lock().unwrap() = Some(std::sync::Arc::new(store));
        assert_eq!(
            service.recovery_target("main", item.id),
            Err(SftpError::ResumeMismatch)
        );
        assert_eq!(service.recovery_snapshot().unwrap().entries.len(), 1);
    }

    #[test]
    #[cfg(windows)]
    fn encrypted_recovery_survives_process_exit_without_destructors() {
        const VARIABLE: &str = "CAPSTAN_TEST_TRANSFER_LEDGER_ROOT";
        let id = Uuid::from_u128(18);
        let mut item = record(id, false);
        item.policy_sha256 = Some("b".repeat(64));
        if let Some(root) = std::env::var_os(VARIABLE) {
            let store = RecoveryStore::open(PathBuf::from(root)).unwrap();
            store.upsert(item).unwrap();
            // The OS must release held files even when Rust drops never run.
            std::process::exit(74);
        }
        let root = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "transfer::recovery::tests::encrypted_recovery_survives_process_exit_without_destructors",
            ])
            .env(VARIABLE, root.path())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(74), "{output:?}");
        let ciphertext = fs::read(slot_path(root.path(), 1)).unwrap();
        assert!(ciphertext.starts_with(b"CAPSTAN_TRANSFER_DPAPI_V2\0"));
        assert!(!ciphertext
            .windows(item.remote_path.as_str().len())
            .any(|part| part == item.remote_path.as_str().as_bytes()));
        let store = RecoveryStore::open(root.path().to_owned()).unwrap();
        assert_eq!(store.record(id).unwrap(), item);
        assert_eq!(
            store.snapshot().unwrap().entries[0].state,
            RecoveryState::AwaitingReconnect
        );
        assert!(!store.snapshot().unwrap().incomplete);
        // Reopening only discovers the descriptor; no connection or picker
        // grant is restored by decryption.
        let service = super::super::TransferService::default();
        *service.recovery.lock().unwrap() = Some(std::sync::Arc::new(store));
        assert!(service.snapshot("main").unwrap().is_empty());
        assert_eq!(
            service.recovery_target("other", id),
            Err(SftpError::ScopeExpired)
        );
        assert!(service.recovery_target("main", id).is_ok());
    }

    #[test]
    #[cfg(windows)]
    fn unreadable_slots_preserve_local_partials_and_do_not_invent_authorization() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("ledger");
        let id = Uuid::from_u128(19);
        let partial = root.path().join(format!(".capstan-{id}.partial"));
        let original = b"retained local content";
        fs::write(&partial, original).unwrap();
        let mut item = record(id, true);
        item.local_partial = Some(partial.clone());
        item.local_target = Some(root.path().join("download.bin"));
        item.total = original.len() as u64;
        item.partial_size = Some(item.total);
        let store = RecoveryStore::open(directory.clone()).unwrap();
        store.upsert(item.clone()).unwrap();
        store.upsert(item).unwrap();
        drop(store);
        for slot in 0..=1 {
            let path = slot_path(&directory, slot);
            let mut sealed = fs::read(&path).unwrap();
            *sealed.last_mut().unwrap() ^= 1;
            fs::write(path, sealed).unwrap();
        }
        let store = RecoveryStore::open(directory).unwrap();
        assert!(store.snapshot().unwrap().entries.is_empty());
        assert!(store.snapshot().unwrap().incomplete);
        assert_eq!(store.record(id).unwrap_err(), SftpError::ScopeExpired);
        assert_eq!(fs::read(&partial).unwrap(), original);
        assert!(!root.path().join("download.bin").exists());
        drop(store);
        assert_eq!(fs::read(partial).unwrap(), original);
    }

    #[test]
    #[cfg(windows)]
    fn encrypted_slots_reject_cross_directory_replay_and_preserve_incomplete_status() {
        let root = tempfile::tempdir().unwrap();
        let id = Uuid::from_u128(17);
        let store = RecoveryStore::open(root.path().to_owned()).unwrap();
        store.upsert(record(id, false)).unwrap();
        store.upsert(record(id, false)).unwrap();
        drop(store);
        let bytes = fs::read(slot_path(root.path(), 0)).unwrap();
        let other = tempfile::tempdir().unwrap();
        fs::write(slot_path(other.path(), 0), bytes).unwrap();
        let replay = RecoveryStore::open(other.path().to_owned()).unwrap();
        assert!(replay.snapshot().unwrap().entries.is_empty());
        assert!(replay.snapshot().unwrap().incomplete);
        drop(replay);
        fs::write(slot_path(root.path(), 0), b"torn slot").unwrap();
        let store = RecoveryStore::open(root.path().to_owned()).unwrap();
        assert!(store.snapshot().unwrap().incomplete);
        store.upsert(record(id, false)).unwrap();
        store.upsert(record(id, false)).unwrap();
        drop(store);
        let store = RecoveryStore::open(root.path().to_owned()).unwrap();
        assert_eq!(store.snapshot().unwrap().entries.len(), 1);
        assert!(store.snapshot().unwrap().incomplete);
    }

    #[test]
    #[cfg(windows)]
    fn local_candidates_are_available_only_when_the_owned_size_matches() {
        let root = tempfile::tempdir().unwrap();
        let id = Uuid::from_u128(4);
        let partial = root.path().join(format!(".capstan-{id}.partial"));
        fs::write(&partial, [0_u8; 4]).unwrap();
        let store = RecoveryStore::open(root.path().join("state")).unwrap();
        let mut item = record(id, true);
        item.local_partial = Some(partial.clone());
        item.local_target = Some(root.path().join("download.bin"));
        item.local_identity = Some(42);
        item.policy_sha256 = Some("a".repeat(64));
        item.source_baseline = Some(Metadata {
            kind: FileKind::File,
            size: Some(ExactU64(item.total)),
            modified_seconds: Some(ExactU64(1)),
            permissions: Some(0o100600),
        });
        store.upsert(item).unwrap();
        assert_eq!(
            store.snapshot().unwrap().entries[0].state,
            RecoveryState::Available
        );
        fs::write(partial, [0_u8; 3]).unwrap();
        assert_eq!(
            store.snapshot().unwrap().entries[0].state,
            RecoveryState::Changed
        );
    }

    #[test]
    #[cfg(windows)]
    fn unknown_files_are_never_reported_as_recovery_candidates() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(".capstan-unknown.partial"), [1_u8; 4]).unwrap();
        let store = RecoveryStore::open(root.path().join("state")).unwrap();
        assert!(store.snapshot().unwrap().entries.is_empty());
    }

    #[test]
    fn recovery_records_bind_partials_to_the_task_and_destination_parent() {
        let id = Uuid::from_u128(9);
        let mut remote = record(id, false);
        remote.remote_partial = Some(
            RemotePath::parse("/other/.capstan-00000000-0000-0000-0000-000000000009.partial")
                .unwrap(),
        );
        assert_eq!(validate_record(&remote), Err(SftpError::PathInvalid));

        let mut local = record(Uuid::from_u128(10), true);
        local.local_partial = Some(std::env::temp_dir().join("unrelated.partial"));
        assert_eq!(validate_record(&local), Err(SftpError::PathInvalid));
    }

    #[test]
    fn download_tree_recovery_records_bind_safe_relative_paths_to_the_remote_file() {
        let id = Uuid::from_u128(11);
        let mut item = record(id, true);
        item.local_partial = Some(std::env::temp_dir().join(format!(".capstan-{id}.partial")));
        item.local_target = Some(std::env::temp_dir().join(format!("{id}.bin")));
        item.remote_path = RemotePath::parse("/remote/tree/sub/file.bin").unwrap();
        item.target = TransferTarget::DownloadTree {
            path: item.remote_path.clone(),
            relative: "tree/sub/file.bin".into(),
        };
        assert_eq!(validate_record(&item), Ok(()));

        item.target = TransferTarget::DownloadTree {
            path: RemotePath::parse("/remote/other/sub/file.bin").unwrap(),
            relative: "tree/sub/file.bin".into(),
        };
        assert_eq!(validate_record(&item), Err(SftpError::PathInvalid));

        item.target = TransferTarget::DownloadTree {
            path: item.remote_path.clone(),
            relative: "tree/../file.bin".into(),
        };
        assert_eq!(validate_record(&item), Err(SftpError::PathInvalid));
    }

    #[test]
    #[cfg(windows)]
    fn download_source_baseline_roundtrips_and_legacy_records_remain_readable() {
        let mut item = record(Uuid::from_u128(13), true);
        item.local_partial =
            Some(std::env::temp_dir().join(format!(".capstan-{}.partial", item.id)));
        let baseline = Metadata {
            kind: FileKind::File,
            size: Some(ExactU64(item.total)),
            modified_seconds: Some(ExactU64(100)),
            permissions: Some(0o100600),
        };
        item.source_baseline = Some(baseline.clone());
        assert_eq!(validate_record(&item), Ok(()));
        let serialized = serde_json::to_value(&item).unwrap();
        let decoded: RecoveryRecord = serde_json::from_value(serialized.clone()).unwrap();
        assert_eq!(decoded.source_baseline, Some(baseline));
        let mut legacy = serialized;
        legacy.as_object_mut().unwrap().remove("sourceBaseline");
        let decoded: RecoveryRecord = serde_json::from_value(legacy).unwrap();
        assert_eq!(decoded.source_baseline, None);
        let directory = tempfile::tempdir().unwrap();
        let store = RecoveryStore::open(directory.path().join("transfer-recovery")).unwrap();
        store.upsert(decoded).unwrap();
        drop(store);
        let service = super::super::TransferService::default();
        service
            .configure_recovery_directory(directory.path().to_owned())
            .unwrap();
        assert!(matches!(
            service.recovered_download("main", item.id),
            Err(SftpError::ResumeMismatch)
        ));
        assert_eq!(service.recovery_snapshot().unwrap().entries.len(), 1);
        item.source_baseline.as_mut().unwrap().size = Some(ExactU64(item.total + 1));
        assert_eq!(validate_record(&item), Err(SftpError::PathInvalid));
    }

    #[test]
    fn missing_original_proof_is_visible_for_manual_inspection_without_new_authority() {
        let mut upload = record(Uuid::from_u128(14), false);
        assert_eq!(view(&upload).state, RecoveryState::ManualInspection);
        upload.policy_sha256 = Some("a".repeat(64));
        assert_eq!(view(&upload).state, RecoveryState::AwaitingReconnect);
        let root = tempfile::tempdir().unwrap();
        let mut download = record(Uuid::from_u128(15), true);
        let partial = root
            .path()
            .join(format!(".capstan-{}.partial", download.id));
        fs::write(&partial, [0_u8; 4]).unwrap();
        download.local_partial = Some(partial);
        download.local_target = Some(root.path().join("target.bin"));
        download.policy_sha256 = Some("a".repeat(64));
        download.local_identity = Some(42);
        let baseline = Metadata {
            kind: FileKind::File,
            size: Some(ExactU64(10)),
            modified_seconds: Some(ExactU64(1)),
            permissions: None,
        };
        assert_eq!(view(&download).state, RecoveryState::ManualInspection);
        download.source_baseline = Some(baseline);
        download.local_identity = None;
        assert_eq!(view(&download).state, RecoveryState::ManualInspection);
        download.local_identity = Some(42);
        assert_eq!(view(&download).state, RecoveryState::Available);
        download.policy_sha256 = None;
        let metadata = serde_json::to_value(view(&download)).unwrap();
        assert_eq!(metadata["state"], "manual_inspection");
        for private in [
            "policySha256",
            "sourceBaseline",
            "localIdentity",
            "localPartialPath",
            "localTarget",
        ] {
            assert!(metadata.get(private).is_none());
        }
    }

    #[test]
    fn cleanup_size_must_include_confirmed_bytes_and_stay_within_the_source_length() {
        let mut item = record(Uuid::from_u128(12), false);
        assert!(valid_cleanup_size(&item, 10));
        assert!(!valid_cleanup_size(&item, 11));
        item.partial_size = Some(4);
        assert!(valid_cleanup_size(&item, 4));
        assert!(!valid_cleanup_size(&item, 3));
        assert!(valid_cleanup_size(&item, 10));
        assert!(!valid_cleanup_size(&item, 11));
    }
}
