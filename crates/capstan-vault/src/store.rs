use crate::{
    envelope::{self, EnvelopeKind, KdfParameters, Opened, HEADER_BYTES, MAX_FILE_BYTES},
    Result, SecretText, VaultData, VaultError,
};
use fs2::FileExt;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;
const MAX_RECOVERY_FILES: usize = 128;
const MAX_RECOVERY_CANDIDATES: usize = 8;
const MAX_BACKUPS: usize = 3;

struct State {
    opened: Option<Opened>,
    digest: Option<[u8; 32]>,
}

/// In-memory Vault owner. `lock` drops the only application-owned decrypted model and keys.
pub struct Vault {
    path: PathBuf,
    state: Mutex<State>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryKind {
    Pending,
    Backup,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryCandidate {
    pub file_name: String,
    pub kind: RecoveryKind,
    pub revision: u64,
    pub digest: [u8; 32],
}

impl Vault {
    pub fn initialize(
        path: impl Into<PathBuf>,
        password: &SecretText,
        parameters: KdfParameters,
    ) -> Result<Self> {
        let path = validate_path(path.into())?;
        validate_new_path(&path)?;
        let _guard = file_lock(&path)?;
        if path.exists() {
            return Err(VaultError::AlreadyExists);
        }
        let opened = envelope::seal(
            VaultData::default(),
            password,
            parameters,
            EnvelopeKind::Vault,
        )?;
        let bytes = opened.bytes.clone();
        atomic_write(&path, &bytes, false)?;
        let digest = sha256(&bytes);
        let store = Self {
            path,
            state: Mutex::new(State {
                opened: Some(opened),
                digest: Some(digest),
            }),
        };
        Ok(store)
    }

    pub fn unlock(path: impl Into<PathBuf>, password: &SecretText) -> Result<Self> {
        let path = validate_path(path.into())?;
        let _guard = file_lock(&path)?;
        let bytes = read_limited(&path)?;
        let mut opened = envelope::open(bytes, password, EnvelopeKind::Vault)?;
        if opened.migrated {
            let previous = opened.bytes.clone();
            envelope::reseal(&mut opened)?;
            archive_previous(&path, &opened, &previous)?;
            atomic_write(&path, &opened.bytes, true)?;
            let _ = prune_backups(&path, &opened);
        }
        let digest = sha256(&opened.bytes);
        Ok(Self {
            path,
            state: Mutex::new(State {
                opened: Some(opened),
                digest: Some(digest),
            }),
        })
    }

    pub fn is_unlocked(&self) -> bool {
        self.state.lock().is_ok_and(|state| state.opened.is_some())
    }
    pub fn lock(&self) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| VaultError::Io)?;
        state.opened.take();
        state.digest.take();
        Ok(())
    }
    pub fn snapshot(&self) -> Result<VaultData> {
        let state = self.state.lock().map_err(|_| VaultError::Io)?;
        Ok(state
            .opened
            .as_ref()
            .ok_or(VaultError::Locked)?
            .data
            .clone())
    }
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Apply one validated model edit and atomically commit it against the on-disk revision.
    pub fn transaction(
        &self,
        expected_revision: u64,
        edit: impl FnOnce(&mut VaultData) -> Result<()>,
    ) -> Result<u64> {
        let mut state = self.state.lock().map_err(|_| VaultError::Io)?;
        let current = state.opened.as_ref().ok_or(VaultError::Locked)?;
        if current.data.revision != expected_revision {
            return Err(VaultError::Conflict);
        }
        let _guard = file_lock(&self.path)?;
        let existing = read_limited(&self.path)?;
        if state.digest != Some(sha256(&existing)) {
            return Err(VaultError::Conflict);
        }
        let mut candidate = current.fork();
        edit(&mut candidate.data)?;
        for host in &mut candidate.data.hosts {
            host.address = crate::normalize_target(&host.address)?;
        }
        candidate.data.revision = candidate
            .data
            .revision
            .checked_add(1)
            .ok_or(VaultError::InvalidData)?;
        candidate.data.validate()?;
        let previous = candidate.bytes.clone();
        envelope::reseal(&mut candidate)?;
        archive_previous(&self.path, &candidate, &previous)?;
        atomic_write(&self.path, &candidate.bytes, true)?;
        let digest = sha256(&candidate.bytes);
        let _ = prune_backups(&self.path, &candidate);
        state.opened = Some(candidate);
        state.digest = Some(digest);
        Ok(state
            .opened
            .as_ref()
            .ok_or(VaultError::Locked)?
            .data
            .revision)
    }

