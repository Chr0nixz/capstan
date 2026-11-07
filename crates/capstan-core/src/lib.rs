//! Application workflows and capabilities, independent of Tauri.
#![deny(unsafe_code)]
#![allow(clippy::pedantic)]

#[cfg(feature = "benchmarks")]
pub mod benchmark;

pub mod application;
pub mod desktop;
pub mod forwarding;
pub mod hotkey;
pub mod launch;
pub mod remote_edit;
pub mod server_status;
pub mod settings;
pub mod sftp;
pub mod shell_integration;
pub mod themes;
pub mod transfer;
pub mod update;

use capstan_ssh::{
    connection_state::{ConnectionErrorCode, ConnectionSnapshot, ConnectionState},
    ConnectionLease, PasswordConnection, PoolKey,
};
use capstan_vault::{
    AgentKind, Credential, CredentialMaterial, Group, Host, KdfParameters, Preferences,
    RecoveryCandidate as VaultRecoveryCandidate, RecoveryKind, SecretText, Snippet, SnippetGroup,
    TrustRecord, Vault, VaultData, VaultError,
};
use secrecy::SecretString;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::task;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use uuid::Uuid;
use zeroize::Zeroize;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error(transparent)]
    Hello(#[from] capstan_vault::hello::HelloError),
    #[error("vault is locked")]
    Locked,
    #[error("vault entity was not found")]
    NotFound,
    #[error("vault data changed")]
    Conflict,
    #[error("group still contains referenced items")]
    GroupInUse,
    #[error("credential is still referenced by a host")]
    CredentialInUse,
    #[error("host key changed")]
    HostKeyChanged,
    #[error("backup trust conflicts require explicit host-key management")]
    TrustConflict,
    #[error("referenced SSH private-key file is unavailable or invalid")]
    CredentialFile,
    #[error("vault authentication failed")]
    VaultAuthentication,
    #[error("vault format is invalid")]
    VaultFormat,
    #[error("vault version is unsupported")]
    VaultVersion,
    #[error("vault KDF parameters exceed policy")]
    VaultKdfPolicy,
    #[error("vault data is invalid")]
    VaultData,
    #[error("secure random generation failed")]
    SecureRandom,
    #[error("vault storage failed")]
    Storage,
    #[error("operation was canceled")]
    Canceled,
    #[error("vault recovery selection expired")]
    RecoveryExpired,
    #[error("too many vault recovery candidates")]
    RecoveryLimit,
    #[error("invalid input")]
    InvalidInput,
    #[error("task failed")]
    TaskFailed,
    #[error(transparent)]
    Ssh(#[from] capstan_ssh::SshError),
    #[error("SSH forwarding failed: {0:?}")]
    Tunnel(capstan_ssh::forwarding::TunnelErrorCode),
}
impl CoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Hello(error) => error.code(),
            Self::Locked => "vault_locked",
            Self::NotFound => "not_found",
            Self::Conflict => "vault_conflict",
            Self::GroupInUse => "group_in_use",
            Self::CredentialInUse => "credential_in_use",
            Self::HostKeyChanged => "host_key_changed",
            Self::TrustConflict => "trust_conflict",
            Self::CredentialFile => "credential_file",
            Self::VaultAuthentication => "vault_authentication",
            Self::VaultFormat => "vault_format",
            Self::VaultVersion => "vault_version",
            Self::VaultKdfPolicy => "vault_kdf_policy",
            Self::VaultData => "vault_data",
            Self::SecureRandom => "secure_random",
            Self::Storage => "vault_io",
            Self::Canceled => "canceled",
            Self::RecoveryExpired => "vault_recovery_expired",
            Self::RecoveryLimit => "vault_recovery_limit",
            Self::InvalidInput => "invalid_input",
            Self::TaskFailed => "task_failed",
            Self::Ssh(error) => error.code(),
            Self::Tunnel(error) => error.code(),
        }
    }
}
impl From<VaultError> for CoreError {
    fn from(error: VaultError) -> Self {
        match error {
            VaultError::Hello(error) => Self::Hello(error),
            VaultError::Locked => Self::Locked,
            VaultError::NotFound => Self::NotFound,
            VaultError::Conflict | VaultError::AlreadyExists => Self::Conflict,
            VaultError::InvalidData => Self::InvalidInput,
            VaultError::RecoveryLimit => Self::RecoveryLimit,
            VaultError::InvalidFormat => Self::VaultFormat,
            VaultError::KdfPolicy => Self::VaultKdfPolicy,
            VaultError::UnsupportedVersion => Self::VaultVersion,
            VaultError::Authentication => Self::VaultAuthentication,
            VaultError::Canceled => Self::Canceled,
            VaultError::Random => Self::SecureRandom,
            VaultError::TrustConflict => Self::TrustConflict,
            _ => Self::Storage,
        }
    }
}
pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CredentialUpdate {
    pub id: Uuid,
    pub expected_revision: u64,
}

