//! Bounded remote editor state and text encoding rules.
//!
//! The module owns no UI and never serializes document bytes. The platform
//! adapter may use these types to stage a selected remote file in a private
//! temporary directory and launch a configured executable with one path
//! argument.
use capstan_ssh::sftp::{ClientError, SftpClient};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::sftp::{FileKind, Metadata, RemotePath};

#[cfg(windows)]
mod journal;
#[cfg(windows)]
mod service;
#[cfg(windows)]
pub use service::{RemoteEditSnapshot, RemoteEditView, RemoteEditorService};

mod publication;
mod temporary;
pub use publication::{apply_remote_bytes, apply_remote_bytes_tracked, PublicationBaseline};
pub use temporary::{RemoteTemporaryView, TemporaryLedger, TemporaryState, MAX_EDITOR_TEMPORARIES};

pub const MAX_EDITOR_FILE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_EDITOR_SESSIONS: usize = 8;
pub const EDITOR_POLL_MS: u64 = 500;
pub const STABLE_SAVE_INTERVAL_MS: u64 = 1000;

/// A bounded identity for local save observations.  It is only used to spot
/// rename-save editors that preserve size and timestamp; the full SHA-256 is
/// still checked before any remote publication.
pub fn content_identity(bytes: &[u8]) -> u128 {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    u128::from_be_bytes(digest[..16].try_into().unwrap_or([0; 16]))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewlineStyle {
    Lf,
    CrLf,
    Cr,
    Mixed,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditError {
    #[error("editor_file_too_large")]
    #[serde(rename = "editor_file_too_large")]
    FileTooLarge,
    #[error("editor_invalid_encoding")]
    #[serde(rename = "editor_invalid_encoding")]
    InvalidEncoding,
    #[error("editor_unsupported_content")]
    #[serde(rename = "editor_unsupported_content")]
    UnsupportedContent,
    #[error("editor_unavailable")]
    EditorUnavailable,
    #[error("editor_conflict")]
    #[serde(rename = "editor_conflict")]
    Conflict,
    #[error("editor_stale_save")]
    #[serde(rename = "editor_stale_save")]
    StaleSave,
    #[error("editor_residual")]
    #[serde(rename = "editor_residual")]
    Residual,
    #[error("resource_limit")]
    ResourceLimit,
    #[error("remote_io")]
    RemoteIo,
    #[error("connection_lost")]
    ConnectionLost,
    #[error("operation_canceled")]
    #[serde(rename = "operation_canceled")]
    Canceled,
    #[error("local_io")]
    LocalIo,
    #[error("scope_expired")]
    ScopeExpired,
    #[error("permission_denied")]
    PermissionDenied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    encoding: Encoding,
    newline: NewlineStyle,
    original: Vec<u8>,
    text: String,
}
impl Document {
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }
    pub fn newline(&self) -> NewlineStyle {
        self.newline
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn original(&self) -> &[u8] {
        &self.original
    }

    /// Encode an editor's UTF-8 text using the remote file's detected encoding.
    /// Newline characters supplied by the editor are retained byte-for-byte in
    /// that encoding; an unchanged document therefore round-trips exactly.
    pub fn encode(&self, text: &str) -> Result<Vec<u8>, EditError> {
        validate_text(text)?;
        let mut output = match self.encoding {
            Encoding::Utf8 | Encoding::Utf8Bom => text.as_bytes().to_vec(),
            Encoding::Utf16Le | Encoding::Utf16Be => {
                let mut bytes = Vec::with_capacity(text.len() * 2);
                for unit in text.encode_utf16() {
                    let encoded = match self.encoding {
                        Encoding::Utf16Le => unit.to_le_bytes(),
                        Encoding::Utf16Be => unit.to_be_bytes(),
                        _ => unreachable!(),
                    };
                    bytes.extend_from_slice(&encoded);
                }
                bytes
            }
        };
        if self.encoding == Encoding::Utf8Bom {
            output.splice(0..0, [0xEF, 0xBB, 0xBF]);
        } else if self.encoding == Encoding::Utf16Le {
            output.splice(0..0, [0xFF, 0xFE]);
        } else if self.encoding == Encoding::Utf16Be {
            output.splice(0..0, [0xFE, 0xFF]);
        }
        if output.len() > MAX_EDITOR_FILE_BYTES {
            return Err(EditError::FileTooLarge);
        }
        Ok(output)
    }
}

pub fn decode(bytes: &[u8]) -> Result<Document, EditError> {
    if bytes.len() > MAX_EDITOR_FILE_BYTES {
        return Err(EditError::FileTooLarge);
    }
    let (encoding, text) = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (
            Encoding::Utf8Bom,
            std::str::from_utf8(&bytes[3..])
                .map_err(|_| EditError::InvalidEncoding)?
                .to_owned(),
        )
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        (Encoding::Utf16Le, decode_utf16(&bytes[2..], true)?)
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        (Encoding::Utf16Be, decode_utf16(&bytes[2..], false)?)
    } else {
        (
            Encoding::Utf8,
            std::str::from_utf8(bytes)
                .map_err(|_| EditError::InvalidEncoding)?
                .to_owned(),
        )
    };
    validate_text(&text)?;
    Ok(Document {
        encoding,
        newline: newline_style(&text),
        original: bytes.to_vec(),
        text,
    })
}

/// Read a remote regular file through the already authenticated SFTP client.
/// The metadata is checked before and after the bounded body read so an
/// editor never stages a file that changed during opening.
pub async fn read_remote_file(
    client: &Arc<SftpClient>,
    path: &RemotePath,
    expected: Option<&Metadata>,
    cancel: &CancellationToken,
) -> Result<(Vec<u8>, Metadata), EditError> {
    let (bytes, metadata) = read_remote_bytes(client, path, expected, cancel).await?;
    let _ = decode(&bytes)?;
    Ok((bytes, metadata))
}

async fn read_remote_bytes(
    client: &Arc<SftpClient>,
    path: &RemotePath,
    expected: Option<&Metadata>,
    cancel: &CancellationToken,
) -> Result<(Vec<u8>, Metadata), EditError> {
    let before = client
        .lstat(path.as_str(), cancel)
        .await
        .map_err(map_client_error)?;
    let before_metadata = metadata(before.clone())?;
    if let Some(expected) = expected {
        if !before_metadata.matches_baseline(expected) {
            return Err(EditError::Conflict);
        }
    }
    if before_metadata.kind != FileKind::File {
        return Err(EditError::UnsupportedContent);
    }
    let size = before_metadata.size.ok_or(EditError::UnsupportedContent)?.0;
    if size > MAX_EDITOR_FILE_BYTES as u64 {
        return Err(EditError::FileTooLarge);
    }
    let file = client
        .open_file(path.as_str(), false, cancel)
        .await
        .map_err(map_client_error)?;
    let mut output = Vec::with_capacity(size as usize);
    let mut offset = 0_u64;
    while offset < size {
        let length = (size - offset).min(crate::sftp::REQUEST_BYTES as u64) as usize;
        let chunk = file
            .read(offset, length, cancel)
            .await
            .map_err(map_client_error)?;
        if chunk.is_empty() {
            let _ = file.close(&CancellationToken::new()).await;
            return Err(EditError::RemoteIo);
        }
        output.extend_from_slice(&chunk);
        offset += chunk.len() as u64;
    }
    file.close(cancel).await.map_err(map_client_error)?;
    let after = client
        .lstat(path.as_str(), cancel)
        .await
        .map_err(map_client_error)?;
    let after_metadata = metadata(after)?;
    if after_metadata != before_metadata {
        return Err(EditError::Conflict);
    }
    Ok((output, before_metadata))
}

async fn check_remote_baseline(
    client: &Arc<SftpClient>,
    path: &RemotePath,
    expected: Option<&Metadata>,
    expected_hash: Option<[u8; 32]>,
    cancel: &CancellationToken,
) -> Result<(), EditError> {
    check_canonical_parent(client, path, cancel).await?;
    if let Some(expected) = expected {
        // A concurrent writer may replace text with binary content. Compare
        // those raw bytes to the baseline instead of reporting a local codec
        // failure and hiding the conflict choices.
        let (bytes, _) = read_remote_bytes(client, path, Some(expected), cancel).await?;
        let hash: [u8; 32] = Sha256::digest(&bytes).into();
        if expected_hash.is_some_and(|expected| hash != expected) {
            return Err(EditError::Conflict);
        }
    } else {
        match client.lstat(path.as_str(), cancel).await {
            Err(ClientError::NotFound) => {}
            Ok(_) => return Err(EditError::Conflict),
            Err(error) => return Err(map_client_error(error)),
        }
    }
    Ok(())
}

async fn check_canonical_parent(
    client: &Arc<SftpClient>,
    path: &RemotePath,
    cancel: &CancellationToken,
) -> Result<(), EditError> {
    let parent = path
        .as_str()
        .rsplit_once('/')
        .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
        .ok_or(EditError::Conflict)?;
    if client
        .realpath(parent, cancel)
        .await
        .map_err(map_client_error)?
        != parent
    {
        return Err(EditError::Conflict);
    }
    Ok(())
}

fn metadata(attributes: capstan_ssh::sftp::Attributes) -> Result<Metadata, EditError> {
    Ok(Metadata {
        kind: FileKind::from_permissions(attributes.permissions),
        size: attributes.size.map(crate::sftp::ExactU64),
        modified_seconds: attributes
            .modified_seconds
            .map(|value| crate::sftp::ExactU64(u64::from(value))),
        permissions: attributes.permissions,
    })
}

fn map_client_error(error: ClientError) -> EditError {
    match error {
        ClientError::Canceled => EditError::Canceled,
        ClientError::ConnectionLost | ClientError::Unavailable => EditError::ConnectionLost,
        ClientError::PermissionDenied => EditError::PermissionDenied,
        ClientError::NotFound => EditError::Conflict,
        ClientError::Unsupported
        | ClientError::InvalidPath
        | ClientError::RemoteIo
        | ClientError::Uncertain
        | ClientError::ResourceLimit => EditError::RemoteIo,
    }
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String, EditError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(EditError::InvalidEncoding);
    }
    let units = bytes
        .chunks_exact(2)
        .map(|pair| {
            if little_endian {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        })
        .collect::<Vec<_>>();
    String::from_utf16(&units).map_err(|_| EditError::InvalidEncoding)
}

fn validate_text(text: &str) -> Result<(), EditError> {
    if text
        .chars()
        .any(|c| c == '\0' || (c.is_control() && !matches!(c, '\t' | '\r' | '\n')))
    {
        Err(EditError::UnsupportedContent)
    } else {
        Ok(())
    }
}
fn newline_style(text: &str) -> NewlineStyle {
    let crlf = text.match_indices("\r\n").count();
    let cr = text.matches('\r').count().saturating_sub(crlf);
    let lf = text.matches('\n').count().saturating_sub(crlf);
    match (crlf > 0, lf > 0, cr > 0) {
        (true, false, false) => NewlineStyle::CrLf,
        (false, true, false) => NewlineStyle::Lf,
        (false, false, true) => NewlineStyle::Cr,
        _ => NewlineStyle::Mixed,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorConfig {
    pub id: Uuid,
    pub executable: PathBuf,
}
impl EditorConfig {
    pub fn new(id: Uuid, executable: PathBuf) -> Result<Self, EditError> {
        if !executable.is_absolute()
            || executable.as_os_str().to_string_lossy().len() > 32767
            || executable
                .as_os_str()
                .to_string_lossy()
                .chars()
                .any(char::is_control)
        {
            return Err(EditError::EditorUnavailable);
        }
        Ok(Self { id, executable })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditState {
    Opening,
    WaitingForStableSave,
    ReadyToUpload,
    Conflict,
    Residual,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalFingerprint {
    pub identity: u128,
    pub size: u64,
    pub modified: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictDecision {
    KeepRemote,
    KeepLocalAsNew,
    ReplaceRemote,
}

#[derive(Debug, Clone)]
struct PendingSave {
    fingerprint: LocalFingerprint,
    first_seen: Instant,
}

/// Core-only edit lifecycle. Local paths and document bytes remain private to
/// Rust; the UI receives only the session ID and bounded state metadata.
#[derive(Debug, Clone)]
pub struct EditSession {
    pub id: Uuid,
    pub remote_path: String,
    pub state: EditState,
    baseline_hash: [u8; 32],
    baseline_metadata: String,
    pending: Option<PendingSave>,
    pub cleanup_required: bool,
}
impl EditSession {
    pub fn begin(
        id: Uuid,
        remote_path: String,
        bytes: &[u8],
        metadata: String,
    ) -> Result<Self, EditError> {
        let _ = decode(bytes)?;
        Ok(Self {
            id,
            remote_path,
            state: EditState::Opening,
            baseline_hash: Sha256::digest(bytes).into(),
            baseline_metadata: metadata,
            pending: None,
            cleanup_required: false,
        })
    }
    pub fn mark_editor_started(&mut self) {
        self.state = EditState::WaitingForStableSave;
    }
    pub fn baseline_hash(&self) -> [u8; 32] {
        self.baseline_hash
    }
    pub fn baseline_metadata(&self) -> &str {
        &self.baseline_metadata
    }
    pub fn is_dirty(&self, bytes: &[u8]) -> bool {
        let hash: [u8; 32] = Sha256::digest(bytes).into();
        hash != self.baseline_hash
    }
    pub fn invalidate_pending(&mut self) {
        self.pending = None;
        if self.state == EditState::ReadyToUpload {
            self.state = EditState::WaitingForStableSave;
        }
    }
    /// A file equal to the committed baseline must never trigger publication.
    pub fn observe_content(
        &mut self,
        fingerprint: LocalFingerprint,
        bytes: &[u8],
        now: Instant,
    ) -> Result<bool, EditError> {
        if !self.is_dirty(bytes) {
            self.invalidate_pending();
            return Ok(false);
        }
        self.observe_save(fingerprint, now)
    }
    pub fn is_stable_snapshot(&self, fingerprint: LocalFingerprint) -> bool {
        self.state == EditState::ReadyToUpload
            && self
                .pending
                .as_ref()
                .is_some_and(|save| save.fingerprint == fingerprint)
    }
    /// Advance the baseline after a successful remote publication.  The
    /// caller must have performed the remote metadata/hash check and commit
    /// before invoking this transition.
    pub fn mark_uploaded(&mut self, bytes: &[u8], metadata: String) -> Result<(), EditError> {
        let _ = decode(bytes)?;
        self.baseline_hash = Sha256::digest(bytes).into();
        self.baseline_metadata = metadata;
        self.pending = None;
        self.state = EditState::WaitingForStableSave;
        self.cleanup_required = false;
        Ok(())
    }
    pub fn observe_save(
        &mut self,
        fingerprint: LocalFingerprint,
        now: Instant,
    ) -> Result<bool, EditError> {
        if self.state != EditState::WaitingForStableSave && self.state != EditState::ReadyToUpload {
            return Err(EditError::StaleSave);
        }
        match &self.pending {
            Some(previous) if previous.fingerprint == fingerprint => {
                if now.saturating_duration_since(previous.first_seen)
                    >= Duration::from_millis(STABLE_SAVE_INTERVAL_MS)
                {
                    self.state = EditState::ReadyToUpload;
                    return Ok(true);
                }
            }
            _ => {
                self.state = EditState::WaitingForStableSave;
                self.pending = Some(PendingSave {
                    fingerprint,
                    first_seen: now,
                })
            }
        }
        Ok(false)
    }
    pub fn mark_conflict(&mut self) -> Result<(), EditError> {
        if self.state != EditState::ReadyToUpload {
            return Err(EditError::StaleSave);
        }
        self.state = EditState::Conflict;
        Ok(())
    }
    pub fn resolve_conflict(&mut self, decision: ConflictDecision) -> Result<(), EditError> {
        if self.state != EditState::Conflict {
            return Err(EditError::Conflict);
        }
        match decision {
            ConflictDecision::KeepRemote => self.state = EditState::Closed,
            ConflictDecision::KeepLocalAsNew | ConflictDecision::ReplaceRemote => {
                self.state = EditState::ReadyToUpload
            }
        }
        Ok(())
    }
    pub fn mark_residual(&mut self) {
        self.state = EditState::Residual;
        self.cleanup_required = true;
    }
    pub fn close(&mut self) {
        self.state = EditState::Closed;
    }
}

/// A bounded in-memory edit registry.  It is deliberately independent of a
/// desktop framework: the platform adapter owns the private staged path and
/// the SFTP command layer owns the authenticated remote lease.  This registry
/// only enforces the lifecycle limit and provides a single owner for each
/// session ID.
#[derive(Default)]
pub struct EditRegistry {
    sessions: std::sync::Mutex<std::collections::HashMap<Uuid, EditSession>>,
}
impl EditRegistry {
    pub fn insert(&self, session: EditSession) -> Result<(), EditError> {
        let mut sessions = self.sessions.lock().map_err(|_| EditError::Residual)?;
        if !sessions.contains_key(&session.id) && sessions.len() >= MAX_EDITOR_SESSIONS {
            return Err(EditError::ResourceLimit);
        }
        sessions.insert(session.id, session);
        Ok(())
    }
    pub fn with<R>(
        &self,
        id: Uuid,
        operation: impl FnOnce(&EditSession) -> Result<R, EditError>,
    ) -> Result<R, EditError> {
        let sessions = self.sessions.lock().map_err(|_| EditError::Residual)?;
        let session = sessions.get(&id).ok_or(EditError::Residual)?;
        operation(session)
    }
    pub fn with_mut<R>(
        &self,
        id: Uuid,
        operation: impl FnOnce(&mut EditSession) -> Result<R, EditError>,
    ) -> Result<R, EditError> {
        let mut sessions = self.sessions.lock().map_err(|_| EditError::Residual)?;
        let session = sessions.get_mut(&id).ok_or(EditError::Residual)?;
        operation(session)
    }
    pub fn remove(&self, id: Uuid) -> Result<EditSession, EditError> {
        self.sessions
            .lock()
            .map_err(|_| EditError::Residual)?
            .remove(&id)
            .ok_or(EditError::Residual)
    }
    pub fn len(&self) -> Result<usize, EditError> {
        Ok(self.sessions.lock().map_err(|_| EditError::Residual)?.len())
    }
    pub fn is_empty(&self) -> Result<bool, EditError> {
        Ok(self.len()? == 0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidualAction {
    RetryUpload,
    SaveLocalCopy,
    Discard,
    KeepForRecovery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryRecord {
    pub session: Uuid,
    pub remote_path: String,
    pub local_name: String,
    pub baseline_hash: String,
    pub created_at: u64,
}

#[derive(Default)]
pub struct RecoveryRegistry {
    records: std::sync::Mutex<Vec<RecoveryRecord>>,
}
impl RecoveryRegistry {
    pub fn insert(&self, record: RecoveryRecord) -> Result<(), EditError> {
        let mut records = self.records.lock().map_err(|_| EditError::Residual)?;
        records.retain(|item| item.session != record.session);
        if records.len() >= MAX_EDITOR_SESSIONS {
            return Err(EditError::ResourceLimit);
        }
        records.push(record);
        Ok(())
    }
    pub fn list(&self) -> Result<Vec<RecoveryRecord>, EditError> {
        Ok(self
            .records
            .lock()
            .map_err(|_| EditError::Residual)?
            .clone())
    }
    pub fn remove(&self, session: Uuid) -> Result<(), EditError> {
        let mut records = self.records.lock().map_err(|_| EditError::Residual)?;
        let before = records.len();
        records.retain(|item| item.session != session);
        if before == records.len() {
            return Err(EditError::Residual);
        }
        Ok(())
    }

    /// Load only the bounded metadata registry.  Missing files are treated as
    /// an empty registry; malformed or oversized files fail closed and never
    /// trigger automatic deletion of a staged editor directory.
    pub fn load(path: &std::path::Path) -> Result<Self, EditError> {
        use std::io::Read;
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(_) => return Err(EditError::Residual),
        };
        let mut bytes = Vec::new();
        file.take(64 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| EditError::Residual)?;
        if bytes.len() > 64 * 1024 {
            return Err(EditError::ResourceLimit);
        }
        let records: Vec<RecoveryRecord> =
            serde_json::from_slice(&bytes).map_err(|_| EditError::Residual)?;
        if records.len() > MAX_EDITOR_SESSIONS
            || records.iter().any(|record| {
                record.remote_path.len() > crate::sftp::MAX_REMOTE_PATH_BYTES
                    || !record.remote_path.starts_with('/')
                    || record.local_name.is_empty()
                    || record.local_name.len() > 255
                    || record.baseline_hash.len() != 64
                    || !record
                        .baseline_hash
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return Err(EditError::Residual);
        }
        let registry = Self::default();
        let mut stored = registry.records.lock().map_err(|_| EditError::Residual)?;
        for record in records {
            if stored
                .iter()
                .any(|existing| existing.session == record.session)
            {
                return Err(EditError::Residual);
            }
            stored.push(record);
        }
        drop(stored);
        Ok(registry)
    }

    /// Publish the registry with a same-directory temporary.  The file is
    /// metadata only; staged document bytes and credentials never enter it.
    pub fn save(&self, path: &std::path::Path) -> Result<(), EditError> {
        let records = self.list()?;
        let bytes = serde_json::to_vec(&records).map_err(|_| EditError::Residual)?;
        if bytes.len() > 64 * 1024 {
            return Err(EditError::ResourceLimit);
        }
        let parent = path.parent().ok_or(EditError::Residual)?;
        std::fs::create_dir_all(parent).map_err(|_| EditError::Residual)?;
        let temporary = parent.join(format!(".{}.partial", Uuid::new_v4()));
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|_| EditError::Residual)?;
            file.write_all(&bytes).map_err(|_| EditError::Residual)?;
            file.sync_all().map_err(|_| EditError::Residual)?;
        }
        #[cfg(windows)]
        let publication = capstan_native_windows::atomic_replace(&temporary, path, true);
        #[cfg(not(windows))]
        let publication = std::fs::rename(&temporary, path);
        if publication.is_err() {
            let _ = std::fs::remove_file(&temporary);
            return Err(EditError::Residual);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supported_encodings_round_trip_and_preserve_bom() {
        for bytes in [
            b"a\r\nb\n".to_vec(),
            vec![0xEF, 0xBB, 0xBF, b'a', b'\n'],
            vec![0xFF, 0xFE, b'a', 0, b'\r', 0],
            vec![0xFE, 0xFF, 0, b'a', 0, b'\n'],
        ] {
            let document = decode(&bytes).unwrap();
            assert_eq!(document.encode(document.text()).unwrap(), bytes);
        }
    }
    #[test]
    fn editor_error_metadata_uses_the_same_stable_codes_as_commands() {
        for error in [
            EditError::FileTooLarge,
            EditError::InvalidEncoding,
            EditError::UnsupportedContent,
            EditError::EditorUnavailable,
            EditError::Conflict,
            EditError::StaleSave,
            EditError::Residual,
            EditError::ResourceLimit,
            EditError::RemoteIo,
            EditError::ConnectionLost,
            EditError::Canceled,
            EditError::LocalIo,
            EditError::ScopeExpired,
            EditError::PermissionDenied,
        ] {
            assert_eq!(
                serde_json::to_value(error).unwrap(),
                serde_json::Value::String(error.to_string())
            );
        }
    }
    #[test]
    fn malformed_and_unsafe_editor_content_is_rejected() {
        assert_eq!(
            decode(&[0xFF, 0xFE, 0x00]).unwrap_err(),
            EditError::InvalidEncoding
        );
        assert_eq!(
            decode(b"bad\0text").unwrap_err(),
            EditError::UnsupportedContent
        );
        assert_eq!(
            decode(&vec![b'x'; MAX_EDITOR_FILE_BYTES + 1]).unwrap_err(),
            EditError::FileTooLarge
        );
        assert!(EditorConfig::new(Uuid::new_v4(), PathBuf::from("relative.exe")).is_err());
    }
    #[test]
    fn stable_save_and_conflict_resolution_require_explicit_transitions() {
        let now = Instant::now();
        let mut session = EditSession::begin(
            Uuid::new_v4(),
            "/etc/app.conf".into(),
            b"one\n",
            "baseline".into(),
        )
        .unwrap();
        session.mark_editor_started();
        let fp = LocalFingerprint {
            identity: 7,
            size: 4,
            modified: 9,
        };
        assert!(!session.observe_save(fp, now).unwrap());
        assert!(session
            .observe_save(fp, now + Duration::from_millis(STABLE_SAVE_INTERVAL_MS))
            .unwrap());
        session.mark_conflict().unwrap();
        session
            .resolve_conflict(ConflictDecision::KeepRemote)
            .unwrap();
        assert_eq!(session.state, EditState::Closed);
    }
    #[test]
    fn recovery_records_are_bounded_and_single_session() {
        let registry = RecoveryRegistry::default();
        let session = Uuid::new_v4();
        registry
            .insert(RecoveryRecord {
                session,
                remote_path: "/etc/app".into(),
                local_name: "app".into(),
                baseline_hash: "00".into(),
                created_at: 1,
            })
            .unwrap();
        registry
            .insert(RecoveryRecord {
                session,
                remote_path: "/etc/app".into(),
                local_name: "app-2".into(),
                baseline_hash: "11".into(),
                created_at: 2,
            })
            .unwrap();
        assert_eq!(registry.list().unwrap().len(), 1);
        registry.remove(session).unwrap();
        assert!(registry.remove(session).is_err());
    }

    #[test]
    fn uploaded_baseline_resets_pending_save_without_reopening_the_session() {
        let now = Instant::now();
        let id = Uuid::new_v4();
        let mut session =
            EditSession::begin(id, "/etc/app.conf".into(), b"one\n", "v1".into()).unwrap();
        session.mark_editor_started();
        let first = LocalFingerprint {
            identity: 1,
            size: 4,
            modified: 1,
        };
        assert!(!session.observe_save(first, now).unwrap());
        assert!(session
            .observe_save(first, now + Duration::from_millis(STABLE_SAVE_INTERVAL_MS))
            .unwrap());
        session.mark_uploaded(b"two\n", "v2".into()).unwrap();
        assert_eq!(session.state, EditState::WaitingForStableSave);
        assert_eq!(session.baseline_metadata(), "v2");
        let expected: [u8; 32] = Sha256::digest(b"two\n").into();
        assert_eq!(session.baseline_hash(), expected);
    }

    #[test]
    fn unchanged_bytes_never_upload_and_a_new_save_restarts_the_stability_interval() {
        let now = Instant::now();
        let mut session =
            EditSession::begin(Uuid::new_v4(), "/a".into(), b"original", "v1".into()).unwrap();
        session.mark_editor_started();
        let fingerprint = LocalFingerprint {
            identity: 1,
            size: 8,
            modified: 1,
        };
        for offset in [0, 1000, 2000, 3000] {
            assert!(!session
                .observe_content(
                    fingerprint,
                    b"original",
                    now + Duration::from_millis(offset)
                )
                .unwrap());
        }
        assert!(!session
            .observe_content(fingerprint, b"changed!", now)
            .unwrap());
        assert!(session
            .observe_content(fingerprint, b"changed!", now + Duration::from_millis(1000))
            .unwrap());
        let newer = LocalFingerprint {
            identity: 2,
            ..fingerprint
        };
        assert!(!session
            .observe_content(newer, b"latest!!", now + Duration::from_millis(1100))
            .unwrap());
        assert!(!session.is_stable_snapshot(fingerprint));
        assert_eq!(session.state, EditState::WaitingForStableSave);
        session.invalidate_pending();
        assert!(!session
            .observe_content(newer, b"latest!!", now + Duration::from_millis(2200))
            .unwrap());
        assert!(session
            .observe_content(newer, b"latest!!", now + Duration::from_millis(3200))
            .unwrap());
        assert!(session.is_stable_snapshot(newer));
    }

    #[test]
    fn edit_registry_replaces_only_the_same_id_and_enforces_capacity() {
        let registry = EditRegistry::default();
        let id = Uuid::new_v4();
        let session = EditSession::begin(id, "/a".into(), b"a", "a".into()).unwrap();
        registry.insert(session).unwrap();
        registry
            .with_mut(id, |session| {
                session.mark_editor_started();
                Ok(())
            })
            .unwrap();
        assert_eq!(registry.len().unwrap(), 1);
        assert_eq!(
            registry.with(id, |session| Ok(session.state)).unwrap(),
            EditState::WaitingForStableSave
        );
        registry.remove(id).unwrap();
        assert!(registry.with(id, |session| Ok(session.state)).is_err());
    }

    #[test]
    fn recovery_registry_round_trips_bounded_metadata_and_rejects_tampering() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("editor-recovery.json");
        let registry = RecoveryRegistry::default();
        registry
            .insert(RecoveryRecord {
                session: Uuid::new_v4(),
                remote_path: "/etc/app.conf".into(),
                local_name: "app.conf".into(),
                baseline_hash: "A".repeat(64),
                created_at: 7,
            })
            .unwrap();
        registry.save(&path).unwrap();
        assert_eq!(
            RecoveryRegistry::load(&path).unwrap().list().unwrap().len(),
            1
        );
        std::fs::write(&path, br#"[{"remotePath":"relative","localName":"x","baselineHash":"00","createdAt":1,"session":"00000000-0000-4000-8000-000000000000"}]"#).unwrap();
        assert!(RecoveryRegistry::load(&path).is_err());
    }
}