    /// Rewrap the unchanged data-encryption key; encrypted payload bytes remain identical.
    pub fn change_master_password(
        &self,
        current_revision: u64,
        password: &SecretText,
        parameters: KdfParameters,
    ) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| VaultError::Io)?;
        let current = state.opened.as_ref().ok_or(VaultError::Locked)?;
        if current.data.revision != current_revision {
            return Err(VaultError::Conflict);
        }
        let _guard = file_lock(&self.path)?;
        let existing = read_limited(&self.path)?;
        if state.digest != Some(sha256(&existing)) {
            return Err(VaultError::Conflict);
        }
        let mut candidate = current.fork();
        let previous = candidate.bytes.clone();
        envelope::rewrap(&mut candidate, password, parameters)?;
        archive_previous(&self.path, &candidate, &previous)?;
        atomic_write(&self.path, &candidate.bytes, true)?;
        let _ = prune_backups(&self.path, &candidate);
        state.digest = Some(sha256(&candidate.bytes));
        state.opened = Some(candidate);
        Ok(())
    }

    pub fn export_backup(
        &self,
        password: &SecretText,
        parameters: KdfParameters,
        destination: &Path,
    ) -> Result<()> {
        self.export_backup_with_replace(password, parameters, destination, false)
    }

    pub fn export_backup_with_replace(
        &self,
        password: &SecretText,
        parameters: KdfParameters,
        destination: &Path,
        replace: bool,
    ) -> Result<()> {
        let state = self.state.lock().map_err(|_| VaultError::Io)?;
        let current = state.opened.as_ref().ok_or(VaultError::Locked)?;
        let opened = envelope::seal(
            current.data.clone(),
            password,
            parameters,
            EnvelopeKind::Backup,
        )?;
        atomic_write(destination, &opened.bytes, replace)
    }

    pub fn preview_backup(path: &Path, password: &SecretText) -> Result<VaultData> {
        let bytes = read_limited(path)?;
        let opened = envelope::open(bytes, password, EnvelopeKind::Backup)?;
        Ok(opened.data)
    }

    /// Enumerate only encrypted, authenticated recovery files. This never modifies the main file.
    pub fn recovery_candidates(
        path: &Path,
        password: &SecretText,
    ) -> Result<Vec<RecoveryCandidate>> {
        let path = validate_path(path.to_path_buf())?;
        let parent = parent_directory(&path);
        let stem = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(VaultError::InvalidData)?;
        let prefix = format!("{stem}.");
        let mut candidates = Vec::new();
        let mut inspected = 0;
        let mut candidate_attempts = 0;
        for entry in fs::read_dir(parent)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(suffix) = name.strip_prefix(&prefix) else {
                continue;
            };
            let is_pending = suffix.ends_with(".pending");
            let is_backup = suffix.starts_with("backup-") && suffix.ends_with(".cav");
            if !is_pending && !is_backup {
                continue;
            }
            if !entry.file_type()?.is_file() {
                continue;
            }
            inspected += 1;
            if inspected > MAX_RECOVERY_FILES {
                return Err(VaultError::RecoveryLimit);
            }
            let candidate_bytes = match read_limited(&entry.path()) {
                Ok(bytes) => bytes,
                Err(VaultError::InvalidFormat) => continue,
                Err(error) => return Err(error),
            };
            // Invalid/truncated headers never derive a password and must not consume
            // the KDF quota needed to authenticate an intact recovery file.
            if envelope::inspect(&candidate_bytes, EnvelopeKind::Vault).is_err() {
                continue;
            }
            if candidate_attempts >= MAX_RECOVERY_CANDIDATES {
                return Err(VaultError::RecoveryLimit);
            }
            candidate_attempts += 1;
            let Ok(opened) = envelope::open(candidate_bytes, password, EnvelopeKind::Vault) else {
                continue;
            };
            if is_backup && !backup_name_matches(suffix, &opened, &opened.bytes) {
                continue;
            }
            candidates.push(RecoveryCandidate {
                file_name: name,
                kind: if is_pending {
                    RecoveryKind::Pending
                } else {
                    RecoveryKind::Backup
                },
                revision: opened.data.revision,
                digest: sha256(&opened.bytes),
            });
        }
        candidates.sort_by(|left, right| {
            right
                .revision
                .cmp(&left.revision)
                .then_with(|| left.file_name.cmp(&right.file_name))
        });
        Ok(candidates)
    }

    /// Restore a listed recovery candidate only after full authentication; the current file is preserved on failure.
    pub fn restore_candidate(
        path: &Path,
        candidate: &RecoveryCandidate,
        password: &SecretText,
    ) -> Result<Self> {
        let path = validate_path(path.to_path_buf())?;
        if candidate.file_name.contains(['/', '\\', ':'])
            || candidate.file_name == "."
            || candidate.file_name == ".."
        {
            return Err(VaultError::InvalidData);
        }
        let stem = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(VaultError::InvalidData)?;
        let suffix = candidate
            .file_name
            .strip_prefix(&format!("{stem}."))
            .ok_or(VaultError::InvalidData)?;
        match candidate.kind {
            RecoveryKind::Pending if !suffix.ends_with(".pending") => {
                return Err(VaultError::InvalidData);
            }
            RecoveryKind::Backup if !suffix.starts_with("backup-") || !suffix.ends_with(".cav") => {
                return Err(VaultError::InvalidData);
            }
            _ => {}
        }
        let source = parent_directory(&path).join(&candidate.file_name);
        let _guard = file_lock(&path)?;
        let bytes = read_limited(&source)?;
        let opened = envelope::open(bytes.clone(), password, EnvelopeKind::Vault)?;
        if opened.data.revision != candidate.revision || sha256(&bytes) != candidate.digest {
            return Err(VaultError::Conflict);
        }
        if candidate.kind == RecoveryKind::Backup
            && !backup_name_matches(suffix, &opened, &opened.bytes)
        {
            return Err(VaultError::Authentication);
        }
        let digest = sha256(&opened.bytes);
        atomic_write(&path, &opened.bytes, true)?;
        Ok(Self {
            path,
            state: Mutex::new(State {
                opened: Some(opened),
                digest: Some(digest),
            }),
        })
    }
}