fn read_private_key_file(path: &std::path::Path) -> Result<SecretText> {
    let file = std::fs::File::open(path).map_err(|_| CoreError::CredentialFile)?;
    let metadata = file.metadata().map_err(|_| CoreError::CredentialFile)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 1024 * 1024 {
        return Err(CoreError::CredentialFile);
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(metadata.len() as usize));
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CoreError::CredentialFile)?;
    if bytes.is_empty() || bytes.len() > 1024 * 1024 {
        return Err(CoreError::CredentialFile);
    }
    match String::from_utf8(std::mem::take(&mut *bytes)) {
        Ok(content) => Ok(SecretText::new(content)),
        Err(error) => {
            let _bytes = zeroize::Zeroizing::new(error.into_bytes());
            Err(CoreError::CredentialFile)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HostView {
    pub id: Uuid,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub credential_id: Uuid,
    pub group_id: Option<Uuid>,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub revision: u64,
    pub working_directory: Option<String>,
    pub startup_command: Option<String>,
    pub encoding: String,
    pub theme: String,
    pub agent_forwarding: bool,
    pub production: bool,
}
impl From<&Host> for HostView {
    fn from(host: &Host) -> Self {
        Self {
            id: host.id,
            name: host.name.clone(),
            address: host.address.clone(),
            port: host.port,
            username: host.username.clone(),
            credential_id: host.credential_id,
            group_id: host.group_id,
            tags: host.tags.clone(),
            favorite: host.favorite,
            revision: host.revision,
            working_directory: host.working_directory.clone(),
            startup_command: host.startup_command.clone(),
            encoding: host.encoding.clone(),
            theme: host.theme.clone(),
            agent_forwarding: host.agent_forwarding,
            production: host.production,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CredentialView {
    pub id: Uuid,
    pub name: String,
    pub revision: u64,
    pub kind: String,
}
impl From<&Credential> for CredentialView {
    fn from(value: &Credential) -> Self {
        Self {
            id: value.id,
            name: value.name.clone(),
            revision: value.revision,
            kind: value.material.kind().to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupView {
    pub id: Uuid,
    pub name: String,
    pub parent_id: Option<Uuid>,
    pub production: bool,
}
impl From<&Group> for GroupView {
    fn from(group: &Group) -> Self {
        Self {
            id: group.id,
            name: group.name.clone(),
            parent_id: group.parent_id,
            production: group.production,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct VaultStatus {
    pub exists: bool,
    pub unlocked: bool,
    pub revision: Option<u64>,
    pub epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryKindView {
    Pending,
    Backup,
}

#[derive(Debug, Clone, Serialize)]
pub struct VaultRecoveryCandidateView {
    pub token: Uuid,
    pub kind: RecoveryKindView,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrustedHostKeyView {
    pub target: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
    pub first_trusted_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupImportMode {
    Merge,
    Overwrite,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupHostDuplicate {
    pub existing_name: String,
    pub incoming_name: String,
    pub target: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupTrustConflict {
    pub target: String,
    pub port: u16,
    pub existing_fingerprint: String,
    pub incoming_fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupPreview {
    pub base_revision: u64,
    pub host_count: usize,
    pub group_count: usize,
    pub credential_count: usize,
    pub trust_count: usize,
    pub snippet_group_count: usize,
    pub snippet_count: usize,
    pub duplicate_hosts: Vec<BackupHostDuplicate>,
    pub trust_conflicts: Vec<BackupTrustConflict>,
}

fn connection_pool_key(
    host: &Host,
    credential_revision: u64,
    vault_epoch: u64,
    fingerprint: String,
    agent_identity: Option<String>,
) -> capstan_ssh::PoolKey {
    capstan_ssh::PoolKey {
        target: capstan_vault::normalize_target(&host.address)
            .unwrap_or_else(|_| host.address.clone()),
        port: host.port,
        username: host.username.clone(),
        credential_id: host.credential_id,
        credential_revision,
        trust_fingerprint: fingerprint,
        vault_epoch,
        agent_identity,
        agent_forwarding: host.agent_forwarding,
        proxy_path: String::new(),
    }
}

fn same_host_target(left: &Host, right: &Host) -> bool {
    capstan_vault::normalize_target(&left.address)
        .ok()
        .zip(capstan_vault::normalize_target(&right.address).ok())
        .is_some_and(|(left_target, right_target)| {
            left_target == right_target
                && left.port == right.port
                && left.username == right.username
        })
}

fn build_backup_preview(current: &VaultData, imported: &VaultData) -> BackupPreview {
    let duplicate_hosts = imported
        .hosts
        .iter()
        .filter_map(|incoming| {
            current
                .hosts
                .iter()
                .find(|existing| same_host_target(existing, incoming))
                .and_then(|existing| {
                    capstan_vault::normalize_target(&incoming.address)
                        .ok()
                        .map(|target| BackupHostDuplicate {
                            existing_name: existing.name.clone(),
                            incoming_name: incoming.name.clone(),
                            target,
                            port: incoming.port,
                        })
                })
        })
        .collect();
    let trust_conflicts = imported
        .trust
        .iter()
        .filter_map(|incoming| {
            current
                .trust
                .iter()
                .find(|existing| {
                    existing.target == incoming.target && existing.port == incoming.port
                })
                .filter(|existing| {
                    existing.fingerprint != incoming.fingerprint
                        || existing.public_key != incoming.public_key
                        || existing.algorithm != incoming.algorithm
                })
                .map(|existing| BackupTrustConflict {
                    target: incoming.target.clone(),
                    port: incoming.port,
                    existing_fingerprint: existing.fingerprint.clone(),
                    incoming_fingerprint: incoming.fingerprint.clone(),
                })
        })
        .collect();
    BackupPreview {
        base_revision: current.revision,
        host_count: imported.hosts.len(),
        group_count: imported.groups.len(),
        credential_count: imported.credentials.len(),
        trust_count: imported.trust.len(),
        snippet_group_count: imported.snippet_groups.len(),
        snippet_count: imported.snippets.len(),
        duplicate_hosts,
        trust_conflicts,
    }
}

fn unique_import_name(name: &str, used: &mut HashSet<String>) -> String {
    let base = name.trim();
    if used.insert(base.to_lowercase()) {
        return base.to_owned();
    }
    for suffix_index in 2_u32.. {
        let suffix = format!(" (imported {suffix_index})");
        let max_base_bytes = 256_usize.saturating_sub(suffix.len());
        let mut shortened = String::new();
        for character in base.chars() {
            if shortened.len() + character.len_utf8() > max_base_bytes {
                break;
            }
            shortened.push(character);
        }
        let candidate = format!("{shortened}{suffix}");
        if used.insert(candidate.to_lowercase()) {
            return candidate;
        }
    }
    unreachable!("name suffix index exhausted")
}

fn merge_backup_data(current: &VaultData, imported: VaultData) -> Result<VaultData> {
    let mut merged = current.clone();
    let mut group_ids = HashMap::new();
    let mut credential_ids = HashMap::new();
    let mut snippet_group_ids = HashMap::new();
    let mut host_ids = HashMap::new();
    let mut group_names = merged
        .groups
        .iter()
        .map(|group| group.name.to_lowercase())
        .collect::<HashSet<_>>();
    let mut credential_names = merged
        .credentials
        .iter()
        .map(|credential| credential.name.to_lowercase())
        .collect::<HashSet<_>>();
    let mut host_names = merged
        .hosts
        .iter()
        .map(|host| host.name.to_lowercase())
        .collect::<HashSet<_>>();

    for group in &imported.groups {
        group_ids.insert(group.id, Uuid::new_v4());
    }
    for group in &imported.snippet_groups {
        snippet_group_ids.insert(group.id, Uuid::new_v4());
    }
    for mut group in imported.groups {
        group.id = *group_ids.get(&group.id).ok_or(CoreError::InvalidInput)?;
        group.parent_id = group
            .parent_id
            .map(|id| group_ids.get(&id).copied().ok_or(CoreError::InvalidInput))
            .transpose()?;
        group.name = unique_import_name(&group.name, &mut group_names);
        merged.groups.push(group);
    }

    // Importing the same backup again must not duplicate M3 data: snippet
    // groups merge by name, and tunnels and snippets skip exact copies.
    for mut group in imported.snippet_groups {
        let name = group.name.trim().to_lowercase();
        if let Some(existing) = merged
            .snippet_groups
            .iter()
            .find(|existing| existing.name.trim().to_lowercase() == name)
        {
            snippet_group_ids.insert(group.id, existing.id);
            continue;
        }
        group.id = *snippet_group_ids
            .get(&group.id)
            .ok_or(CoreError::InvalidInput)?;
        merged.snippet_groups.push(group);
    }

    for credential in &imported.credentials {
        credential_ids.insert(credential.id, Uuid::new_v4());
    }
    for mut credential in imported.credentials {
        credential.id = *credential_ids
            .get(&credential.id)
            .ok_or(CoreError::InvalidInput)?;
        credential.name = unique_import_name(&credential.name, &mut credential_names);
        merged.credentials.push(credential);
    }

    for mut host in imported.hosts {
        if let Some(existing) = merged
            .hosts
            .iter()
            .find(|existing| same_host_target(existing, &host))
        {
            host_ids.insert(host.id, existing.id);
            continue;
        }
        let previous_id = host.id;
        host.id = Uuid::new_v4();
        host_ids.insert(previous_id, host.id);
        host.credential_id = *credential_ids
            .get(&host.credential_id)
            .ok_or(CoreError::InvalidInput)?;
        host.group_id = host
            .group_id
            .map(|id| group_ids.get(&id).copied().ok_or(CoreError::InvalidInput))
            .transpose()?;
        host.name = unique_import_name(&host.name, &mut host_names);
        merged.hosts.push(host);
    }

    for mut tunnel in imported.tunnels {
        tunnel.host_id = *host_ids
            .get(&tunnel.host_id)
            .ok_or(CoreError::InvalidInput)?;
        // A second copy of a forward would only compete for the same port.
        if merged.tunnels.iter().any(|existing| {
            existing.host_id == tunnel.host_id && existing.forward == tunnel.forward
        }) {
            continue;
        }
        tunnel.id = Uuid::new_v4();
        merged.tunnels.push(tunnel);
    }
    for trust in imported.trust {
        if merged
            .trust
            .iter()
            .all(|existing| existing.target != trust.target || existing.port != trust.port)
        {
            merged.trust.push(trust);
        }
    }
    for mut snippet in imported.snippets {
        snippet.group_id = snippet
            .group_id
            .map(|id| {
                snippet_group_ids
                    .get(&id)
                    .copied()
                    .ok_or(CoreError::InvalidInput)
            })
            .transpose()?;
        // Only exact copies are skipped, so a differing snippet is never lost.
        if merged.snippets.iter().any(|existing| {
            existing.group_id == snippet.group_id
                && existing.name == snippet.name
                && existing.template == snippet.template
                && existing.auto_enter == snippet.auto_enter
        }) {
            continue;
        }
        snippet.id = Uuid::new_v4();
        merged.snippets.push(snippet);
    }
    merged.validate()?;
    Ok(merged)
}

fn startup_shell_input(host: &Host) -> Option<Vec<u8>> {
    if host.working_directory.is_none() && host.startup_command.is_none() {
        return None;
    }
    let mut input = String::new();
    if let Some(directory) = &host.working_directory {
        let quoted = directory.replace('\'', "'\\''");
        input.push_str("if cd -- '");
        input.push_str(&quoted);
        input.push_str("'; then\n");
        if let Some(command) = &host.startup_command {
            input.push_str(command);
            input.push('\n');
        }
        input.push_str("fi\n");
    } else if let Some(command) = &host.startup_command {
        input.push_str(command);
        input.push('\n');
    }
    Some(input.into_bytes())
}

#[derive(Clone)]
pub struct VaultManager {
    path: PathBuf,
    vault: Arc<RwLock<Option<Arc<Vault>>>>,
    epoch: Arc<std::sync::atomic::AtomicU64>,
    ssh_pool: Arc<capstan_ssh::ConnectionPool<capstan_ssh::PasswordConnection>>,
    lifecycle: Arc<tokio::sync::Mutex<()>>,
    recovery_tokens: Arc<Mutex<HashMap<Uuid, RecoverySelection>>>,
    connections: Arc<ConnectionSupervisor>,
    forwarding: Arc<forwarding::ForwardingService>,
    server_status: server_status::StatusService,
    hello_invalidated: Arc<std::sync::atomic::AtomicBool>,
}

struct ConnectionSupervisor {
    entries: Mutex<HashMap<Uuid, Arc<ManagedConnection>>>,
    events: tokio::sync::broadcast::Sender<ConnectionSnapshot>,
    probes: tokio::sync::watch::Sender<u64>,
    locked: std::sync::atomic::AtomicBool,
    transfers: Mutex<std::sync::Weak<transfer::TransferService>>,
}
impl Default for ConnectionSupervisor {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            events: tokio::sync::broadcast::channel(256).0,
            probes: tokio::sync::watch::channel(0).0,
            locked: std::sync::atomic::AtomicBool::new(false),
            transfers: Mutex::new(std::sync::Weak::new()),
        }
    }
}
struct ManagedConnection {
    record: Mutex<ConnectionRecord>,
    gate: tokio::sync::Mutex<()>,
    stop: CancellationToken,
    tasks: TaskTracker,
    changed: tokio::sync::watch::Sender<u64>,
}
struct ConnectionRecord {
    snapshot: ConnectionSnapshot,
    host_id: Uuid,
    host_revision: u64,
    policy: PoolKey,
    transport: std::sync::Weak<PasswordConnection>,
    transport_generation: u64,
    owners: HashMap<Uuid, CancellationToken>,
    attempt: Option<Uuid>,
    operation: CancellationToken,
    monitoring: bool,
    /// The broker of the last supervised attempt, reused when a network or
    /// wake signal restarts a retry cycle that ran out.
    broker: Option<Arc<capstan_ssh::TrustBroker>>,
}
impl ConnectionSupervisor {
    fn entries(&self) -> Result<Vec<Arc<ManagedConnection>>> {
        Ok(self
            .entries
            .lock()
            .map_err(|_| CoreError::TaskFailed)?
            .values()
            .cloned()
            .collect())
    }
    fn publish(
        &self,
        record: &mut ConnectionRecord,
        state: ConnectionState,
        error: Option<ConnectionErrorCode>,
    ) {
        let next = ConnectionSnapshot {
            state,
            error_code: error,
            ..record.snapshot.clone()
        };
        if capstan_ssh::connection_state::validate_successor(&record.snapshot, &next).is_ok() {
            record.snapshot = next.clone();
            let _ = self.events.send(next);
        }
    }
    fn next_generation(
        &self,
        entry: &ManagedConnection,
        record: &mut ConnectionRecord,
        attempt: Uuid,
    ) -> Result<ConnectionSnapshot> {
        record.snapshot.generation = record
            .snapshot
            .generation
            .checked_add(1)
            .ok_or(CoreError::InvalidInput)?;
        record.snapshot.state = ConnectionState::Connecting;
        record.snapshot.error_code = None;
        record.operation = CancellationToken::new();
        record.attempt = Some(attempt);
        let _ = self.events.send(record.snapshot.clone());
        entry
            .changed
            .send_modify(|sequence| *sequence = sequence.wrapping_add(1));
        Ok(record.snapshot.clone())
    }
    fn transfers(&self) -> Option<Arc<transfer::TransferService>> {
        self.transfers.lock().ok()?.upgrade()
    }
    fn interested(&self, entry: &ManagedConnection) -> bool {
        entry.record.lock().is_ok_and(|record| {
            !record.owners.is_empty()
                || self
                    .transfers()
                    .is_some_and(|service| service.has_connection_work(&record.policy))
        })
    }
}

struct RecoverySelection {
    candidate: VaultRecoveryCandidate,
    expires_at: Instant,
}

fn connection_error(error: &CoreError) -> ConnectionErrorCode {
    match error {
        CoreError::Ssh(error) => error.into(),
        CoreError::Locked => ConnectionErrorCode::VaultLocked,
        CoreError::HostKeyChanged => ConnectionErrorCode::HostKeyChanged,
        CoreError::Canceled => ConnectionErrorCode::Canceled,
        _ => ConnectionErrorCode::InvalidInput,
    }
}

impl VaultManager {
    pub fn subscribe_connections(&self) -> tokio::sync::broadcast::Receiver<ConnectionSnapshot> {
        self.connections.events.subscribe()
    }
    pub fn connection_snapshots(&self) -> Result<Vec<ConnectionSnapshot>> {
        self.connections
            .entries()?
            .iter()
            .map(|entry| {
                entry
                    .record
                    .lock()
                    .map(|record| record.snapshot.clone())
                    .map_err(|_| CoreError::TaskFailed)
            })
            .collect()
    }

    pub fn desktop_transfers(
        &self,
        transfers: &transfer::TransferService,
        window: &str,
    ) -> std::result::Result<Vec<desktop::DesktopTransfer>, sftp::SftpError> {
        let mut reconnecting = Vec::new();
        for entry in self
            .connections
            .entries()
            .map_err(|_| sftp::SftpError::ConnectionLost)?
        {
            let record = entry
                .record
                .lock()
                .map_err(|_| sftp::SftpError::ConnectionLost)?;
            if record.snapshot.state == ConnectionState::Reconnecting {
                reconnecting.push(record.policy.clone());
            }
        }
        transfers.desktop_snapshot(window, &reconnecting)
    }
    pub fn configure_connection_recovery(
        &self,
        transfers: &Arc<transfer::TransferService>,
    ) -> Result<()> {
        *self
            .connections
            .transfers
            .lock()
            .map_err(|_| CoreError::TaskFailed)? = Arc::downgrade(transfers);
        Ok(())
    }
    /// Native callbacks only coalesce this signal; all network work stays here.
    /// A retry cycle that ran out starts again, since the network may be back;
    /// authentication, trust, lock and cancellation failures stay stopped.
    pub fn probe_connections(&self) {
        self.connections
            .probes
            .send_modify(|sequence| *sequence = sequence.wrapping_add(1));
        if self
            .connections
            .locked
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return;
        }
        let Ok(entries) = self.connections.entries() else {
            return;
        };
        for entry in entries {
            if entry.stop.is_cancelled() || !self.connections.interested(&entry) {
                continue;
            }
            // A held gate means a start, reconnect or retry already owns it.
            let Ok(_gate) = entry.gate.try_lock() else {
                continue;
            };
            let broker = {
                let Ok(mut record) = entry.record.lock() else {
                    continue;
                };
                if record.monitoring
                    || record.snapshot.state != ConnectionState::Failed
                    || record.snapshot.error_code != Some(ConnectionErrorCode::ReconnectExhausted)
                {
                    continue;
                }
                let Some(broker) = record.broker.clone() else {
                    continue;
                };
                if self
                    .connections
                    .next_generation(&entry, &mut record, Uuid::new_v4())
                    .is_err()
                {
                    continue;
                }
                broker
            };
            self.monitor_connection(entry.clone(), broker);
        }
    }
    fn managed_connection(&self, id: Uuid) -> Result<Arc<ManagedConnection>> {
        self.connections
            .entries
            .lock()
            .map_err(|_| CoreError::TaskFailed)?
            .get(&id)
            .cloned()
            .ok_or(CoreError::NotFound)
    }
    fn host_policy(&self, host_id: Uuid, attempt: Uuid) -> Result<(Host, PoolKey)> {
        let data = self.current()?.snapshot()?;
        let host = data
            .hosts
            .into_iter()
            .find(|host| host.id == host_id)
            .ok_or(CoreError::NotFound)?;
        let credential = data
            .credentials
            .into_iter()
            .find(|credential| credential.id == host.credential_id)
            .ok_or(CoreError::NotFound)?;
        let target = capstan_vault::normalize_target(&host.address)?;
        let known = self.known_host_key(&target, host.port)?;
        let identity = match credential.material {
            CredentialMaterial::Agent { fingerprint, .. } => Some(fingerprint),
            _ => None,
        };
        let key = connection_pool_key(
            &host,
            credential.revision,
            self.epoch(),
            known.map_or_else(|| format!("unknown-{attempt}"), |key| key.fingerprint),
            identity,
        );
        Ok((host, key))
    }
    fn claim_connection(
        &self,
        host_id: Uuid,
        owner: Uuid,
        cancel: &CancellationToken,
    ) -> Result<Arc<ManagedConnection>> {
        if cancel.is_cancelled() {
            return Err(CoreError::Canceled);
        }
        if self
            .connections
            .locked
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(CoreError::Locked);
        }
        let (host, policy) = self.host_policy(host_id, owner)?;
        let mut entries = self
            .connections
            .entries
            .lock()
            .map_err(|_| CoreError::TaskFailed)?;
        for entry in entries.values() {
            let mut record = entry.record.lock().map_err(|_| CoreError::TaskFailed)?;
            let mut pending_policy = record.policy.clone();
            if pending_policy.trust_fingerprint.starts_with("unknown-")
                && record.host_id == host_id
                && record.host_revision == host.revision
            {
                pending_policy.trust_fingerprint = policy.trust_fingerprint.clone();
            }
            if !entry.stop.is_cancelled()
                && pending_policy == policy
                && record.snapshot.state != ConnectionState::Disconnected
            {
                record.owners.insert(owner, cancel.clone());
                return Ok(entry.clone());
            }
        }
        // Finished records have no runtime resources and need not grow with tab churn.
        entries.retain(|_, entry| !entry.stop.is_cancelled() || !entry.tasks.is_empty());
        let snapshot = ConnectionSnapshot {
            connection_id: Uuid::new_v4(),
            generation: 1,
            state: ConnectionState::Connecting,
            error_code: None,
        };
        let entry = Arc::new(ManagedConnection {
            record: Mutex::new(ConnectionRecord {
                snapshot: snapshot.clone(),
                host_id,
                host_revision: host.revision,
                policy,
                transport: std::sync::Weak::new(),
                transport_generation: 0,
                owners: HashMap::from([(owner, cancel.clone())]),
                attempt: Some(owner),
                operation: CancellationToken::new(),
                monitoring: false,
                broker: None,
            }),
            gate: tokio::sync::Mutex::new(()),
            stop: CancellationToken::new(),
            tasks: TaskTracker::new(),
            changed: tokio::sync::watch::channel(0).0,
        });
        entries.insert(snapshot.connection_id, entry.clone());
        let _ = self.connections.events.send(snapshot);
        Ok(entry)
    }
    fn adopt_connection(
        &self,
        entry: &ManagedConnection,
        lease: &ConnectionLease<PasswordConnection>,
    ) -> Result<()> {
        let mut record = entry.record.lock().map_err(|_| CoreError::TaskFailed)?;
        if entry.stop.is_cancelled()
            || record.operation.is_cancelled()
            || self
                .connections
                .locked
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(CoreError::Canceled);
        }
        record.transport = lease.downgrade_connection();
        record.transport_generation = lease.connection().connection_generation();
        // Unknown-host acquisitions gain the canonical trusted alias after confirmation.
        if let Ok((_, policy)) = self.host_policy(record.host_id, Uuid::new_v4()) {
            record.policy = policy;
        }
        record.attempt = None;
        self.connections
            .publish(&mut record, ConnectionState::Online, None);
        entry
            .changed
            .send_modify(|sequence| *sequence = sequence.wrapping_add(1));
        Ok(())
    }
    pub fn connection_for_owner(&self, owner: Uuid) -> Result<ConnectionSnapshot> {
        for entry in self.connections.entries()? {
            let record = entry.record.lock().map_err(|_| CoreError::TaskFailed)?;
            if record.owners.contains_key(&owner) {
                return Ok(record.snapshot.clone());
            }
        }
        Err(CoreError::NotFound)
    }
    pub fn release_connection_owner(&self, owner: Uuid) {
        if let Ok(entries) = self.connections.entries() {
            for entry in entries {
                if let Ok(mut record) = entry.record.lock() {
                    if let Some(cancel) = record.owners.remove(&owner) {
                        cancel.cancel();
                    } else {
                        continue;
                    }
                }
                if !self.connections.interested(&entry) {
                    entry.stop.cancel();
                    if let Ok(record) = entry.record.lock() {
                        record.operation.cancel();
                    }
                }
            }
        }
    }
    pub async fn cancel_connection_attempt(&self, attempt: Uuid) {
        if let Ok(entries) = self.connections.entries() {
            for entry in entries {
                let wait_for_attempt = if let Ok(record) = entry.record.lock() {
                    if let Some(owner_cancel) = record.owners.get(&attempt) {
                        owner_cancel.cancel();
                    }
                    if record.attempt == Some(attempt) {
                        record.operation.cancel();
                        record.monitoring
                    } else {
                        false
                    }
                } else {
                    false
                };
                if wait_for_attempt {
                    let mut changed = entry.changed.subscribe();
                    loop {
                        let still_current = entry.record.lock().is_ok_and(|record| {
                            record.attempt == Some(attempt) && record.monitoring
                        });
                        if !still_current || changed.changed().await.is_err() {
                            break;
                        }
                    }
                }
            }
        }
    }
    pub async fn release_connection_owner_and_wait(&self, owner: Uuid) {
        self.release_connection_owner(owner);
        if let Ok(entries) = self.connections.entries() {
            for entry in entries {
                if entry.stop.is_cancelled() {
                    entry.tasks.close();
                    entry.tasks.wait().await;
                    if let Ok(mut record) = entry.record.lock() {
                        self.connections
                            .publish(&mut record, ConnectionState::Disconnected, None);
                    }
                }
            }
        }
    }
    async fn suspend_connection_retries(&self) {
        self.connections
            .locked
            .store(true, std::sync::atomic::Ordering::Release);
        if let Ok(entries) = self.connections.entries() {
            let mut canceled = Vec::new();
            for entry in &entries {
                if let Ok(record) = entry.record.lock() {
                    if record.snapshot.state != ConnectionState::Online {
                        record.operation.cancel();
                        canceled.push(entry.clone());
                    }
                }
            }
            for entry in entries {
                let _gate = entry.gate.lock().await;
            }
            for entry in canceled {
                entry.tasks.close();
                entry.tasks.wait().await;
                entry.tasks.reopen();
            }
        }
    }
    pub async fn disconnect_connection(
        &self,
        id: Uuid,
        generation: u64,
    ) -> Result<ConnectionSnapshot> {
        let entry = self.managed_connection(id)?;
        let transport = {
            let mut record = entry.record.lock().map_err(|_| CoreError::TaskFailed)?;
            if record.snapshot.generation != generation {
                return Err(CoreError::InvalidInput);
            }
            entry.stop.cancel();
            record.operation.cancel();
            for cancel in record.owners.values() {
                cancel.cancel();
            }
            record.owners.clear();
            record.transport.upgrade()
        };
        // Reauthorized logical lifetimes can converge on the same pooled
        // transport after an unlock. Explicit disconnect owns that transport,
        // including every logical alias; no alias may restart it behind the user.
        let mut affected = vec![entry.clone()];
        if let Some(transport) = &transport {
            for candidate in self.connections.entries()? {
                if Arc::ptr_eq(&candidate, &entry) {
                    continue;
                }
                let mut record = candidate.record.lock().map_err(|_| CoreError::TaskFailed)?;
                if record
                    .transport
                    .upgrade()
                    .is_some_and(|current| Arc::ptr_eq(&current, transport))
                {
                    candidate.stop.cancel();
                    record.operation.cancel();
                    for cancel in record.owners.values() {
                        cancel.cancel();
                    }
                    record.owners.clear();
                    affected.push(candidate.clone());
                }
            }
            transport.disconnect().await;
        }
        for candidate in affected {
            let candidate_id = candidate
                .record
                .lock()
                .map_err(|_| CoreError::TaskFailed)?
                .snapshot
                .connection_id;
            self.stop_connection_tunnels(candidate_id, true).await;
            candidate.tasks.close();
            candidate.tasks.wait().await;
            let mut record = candidate.record.lock().map_err(|_| CoreError::TaskFailed)?;
            self.connections.publish(
                &mut record,
                ConnectionState::Disconnected,
                Some(ConnectionErrorCode::Canceled),
            );
        }
        let snapshot = entry
            .record
            .lock()
            .map_err(|_| CoreError::TaskFailed)?
            .snapshot
            .clone();
        Ok(snapshot)
    }
    pub async fn reconnect_connection(
        &self,
        id: Uuid,
        generation: u64,
        attempt: Uuid,
        broker: Arc<capstan_ssh::TrustBroker>,
    ) -> Result<ConnectionSnapshot> {
        if self
            .connections
            .locked
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(CoreError::Locked);
        }
        self.current()?;
        let entry = self.managed_connection(id)?;
        let _gate = entry.gate.lock().await;
        let snapshot = {
            let mut record = entry.record.lock().map_err(|_| CoreError::TaskFailed)?;
            if record.snapshot.generation != generation
                || record.monitoring
                || record.owners.is_empty()
                || entry.stop.is_cancelled()
                || !matches!(
                    record.snapshot.state,
                    ConnectionState::Failed | ConnectionState::Reconnecting
                )
            {
                return Err(CoreError::InvalidInput);
            }
            self.connections
                .next_generation(&entry, &mut record, attempt)?
        };
        self.monitor_connection(entry.clone(), broker);
        Ok(snapshot)
    }
    fn monitor_connection(
        &self,
        entry: Arc<ManagedConnection>,
        broker: Arc<capstan_ssh::TrustBroker>,
    ) {
        if let Ok(mut record) = entry.record.lock() {
            if record.monitoring || entry.stop.is_cancelled() {
                return;
            }
            record.monitoring = true;
            record.broker = Some(broker.clone());
        } else {
            return;
        }
        let vault = self.clone();
        let tracked = entry.tasks.token();
        let signals = self.connections.probes.subscribe();
        tokio::spawn(async move {
            let _tracked = tracked;
            vault.supervise_connection(&entry, broker, signals).await;
            if let Ok(mut record) = entry.record.lock() {
                record.monitoring = false;
                entry
                    .changed
                    .send_modify(|sequence| *sequence = sequence.wrapping_add(1));
            }
        });
    }
    async fn supervise_connection(
        &self,
        entry: &Arc<ManagedConnection>,
        broker: Arc<capstan_ssh::TrustBroker>,
        mut signals: tokio::sync::watch::Receiver<u64>,
    ) {
        let mut failures = 0_u32;
        loop {
            if entry.stop.is_cancelled() || !self.connections.interested(entry) {
                return;
            }
            let (state, transport) = match entry.record.lock() {
                Ok(record) => (record.snapshot.state, record.transport.upgrade()),
                Err(_) => return,
            };
            if state == ConnectionState::Online {
                let alive = transport
                    .as_ref()
                    .is_some_and(|connection| !connection.is_closed());
                if alive {
                    drop(transport);
                    tokio::select! {
                        biased;
                        () = entry.stop.cancelled() => return,
                        _ = signals.changed() => {
                            let transport = entry.record.lock().ok().and_then(|record| record.transport.upgrade());
                            if let Some(connection) = transport {
                                if connection.probe().await.is_err() { connection.disconnect().await; }
                            }
                        }
                        () = tokio::time::sleep(Duration::from_millis(250)) => {}
                    }
                    continue;
                }
                if let Ok(mut record) = entry.record.lock() {
                    self.connections.publish(
                        &mut record,
                        ConnectionState::Reconnecting,
                        Some(ConnectionErrorCode::ConnectionLost),
                    );
                }
                if let Some(connection) = transport {
                    connection.disconnect().await;
                }
                let connection_id = match entry.record.lock() {
                    Ok(record) => record.snapshot.connection_id,
                    Err(_) => return,
                };
                self.stop_connection_tunnels(connection_id, false).await;
                failures = 0;
            }
            if self
                .connections
                .locked
                .load(std::sync::atomic::Ordering::Acquire)
            {
                if let Ok(mut record) = entry.record.lock() {
                    self.connections.publish(
                        &mut record,
                        ConnectionState::Failed,
                        Some(ConnectionErrorCode::VaultLocked),
                    );
                }
                return;
            }
            let operation = match entry.record.lock() {
                Ok(record) => record.operation.clone(),
                Err(_) => return,
            };
            let delay = Duration::from_secs((1_u64 << failures.min(5)).min(30));
            tokio::select! {
                biased;
                () = entry.stop.cancelled() => return,
                () = operation.cancelled() => {
                    let error = if self.connections.locked.load(std::sync::atomic::Ordering::Acquire) { ConnectionErrorCode::VaultLocked } else { ConnectionErrorCode::Canceled };
                    if let Ok(mut record) = entry.record.lock() { self.connections.publish(&mut record, ConnectionState::Failed, Some(error)); }
                    return;
                }
                _ = signals.changed() => {},
                () = tokio::time::sleep(delay) => {},
            }
            let _gate = entry.gate.lock().await;
            if entry.stop.is_cancelled() || !self.connections.interested(entry) {
                return;
            }
            let (host_id, host_revision, policy, attempt, operation) = {
                let Ok(mut record) = entry.record.lock() else {
                    return;
                };
                if record.snapshot.state == ConnectionState::Online {
                    continue;
                }
                if record.snapshot.state != ConnectionState::Connecting
                    && self
                        .connections
                        .next_generation(entry, &mut record, Uuid::new_v4())
                        .is_err()
                {
                    return;
                }
                (
                    record.host_id,
                    record.host_revision,
                    record.policy.clone(),
                    record.attempt.unwrap_or_else(Uuid::new_v4),
                    record.operation.clone(),
                )
            };
            let result = async {
                let (host, current) = self.host_policy(host_id, attempt)?;
                if host.revision != host_revision || !transfer::same_retry_policy(&policy, &current)
                {
                    return Err(CoreError::InvalidInput);
                }
                let (_host, lease) = self
                    .acquire_host_connection(host_id, attempt, broker.clone(), operation.clone())
                    .await?;
                self.adopt_connection(entry, &lease)?;
                self.start_auto_tunnels(entry, &lease).await;
                if let Some(transfers) = self.connections.transfers() {
                    transfers.recover_connection(lease, &operation).await;
                }
                Ok::<_, CoreError>(())
            };
            let result = result.await;
            broker.cancel_attempt(attempt);
            match result {
                Ok(()) => failures = 0,
                Err(error) => {
                    failures += 1;
                    let retryable = matches!(
                        error,
                        CoreError::Ssh(
                            capstan_ssh::SshError::Timeout | capstan_ssh::SshError::Connection
                        )
                    );
                    if let Ok(mut record) = entry.record.lock() {
                        let code = if self
                            .connections
                            .locked
                            .load(std::sync::atomic::Ordering::Acquire)
                        {
                            ConnectionErrorCode::VaultLocked
                        } else if failures >= 8 && retryable {
                            ConnectionErrorCode::ReconnectExhausted
                        } else {
                            connection_error(&error)
                        };
                        self.connections.publish(
                            &mut record,
                            if retryable && failures < 8 {
                                ConnectionState::Reconnecting
                            } else {
                                ConnectionState::Failed
                            },
                            Some(code),
                        );
                    }
                    if !retryable || failures >= 8 {
                        return;
                    }
                }
            }
        }
    }
}

const RECOVERY_SELECTION_LIFETIME: Duration = Duration::from_secs(5 * 60);
enum ConnectMaterial {
    Password(SecretString),
    ImportedKey {
        content: SecretText,
        passphrase: Option<SecretText>,
    },
    Agent {
        provider: capstan_ssh::AgentProvider,
        fingerprint: String,
    },
}
impl VaultManager {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            vault: Arc::new(RwLock::new(None)),
            epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            ssh_pool: Arc::new(capstan_ssh::ConnectionPool::new(
                std::time::Duration::from_secs(30),
            )),
            lifecycle: Arc::new(tokio::sync::Mutex::new(())),
            recovery_tokens: Arc::new(Mutex::new(HashMap::new())),
            connections: Arc::new(ConnectionSupervisor::default()),
            forwarding: Arc::new(forwarding::ForwardingService::default()),
            server_status: server_status::StatusService::default(),
            hello_invalidated: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
    pub fn epoch(&self) -> u64 {
        self.epoch.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn status(&self) -> VaultStatus {
        let vault = self.vault.read().ok().and_then(|state| state.clone());
        let revision = vault
            .as_ref()
            .and_then(|vault| vault.snapshot().ok().map(|data| data.revision));
        VaultStatus {
            exists: self.path.exists(),
            unlocked: vault.is_some(),
            revision,
            epoch: self.epoch(),
        }
    }
    fn current(&self) -> Result<Arc<Vault>> {
        self.vault
            .read()
            .map_err(|_| CoreError::Storage)?
            .clone()
            .ok_or(CoreError::Locked)
    }
    fn ensure_locked(&self) -> Result<()> {
        if self.vault.read().map_err(|_| CoreError::Storage)?.is_some() {
            return Err(CoreError::Conflict);
        }
        Ok(())
    }
    fn clear_recovery_tokens(&self) -> Result<()> {
        self.recovery_tokens
            .lock()
            .map_err(|_| CoreError::Storage)?
            .clear();
        Ok(())
    }
    fn cleanup_hello_after_password_authentication(vault: &Vault) -> bool {
        // Optional enrollment cleanup must not prevent authenticated unlock or recovery.
        vault
            .cleanup_invalid_hello(&capstan_vault::hello::PlatformHello::default())
            .unwrap_or(true)
    }
    pub async fn initialize(&self, password: SecretText, parameters: KdfParameters) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        self.ensure_locked()?;
        let path = self.path.clone();
        let vault = task::spawn_blocking(move || Vault::initialize(path, &password, parameters))
            .await
            .map_err(|_| CoreError::TaskFailed)??;
        let mut state = self.vault.write().map_err(|_| CoreError::Storage)?;
        if state.is_some() {
            return Err(CoreError::Conflict);
        }
        *state = Some(Arc::new(vault));
        self.clear_recovery_tokens()?;
        self.epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        self.connections
            .locked
            .store(false, std::sync::atomic::Ordering::Release);
        Ok(())
    }
    pub async fn unlock(&self, password: SecretText) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        self.ensure_locked()?;
        let path = self.path.clone();
        let (vault, invalidated) = task::spawn_blocking(move || {
            let vault = Vault::unlock(path, &password)?;
            let invalidated = Self::cleanup_hello_after_password_authentication(&vault);
            Ok::<_, VaultError>((vault, invalidated))
        })
        .await
        .map_err(|_| CoreError::TaskFailed)??;
        let mut state = self.vault.write().map_err(|_| CoreError::Storage)?;
        if state.is_some() {
            return Err(CoreError::Conflict);
        }
        *state = Some(Arc::new(vault));
        self.hello_invalidated
            .store(invalidated, std::sync::atomic::Ordering::Release);
        self.clear_recovery_tokens()?;
        self.epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        self.connections
            .locked
            .store(false, std::sync::atomic::Ordering::Release);
        Ok(())
    }
    pub async fn hello_status(&self) -> Result<capstan_vault::hello::HelloStatus> {
        let path = self.path.clone();
        let mut status = task::spawn_blocking(move || {
            Vault::hello_status(&path, &capstan_vault::hello::PlatformHello::default())
        })
        .await
        .map_err(|_| CoreError::TaskFailed)?;
        status.invalidated = self
            .hello_invalidated
            .load(std::sync::atomic::Ordering::Acquire);
        Ok(status)
    }
    pub async fn enable_hello(
        &self,
        password: SecretText,
        provider: Arc<dyn capstan_vault::hello::HelloProvider>,
    ) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        let vault = self.current()?;
        task::spawn_blocking(move || vault.enable_hello(&password, provider.as_ref()))
            .await
            .map_err(|_| CoreError::TaskFailed)??;
        self.hello_invalidated
            .store(false, std::sync::atomic::Ordering::Release);
        Ok(())
    }
    pub async fn disable_hello(&self) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        let vault = self.current()?;
        task::spawn_blocking(move || {
            vault.disable_hello(&capstan_vault::hello::PlatformHello::default())
        })
        .await
        .map_err(|_| CoreError::TaskFailed)??;
        self.hello_invalidated
            .store(false, std::sync::atomic::Ordering::Release);
        Ok(())
    }
    pub async fn unlock_hello(
        &self,
        provider: Arc<dyn capstan_vault::hello::HelloProvider>,
    ) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        self.ensure_locked()?;
        let path = self.path.clone();
        let vault = task::spawn_blocking(move || Vault::unlock_with_hello(path, provider.as_ref()))
            .await
            .map_err(|_| CoreError::TaskFailed)??;
        let mut state = self.vault.write().map_err(|_| CoreError::Storage)?;
        if state.is_some() {
            return Err(CoreError::Conflict);
        }
        *state = Some(Arc::new(vault));
        self.clear_recovery_tokens()?;
        self.epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        self.connections
            .locked
            .store(false, std::sync::atomic::Ordering::Release);
        self.hello_invalidated
            .store(false, std::sync::atomic::Ordering::Release);
        Ok(())
    }
    pub async fn lock(&self) -> Result<()> {
        self.suspend_connection_retries().await;
        let _lifecycle = self.lifecycle.lock().await;
        self.connections
            .locked
            .store(true, std::sync::atomic::Ordering::Release);
        self.clear_recovery_tokens()?;
        let vault = self.vault.write().map_err(|_| CoreError::Storage)?.take();
        if let Some(vault) = vault {
            vault.lock()?;
        }
        self.epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Ok(())
    }
    pub async fn list_recovery_candidates(
        &self,
        password: SecretText,
    ) -> Result<Vec<VaultRecoveryCandidateView>> {
        let _lifecycle = self.lifecycle.lock().await;
        self.ensure_locked()?;
        let path = self.path.clone();
        let candidates = task::spawn_blocking(move || Vault::recovery_candidates(&path, &password))
            .await
            .map_err(|_| CoreError::TaskFailed)??;
        let now = Instant::now();
        let expires_at = now + RECOVERY_SELECTION_LIFETIME;
        let mut tokens = self
            .recovery_tokens
            .lock()
            .map_err(|_| CoreError::Storage)?;
        tokens.clear();
        let mut views = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let token = Uuid::new_v4();
            let kind = match candidate.kind {
                RecoveryKind::Pending => RecoveryKindView::Pending,
                RecoveryKind::Backup => RecoveryKindView::Backup,
            };
            views.push(VaultRecoveryCandidateView {
                token,
                kind,
                revision: candidate.revision,
            });
            tokens.insert(
                token,
                RecoverySelection {
                    candidate,
                    expires_at,
                },
            );
        }
        Ok(views)
    }
    pub async fn restore_recovery_candidate(
        &self,
        token: Uuid,
        password: SecretText,
    ) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        self.ensure_locked()?;
        let candidate = {
            let mut tokens = self
                .recovery_tokens
                .lock()
                .map_err(|_| CoreError::Storage)?;
            let now = Instant::now();
            tokens.retain(|_, selection| selection.expires_at > now);
            tokens
                .get(&token)
                .map(|selection| selection.candidate.clone())
                .ok_or(CoreError::RecoveryExpired)?
        };
        let path = self.path.clone();
        let (vault, invalidated) = task::spawn_blocking(move || {
            let vault = Vault::restore_candidate(&path, &candidate, &password)?;
            let invalidated = Self::cleanup_hello_after_password_authentication(&vault);
            Ok::<_, VaultError>((vault, invalidated))
        })
        .await
        .map_err(|_| CoreError::TaskFailed)??;
        let mut state = self.vault.write().map_err(|_| CoreError::Storage)?;
        if state.is_some() {
            return Err(CoreError::Conflict);
        }
        *state = Some(Arc::new(vault));
        self.hello_invalidated
            .store(invalidated, std::sync::atomic::Ordering::Release);
        self.clear_recovery_tokens()?;
        self.epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        self.connections
            .locked
            .store(false, std::sync::atomic::Ordering::Release);
        Ok(())
    }
    pub async fn disconnect_ssh(&self) {
        if let Ok(snapshots) = self.connection_snapshots() {
            for snapshot in snapshots {
                let _ = self
                    .disconnect_connection(snapshot.connection_id, snapshot.generation)
                    .await;
            }
        }
        for connection in self.ssh_pool.drain().await {
            connection.disconnect().await;
        }
        self.stop_all_tunnels().await;
    }
    pub fn hosts(&self) -> Result<Vec<HostView>> {
        let data = self.current()?.snapshot()?;
        Ok(data.hosts.iter().map(HostView::from).collect())
    }
    pub fn credentials(&self) -> Result<Vec<CredentialView>> {
        let data = self.current()?.snapshot()?;
        Ok(data.credentials.iter().map(CredentialView::from).collect())
    }
    pub fn groups(&self) -> Result<Vec<GroupView>> {
        let data = self.current()?.snapshot()?;
        Ok(data.groups.iter().map(GroupView::from).collect())
    }
    pub fn snippets(&self) -> Result<Vec<Snippet>> {
        let mut snippets = self.current()?.snapshot()?.snippets;
        snippets.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(snippets)
    }
    pub fn snippet_groups(&self) -> Result<Vec<SnippetGroup>> {
        let mut groups = self.current()?.snapshot()?.snippet_groups;
        groups.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(groups)
    }
    pub fn trusted_host_keys(&self) -> Result<Vec<TrustedHostKeyView>> {
        let data = self.current()?.snapshot()?;
        let mut trusted: Vec<_> = data
            .trust
            .iter()
            .map(|record| TrustedHostKeyView {
                target: record.target.clone(),
                port: record.port,
                algorithm: record.algorithm.clone(),
                fingerprint: record.fingerprint.clone(),
                first_trusted_at: record.first_trusted_at,
            })
            .collect();
        trusted.sort_by(|left, right| {
            left.target
                .cmp(&right.target)
                .then_with(|| left.port.cmp(&right.port))
        });
        Ok(trusted)
    }
    pub async fn forget_trusted_host_key(
        &self,
        target: String,
        port: u16,
        expected_fingerprint: String,
        expected_revision: u64,
    ) -> Result<u64> {
        let target = capstan_vault::normalize_target(&target).map_err(CoreError::from)?;
        if port == 0 || expected_fingerprint.is_empty() || expected_fingerprint.len() > 256 {
            return Err(CoreError::InvalidInput);
        }
        let vault = self.current()?;
        let data = vault.snapshot()?;
        if data.revision != expected_revision {
            return Err(CoreError::Conflict);
        }
        if !data.trust.iter().any(|record| {
            record.target == target
                && record.port == port
                && record.fingerprint == expected_fingerprint
        }) {
            return Err(CoreError::Conflict);
        }
        let epoch = self.epoch.clone();
        task::spawn_blocking(move || {
            let revision = vault.transaction(expected_revision, move |data| {
                let before = data.trust.len();
                data.trust.retain(|record| {
                    record.target != target
                        || record.port != port
                        || record.fingerprint != expected_fingerprint
                });
                if before == data.trust.len() {
                    return Err(VaultError::Conflict);
                }
                Ok(())
            })?;
            epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            Ok::<_, VaultError>(revision)
        })
        .await
        .map_err(|_| CoreError::TaskFailed)?
        .map_err(CoreError::from)
    }
    pub fn revision(&self) -> Result<u64> {
        Ok(self.current()?.snapshot()?.revision)
    }
    pub fn preferences(&self) -> Result<Preferences> {
        Ok(self.current()?.snapshot()?.preferences)
    }
    pub async fn change_master_password(
        &self,
        password: SecretText,
        expected_revision: u64,
    ) -> Result<()> {
        let vault = self.current()?;
        task::spawn_blocking(move || {
            vault.change_master_password(expected_revision, &password, KdfParameters::default())
        })
        .await
        .map_err(|_| CoreError::TaskFailed)??;
        Ok(())
    }
    pub async fn export_backup(&self, destination: PathBuf, password: SecretText) -> Result<()> {
        let vault = self.current()?;
        task::spawn_blocking(move || {
            vault.export_backup(&password, KdfParameters::default(), &destination)
        })
        .await
        .map_err(|_| CoreError::TaskFailed)??;
        Ok(())
    }
    pub async fn export_backup_replace(
        &self,
        destination: PathBuf,
        password: SecretText,
    ) -> Result<()> {
        let vault = self.current()?;
        task::spawn_blocking(move || {
            vault.export_backup_with_replace(
                &password,
                KdfParameters::default(),
                &destination,
                true,
            )
        })
        .await
        .map_err(|_| CoreError::TaskFailed)??;
        Ok(())
    }
    pub async fn preview_backup(
        &self,
        source: PathBuf,
        password: SecretText,
    ) -> Result<BackupPreview> {
        let current = self.current()?.snapshot()?;
        let imported = task::spawn_blocking(move || Vault::preview_backup(&source, &password))
            .await
            .map_err(|_| CoreError::TaskFailed)??;
        Ok(build_backup_preview(&current, &imported))
    }
    pub async fn import_backup(
        &self,
        source: PathBuf,
        password: SecretText,
        mode: BackupImportMode,
        expected_revision: u64,
        expected_epoch: u64,
    ) -> Result<u64> {
        if !self.epoch_matches(expected_epoch) {
            return Err(CoreError::Canceled);
        }
        let vault = self.current()?;
        let current = vault.snapshot()?;
        if current.revision != expected_revision {
            return Err(CoreError::Conflict);
        }
        let imported = task::spawn_blocking(move || Vault::preview_backup(&source, &password))
            .await
            .map_err(|_| CoreError::TaskFailed)??;
        let preview = build_backup_preview(&current, &imported);
        if !preview.trust_conflicts.is_empty() {
            return Err(CoreError::TrustConflict);
        }
        let preferences = current.preferences.clone();
        let mut candidate = match mode {
            BackupImportMode::Merge => merge_backup_data(&current, imported)?,
            BackupImportMode::Overwrite => imported,
        };
        candidate.preferences = preferences;
        let _forwarding = self.forwarding.gate.lock().await;
        self.stop_all_tunnels_locked().await?;
        if !self.epoch_matches(expected_epoch) {
            return Err(CoreError::Canceled);
        }
        let revision = task::spawn_blocking(move || {
            vault.transaction(expected_revision, move |data| {
                *data = candidate;
                Ok(())
            })
        })
        .await
        .map_err(|_| CoreError::TaskFailed)??;
        self.epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Ok(revision)
    }
    pub async fn update_preferences(
        &self,
        preferences: Preferences,
        expected_revision: u64,
    ) -> Result<u64> {
        let vault = self.current()?;
        task::spawn_blocking(move || {
            vault.transaction(expected_revision, move |data| {
                data.preferences = preferences;
                Ok(())
            })
        })
        .await
        .map_err(|_| CoreError::TaskFailed)?
        .map_err(Into::into)
    }
    pub fn epoch_matches(&self, expected: u64) -> bool {
        self.epoch() == expected
    }

    pub async fn add_password_credential(&self, name: String, password: String) -> Result<Uuid> {
        self.save_password_credential(name, password, None).await
    }
    pub async fn save_password_credential(
        &self,
        name: String,
        mut password: String,
        update: Option<CredentialUpdate>,
    ) -> Result<Uuid> {
        if name.trim().is_empty() || name.len() > 256 || password.len() > 8192 {
            password.zeroize();
            return Err(CoreError::InvalidInput);
        }
        let secret = SecretText::new(std::mem::take(&mut password));
        password.zeroize();
        let credential = Credential {
            id: Uuid::new_v4(),
            name,
            revision: 0,
            material: CredentialMaterial::Password { value: secret },
        };
        self.store_credential(credential, update).await
    }
    pub async fn add_imported_key_credential(
        &self,
        name: String,
        content: String,
        passphrase: Option<String>,
    ) -> Result<Uuid> {
        self.save_imported_key_credential(name, content, passphrase, None)
            .await
    }
    pub async fn save_imported_key_credential(
        &self,
        name: String,
        mut content: String,
        mut passphrase: Option<String>,
        update: Option<CredentialUpdate>,
    ) -> Result<Uuid> {
        if content.len() > 1024 * 1024 {
            content.zeroize();
            if let Some(passphrase) = &mut passphrase {
                passphrase.zeroize();
            }
            return Err(CoreError::InvalidInput);
        }
        let validation = capstan_ssh::validate_private_key(&content, passphrase.as_deref());
        if validation.is_err() {
            content.zeroize();
            if let Some(passphrase) = &mut passphrase {
                passphrase.zeroize();
            }
        }
        validation?;
        let key = SecretText::new(std::mem::take(&mut content));
        content.zeroize();
        let passphrase = passphrase.take().map(SecretText::new);
        let credential = Credential {
            id: Uuid::new_v4(),
            name,
            revision: 0,
            material: CredentialMaterial::ImportedKey {
                content: key,
                passphrase,
            },
        };
        self.store_credential(credential, update).await
    }
    pub async fn add_key_file_credential(
        &self,
        name: String,
        path: PathBuf,
        passphrase: Option<String>,
    ) -> Result<Uuid> {
        self.save_key_file_credential(name, path, passphrase, None)
            .await
    }
    pub async fn save_key_file_credential(
        &self,
        name: String,
        path: PathBuf,
        mut passphrase: Option<String>,
        update: Option<CredentialUpdate>,
    ) -> Result<Uuid> {
        if name.trim().is_empty() || name.len() > 256 || !path.is_absolute() {
            if let Some(passphrase) = &mut passphrase {
                passphrase.zeroize();
            }
            return Err(CoreError::InvalidInput);
        }
        let read_path = path.clone();
        let passphrase = passphrase.take().map(SecretText::new);
        let content = task::spawn_blocking(move || read_private_key_file(&read_path))
            .await
            .map_err(|_| CoreError::TaskFailed)??;
        capstan_ssh::validate_private_key(
            content.expose(),
            passphrase.as_ref().map(SecretText::expose),
        )?;
        let credential = Credential {
            id: Uuid::new_v4(),
            name,
            revision: 0,
            material: CredentialMaterial::KeyFile {
                path: path.to_string_lossy().into_owned(),
                passphrase,
            },
        };
        self.store_credential(credential, update).await
    }
    pub async fn add_agent_credential(
        &self,
        name: String,
        agent: AgentKind,
        fingerprint: String,
    ) -> Result<Uuid> {
        self.save_agent_credential(name, agent, fingerprint, None)
            .await
    }
    pub async fn save_agent_credential(
        &self,
        name: String,
        agent: AgentKind,
        fingerprint: String,
        update: Option<CredentialUpdate>,
    ) -> Result<Uuid> {
        if name.trim().is_empty()
            || name.len() > 256
            || !fingerprint.starts_with("SHA256:")
            || fingerprint.len() > 256
        {
            return Err(CoreError::InvalidInput);
        }
        let provider = match agent {
            AgentKind::OpenSsh => capstan_ssh::AgentProvider::OpenSsh,
            AgentKind::Pageant => capstan_ssh::AgentProvider::Pageant,
        };
        let identities = capstan_ssh::list_agent_identities(provider).await?;
        if identities
            .iter()
            .all(|identity| identity.fingerprint != fingerprint)
        {
            return Err(capstan_ssh::SshError::AgentIdentityUnavailable.into());
        }
        let credential = Credential {
            id: Uuid::new_v4(),
            name,
            revision: 0,
            material: CredentialMaterial::Agent { agent, fingerprint },
        };
        self.store_credential(credential, update).await
    }
    #[cfg(test)]
    async fn add_credential(&self, credential: Credential) -> Result<()> {
        self.store_credential(credential, None).await.map(|_| ())
    }
    async fn store_credential(
        &self,
        mut credential: Credential,
        update: Option<CredentialUpdate>,
    ) -> Result<Uuid> {
        let vault = self.current()?;
        let revision = update.map_or(vault.snapshot()?.revision, |edit| edit.expected_revision);
        let id = update.map_or(credential.id, |edit| edit.id);
        let epoch = self.epoch.clone();
        task::spawn_blocking(move || {
            vault.transaction(revision, move |data| {
                if let Some(edit) = update {
                    if !matches!(credential.material, CredentialMaterial::Agent { .. })
                        && data
                            .hosts
                            .iter()
                            .any(|host| host.credential_id == edit.id && host.agent_forwarding)
                    {
                        return Err(VaultError::InvalidData);
                    }
                    let current = data
                        .credentials
                        .iter_mut()
                        .find(|item| item.id == edit.id)
                        .ok_or(VaultError::NotFound)?;
                    credential.id = edit.id;
                    credential.revision = current
                        .revision
                        .checked_add(1)
                        .ok_or(VaultError::InvalidData)?;
                    *current = credential;
                } else {
                    data.credentials.push(credential);
                }
                Ok(())
            })?;
            if update.is_some() {
                epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            }
            Ok::<_, VaultError>(id)
        })
        .await
        .map_err(|_| CoreError::TaskFailed)?
        .map_err(Into::into)
    }
    pub async fn rename_credential(
        &self,
        id: Uuid,
        name: String,
        expected_revision: u64,
    ) -> Result<Uuid> {
        if name.trim().is_empty() || name.len() > 256 {
            return Err(CoreError::InvalidInput);
        }
        let mut credential = self
            .current()?
            .snapshot()?
            .credentials
            .into_iter()
            .find(|item| item.id == id)
            .ok_or(CoreError::NotFound)?;
        credential.name = name;
        self.store_credential(
            credential,
            Some(CredentialUpdate {
                id,
                expected_revision,
            }),
        )
        .await
    }
    pub async fn delete_credential(&self, id: Uuid, expected_revision: u64) -> Result<u64> {
        let vault = self.current()?;
        let data = vault.snapshot()?;
        if data.revision != expected_revision {
            return Err(CoreError::Conflict);
        }
        if data.hosts.iter().any(|host| host.credential_id == id) {
            return Err(CoreError::CredentialInUse);
        }
        let epoch = self.epoch.clone();
        task::spawn_blocking(move || {
            let revision = vault.transaction(expected_revision, move |data| {
                let position = data
                    .credentials
                    .iter()
                    .position(|item| item.id == id)
                    .ok_or(VaultError::NotFound)?;
                data.credentials.remove(position);
                Ok(())
            })?;
            epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            Ok::<_, VaultError>(revision)
        })
        .await
        .map_err(|_| CoreError::TaskFailed)?
        .map_err(Into::into)
    }
    pub async fn save_host(&self, host: Host, expected_revision: u64) -> Result<u64> {
        let vault = self.current()?;
        let id = host.id;
        task::spawn_blocking(move || {
            vault.transaction(expected_revision, move |data| {
                let mut host = host;
                if host.agent_forwarding
                    && !data.credentials.iter().any(|credential| {
                        credential.id == host.credential_id
                            && matches!(&credential.material, CredentialMaterial::Agent { .. })
                    })
                {
                    return Err(VaultError::InvalidData);
                }
                if let Some(existing) = data.hosts.iter_mut().find(|existing| existing.id == id) {
                    if existing.revision != host.revision {
                        return Err(VaultError::Conflict);
                    }
                    host.revision = existing
                        .revision
                        .checked_add(1)
                        .ok_or(VaultError::InvalidData)?;
                    *existing = host;
                } else {
                    host.revision = 0;
                    data.hosts.push(host);
                }
                Ok(())
            })
        })
        .await
        .map_err(|_| CoreError::TaskFailed)?
        .map_err(Into::into)
    }
    pub async fn delete_host(&self, id: Uuid, expected_revision: u64) -> Result<u64> {
        let vault = self.current()?;
        if vault.snapshot()?.revision != expected_revision {
            return Err(CoreError::Conflict);
        }
        let _forwarding = self.forwarding.gate.lock().await;
        self.stop_host_tunnels_locked(id).await?;
        task::spawn_blocking(move || {
            vault.transaction(expected_revision, move |data| {
                let before = data.hosts.len();
                data.hosts.retain(|host| host.id != id);
                if data.hosts.len() == before {
                    return Err(VaultError::NotFound);
                }
                data.tunnels.retain(|tunnel| tunnel.host_id != id);
                Ok(())
            })
        })
        .await
        .map_err(|_| CoreError::TaskFailed)?
        .map_err(Into::into)
    }
    pub async fn save_group(&self, group: Group, expected_revision: u64) -> Result<u64> {
        if group.id.is_nil() || group.name.trim().is_empty() || group.name.len() > 256 {
            return Err(CoreError::InvalidInput);
        }
        let vault = self.current()?;
        let id = group.id;
        task::spawn_blocking(move || {
            vault.transaction(expected_revision, move |data| {
                if let Some(existing) = data.groups.iter_mut().find(|existing| existing.id == id) {
                    *existing = group;
                } else {
                    data.groups.push(group);
                }
                Ok(())
            })
        })
        .await
        .map_err(|_| CoreError::TaskFailed)?
        .map_err(Into::into)
    }
    pub async fn save_snippet_group(
        &self,
        group: SnippetGroup,
        expected_revision: u64,
    ) -> Result<u64> {
        group.validate().map_err(CoreError::from)?;
        let vault = self.current()?;
        let id = group.id;
        task::spawn_blocking(move || {
            vault.transaction(expected_revision, move |data| {
                if let Some(existing) = data
                    .snippet_groups
                    .iter_mut()
                    .find(|existing| existing.id == id)
                {
                    *existing = group;
                } else {
                    data.snippet_groups.push(group);
                }
                Ok(())
            })
        })
        .await
        .map_err(|_| CoreError::TaskFailed)?
        .map_err(Into::into)
    }
    pub async fn delete_snippet_group(&self, id: Uuid, expected_revision: u64) -> Result<u64> {
        let vault = self.current()?;
        let result = task::spawn_blocking(move || {
            vault.transaction(expected_revision, move |data| {
                if !data.snippet_groups.iter().any(|group| group.id == id) {
                    return Err(VaultError::NotFound);
                }
                if data
                    .snippets
                    .iter()
                    .any(|snippet| snippet.group_id == Some(id))
                {
                    return Err(VaultError::AlreadyExists);
                }
                data.snippet_groups.retain(|group| group.id != id);
                Ok(())
            })
        })
        .await
        .map_err(|_| CoreError::TaskFailed)?;
        match result {
            Err(VaultError::AlreadyExists) => Err(CoreError::GroupInUse),
            other => other.map_err(Into::into),
        }
    }
    pub async fn save_snippet(&self, snippet: Snippet, expected_revision: u64) -> Result<u64> {
        let group_ids = self
            .current()?
            .snapshot()?
            .snippet_groups
            .into_iter()
            .map(|group| group.id)
            .collect::<Vec<_>>();
        snippet.validate(&group_ids).map_err(CoreError::from)?;
        let vault = self.current()?;
        let id = snippet.id;
        task::spawn_blocking(move || {
            vault.transaction(expected_revision, move |data| {
                if let Some(existing) = data.snippets.iter_mut().find(|existing| existing.id == id)
                {
                    *existing = snippet;
                } else {
                    data.snippets.push(snippet);
                }
                Ok(())
            })
        })
        .await
        .map_err(|_| CoreError::TaskFailed)?
        .map_err(Into::into)
    }
    pub async fn delete_snippet(&self, id: Uuid, expected_revision: u64) -> Result<u64> {
        let vault = self.current()?;
        task::spawn_blocking(move || {
            vault.transaction(expected_revision, move |data| {
                let position = data
                    .snippets
                    .iter()
                    .position(|snippet| snippet.id == id)
                    .ok_or(VaultError::NotFound)?;
                data.snippets.remove(position);
                Ok(())
            })
        })
        .await
        .map_err(|_| CoreError::TaskFailed)?
        .map_err(Into::into)
    }
    pub async fn delete_group(&self, id: Uuid, expected_revision: u64) -> Result<u64> {
        let vault = self.current()?;
        let result = task::spawn_blocking(move || {
            vault.transaction(expected_revision, move |data| {
                if !data.groups.iter().any(|group| group.id == id) {
                    return Err(VaultError::NotFound);
                }
                if data.hosts.iter().any(|host| host.group_id == Some(id))
                    || data.groups.iter().any(|group| group.parent_id == Some(id))
                {
                    return Err(VaultError::AlreadyExists);
                }
                data.groups.retain(|group| group.id != id);
                Ok(())
            })
        })
        .await
        .map_err(|_| CoreError::TaskFailed)?;
        match result {
            Err(VaultError::AlreadyExists) => Err(CoreError::GroupInUse),
            other => other.map_err(Into::into),
        }
    }
    pub fn host_and_credential(&self, id: Uuid) -> Result<(Host, CredentialMaterial, u64, u64)> {
        let vault = self.current()?;
        let data = vault.snapshot()?;
        let host = data
            .hosts
            .into_iter()
            .find(|host| host.id == id)
            .ok_or(CoreError::NotFound)?;
        let credential = data
            .credentials
            .into_iter()
            .find(|credential| credential.id == host.credential_id)
            .ok_or(CoreError::NotFound)?;
        Ok((host, credential.material, credential.revision, self.epoch()))
    }
    async fn acquire_host_connection(
        &self,
        host_id: Uuid,
        attempt_id: Uuid,
        broker: Arc<capstan_ssh::TrustBroker>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<(
        Host,
        capstan_ssh::ConnectionLease<capstan_ssh::PasswordConnection>,
    )> {
        let (host, credential, credential_revision, vault_epoch) =
            self.host_and_credential(host_id)?;
        let authentication = match credential {
            CredentialMaterial::Password { value } => {
                ConnectMaterial::Password(SecretString::from(value.expose().to_owned()))
            }
            CredentialMaterial::ImportedKey {
                content,
                passphrase,
            } => ConnectMaterial::ImportedKey {
                content,
                passphrase,
            },
            CredentialMaterial::Agent { agent, fingerprint } => ConnectMaterial::Agent {
                provider: match agent {
                    AgentKind::OpenSsh => capstan_ssh::AgentProvider::OpenSsh,
                    AgentKind::Pageant => capstan_ssh::AgentProvider::Pageant,
                },
                fingerprint,
            },
            CredentialMaterial::KeyFile { path, passphrase } => {
                let path = PathBuf::from(path);
                let read_path = path.clone();
                let content = task::spawn_blocking(move || read_private_key_file(&read_path))
                    .await
                    .map_err(|_| CoreError::TaskFailed)??;
                capstan_ssh::validate_private_key(
                    content.expose(),
                    passphrase.as_ref().map(SecretText::expose),
                )?;
                ConnectMaterial::ImportedKey {
                    content,
                    passphrase,
                }
            }
        };
        if host.agent_forwarding && !matches!(&authentication, ConnectMaterial::Agent { .. }) {
            return Err(CoreError::InvalidInput);
        }
        let agent_forwarding = host.agent_forwarding;
        let binding = capstan_ssh::AttemptBinding {
            attempt_id,
            host_id: host.id,
            host_revision: host.revision,
            vault_epoch,
            target: capstan_vault::normalize_target(&host.address).map_err(CoreError::from)?,
            port: host.port,
        };
        let repository: Arc<dyn capstan_ssh::TrustRepository> = Arc::new(self.clone());
        let known = self.known_host_key(&binding.target, host.port)?;
        let is_unknown = known.is_none();
        let agent_identity = match &authentication {
            ConnectMaterial::Agent { fingerprint, .. } => Some(fingerprint.clone()),
            ConnectMaterial::Password(_) | ConnectMaterial::ImportedKey { .. } => None,
        };
        let pool_key = connection_pool_key(
            &host,
            credential_revision,
            vault_epoch,
            known
                .as_ref()
                .map(|key| key.fingerprint.clone())
                .unwrap_or_else(|| format!("unknown-{attempt_id}")),
            agent_identity.clone(),
        );
        let address = host.address.clone();
        let username = host.username.clone();
        let port = host.port;
        let broker_for_connect = broker.clone();
        let repository_for_connect = repository.clone();
        let binding_for_connect = binding.clone();
        let mut lease = self
            .ssh_pool
            .acquire_checked(
                pool_key,
                cancel.clone(),
                |connection| !connection.is_closed(),
                move |connection_cancel| async move {
                    match authentication {
                        ConnectMaterial::Password(password) => {
                            capstan_ssh::connect_password(
                                address,
                                port,
                                username,
                                password,
                                binding_for_connect,
                                repository_for_connect,
                                broker_for_connect,
                                connection_cancel,
                            )
                            .await
                        }
                        ConnectMaterial::ImportedKey {
                            content,
                            passphrase,
                        } => {
                            capstan_ssh::connect_private_key(
                                address,
                                port,
                                username,
                                content.expose(),
                                passphrase.as_ref().map(SecretText::expose),
                                binding_for_connect,
                                repository_for_connect,
                                broker_for_connect,
                                connection_cancel,
                            )
                            .await
                        }
                        ConnectMaterial::Agent {
                            provider,
                            fingerprint,
                        } => {
                            capstan_ssh::connect_agent(
                                address,
                                port,
                                username,
                                provider,
                                fingerprint,
                                agent_forwarding,
                                binding_for_connect,
                                repository_for_connect,
                                broker_for_connect,
                                connection_cancel,
                            )
                            .await
                        }
                    }
                },
            )
            .await?;
        if cancel.is_cancelled() || !repository.is_current(&binding).await {
            return Err(CoreError::Canceled);
        }
        if is_unknown {
            if let Some(known_key) = self.known_host_key(&binding.target, host.port)? {
                let trusted_key = connection_pool_key(
                    &host,
                    credential_revision,
                    vault_epoch,
                    known_key.fingerprint,
                    agent_identity,
                );
                self.ssh_pool.alias(&lease, trusted_key.clone()).await;
                lease = self
                    .ssh_pool
                    .acquire_checked(
                        trusted_key,
                        cancel.clone(),
                        |connection| !connection.is_closed(),
                        |_| async { Err(capstan_ssh::SshError::Connection) },
                    )
                    .await?;
            }
        }
        Ok((host, lease))
    }

    pub async fn connect_password_host_with_lease(
        &self,
        host_id: Uuid,
        attempt_id: Uuid,
        broker: Arc<capstan_ssh::TrustBroker>,
        cancel: CancellationToken,
    ) -> Result<(
        Host,
        capstan_ssh::SshShell,
        ConnectionLease<PasswordConnection>,
    )> {
        let entry = self.claim_connection(host_id, attempt_id, &cancel)?;
        let _tracked = entry.tasks.token();
        let result = async {
            let _gate = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(CoreError::Canceled),
                () = entry.stop.cancelled() => return Err(CoreError::Canceled),
                gate = entry.gate.lock() => gate,
            };
            let operation = {
                let mut record = entry.record.lock().map_err(|_| CoreError::TaskFailed)?;
                if matches!(
                    record.snapshot.state,
                    ConnectionState::Reconnecting | ConnectionState::Failed
                ) || (record.snapshot.state == ConnectionState::Online
                    && record
                        .transport
                        .upgrade()
                        .is_none_or(|connection| connection.is_closed()))
                {
                    if record.snapshot.state == ConnectionState::Online {
                        self.connections.publish(
                            &mut record,
                            ConnectionState::Reconnecting,
                            Some(ConnectionErrorCode::ConnectionLost),
                        );
                    }
                    self.connections
                        .next_generation(&entry, &mut record, attempt_id)?;
                }
                record.operation.clone()
            };
            let acquisition =
                self.acquire_host_connection(host_id, attempt_id, broker.clone(), cancel.clone());
            tokio::pin!(acquisition);
            let (host, lease) = tokio::select! {
                biased;
                () = entry.stop.cancelled() => { cancel.cancel(); acquisition.await? },
                () = operation.cancelled() => { cancel.cancel(); acquisition.await? },
                result = &mut acquisition => result?,
            };
            self.adopt_connection(&entry, &lease)?;
            // Automatic tunnels can outlive a rejected or canceled first shell.
            self.monitor_connection(entry.clone(), broker.clone());
            self.start_auto_tunnels(&entry, &lease).await;
            let shell = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(CoreError::Canceled),
                () = entry.stop.cancelled() => return Err(CoreError::Canceled),
                result = lease.open_shell(80, 30) => result?,
            };
            if cancel.is_cancelled() || !self.status().unlocked {
                let _ = shell.control.close().await;
                return Err(CoreError::Canceled);
            }
            if let Some(input) = startup_shell_input(&host) {
                if let Err(error) = shell.control.write(input).await {
                    let _ = shell.control.close().await;
                    return Err(error.into());
                }
            }
            self.start_server_status(&lease);
            Ok((host, shell, lease))
        }
        .await;
        if let Err(error) = &result {
            self.release_connection_owner(attempt_id);
            if let Ok(mut record) = entry.record.lock() {
                if record.snapshot.state != ConnectionState::Online {
                    let state = if record.owners.is_empty() {
                        ConnectionState::Failed
                    } else {
                        ConnectionState::Reconnecting
                    };
                    self.connections
                        .publish(&mut record, state, Some(connection_error(error)));
                }
            }
        }
        result
    }

    /// Opens a fresh login shell only on the specified authenticated generation.
    /// Configured startup commands and previous terminal input are never replayed.
    pub async fn open_reconnected_shell(
        &self,
        connection_id: Uuid,
        generation: u64,
        owner: Uuid,
        cancel: CancellationToken,
    ) -> Result<(
        Uuid,
        capstan_ssh::SshShell,
        ConnectionLease<PasswordConnection>,
    )> {
        let entry = self.managed_connection(connection_id)?;
        let _tracked = entry.tasks.token();
        let _gate = entry.gate.lock().await;
        let (host_id, policy) = {
            let mut record = entry.record.lock().map_err(|_| CoreError::TaskFailed)?;
            if record.snapshot.generation != generation
                || record.snapshot.state != ConnectionState::Online
                || entry.stop.is_cancelled()
            {
                return Err(CoreError::InvalidInput);
            }
            if self
                .connections
                .locked
                .load(std::sync::atomic::Ordering::Acquire)
            {
                return Err(CoreError::Locked);
            }
            record.owners.insert(owner, cancel.clone());
            (record.host_id, record.policy.clone())
        };
        let result = async {
            let lease = self
                .ssh_pool
                .acquire_checked(
                    policy,
                    cancel.clone(),
                    |connection| !connection.is_closed(),
                    |_| async { Err(capstan_ssh::SshError::Connection) },
                )
                .await?;
            if lease.connection().connection_generation()
                != entry
                    .record
                    .lock()
                    .map_err(|_| CoreError::TaskFailed)?
                    .transport_generation
            {
                return Err(CoreError::InvalidInput);
            }
            let shell = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(CoreError::Canceled),
                () = entry.stop.cancelled() => return Err(CoreError::Canceled),
                result = lease.open_shell(80, 30) => result?,
            };
            self.start_server_status(&lease);
            Ok((host_id, shell, lease))
        }
        .await;
        if result.is_err() {
            self.release_connection_owner(owner);
        }
        result
    }

    pub async fn connect_password_host(
        &self,
        host_id: Uuid,
        attempt_id: Uuid,
        broker: Arc<capstan_ssh::TrustBroker>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<(Host, capstan_ssh::SshShell)> {
        let (host, shell, _lease) = self
            .connect_password_host_with_lease(host_id, attempt_id, broker, cancel)
            .await?;
        Ok((host, shell))
    }
    pub async fn remember_host_key(
        &self,
        expected_epoch: u64,
        host_id: Uuid,
        host_revision: u64,
        trust: TrustRecord,
    ) -> Result<()> {
        if !self.epoch_matches(expected_epoch) {
            return Err(CoreError::Canceled);
        }
        let vault = self.current()?;
        let data = vault.snapshot()?;
        let host = data
            .hosts
            .iter()
            .find(|host| host.id == host_id)
            .ok_or(CoreError::NotFound)?;
        if host.revision != host_revision || host.address != trust.target || host.port != trust.port
        {
            return Err(CoreError::Canceled);
        }
        if let Some(existing) = data
            .trust
            .iter()
            .find(|record| record.target == trust.target && record.port == trust.port)
        {
            if existing.fingerprint != trust.fingerprint || existing.public_key != trust.public_key
            {
                return Err(CoreError::HostKeyChanged);
            }
            return Ok(());
        }
        let expected_revision = data.revision;
        let epoch = self.epoch.clone();
        task::spawn_blocking(move || {
            vault.transaction(expected_revision, move |data| {
                if epoch.load(std::sync::atomic::Ordering::Acquire) != expected_epoch {
                    return Err(VaultError::Canceled);
                }
                if data
                    .hosts
                    .iter()
                    .all(|host| host.id != host_id || host.revision != host_revision)
                {
                    return Err(VaultError::Canceled);
                }
                data.trust.push(trust);
                Ok(())
            })
        })
        .await
        .map_err(|_| CoreError::TaskFailed)?
        .map_err(CoreError::from)?;
        if !self.epoch_matches(expected_epoch) {
            return Err(CoreError::Canceled);
        }
        Ok(())
    }
    pub fn known_host_key(&self, target: &str, port: u16) -> Result<Option<TrustRecord>> {
        let data = self.current()?.snapshot()?;
        Ok(data
            .trust
            .into_iter()
            .find(|record| record.target == target && record.port == port))
    }
}

#[async_trait::async_trait]
impl capstan_ssh::TrustRepository for VaultManager {
    async fn authenticating(&self, attempt: &capstan_ssh::AttemptBinding) {
        if let Ok(entries) = self.connections.entries() {
            for entry in entries {
                if let Ok(mut record) = entry.record.lock() {
                    if record.attempt == Some(attempt.attempt_id) && !entry.stop.is_cancelled() {
                        self.connections.publish(
                            &mut record,
                            ConnectionState::Authenticating,
                            None,
                        );
                    }
                }
            }
        }
    }
    async fn lookup(
        &self,
        target: &str,
        port: u16,
    ) -> std::result::Result<Option<capstan_ssh::HostPublicKey>, capstan_ssh::SshError> {
        let record = self
            .known_host_key(target, port)
            .map_err(|_| capstan_ssh::SshError::Canceled)?;
        Ok(record.map(|record| capstan_ssh::HostPublicKey {
            algorithm: record.algorithm,
            public_key: record.public_key,
            fingerprint: record.fingerprint,
        }))
    }
    async fn is_current(&self, attempt: &capstan_ssh::AttemptBinding) -> bool {
        if !self.epoch_matches(attempt.vault_epoch) {
            return false;
        }
        let Ok(vault) = self.current() else {
            return false;
        };
        let Ok(data) = vault.snapshot() else {
            return false;
        };
        data.hosts.iter().any(|host| {
            host.id == attempt.host_id
                && host.revision == attempt.host_revision
                && host.port == attempt.port
                && capstan_vault::normalize_target(&host.address)
                    .is_ok_and(|address| address == attempt.target)
        })
    }
    async fn remember(
        &self,
        attempt: &capstan_ssh::AttemptBinding,
        key: &capstan_ssh::HostPublicKey,
    ) -> std::result::Result<(), capstan_ssh::SshError> {
        let trust = TrustRecord {
            target: attempt.target.clone(),
            port: attempt.port,
            algorithm: key.algorithm.clone(),
            public_key: key.public_key.clone(),
            fingerprint: key.fingerprint.clone(),
            first_trusted_at: unix_time_seconds(),
            revision: 1,
        };
        self.remember_host_key(
            attempt.vault_epoch,
            attempt.host_id,
            attempt.host_revision,
            trust,
        )
        .await
        .map_err(|error| {
            if matches!(error, CoreError::HostKeyChanged) {
                capstan_ssh::SshError::HostKeyChanged
            } else {
                capstan_ssh::SshError::Canceled
            }
        })
    }
}

pub fn unix_time_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn vault_manager_keeps_secrets_out_of_views_and_invalidates_epochs_on_lock() {
        let directory = tempfile::tempdir().unwrap();
        let manager = VaultManager::new(directory.path().join("vault.capstan"));
        let password = SecretText::new("master password for tests".into());
        manager
            .initialize(
                password,
                KdfParameters {
                    memory_kib: 19_456,
                    iterations: 2,
                    parallelism: 1,
                },
            )
            .await
            .unwrap();
        let credential_id = manager
            .add_password_credential("test".into(), "secret-marker".into())
            .await
            .unwrap();
        let revision = manager.revision().unwrap();
        let host = Host {
            id: Uuid::new_v4(),
            name: "loopback".into(),
            address: "127.0.0.1".into(),
            port: 22,
            username: "alice".into(),
            credential_id,
            group_id: None,
            tags: vec![],
            favorite: false,
            revision: 0,
            working_directory: None,
            startup_command: None,
            encoding: "utf-8".into(),
            theme: "graphite".into(),
            agent_forwarding: false,
            production: false,
        };
        manager.save_host(host, revision).await.unwrap();
        let views = serde_json::to_string(&manager.credentials().unwrap()).unwrap();
        assert!(!views.contains("secret-marker"));
        let epoch = manager.epoch();
        manager.lock().await.unwrap();
        assert!(!manager.epoch_matches(epoch));
        assert_eq!(manager.hosts().unwrap_err().code(), "vault_locked");
    }

    #[tokio::test]
    async fn authenticated_recovery_requires_explicit_selection_and_preserves_main_on_failure() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let manager = VaultManager::new(path.clone());
        let master = SecretText::new("recovery master password".into());
        let parameters = KdfParameters {
            memory_kib: 19_456,
            iterations: 2,
            parallelism: 1,
        };
        manager
            .initialize(master.clone(), parameters)
            .await
            .unwrap();
        manager
            .update_preferences(
                Preferences {
                    auto_lock_minutes: Some(10),
                    disconnect_on_lock: false,
                },
                0,
            )
            .await
            .unwrap();
        manager
            .update_preferences(
                Preferences {
                    auto_lock_minutes: None,
                    disconnect_on_lock: false,
                },
                1,
            )
            .await
            .unwrap();
        manager.lock().await.unwrap();

        std::fs::write(&path, b"damaged main vault").unwrap();
        let damaged_main = std::fs::read(&path).unwrap();
        assert!(manager
            .list_recovery_candidates(SecretText::new("wrong recovery password".into()))
            .await
            .unwrap()
            .is_empty());

        let candidates = manager
            .list_recovery_candidates(master.clone())
            .await
            .unwrap();
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.revision)
                .collect::<Vec<_>>(),
            vec![1, 0]
        );
        let serialized = serde_json::to_string(&candidates).unwrap();
        assert!(!serialized.contains("vault.capstan"));
        assert!(!serialized.contains("backup-"));

        assert_eq!(
            manager
                .restore_recovery_candidate(
                    candidates[0].token,
                    SecretText::new("wrong recovery password".into()),
                )
                .await
                .unwrap_err()
                .code(),
            "vault_authentication"
        );
        assert_eq!(std::fs::read(&path).unwrap(), damaged_main);
        assert!(!manager.status().unlocked);

        manager
            .restore_recovery_candidate(candidates[0].token, master.clone())
            .await
            .unwrap();
        assert!(manager.status().unlocked);
        assert_eq!(manager.revision().unwrap(), 1);
        assert_eq!(manager.preferences().unwrap().auto_lock_minutes, Some(10));

        manager.lock().await.unwrap();
        assert_eq!(
            manager
                .restore_recovery_candidate(candidates[0].token, master.clone())
                .await
                .unwrap_err()
                .code(),
            "vault_recovery_expired"
        );
        let reopened = VaultManager::new(path);
        reopened.unlock(master).await.unwrap();
        assert_eq!(reopened.revision().unwrap(), 1);
        assert_eq!(reopened.preferences().unwrap().auto_lock_minutes, Some(10));
    }

    #[tokio::test]
    async fn trusted_host_key_management_redacts_public_key_and_uses_revisioned_forget() {
        let directory = tempfile::tempdir().unwrap();
        let manager = VaultManager::new(directory.path().join("vault.capstan"));
        manager
            .initialize(
                SecretText::new("master password for tests".into()),
                KdfParameters {
                    memory_kib: 19_456,
                    iterations: 2,
                    parallelism: 1,
                },
            )
            .await
            .unwrap();
        let vault = manager.current().unwrap();
        vault
            .transaction(0, |data| {
                data.trust.push(TrustRecord {
                    target: "example.test".into(),
                    port: 2222,
                    algorithm: "ssh-ed25519".into(),
                    public_key: "sensitive-public-key-material".into(),
                    fingerprint: "SHA256:trusted-fingerprint".into(),
                    first_trusted_at: 1_800_000_000,
                    revision: 1,
                });
                Ok(())
            })
            .unwrap();

        let views = manager.trusted_host_keys().unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].target, "example.test");
        assert_eq!(views[0].port, 2222);
        let serialized = serde_json::to_string(&views).unwrap();
        assert!(!serialized.contains("sensitive-public-key-material"));
        let original_epoch = manager.epoch();

        assert_eq!(
            manager
                .forget_trusted_host_key(
                    "EXAMPLE.TEST.".into(),
                    2222,
                    "SHA256:stale-fingerprint".into(),
                    1,
                )
                .await
                .unwrap_err()
                .code(),
            "vault_conflict"
        );
        assert_eq!(manager.trusted_host_keys().unwrap().len(), 1);
        assert!(manager.epoch_matches(original_epoch));

        assert_eq!(
            manager
                .forget_trusted_host_key(
                    "EXAMPLE.TEST.".into(),
                    2222,
                    "SHA256:trusted-fingerprint".into(),
                    1,
                )
                .await
                .unwrap(),
            2
        );
        assert!(manager.trusted_host_keys().unwrap().is_empty());
        assert!(!manager.epoch_matches(original_epoch));
        assert!(manager
            .known_host_key("example.test", 2222)
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn credential_edits_preserve_identity_reject_stale_changes_and_guard_references() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let manager = VaultManager::new(path.clone());
        manager
            .initialize(
                SecretText::new("test master phrase".into()),
                KdfParameters {
                    memory_kib: 19_456,
                    iterations: 2,
                    parallelism: 1,
                },
            )
            .await
            .unwrap();
        let id = manager
            .add_password_credential("original".into(), "old-secret".into())
            .await
            .unwrap();
        manager
            .rename_credential(id, "renamed".into(), 1)
            .await
            .unwrap();
        let epoch = manager.epoch();
        assert_eq!(
            manager
                .save_password_credential(
                    "renamed".into(),
                    "new-secret".into(),
                    Some(CredentialUpdate {
                        id,
                        expected_revision: 2,
                    })
                )
                .await
                .unwrap(),
            id
        );
        assert!(!manager.epoch_matches(epoch));
        assert_eq!(manager.credentials().unwrap()[0].revision, 2);
        assert_eq!(
            manager
                .rename_credential(id, "stale".into(), 2)
                .await
                .unwrap_err()
                .code(),
            "vault_conflict"
        );
        let reopened = Vault::unlock(&path, &SecretText::new("test master phrase".into())).unwrap();
        let credentials = reopened.snapshot().unwrap().credentials;
        assert_eq!(credentials.len(), 1);
        assert!(
            matches!(&credentials[0].material, CredentialMaterial::Password { value } if value.expose() == "new-secret")
        );
        assert!(!serde_json::to_string(&manager.credentials().unwrap())
            .unwrap()
            .contains("new-secret"));
        let host_id = Uuid::new_v4();
        manager
            .save_host(
                Host {
                    id: host_id,
                    name: "test".into(),
                    address: "example.test".into(),
                    port: 22,
                    username: "test".into(),
                    credential_id: id,
                    group_id: None,
                    tags: vec![],
                    favorite: false,
                    revision: 0,
                    working_directory: None,
                    startup_command: None,
                    encoding: "utf-8".into(),
                    theme: "graphite".into(),
                    agent_forwarding: false,
                    production: false,
                },
                3,
            )
            .await
            .unwrap();
        assert_eq!(
            manager.delete_credential(id, 4).await.unwrap_err().code(),
            "credential_in_use"
        );
        assert_eq!(manager.credentials().unwrap().len(), 1);
        manager.delete_host(host_id, 4).await.unwrap();
        assert_eq!(
            manager.delete_credential(id, 4).await.unwrap_err().code(),
            "vault_conflict"
        );
        manager.delete_credential(id, 5).await.unwrap();
        assert!(manager.credentials().unwrap().is_empty());
    }

    #[tokio::test]
    async fn agent_forwarding_can_only_be_saved_for_an_agent_credential() {
        let directory = tempfile::tempdir().unwrap();
        let manager = VaultManager::new(directory.path().join("vault.capstan"));
        manager
            .initialize(
                SecretText::new("master password for tests".into()),
                KdfParameters {
                    memory_kib: 19_456,
                    iterations: 2,
                    parallelism: 1,
                },
            )
            .await
            .unwrap();
        let password_credential = manager
            .add_password_credential("password".into(), "secret-marker".into())
            .await
            .unwrap();
        let host_id = Uuid::new_v4();
        let forwarding_host = Host {
            id: host_id,
            name: "forwarding target".into(),
            address: "example.test".into(),
            port: 22,
            username: "alice".into(),
            credential_id: password_credential,
            group_id: None,
            tags: vec![],
            favorite: false,
            revision: 0,
            working_directory: None,
            startup_command: None,
            encoding: "utf-8".into(),
            theme: "graphite".into(),
            agent_forwarding: true,
            production: false,
        };
        let revision = manager.revision().unwrap();
        assert_eq!(
            manager
                .save_host(forwarding_host.clone(), revision)
                .await
                .unwrap_err()
                .code(),
            "invalid_input"
        );
        assert!(manager.hosts().unwrap().is_empty());

        let agent_credential = Credential {
            id: Uuid::new_v4(),
            name: "agent".into(),
            revision: 0,
            material: CredentialMaterial::Agent {
                agent: AgentKind::OpenSsh,
                fingerprint: "SHA256:test-key".into(),
            },
        };
        let agent_credential_id = agent_credential.id;
        manager.add_credential(agent_credential).await.unwrap();
        let mut forwarding_host = forwarding_host;
        forwarding_host.credential_id = agent_credential_id;
        let revision = manager.revision().unwrap();
        manager.save_host(forwarding_host, revision).await.unwrap();
        assert_eq!(manager.hosts().unwrap().len(), 1);
        assert!(manager.hosts().unwrap()[0].agent_forwarding);
    }

    #[test]
    fn agent_forwarding_policy_separates_connection_pool_keys() {
        let credential_id = Uuid::new_v4();
        let mut host = Host {
            id: Uuid::new_v4(),
            name: "host".into(),
            address: "example.test".into(),
            port: 22,
            username: "alice".into(),
            credential_id,
            group_id: None,
            tags: vec![],
            favorite: false,
            revision: 0,
            working_directory: None,
            startup_command: None,
            encoding: "utf-8".into(),
            theme: "graphite".into(),
            agent_forwarding: false,
            production: false,
        };
        let disabled = connection_pool_key(&host, 0, 1, "SHA256:host".into(), None);
        host.agent_forwarding = true;
        let enabled = connection_pool_key(&host, 0, 1, "SHA256:host".into(), None);
        assert_ne!(disabled, enabled);
    }

    #[tokio::test]
    async fn group_updates_reject_cycles_and_deletion_preserves_children() {
        let directory = tempfile::tempdir().unwrap();
        let manager = VaultManager::new(directory.path().join("vault.capstan"));
        manager
            .initialize(
                SecretText::new("master password for tests".into()),
                KdfParameters {
                    memory_kib: 19_456,
                    iterations: 2,
                    parallelism: 1,
                },
            )
            .await
            .unwrap();
        let parent_id = Uuid::new_v4();
        let child_id = Uuid::new_v4();
        let revision = manager.revision().unwrap();
        manager
            .save_group(
                Group {
                    id: parent_id,
                    name: "Engineering".into(),
                    parent_id: None,
                    production: false,
                },
                revision,
            )
            .await
            .unwrap();
        let revision = manager.revision().unwrap();
        manager
            .save_group(
                Group {
                    id: child_id,
                    name: "Build".into(),
                    parent_id: Some(parent_id),
                    production: false,
                },
                revision,
            )
            .await
            .unwrap();

        let revision = manager.revision().unwrap();
        let cycle = manager
            .save_group(
                Group {
                    id: parent_id,
                    name: "Engineering".into(),
                    parent_id: Some(child_id),
                    production: false,
                },
                revision,
            )
            .await;
        assert_eq!(cycle.unwrap_err().code(), "invalid_input");
        assert_eq!(manager.revision().unwrap(), revision);
        assert_eq!(
            manager
                .delete_group(parent_id, revision)
                .await
                .unwrap_err()
                .code(),
            "group_in_use"
        );

        let revision = manager.revision().unwrap();
        manager.delete_group(child_id, revision).await.unwrap();
        let revision = manager.revision().unwrap();
        manager.delete_group(parent_id, revision).await.unwrap();
        assert!(manager.groups().unwrap().is_empty());
    }

    #[tokio::test]
    async fn vault_preferences_are_revisioned_validated_and_persisted() {
        let directory = tempfile::tempdir().unwrap();
        let manager = VaultManager::new(directory.path().join("vault.capstan"));
        manager
            .initialize(
                SecretText::new("master password for tests".into()),
                KdfParameters {
                    memory_kib: 19_456,
                    iterations: 2,
                    parallelism: 1,
                },
            )
            .await
            .unwrap();
        assert_eq!(manager.preferences().unwrap().auto_lock_minutes, Some(30));
        assert!(!manager.preferences().unwrap().disconnect_on_lock);

        let revision = manager.revision().unwrap();
        let preferences = Preferences {
            auto_lock_minutes: None,
            disconnect_on_lock: true,
        };
        assert_eq!(
            manager
                .update_preferences(preferences, revision)
                .await
                .unwrap(),
            revision + 1
        );
        assert_eq!(manager.preferences().unwrap().auto_lock_minutes, None);
        assert!(manager.preferences().unwrap().disconnect_on_lock);

        let revision = manager.revision().unwrap();
        assert_eq!(
            manager
                .update_preferences(
                    Preferences {
                        auto_lock_minutes: Some(0),
                        disconnect_on_lock: false,
                    },
                    revision,
                )
                .await
                .unwrap_err()
                .code(),
            "invalid_input"
        );
        assert_eq!(manager.revision().unwrap(), revision);
        assert_eq!(
            manager
                .update_preferences(Preferences::default(), revision - 1)
                .await
                .unwrap_err()
                .code(),
            "vault_conflict"
        );

        manager.lock().await.unwrap();
        let reopened = VaultManager::new(manager.path().to_path_buf());
        reopened
            .unlock(SecretText::new("master password for tests".into()))
            .await
            .unwrap();
        assert!(reopened.preferences().unwrap().disconnect_on_lock);
        assert_eq!(reopened.preferences().unwrap().auto_lock_minutes, None);
    }

    #[tokio::test]
    async fn snippets_are_revisioned_encrypted_and_reject_deleting_referenced_groups() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let manager = VaultManager::new(path.clone());
        manager
            .initialize(
                SecretText::new("master password for tests".into()),
                KdfParameters {
                    memory_kib: 19_456,
                    iterations: 2,
                    parallelism: 1,
                },
            )
            .await
            .unwrap();

        let group = SnippetGroup {
            id: Uuid::new_v4(),
            name: "Operations".into(),
        };
        assert_eq!(
            manager.save_snippet_group(group.clone(), 0).await.unwrap(),
            1
        );
        let snippet = Snippet {
            id: Uuid::new_v4(),
            group_id: Some(group.id),
            name: "List pods".into(),
            template: "kubectl get pods -n {{namespace:default}}".into(),
            auto_enter: true,
        };
        assert_eq!(manager.save_snippet(snippet.clone(), 1).await.unwrap(), 2);
        assert_eq!(manager.snippet_groups().unwrap()[0].id, group.id);
        assert_eq!(manager.snippets().unwrap()[0].template, snippet.template);
        assert_eq!(
            manager
                .delete_snippet_group(group.id, 2)
                .await
                .unwrap_err()
                .code(),
            "group_in_use"
        );
        assert_eq!(manager.revision().unwrap(), 2);
        assert_eq!(
            manager
                .save_snippet(snippet.clone(), 1)
                .await
                .unwrap_err()
                .code(),
            "vault_conflict"
        );

        let mut edited = snippet.clone();
        edited.auto_enter = false;
        edited.template = "kubectl get pods -n {{namespace}}".into();
        assert_eq!(manager.save_snippet(edited, 2).await.unwrap(), 3);
        assert!(!manager.snippets().unwrap()[0].auto_enter);
        assert_eq!(manager.delete_snippet(snippet.id, 3).await.unwrap(), 4);
        assert_eq!(manager.delete_snippet_group(group.id, 4).await.unwrap(), 5);

        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes
            .windows(b"kubectl get pods".len())
            .any(|window| window == b"kubectl get pods"));
        manager.lock().await.unwrap();
        let reopened = VaultManager::new(path);
        reopened
            .unlock(SecretText::new("master password for tests".into()))
            .await
            .unwrap();
        assert!(reopened.snippet_groups().unwrap().is_empty());
        assert!(reopened.snippets().unwrap().is_empty());
    }

    #[tokio::test]
    async fn imported_private_key_is_encrypted_and_exposed_only_as_a_credential_view() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let manager = VaultManager::new(path.clone());
        manager
            .initialize(
                SecretText::new("master password for tests".into()),
                KdfParameters {
                    memory_kib: 19_456,
                    iterations: 2,
                    parallelism: 1,
                },
            )
            .await
            .unwrap();
        let private_key =
            russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)
                .unwrap();
        let private_key_text = private_key
            .to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .unwrap()
            .to_string();
        let id = manager
            .add_imported_key_credential("test key".into(), private_key_text.clone(), None)
            .await
            .unwrap();
        let views = manager.credentials().unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].id, id);
        assert_eq!(views[0].kind, "imported_key");
        assert!(!serde_json::to_string(&views)
            .unwrap()
            .contains(&private_key_text));
        let encrypted = std::fs::read(&path).unwrap();
        assert!(!encrypted
            .windows(private_key_text.len())
            .any(|window| window == private_key_text.as_bytes()));

        let invalid = manager
            .add_imported_key_credential("bad key".into(), "not a private key".into(), None)
            .await
            .unwrap_err();
        assert_eq!(invalid.code(), "ssh_private_key_invalid");
        assert_eq!(manager.credentials().unwrap().len(), 1);

        manager.lock().await.unwrap();
        let reopened = VaultManager::new(path);
        reopened
            .unlock(SecretText::new("master password for tests".into()))
            .await
            .unwrap();
        assert_eq!(reopened.credentials().unwrap()[0].id, id);
    }

    #[test]
    fn backup_merge_remaps_references_renames_collisions_and_reports_trust_changes() {
        let current_credential = Uuid::new_v4();
        let current_group = Uuid::new_v4();
        let current_host = Uuid::new_v4();
        let mut current = VaultData::default();
        current.credentials.push(Credential {
            id: current_credential,
            name: "Shared key".into(),
            revision: 0,
            material: CredentialMaterial::Password {
                value: SecretText::new("secret current password".into()),
            },
        });
        current.groups.push(Group {
            id: current_group,
            name: "Team".into(),
            parent_id: None,
            production: false,
        });
        current.hosts.push(Host {
            id: current_host,
            name: "Existing".into(),
            address: "shared.example.test".into(),
            port: 22,
            username: "alice".into(),
            credential_id: current_credential,
            group_id: Some(current_group),
            tags: vec![],
            favorite: false,
            revision: 0,
            working_directory: None,
            startup_command: None,
            encoding: "utf-8".into(),
            theme: "graphite".into(),
            agent_forwarding: false,
            production: false,
        });

        let incoming_credential = Uuid::new_v4();
        let incoming_group = Uuid::new_v4();
        let mut imported = VaultData::default();
        imported.credentials.push(Credential {
            id: incoming_credential,
            name: "Shared key".into(),
            revision: 0,
            material: CredentialMaterial::Password {
                value: SecretText::new("secret imported password".into()),
            },
        });
        imported.groups.push(Group {
            id: incoming_group,
            name: "Team".into(),
            parent_id: None,
            production: false,
        });
        let incoming_snippet_group = Uuid::new_v4();
        let incoming_snippet_id = Uuid::new_v4();
        imported.snippet_groups.push(SnippetGroup {
            id: incoming_snippet_group,
            name: "Commands".into(),
        });
        imported.snippets.push(Snippet {
            id: incoming_snippet_id,
            group_id: Some(incoming_snippet_group),
            name: "List pods".into(),
            template: "kubectl get pods".into(),
            auto_enter: true,
        });
        imported.hosts.push(Host {
            id: Uuid::new_v4(),
            name: "Duplicate target".into(),
            address: "SHARED.example.test.".into(),
            port: 22,
            username: "alice".into(),
            credential_id: incoming_credential,
            group_id: Some(incoming_group),
            tags: vec![],
            favorite: false,
            revision: 0,
            working_directory: None,
            startup_command: None,
            encoding: "utf-8".into(),
            theme: "graphite".into(),
            agent_forwarding: false,
            production: false,
        });
        imported.hosts.push(Host {
            id: Uuid::new_v4(),
            name: "New host".into(),
            address: "new.example.test".into(),
            port: 2222,
            username: "bob".into(),
            credential_id: incoming_credential,
            group_id: Some(incoming_group),
            tags: vec![],
            favorite: false,
            revision: 0,
            working_directory: None,
            startup_command: None,
            encoding: "utf-8".into(),
            theme: "graphite".into(),
            agent_forwarding: false,
            production: false,
        });
        imported.tunnels.push(capstan_vault::TunnelConfig {
            id: Uuid::new_v4(),
            host_id: imported.hosts[0].id,
            auto_start: true,
            forward: capstan_vault::TunnelForward::Socks5 {
                bind_host: "127.0.0.1".into(),
                bind_port: 1080,
            },
        });

        let preview = build_backup_preview(&current, &imported);
        assert_eq!(preview.duplicate_hosts.len(), 1);
        assert_eq!(preview.duplicate_hosts[0].target, "shared.example.test");
        assert!(preview.trust_conflicts.is_empty());

        let merged = merge_backup_data(&current, imported.clone()).unwrap();
        assert_eq!(merged.hosts.len(), 2);
        assert_eq!(merged.groups.len(), 2);
        assert_eq!(merged.credentials.len(), 2);
        assert_eq!(merged.snippets.len(), 1);
        assert_eq!(merged.snippet_groups.len(), 1);
        assert_ne!(merged.snippets[0].id, incoming_snippet_id);
        assert_ne!(merged.snippet_groups[0].id, incoming_snippet_group);
        assert_eq!(
            merged.snippets[0].group_id,
            Some(merged.snippet_groups[0].id)
        );
        assert_eq!(merged.tunnels.len(), 1);
        assert_eq!(merged.tunnels[0].host_id, current.hosts[0].id);

        // Importing the same backup again adds no copies of M3 data, while a
        // snippet that differs is kept in the existing group.
        let again = merge_backup_data(&merged, imported.clone()).unwrap();
        assert_eq!(again.snippet_groups.len(), 1);
        assert_eq!(again.snippets.len(), 1);
        assert_eq!(again.tunnels.len(), 1);
        let mut edited = imported.clone();
        edited.snippets[0].template = "kubectl get pods -A".into();
        let edited = merge_backup_data(&again, edited).unwrap();
        assert_eq!(edited.snippet_groups.len(), 1);
        assert_eq!(edited.snippets.len(), 2);
        assert!(edited
            .snippets
            .iter()
            .all(|snippet| snippet.group_id == Some(edited.snippet_groups[0].id)));

        let new_host = merged
            .hosts
            .iter()
            .find(|host| host.address == "new.example.test")
            .unwrap();
        assert_ne!(new_host.credential_id, incoming_credential);
        assert_ne!(new_host.group_id, Some(incoming_group));
        assert!(merged
            .groups
            .iter()
            .any(|group| group.name == "Team (imported 2)"));
        assert!(merged
            .credentials
            .iter()
            .any(|credential| { credential.name == "Shared key (imported 2)" }));

        let old_trust = TrustRecord {
            target: "shared.example.test".into(),
            port: 22,
            algorithm: "ssh-ed25519".into(),
            public_key: "ssh-ed25519 AAAA-old".into(),
            fingerprint: "SHA256:old".into(),
            first_trusted_at: 1,
            revision: 1,
        };
        current.trust.push(old_trust);
        imported.trust.push(TrustRecord {
            target: "shared.example.test".into(),
            port: 22,
            algorithm: "ssh-ed25519".into(),
            public_key: "ssh-ed25519 AAAA-new".into(),
            fingerprint: "SHA256:new".into(),
            first_trusted_at: 2,
            revision: 1,
        });
        let preview = build_backup_preview(&current, &imported);
        assert_eq!(preview.trust_conflicts.len(), 1);
        assert_eq!(
            preview.trust_conflicts[0].existing_fingerprint,
            "SHA256:old"
        );
        assert_eq!(
            preview.trust_conflicts[0].incoming_fingerprint,
            "SHA256:new"
        );
    }

    #[tokio::test]
    async fn changing_master_password_invalidates_the_old_password_after_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let manager = VaultManager::new(path.clone());
        manager
            .initialize(
                SecretText::new("old master password for tests".into()),
                KdfParameters {
                    memory_kib: 19_456,
                    iterations: 2,
                    parallelism: 1,
                },
            )
            .await
            .unwrap();
        let revision = manager.revision().unwrap();
        manager
            .change_master_password(
                SecretText::new("new master password for tests".into()),
                revision,
            )
            .await
            .unwrap();
        manager.lock().await.unwrap();

        let reopened = VaultManager::new(path);
        assert_eq!(
            reopened
                .unlock(SecretText::new("old master password for tests".into()))
                .await
                .unwrap_err()
                .code(),
            "vault_authentication"
        );
        reopened
            .unlock(SecretText::new("new master password for tests".into()))
            .await
            .unwrap();
        assert_eq!(reopened.revision().unwrap(), revision);
    }

    #[tokio::test]
    async fn backup_preview_and_merge_commit_only_after_validating_the_separate_password() {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.capstan");
        let backup_path = directory.path().join("transfer.cbak");
        let target_path = directory.path().join("target.capstan");
        let fast = KdfParameters {
            memory_kib: 19_456,
            iterations: 2,
            parallelism: 1,
        };
        let master = SecretText::new("source master password for tests".into());
        let source = VaultManager::new(source_path.clone());
        source.initialize(master.clone(), fast).await.unwrap();
        let credential_id = source
            .add_password_credential("Imported login".into(), "remote password marker".into())
            .await
            .unwrap();
        let group_id = Uuid::new_v4();
        let revision = source.revision().unwrap();
        source
            .save_group(
                Group {
                    id: group_id,
                    name: "Imported group".into(),
                    parent_id: None,
                    production: false,
                },
                revision,
            )
            .await
            .unwrap();
        let revision = source.revision().unwrap();
        source
            .save_host(
                Host {
                    id: Uuid::new_v4(),
                    name: "Imported server".into(),
                    address: "imported.example.test".into(),
                    port: 2222,
                    username: "alice".into(),
                    credential_id,
                    group_id: Some(group_id),
                    tags: vec!["migration".into()],
                    favorite: true,
                    revision: 0,
                    working_directory: None,
                    startup_command: None,
                    encoding: "utf-8".into(),
                    theme: "graphite".into(),
                    agent_forwarding: false,
                    production: false,
                },
                revision,
            )
            .await
            .unwrap();
        source.lock().await.unwrap();
        let unlocked_source = Vault::unlock(source_path, &master).unwrap();
        let backup_password = SecretText::new("separate backup phrase for tests".into());
        unlocked_source
            .export_backup(&backup_password, fast, &backup_path)
            .unwrap();

        let target = VaultManager::new(target_path.clone());
        target
            .initialize(
                SecretText::new("target master password for tests".into()),
                fast,
            )
            .await
            .unwrap();
        let expected_revision = target.revision().unwrap();
        let expected_epoch = target.epoch();
        assert_eq!(
            target
                .preview_backup(
                    backup_path.clone(),
                    SecretText::new("incorrect separate phrase".into()),
                )
                .await
                .unwrap_err()
                .code(),
            "vault_authentication"
        );
        assert_eq!(target.revision().unwrap(), expected_revision);

        let preview = target
            .preview_backup(backup_path.clone(), backup_password.clone())
            .await
            .unwrap();
        assert_eq!(preview.host_count, 1);
        assert_eq!(preview.group_count, 1);
        assert_eq!(preview.credential_count, 1);
        target
            .import_backup(
                backup_path,
                backup_password,
                BackupImportMode::Merge,
                expected_revision,
                expected_epoch,
            )
            .await
            .unwrap();
        assert_eq!(target.hosts().unwrap()[0].name, "Imported server");
        assert_eq!(target.groups().unwrap()[0].name, "Imported group");
        assert_eq!(target.credentials().unwrap()[0].kind, "password");

        target.lock().await.unwrap();
        let reopened = VaultManager::new(target_path);
        reopened
            .unlock(SecretText::new("target master password for tests".into()))
            .await
            .unwrap();
        assert_eq!(
            reopened.hosts().unwrap()[0].address,
            "imported.example.test"
        );
    }

    #[tokio::test]
    async fn referenced_key_files_are_revalidated_on_use_and_not_returned_in_views() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let key_path = directory.path().join("id_ed25519");
        let manager = VaultManager::new(path);
        manager
            .initialize(
                SecretText::new("master password for tests".into()),
                KdfParameters {
                    memory_kib: 19_456,
                    iterations: 2,
                    parallelism: 1,
                },
            )
            .await
            .unwrap();
        let key =
            russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)
                .unwrap()
                .to_openssh(russh::keys::ssh_key::LineEnding::LF)
                .unwrap()
                .to_string();
        std::fs::write(&key_path, &key).unwrap();
        let credential_id = manager
            .add_key_file_credential("Machine key".into(), key_path.clone(), None)
            .await
            .unwrap();
        let views = manager.credentials().unwrap();
        assert_eq!(views[0].id, credential_id);
        assert_eq!(views[0].kind, "key_file");
        assert!(!serde_json::to_string(&views)
            .unwrap()
            .contains(key_path.to_string_lossy().as_ref()));

        let revision = manager.revision().unwrap();
        let host_id = Uuid::new_v4();
        manager
            .save_host(
                Host {
                    id: host_id,
                    name: "File key server".into(),
                    address: "file-key.example.test".into(),
                    port: 22,
                    username: "alice".into(),
                    credential_id,
                    group_id: None,
                    tags: vec![],
                    favorite: false,
                    revision: 0,
                    working_directory: None,
                    startup_command: None,
                    encoding: "utf-8".into(),
                    theme: "graphite".into(),
                    agent_forwarding: false,
                    production: false,
                },
                revision,
            )
            .await
            .unwrap();
        std::fs::remove_file(key_path).unwrap();
        let (broker, _) = capstan_ssh::TrustBroker::new();
        assert_eq!(
            manager
                .connect_password_host(
                    host_id,
                    Uuid::new_v4(),
                    broker,
                    tokio_util::sync::CancellationToken::new(),
                )
                .await
                .err()
                .expect("missing key file should block SSH startup")
                .code(),
            "credential_file"
        );
    }

    #[test]
    fn remote_startup_input_quotes_directories_and_combines_each_setting_once() {
        let mut host = Host {
            id: Uuid::new_v4(),
            name: "shell".into(),
            address: "example.test".into(),
            port: 22,
            username: "alice".into(),
            credential_id: Uuid::new_v4(),
            group_id: None,
            tags: vec![],
            favorite: false,
            revision: 0,
            working_directory: Some("/srv/user's files".into()),
            startup_command: Some("printf 'ready\\n'".into()),
            encoding: "utf-8".into(),
            theme: "graphite".into(),
            agent_forwarding: false,
            production: false,
        };
        assert_eq!(
            startup_shell_input(&host).as_deref(),
            Some(b"if cd -- '/srv/user'\\''s files'; then\nprintf 'ready\\n'\nfi\n".as_slice()),
        );
        host.working_directory = None;
        assert_eq!(
            startup_shell_input(&host).as_deref(),
            Some(b"printf 'ready\\n'\n".as_slice())
        );
        host.startup_command = None;
        assert_eq!(startup_shell_input(&host), None);
    }
}

