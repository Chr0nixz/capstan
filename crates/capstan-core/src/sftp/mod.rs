//! Framework-independent boundary types for the M2 file workflows.
//!
//! These types do not authorize an SSH connection or local filesystem access.
//! The runtime service must bind capabilities to an authenticated terminal.
mod commit;
pub mod cwd;
pub(crate) use commit::publish_remote_file;
mod path;
mod scope;
pub(crate) mod service;

pub use crate::transfer::{TransferContext, TransferTarget};
pub use path::{RemoteName, RemotePath};
pub use scope::{Binding, BrowseScope, ScopeRegistry};
pub use service::{
    AttachedSftp, Confirmation, DeleteEntryResult, DeleteOutcome, DeleteTreeReport,
    DeleteTreeSummary, Mutation, RemoteFileContext, RemoteTree, RemoteTreeFile, SftpService,
};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, str::FromStr};
use thiserror::Error;
use uuid::Uuid;

pub const MAX_REMOTE_PATH_BYTES: usize = 4096;
pub const MAX_REMOTE_NAME_BYTES: usize = 255;
pub const MAX_OSC_BYTES: usize = 8192;
pub const DIRECTORY_PAGE_ENTRIES: usize = 256;
pub const MAX_BROWSE_SCOPES: usize = 128;
pub const MAX_ENTRIES_PER_SCOPE: usize = 4096;
pub const REQUEST_BYTES: usize = 32 * 1024;
pub const REQUESTS_PER_FILE: usize = 64;
pub const TRANSFER_WORKERS: usize = 3;
pub const MAX_BUFFER_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_QUEUED_TRANSFERS: usize = 1024;
pub const MAX_FINAL_TRANSFERS: usize = 128;
pub const MAX_LOCAL_GRANTS: usize = 128;
pub const LOCAL_GRANT_LIFETIME_SECONDS: u64 = 300;
pub const EXIT_DRAIN_TIMEOUT_SECONDS: u64 = 30;
pub const MAX_CWD_ALIASES: usize = 8;
pub const MAX_DIRECTORY_DEPTH: usize = 64;
pub const REQUEST_TIMEOUT_SECONDS: u64 = 15;
pub const CANCEL_TIMEOUT_SECONDS: u64 = 5;
pub const PROGRESS_INTERVAL_MS: u64 = 100;

macro_rules! capability_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);
        impl $name {
            pub(crate) fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }
    };
}
capability_id!(SftpSessionId);
capability_id!(RemoteEntryId);

/// JSON decimal strings keep all SFTP offsets and sizes exact in JavaScript.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExactU64(pub u64);

impl Serialize for ExactU64 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ExactU64 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct DecimalVisitor;
        impl serde::de::Visitor<'_> for DecimalVisitor {
            type Value = ExactU64;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a canonical unsigned 64-bit decimal string")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                if value.is_empty()
                    || (value.len() > 1 && value.starts_with('0'))
                    || !value.bytes().all(|byte| byte.is_ascii_digit())
                {
                    return Err(E::custom("invalid unsigned decimal string"));
                }
                u64::from_str(value)
                    .map(ExactU64)
                    .map_err(|_| E::custom("unsigned decimal string exceeds u64"))
            }
        }
        deserializer.deserialize_str(DecimalVisitor)
    }
}

/// Codes are stable. Server-provided messages and paths never enter diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SftpError {
    #[error("sftp_unavailable")]
    SftpUnavailable,
    #[error("permission_denied")]
    PermissionDenied,
    #[error("path_invalid")]
    PathInvalid,
    #[error("not_found")]
    NotFound,
    #[error("unsupported_file_type")]
    UnsupportedFileType,
    #[error("target_conflict")]
    TargetConflict,
    #[error("resume_mismatch")]
    ResumeMismatch,
    #[error("local_io")]
    LocalIo,
    #[error("remote_io")]
    RemoteIo,
    #[error("editor_unavailable")]
    EditorUnavailable,
    #[error("connection_lost")]
    ConnectionLost,
    #[error("operation_canceled")]
    OperationCanceled,
    #[error("scope_expired")]
    ScopeExpired,
    #[error("resource_limit")]
    ResourceLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    File,
    Directory,
    Symlink,
    Other,
    Unknown,
}
impl FileKind {
    /// POSIX file types are a masked value, not independent bitflags.
    pub fn from_permissions(mode: Option<u32>) -> Self {
        match mode.map(|mode| mode & 0o170_000) {
            Some(0o100_000) => Self::File,
            Some(0o040_000) => Self::Directory,
            Some(0o120_000) => Self::Symlink,
            Some(0) | None => Self::Unknown,
            Some(_) => Self::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Metadata {
    pub kind: FileKind,
    pub size: Option<ExactU64>,
    pub modified_seconds: Option<ExactU64>,
    pub permissions: Option<u32>,
}
impl Metadata {
    /// Missing attributes cannot prove a target is still the confirmed version.
    pub fn matches_baseline(&self, baseline: &Self) -> bool {
        self.kind != FileKind::Unknown
            && self.size.is_some()
            && self.modified_seconds.is_some()
            && self == baseline
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DirectoryEntry {
    pub id: RemoteEntryId,
    pub name: String,
    pub metadata: Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DirectoryPage {
    pub session_id: SftpSessionId,
    pub request_revision: ExactU64,
    pub page_sequence: u32,
    pub path: RemotePath,
    pub entries: Vec<DirectoryEntry>,
    pub has_more: bool,
    pub omitted_entries: u32,
}

pub struct PageInput {
    pub request_revision: ExactU64,
    pub page_sequence: u32,
    pub entries: Vec<(RemoteName, Metadata)>,
    pub has_more: bool,
    pub omitted_entries: u32,
}

#[cfg(test)]
mod tests;