fn validate_path(path: PathBuf) -> Result<PathBuf> {
    if path.file_name().is_none() || path.file_name().and_then(|name| name.to_str()).is_none() {
        return Err(VaultError::InvalidData);
    }
    Ok(path)
}
fn validate_new_path(path: &Path) -> Result<()> {
    let parent = parent_directory(path);
    fs::create_dir_all(parent)?;
    if !parent.is_dir() {
        return Err(VaultError::Io);
    }
    Ok(())
}
fn lock_path(path: &Path) -> Result<PathBuf> {
    let mut value = path.as_os_str().to_os_string();
    value.push(".lock");
    Ok(PathBuf::from(value))
}
fn file_lock(path: &Path) -> Result<File> {
    fs::create_dir_all(parent_directory(path))?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path(path)?)?;
    file.lock_exclusive()?;
    Ok(file)
}
fn read_limited(path: &Path) -> Result<Vec<u8>> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    if !(HEADER_BYTES as u64 + 16..=MAX_FILE_BYTES as u64).contains(&size) {
        return Err(VaultError::InvalidFormat);
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != size {
        return Err(VaultError::InvalidFormat);
    }
    Ok(bytes)
}
fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn backup_mac(opened: &Opened, sequence: u64, bytes: &[u8]) -> Result<[u8; 32]> {
    backup_mac_for_key(&opened.dek, sequence, bytes)
}
fn backup_mac_for_key(key: &[u8; 32], sequence: u64, bytes: &[u8]) -> Result<[u8; 32]> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| VaultError::InvalidData)?;
    mac.update(b"Capstan encrypted backup filename v1\0");
    mac.update(&sequence.to_le_bytes());
    mac.update(&sha256(bytes));
    Ok(mac.finalize().into_bytes().into())
}
fn backup_name_matches(file_name: &str, opened: &Opened, bytes: &[u8]) -> bool {
    let Some(name) = file_name
        .strip_prefix("backup-")
        .and_then(|name| name.strip_suffix(".cav"))
    else {
        return false;
    };
    let Some((sequence, encoded_mac)) = name.split_once('-') else {
        return false;
    };
    let Ok(sequence) = sequence.parse::<u64>() else {
        return false;
    };
    if encoded_mac.len() != 64 {
        return false;
    }
    let Some(encoded) = decode_hex(encoded_mac) else {
        return false;
    };
    let Ok(expected) = backup_mac(opened, sequence, bytes) else {
        return false;
    };
    expected == encoded.as_slice()
}
fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            let high = (chunk[0] as char).to_digit(16)? as u8;
            let low = (chunk[1] as char).to_digit(16)? as u8;
            Some(high << 4 | low)
        })
        .collect()
}
fn archive_previous(path: &Path, opened: &Opened, previous: &[u8]) -> Result<()> {
    let parent = parent_directory(path);
    let prefix = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(VaultError::InvalidData)?;
    let mut highest = 0_u64;
    let mut inspected = 0;
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(suffix) = name.strip_prefix(&format!("{prefix}.")) else {
            continue;
        };
        if !suffix.starts_with("backup-")
            || !suffix.ends_with(".cav")
            || !entry.file_type()?.is_file()
        {
            continue;
        }
        inspected += 1;
        if inspected > MAX_RECOVERY_FILES {
            return Err(VaultError::RecoveryLimit);
        }
        let Ok(bytes) = read_limited(&entry.path()) else {
            continue;
        };
        if envelope::inspect(&bytes, EnvelopeKind::Vault).is_err() {
            continue;
        }
        // Backup validity is checked with the already-unlocked data key below. The old master password
        // intentionally remains necessary to restore a backup created before a password change.
        if let Some(value) = suffix
            .strip_prefix("backup-")
            .and_then(|value| value.strip_suffix(".cav"))
        {
            if let Some((sequence, _)) = value.split_once('-') {
                if let Ok(sequence) = sequence.parse::<u64>() {
                    if backup_name_matches(suffix, opened, &bytes) {
                        highest = highest.max(sequence);
                    }
                }
            }
        }
    }
    let sequence = highest.checked_add(1).ok_or(VaultError::InvalidData)?;
    let mac = backup_mac(opened, sequence, previous)?;
    let name = format!("{prefix}.backup-{sequence:020}-{}.cav", hex(&mac));
    atomic_write(&parent.join(name), previous, false)?;
    Ok(())
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn prune_backups(path: &Path, opened: &Opened) -> Result<()> {
    let parent = parent_directory(path);
    let prefix = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(VaultError::InvalidData)?;
    let mut valid = Vec::new();
    let mut inspected = 0;
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(suffix) = name.strip_prefix(&format!("{prefix}.")) else {
            continue;
        };
        let Some(value) = suffix
            .strip_prefix("backup-")
            .and_then(|value| value.strip_suffix(".cav"))
        else {
            continue;
        };
        let Some((sequence, _)) = value.split_once('-') else {
            continue;
        };
        let Ok(sequence) = sequence.parse::<u64>() else {
            continue;
        };
        if !entry.file_type()?.is_file() {
            continue;
        }
        inspected += 1;
        if inspected > MAX_RECOVERY_FILES {
            return Err(VaultError::RecoveryLimit);
        }
        let Ok(bytes) = read_limited(&entry.path()) else {
            continue;
        };
        if envelope::inspect(&bytes, EnvelopeKind::Vault).is_ok()
            && backup_name_matches(suffix, opened, &bytes)
        {
            valid.push((sequence, entry.path()));
        }
    }
    valid.sort_by_key(|(sequence, _)| *sequence);
    let remove_count = valid.len().saturating_sub(MAX_BACKUPS);
    for (_, old_path) in valid.into_iter().take(remove_count) {
        fs::remove_file(old_path)?;
    }
    Ok(())
}
fn parent_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}
fn atomic_write(path: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    atomic_write_with_hook(path, bytes, replace, |_| Ok(()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AtomicWriteStage {
    TemporaryCreated,
    BytesWritten,
    Flushed,
    Synced,
    BeforeReplace,
    TargetReplaced,
}

fn atomic_write_with_hook(
    path: &Path,
    bytes: &[u8],
    replace: bool,
    mut hook: impl FnMut(AtomicWriteStage) -> Result<()>,
) -> Result<()> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(VaultError::InvalidData);
    }
    let parent = parent_directory(path);
    fs::create_dir_all(parent)?;
    if !replace && path.exists() {
        return Err(VaultError::AlreadyExists);
    }
    let mut temporary = path.as_os_str().to_os_string();
    temporary.push(format!(".{}.pending", Uuid::new_v4()));
    let temporary = PathBuf::from(temporary);
    let write_result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        hook(AtomicWriteStage::TemporaryCreated)?;
        file.write_all(bytes)?;
        hook(AtomicWriteStage::BytesWritten)?;
        file.flush()?;
        hook(AtomicWriteStage::Flushed)?;
        file.sync_all()?;
        hook(AtomicWriteStage::Synced)?;
        drop(file);
        hook(AtomicWriteStage::BeforeReplace)?;
        replace_file(&temporary, path, replace)?;
        hook(AtomicWriteStage::TargetReplaced)?;
        sync_directory(parent)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}
#[cfg(windows)]
fn replace_file(source: &Path, target: &Path, replace: bool) -> Result<()> {
    capstan_native_windows::atomic_replace(source, target, replace).map_err(|_| VaultError::Io)?;
    Ok(())
}
#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path, replace: bool) -> Result<()> {
    if !replace && target.exists() {
        return Err(VaultError::AlreadyExists);
    }
    fs::rename(source, target)?;
    Ok(())
}
#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}
#[cfg(not(unix))]
fn sync_directory(_: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Credential, CredentialMaterial, Host};

    fn password() -> SecretText {
        SecretText::new("test master password".to_owned())
    }
    fn fast() -> KdfParameters {
        KdfParameters {
            memory_kib: 19_456,
            iterations: 2,
            parallelism: 1,
        }
    }
    fn host_data() -> VaultData {
        let credential_id = Uuid::new_v4();
        let host = Host {
            id: Uuid::new_v4(),
            name: "build".into(),
            address: "Example.COM.".into(),
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
        let credential = Credential {
            id: credential_id,
            name: "build password".into(),
            revision: 0,
            material: CredentialMaterial::Password {
                value: SecretText::new("secret-value".into()),
            },
        };
        VaultData {
            credentials: vec![credential],
            hosts: vec![host],
            ..VaultData::default()
        }
    }
    #[test]
    fn transactions_survive_restart_and_reject_lost_updates() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let first = Vault::initialize(&path, &password(), fast()).unwrap();
        first
            .transaction(0, |data| {
                *data = host_data();
                Ok(())
            })
            .unwrap();
        let second = Vault::unlock(&path, &password()).unwrap();
        assert_eq!(second.snapshot().unwrap().hosts[0].address, "example.com");
        second
            .transaction(1, |data| {
                data.hosts[0].favorite = true;
                Ok(())
            })
            .unwrap();
        assert!(matches!(
            first.transaction(1, |_| Ok(())),
            Err(VaultError::Conflict)
        ));
        second.lock().unwrap();
        assert!(matches!(second.snapshot(), Err(VaultError::Locked)));
        let restored = Vault::unlock(&path, &password()).unwrap();
        assert_eq!(restored.snapshot().unwrap().revision, 2);
    }
    #[test]
    fn malformed_or_wrong_password_never_replaces_original() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        Vault::initialize(&path, &password(), fast()).unwrap();
        let original = fs::read(&path).unwrap();
        assert!(matches!(
            Vault::unlock(&path, &SecretText::new("wrong password".into())),
            Err(VaultError::Authentication)
        ));
        assert_eq!(fs::read(&path).unwrap(), original);
        let mut invalid = original.clone();
        invalid[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
        fs::write(&path, invalid).unwrap();
        assert!(matches!(
            Vault::unlock(&path, &password()),
            Err(VaultError::KdfPolicy)
        ));
    }

    #[test]
    fn atomic_write_faults_before_replace_keep_the_old_file_and_cleanup_temporary_files() {
        let stages = [
            AtomicWriteStage::TemporaryCreated,
            AtomicWriteStage::BytesWritten,
            AtomicWriteStage::Flushed,
            AtomicWriteStage::Synced,
            AtomicWriteStage::BeforeReplace,
        ];
        for stage in stages {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("vault.capstan");
            let original = b"previous complete envelope";
            fs::write(&path, original).unwrap();

            assert!(matches!(
                atomic_write_with_hook(&path, b"next complete envelope", true, |at| {
                    if at == stage {
                        Err(VaultError::Io)
                    } else {
                        Ok(())
                    }
                }),
                Err(VaultError::Io)
            ));
            assert_eq!(fs::read(&path).unwrap(), original);
            assert!(!fs::read_dir(directory.path())
                .unwrap()
                .filter_map(std::result::Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().ends_with(".pending")));
        }
    }

    #[test]
    fn atomic_write_failure_after_replace_leaves_the_new_complete_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        fs::write(&path, b"previous complete envelope").unwrap();

        assert!(matches!(
            atomic_write_with_hook(&path, b"next complete envelope", true, |stage| {
                if stage == AtomicWriteStage::TargetReplaced {
                    Err(VaultError::Io)
                } else {
                    Ok(())
                }
            }),
            Err(VaultError::Io)
        ));
        assert_eq!(fs::read(path).unwrap(), b"next complete envelope");
    }

    #[test]
    fn previous_commits_are_kept_as_authenticated_recoverable_backups() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let vault = Vault::initialize(&path, &password(), fast()).unwrap();
        vault
            .transaction(0, |data| {
                data.revision = 5;
                Ok(())
            })
            .unwrap();
        let candidates = Vault::recovery_candidates(&path, &password()).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].kind, RecoveryKind::Backup);
        assert_eq!(candidates[0].revision, 0);
        Vault::restore_candidate(&path, &candidates[0], &password()).unwrap();
        assert_eq!(
            Vault::unlock(&path, &password())
                .unwrap()
                .snapshot()
                .unwrap()
                .revision,
            0
        );
    }

    #[test]
    fn recovery_restore_is_scoped_to_candidates_for_the_requested_vault() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let other_path = directory.path().join("other.capstan");
        Vault::initialize(&path, &password(), fast()).unwrap();
        Vault::initialize(&other_path, &password(), fast()).unwrap();
        let original = fs::read(&path).unwrap();
        let forged = RecoveryCandidate {
            file_name: "other.capstan.0.pending".into(),
            kind: RecoveryKind::Pending,
            revision: 0,
            digest: [0; 32],
        };

        assert!(matches!(
            Vault::restore_candidate(&path, &forged, &password()),
            Err(VaultError::InvalidData)
        ));
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn recovery_candidate_scan_bounds_password_derivations() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        Vault::initialize(&path, &password(), fast()).unwrap();
        let encoded = fs::read(&path).unwrap();
        for index in 0..=MAX_RECOVERY_CANDIDATES {
            fs::write(
                directory
                    .path()
                    .join(format!("vault.capstan.candidate-{index}.pending")),
                &encoded,
            )
            .unwrap();
        }

        assert!(matches!(
            Vault::recovery_candidates(&path, &password()),
            Err(VaultError::RecoveryLimit)
        ));
    }

    #[test]
    fn unrelated_directory_entries_do_not_block_writes_rotation_or_password_changes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let vault = Vault::initialize(&path, &password(), fast()).unwrap();
        for index in 0..MAX_RECOVERY_FILES + 16 {
            fs::write(
                directory.path().join(format!("unrelated-{index}.txt")),
                b"noise",
            )
            .unwrap();
        }
        for revision in 0..5 {
            vault.transaction(revision, |_| Ok(())).unwrap();
        }
        let candidates = Vault::recovery_candidates(&path, &password()).unwrap();
        assert_eq!(
            candidates
                .iter()
                .map(|item| item.revision)
                .collect::<Vec<_>>(),
            vec![4, 3, 2]
        );
        let replacement = SecretText::new("replacement password".into());
        vault
            .change_master_password(5, &replacement, fast())
            .unwrap();
        assert_eq!(
            Vault::unlock(&path, &replacement)
                .unwrap()
                .snapshot()
                .unwrap()
                .revision,
            5
        );
        assert_eq!(
            Vault::recovery_candidates(&path, &password())
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn invalid_pending_files_do_not_spend_the_recovery_kdf_quota() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let vault = Vault::initialize(&path, &password(), fast()).unwrap();
        for index in 0..MAX_RECOVERY_CANDIDATES + 2 {
            fs::write(
                directory
                    .path()
                    .join(format!("vault.capstan.empty-{index}.pending")),
                [],
            )
            .unwrap();
            fs::write(
                directory
                    .path()
                    .join(format!("vault.capstan.invalid-{index}.pending")),
                vec![0; HEADER_BYTES + 16],
            )
            .unwrap();
            fs::create_dir(
                directory
                    .path()
                    .join(format!("vault.capstan.directory-{index}.pending")),
            )
            .unwrap();
        }
        vault.transaction(0, |_| Ok(())).unwrap();
        let candidates = Vault::recovery_candidates(&path, &password()).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].revision, 0);
        assert_eq!(candidates[0].kind, RecoveryKind::Backup);
    }

    #[test]
    fn stale_recovery_selection_cannot_restore_replaced_candidate_contents() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let alternate_path = directory.path().join("alternate.capstan");
        let original = Vault::initialize(&path, &password(), fast()).unwrap();
        Vault::initialize(&alternate_path, &password(), fast()).unwrap();
        let pending_path = directory.path().join("vault.capstan.crash.pending");
        fs::copy(&path, &pending_path).unwrap();
        let candidate = Vault::recovery_candidates(&path, &password())
            .unwrap()
            .into_iter()
            .find(|item| item.file_name == "vault.capstan.crash.pending")
            .unwrap();
        fs::copy(&alternate_path, &pending_path).unwrap();
        let main_before = fs::read(&path).unwrap();

        assert!(matches!(
            Vault::restore_candidate(&path, &candidate, &password()),
            Err(VaultError::Conflict)
        ));
        assert_eq!(fs::read(&path).unwrap(), main_before);
        assert_eq!(original.snapshot().unwrap().revision, 0);
    }

    #[test]
    fn backup_rotation_keeps_the_three_newest_authenticated_versions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let vault = Vault::initialize(&path, &password(), fast()).unwrap();
        for expected in 0..5 {
            vault
                .transaction(expected, |data| {
                    data.revision = expected;
                    Ok(())
                })
                .unwrap();
        }
        let candidates = Vault::recovery_candidates(&path, &password()).unwrap();
        assert_eq!(candidates.len(), 3);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.revision)
                .collect::<Vec<_>>(),
            vec![4, 3, 2]
        );
    }

    #[test]
    fn sidecar_paths_without_a_parent_use_the_current_directory() {
        assert_eq!(parent_directory(Path::new("vault.capstan")), Path::new("."));
    }

    #[test]
    fn master_password_change_preserves_payload_and_prior_master_backup() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let vault = Vault::initialize(&path, &password(), fast()).unwrap();
        vault
            .transaction(0, |data| {
                *data = host_data();
                Ok(())
            })
            .unwrap();
        let old_payload = fs::read(&path).unwrap()[160..].to_vec();
        let replacement = SecretText::new("new master password phrase".into());
        vault
            .change_master_password(1, &replacement, fast())
            .unwrap();
        let updated = fs::read(&path).unwrap();
        assert_eq!(&updated[160..], old_payload);
        assert!(Vault::unlock(&path, &password()).is_err());
        assert_eq!(
            Vault::unlock(&path, &replacement)
                .unwrap()
                .snapshot()
                .unwrap()
                .hosts
                .len(),
            1
        );
        let backups = Vault::recovery_candidates(&path, &password()).unwrap();
        assert_eq!(backups.len(), 2);
    }
    #[test]
    fn independent_backup_password_roundtrips_without_exposing_secrets() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.capstan");
        let backup = directory.path().join("portable.cbak");
        let vault = Vault::initialize(&path, &password(), fast()).unwrap();
        vault
            .transaction(0, |data| {
                *data = host_data();
                Ok(())
            })
            .unwrap();
        let export_password = SecretText::new("independent backup phrase".into());
        fs::write(&backup, b"existing destination").unwrap();
        assert!(vault
            .export_backup(&export_password, fast(), &backup)
            .is_err());
        assert_eq!(fs::read(&backup).unwrap(), b"existing destination");
        vault
            .export_backup_with_replace(&export_password, fast(), &backup, true)
            .unwrap();
        assert_eq!(
            Vault::preview_backup(&backup, &export_password)
                .unwrap()
                .hosts
                .len(),
            1
        );
        assert!(Vault::preview_backup(&backup, &password()).is_err());
        let bytes = fs::read(backup).unwrap();
        assert!(!bytes.windows(12).any(|part| part == b"secret-value"));
    }
}