#[cfg(test)]
mod hello_workflow_tests {
    use super::*;
    use capstan_vault::hello::{HelloError, HelloProvider, HelloResult};
    use zeroize::Zeroizing;

    type DeviceKeys = Mutex<HashMap<String, (Vec<u8>, Zeroizing<[u8; 32]>)>>;

    #[derive(Default)]
    struct Device {
        keys: DeviceKeys,
        canceled: std::sync::atomic::AtomicBool,
        gate: Mutex<Option<Arc<ConsentGate>>>,
    }
    #[derive(Default)]
    struct ConsentGate {
        entered: tokio::sync::Notify,
        released: Mutex<bool>,
        changed: std::sync::Condvar,
    }
    impl ConsentGate {
        fn release(&self) {
            *self.released.lock().unwrap() = true;
            self.changed.notify_all();
        }
    }
    impl HelloProvider for Device {
        fn available(&self) -> bool {
            true
        }
        fn protect(&self, name: &str, secret: &[u8; 32]) -> HelloResult<Vec<u8>> {
            let blob = vec![0; 256];
            self.keys
                .lock()
                .unwrap()
                .insert(name.into(), (blob.clone(), Zeroizing::new(*secret)));
            Ok(blob)
        }
        fn unprotect(&self, name: &str, blob: &[u8]) -> HelloResult<Zeroizing<[u8; 32]>> {
            if let Some(gate) = self.gate.lock().unwrap().take() {
                gate.entered.notify_one();
                let mut released = gate.released.lock().unwrap();
                while !*released {
                    released = gate.changed.wait(released).unwrap();
                }
            }
            if self.canceled.load(std::sync::atomic::Ordering::Acquire) {
                return Err(HelloError::Canceled);
            }
            let keys = self.keys.lock().unwrap();
            let (expected, secret) = keys.get(name).ok_or(HelloError::KeyMissing)?;
            if blob != expected {
                return Err(HelloError::Data);
            }
            Ok(secret.clone())
        }
        fn exists(&self, name: &str) -> HelloResult<bool> {
            Ok(self.keys.lock().unwrap().contains_key(name))
        }
        fn delete(&self, name: &str) -> HelloResult<()> {
            self.keys.lock().unwrap().remove(name);
            Ok(())
        }
    }
    fn password() -> SecretText {
        SecretText::new("core Hello test master password".into())
    }
    fn fast() -> KdfParameters {
        KdfParameters {
            memory_kib: 19_456,
            iterations: 2,
            parallelism: 1,
        }
    }

    #[tokio::test]
    async fn hello_unlock_obeys_lock_epochs_and_password_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let manager = VaultManager::new(directory.path().join("vault.capstan"));
        let device = Arc::new(Device::default());
        manager.initialize(password(), fast()).await.unwrap();
        manager
            .enable_hello(password(), device.clone())
            .await
            .unwrap();
        let epoch = manager.epoch();
        manager.lock().await.unwrap();
        device
            .canceled
            .store(true, std::sync::atomic::Ordering::Release);
        assert_eq!(
            manager
                .unlock_hello(device.clone())
                .await
                .unwrap_err()
                .code(),
            "hello_canceled"
        );
        assert!(!manager.status().unlocked);
        assert_eq!(manager.epoch(), epoch + 1);
        assert!(capstan_vault::hello::sidecar_path(manager.path()).exists());
        device
            .canceled
            .store(false, std::sync::atomic::Ordering::Release);
        manager.unlock_hello(device.clone()).await.unwrap();
        assert!(manager.status().unlocked);
        assert_eq!(manager.epoch(), epoch + 2);
        manager
            .current()
            .unwrap()
            .transaction(0, |_| Ok(()))
            .unwrap();
        assert_eq!(manager.revision().unwrap(), 1);
        assert!(matches!(
            manager.unlock_hello(device).await,
            Err(CoreError::Conflict)
        ));
        manager.lock().await.unwrap();
        manager.unlock(password()).await.unwrap();
        assert_eq!(manager.revision().unwrap(), 1);
    }
    #[tokio::test]
    async fn corrupt_hello_is_cleaned_only_after_successful_master_unlock() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let manager = VaultManager::new(path.clone());
        manager.initialize(password(), fast()).await.unwrap();
        let original = std::fs::read(&path).unwrap();
        let sidecar = capstan_vault::hello::sidecar_path(&path);
        std::fs::write(&sidecar, b"corrupt Hello enrollment").unwrap();
        manager.lock().await.unwrap();
        assert!(manager
            .unlock(SecretText::new("wrong".into()))
            .await
            .is_err());
        assert!(sidecar.exists());
        assert!(!manager.status().unlocked);
        manager.unlock(password()).await.unwrap();
        assert!(manager.status().unlocked);
        assert!(!sidecar.exists());
        assert!(manager.hello_status().await.unwrap().invalidated);
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[tokio::test]
    async fn recovery_cleans_incompatible_hello_before_master_password_change() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let manager = VaultManager::new(path.clone());
        let device = Arc::new(Device::default());
        manager.initialize(password(), fast()).await.unwrap();
        let original = std::fs::read(&path).unwrap();
        manager
            .enable_hello(password(), device.clone())
            .await
            .unwrap();
        for next in ["second master password", "third master password"] {
            manager
                .current()
                .unwrap()
                .change_master_password(0, &SecretText::new(next.into()), fast())
                .unwrap();
        }
        manager.lock().await.unwrap();
        let epoch = manager.epoch();
        let sidecar = capstan_vault::hello::sidecar_path(&path);
        let enrolled = std::fs::read(&sidecar).unwrap();
        let before_restore = std::fs::read(&path).unwrap();
        let candidates = manager.list_recovery_candidates(password()).await.unwrap();
        assert_eq!(candidates.len(), 1);
        let token = candidates[0].token;

        assert_eq!(
            manager
                .restore_recovery_candidate(token, SecretText::new("wrong".into()))
                .await
                .unwrap_err()
                .code(),
            "vault_authentication"
        );
        assert!(!manager.status().unlocked);
        assert_eq!(manager.epoch(), epoch);
        assert!(!manager.hello_status().await.unwrap().invalidated);
        assert_eq!(std::fs::read(&sidecar).unwrap(), enrolled);
        assert_eq!(std::fs::read(&path).unwrap(), before_restore);

        manager
            .restore_recovery_candidate(token, password())
            .await
            .unwrap();
        assert!(manager.status().unlocked);
        assert_eq!(manager.epoch(), epoch + 1);
        assert!(!manager
            .connections
            .locked
            .load(std::sync::atomic::Ordering::Acquire));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(!sidecar.exists());
        let status = manager.hello_status().await.unwrap();
        assert!(!status.enabled);
        assert!(status.invalidated);
        assert_eq!(device.keys.lock().unwrap().len(), 1);

        let replacement = SecretText::new("fourth master password".into());
        manager
            .change_master_password(replacement.clone(), manager.revision().unwrap())
            .await
            .unwrap();
        assert!(Vault::unlock(&path, &replacement).is_ok());
        assert!(Vault::unlock(&path, &password()).is_err());

        // A later authenticated restore without an enrollment clears the old notice.
        manager.lock().await.unwrap();
        let candidates = manager.list_recovery_candidates(password()).await.unwrap();
        manager
            .restore_recovery_candidate(candidates[0].token, password())
            .await
            .unwrap();
        assert!(!manager.hello_status().await.unwrap().invalidated);
    }

    #[tokio::test]
    async fn recovery_remains_unlocked_and_reports_failed_hello_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let manager = VaultManager::new(path.clone());
        manager.initialize(password(), fast()).await.unwrap();
        let original = std::fs::read(&path).unwrap();
        manager
            .current()
            .unwrap()
            .transaction(0, |_| Ok(()))
            .unwrap();
        manager.lock().await.unwrap();
        // An unexpected directory makes sidecar removal fail on every platform.
        let sidecar = capstan_vault::hello::sidecar_path(&path);
        std::fs::create_dir(&sidecar).unwrap();
        let unexpected = sidecar.join("preserved.txt");
        std::fs::write(&unexpected, b"preserve unexpected files").unwrap();

        let candidates = manager.list_recovery_candidates(password()).await.unwrap();
        manager
            .restore_recovery_candidate(candidates[0].token, password())
            .await
            .unwrap();
        assert!(manager.status().unlocked);
        let status = manager.hello_status().await.unwrap();
        assert!(status.enabled);
        assert!(status.invalidated);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(
            std::fs::read(&unexpected).unwrap(),
            b"preserve unexpected files"
        );
        assert!(Vault::unlock(&path, &password()).is_ok());
        assert_eq!(
            manager
                .current()
                .unwrap()
                .transaction(0, |_| Ok(()))
                .unwrap(),
            1
        );
    }
    #[test]
    fn hello_error_codes_survive_the_core_boundary() {
        for error in [
            HelloError::Unavailable,
            HelloError::Canceled,
            HelloError::KeyMissing,
            HelloError::Mismatch,
            HelloError::Data,
        ] {
            assert_eq!(
                CoreError::from(VaultError::from(error)).code(),
                error.code()
            );
        }
    }

    #[tokio::test]
    async fn a_lock_waiting_on_hello_consent_still_wins_after_unlock_finishes() {
        use std::{future::Future, task::Poll};
        let directory = tempfile::tempdir().unwrap();
        let manager = Arc::new(VaultManager::new(directory.path().join("vault.capstan")));
        let device = Arc::new(Device::default());
        manager.initialize(password(), fast()).await.unwrap();
        manager
            .enable_hello(password(), device.clone())
            .await
            .unwrap();
        manager.lock().await.unwrap();
        let gate = Arc::new(ConsentGate::default());
        *device.gate.lock().unwrap() = Some(gate.clone());
        let unlocking_manager = manager.clone();
        let unlocking = tokio::spawn(async move { unlocking_manager.unlock_hello(device).await });
        let entered = tokio::time::timeout(Duration::from_secs(5), gate.entered.notified()).await;
        if entered.is_err() {
            gate.release();
        }
        entered.unwrap();
        let mut locking = Box::pin(manager.lock());
        let pending = std::future::poll_fn(|context| {
            Poll::Ready(locking.as_mut().poll(context).is_pending())
        })
        .await;
        gate.release();
        unlocking.await.unwrap().unwrap();
        locking.await.unwrap();
        assert!(pending);
        assert!(!manager.status().unlocked);
        assert!(manager
            .connections
            .locked
            .load(std::sync::atomic::Ordering::Acquire));
    }
}
