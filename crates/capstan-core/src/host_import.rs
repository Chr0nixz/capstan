//! Hosts created outside the host editor: quick connect and SSH config import.
//!
//! Both reuse a saved host with the same normalized address, port, user and
//! jump host instead of adding a duplicate. New hosts without a usable key get
//! the shared credential that asks at connect time.
use crate::{
    launch::{parse_ssh_url, ssh_target_matches, SshTarget},
    read_private_key_file,
    ssh_config::{Candidate, Note},
    CoreError, Result, VaultManager,
};
use capstan_vault::{Credential, CredentialMaterial, Group, Host, SecretText, VaultData};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};
use tokio::task;
use uuid::Uuid;

/// Longest quick connect text accepted.
const MAX_QUICK_TEXT: usize = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickConnect {
    pub host_id: Uuid,
    /// A new host was saved for this target.
    pub created: bool,
}

/// `user@host`, `user@host:port`, `ssh user@host -p 2222` or an `ssh://`
/// URL. A user is required; no other `ssh` option is accepted.
#[must_use]
pub fn parse_quick_target(text: &str) -> Option<SshTarget> {
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_QUICK_TEXT || text.chars().any(char::is_control) {
        return None;
    }
    let mut words: Vec<&str> = text.split_whitespace().collect();
    if words.first() == Some(&"ssh") {
        words.remove(0);
    }
    let mut port = None;
    let mut authority = None;
    let mut index = 0;
    while index < words.len() {
        match words[index] {
            "-p" => {
                let value = words.get(index + 1)?;
                if !value.bytes().all(|byte| byte.is_ascii_digit()) || port.is_some() {
                    return None;
                }
                port = Some(value.parse::<u16>().ok().filter(|port| *port != 0)?);
                index += 2;
            }
            word if word.starts_with('-') => return None,
            word => {
                if authority.replace(word).is_some() {
                    return None;
                }
                index += 1;
            }
        }
    }
    let authority = authority?;
    let bare = authority.strip_prefix("ssh://").unwrap_or(authority);
    if bare.contains(['/', '?', '#', ';']) {
        return None;
    }
    let mut target = parse_ssh_url(&format!("ssh://{bare}"))?;
    target.username.as_ref()?;
    if let Some(port) = port {
        if target.port != 22 && target.port != port {
            return None;
        }
        target.port = port;
    }
    Some(target)
}

/// Name for a host created from a target, such as `deploy@web-01:2222`.
fn display_name(username: &str, host: &str, port: u16) -> String {
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    let name = if port == 22 {
        format!("{username}@{host}")
    } else {
        format!("{username}@{host}:{port}")
    };
    name.chars().take(256).collect()
}

fn new_host(
    name: String,
    address: String,
    port: u16,
    username: String,
    credential_id: Uuid,
    group_id: Option<Uuid>,
) -> Host {
    Host {
        id: Uuid::new_v4(),
        name,
        address,
        port,
        username,
        credential_id,
        group_id,
        tags: Vec::new(),
        favorite: false,
        revision: 0,
        working_directory: None,
        startup_command: None,
        encoding: "utf-8".into(),
        theme: "graphite".into(),
        agent_forwarding: false,
        production: false,
        jump_host_id: None,
    }
}

/// `host` reaches the same account by the same route.
fn same_route(host: &Host, address: &str, port: u16, username: &str, jump: Option<Uuid>) -> bool {
    let target = SshTarget {
        username: Some(username.to_owned()),
        host: address.to_owned(),
        port,
    };
    host.jump_host_id == jump
        && ssh_target_matches(&target, &host.address, host.port, &host.username)
}

fn saved_host<'a>(
    data: &'a VaultData,
    address: &str,
    port: u16,
    username: &str,
    jump: Option<Uuid>,
) -> Option<&'a Host> {
    data.hosts
        .iter()
        .find(|host| same_route(host, address, port, username, jump))
}

fn interactive_credential(data: &VaultData) -> Option<Uuid> {
    data.credentials
        .iter()
        .find(|credential| matches!(credential.material, CredentialMaterial::Interactive {}))
        .map(|credential| credential.id)
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
}

/// One SSH config host as shown before import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub index: u32,
    pub alias: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub identity_file: Option<String>,
    /// The key needs a passphrase, asked for during import.
    pub encrypted: bool,
    pub notes: Vec<Note>,
    /// Index of the host this one is reached through; importing this host
    /// imports that one too.
    pub jump: Option<u32>,
    /// A host with the same address, port, user and jump host is already saved.
    pub saved: bool,
}

/// SSH config hosts checked against the Vault and their key files.
#[derive(Debug, Clone, Default)]
pub struct ImportPlan {
    entries: Vec<(Candidate, bool)>,
}

/// What importing some entries of a plan adds to the Vault.
struct Resolution<'a> {
    /// New hosts as entry index, candidate, host id and jump host, each jump
    /// host before the hosts reached through it.
    selected: Vec<(usize, &'a Candidate, Uuid, Option<Uuid>)>,
    /// Chosen entries that match a saved host or another chosen entry.
    existing: u32,
}

/// The saved key file credential for `path`. Compared as paths: Windows
/// accepts either separator.
fn saved_key_credential(data: &VaultData, path: &Path) -> Option<Uuid> {
    data.credentials
        .iter()
        .find(|credential| {
            matches!(&credential.material, CredentialMaterial::KeyFile { path: saved, .. } if Path::new(saved) == path)
        })
        .map(|credential| credential.id)
}

impl ImportPlan {
    /// Key files that importing `indices` into `data` needs a passphrase for,
    /// jump hosts included, in first-use order. Hosts that are already saved
    /// and keys the Vault already holds need none.
    fn passphrase_keys(&self, data: &VaultData, indices: &[u32]) -> Result<Vec<PathBuf>> {
        let mut keys = Vec::new();
        for (index, candidate, _, _) in self.resolve(data, indices)?.selected {
            if let (Some(path), true) = (&candidate.identity_file, self.entries[index].1) {
                if !keys.contains(path) && saved_key_credential(data, path).is_none() {
                    keys.push(path.clone());
                }
            }
        }
        Ok(keys)
    }

    /// Matches the entries at `indices`, and the jump hosts they need, with
    /// saved hosts and with each other; the rest become new hosts.
    fn resolve(&self, data: &VaultData, indices: &[u32]) -> Result<Resolution<'_>> {
        let chosen: HashSet<usize> = indices
            .iter()
            .filter_map(|index| usize::try_from(*index).ok())
            .collect();
        let mut existing = 0_u32;
        // Each entry's host: a saved one, or a new one. Jump hosts come
        // first, so a host's jump is always resolved already.
        let mut resolved: HashMap<usize, Uuid> = HashMap::new();
        let mut selected: Vec<(usize, &Candidate, Uuid, Option<Uuid>)> = Vec::new();
        for index in self.with_jumps(indices) {
            let candidate = &self.entries[index].0;
            let jump = match candidate.jump {
                Some(hop) => Some(*resolved.get(&hop).ok_or(CoreError::InvalidInput)?),
                None => None,
            };
            let duplicate = saved_host(
                data,
                &candidate.address,
                candidate.port,
                &candidate.username,
                jump,
            )
            .map(|host| host.id)
            .or_else(|| {
                selected
                    .iter()
                    .find(|(_, other, _, other_jump)| {
                        other.address == candidate.address
                            && other.port == candidate.port
                            && other.username == candidate.username
                            && *other_jump == jump
                    })
                    .map(|(_, _, id, _)| *id)
            });
            let id = match duplicate {
                Some(id) => {
                    if chosen.contains(&index) {
                        existing += 1;
                    }
                    id
                }
                None => {
                    let id = Uuid::new_v4();
                    selected.push((index, candidate, id, jump));
                    id
                }
            };
            resolved.insert(index, id);
        }
        Ok(Resolution { selected, existing })
    }

    /// The entries at `indices` and every jump host they need, each jump host
    /// before the hosts reached through it. Unknown indices are ignored.
    fn with_jumps(&self, indices: &[u32]) -> Vec<usize> {
        let mut order = Vec::new();
        let mut indices: Vec<usize> = indices
            .iter()
            .filter_map(|index| usize::try_from(*index).ok())
            .filter(|index| *index < self.entries.len())
            .collect();
        indices.sort_unstable();
        for index in indices {
            let mut chain = Vec::new();
            let mut next = Some(index);
            while let Some(hop) = next {
                if order.contains(&hop)
                    || chain.contains(&hop)
                    || chain.len() > capstan_vault::MAX_JUMP_HOPS
                {
                    break;
                }
                chain.push(hop);
                next = self
                    .entries
                    .get(hop)
                    .and_then(|(candidate, _)| candidate.jump);
            }
            order.extend(chain.into_iter().rev());
        }
        order
    }

    /// The saved host each entry already matches, following jump hosts.
    fn saved(&self, data: &VaultData) -> Vec<Option<Uuid>> {
        let mut saved = vec![None; self.entries.len()];
        let all: Vec<u32> = (0..self.entries.len())
            .filter_map(|index| u32::try_from(index).ok())
            .collect();
        for index in self.with_jumps(&all) {
            let candidate = &self.entries[index].0;
            let jump = match candidate.jump {
                Some(hop) => match saved.get(hop).copied().flatten() {
                    Some(id) => Some(id),
                    None => continue,
                },
                None => None,
            };
            saved[index] = saved_host(
                data,
                &candidate.address,
                candidate.port,
                &candidate.username,
                jump,
            )
            .map(|host| host.id);
        }
        saved
    }
}

/// Whether a key file is usable, and whether it needs a passphrase.
fn inspect_key(path: &Path) -> Option<bool> {
    let content = read_private_key_file(path).ok()?;
    if capstan_ssh::private_key_requires_passphrase(content.expose()) {
        return Some(true);
    }
    capstan_ssh::validate_private_key(content.expose(), None)
        .ok()
        .map(|()| false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub created: u32,
    /// Selected hosts that were already saved.
    pub existing: u32,
}

/// Names for records an import may create, in the user's language.
pub struct ImportNames {
    pub interactive_credential: String,
    pub group: String,
}

impl VaultManager {
    /// Finds the saved host for `text` (see [`parse_quick_target`]) or saves
    /// a new one that asks at connect time.
    ///
    /// # Errors
    ///
    /// Fails for text that is not a target with a user, an invalid address,
    /// a locked Vault or a concurrent change.
    pub async fn quick_connect_host(
        &self,
        text: &str,
        credential_name: String,
    ) -> Result<QuickConnect> {
        let target = parse_quick_target(text).ok_or(CoreError::InvalidInput)?;
        let username = target.username.clone().ok_or(CoreError::InvalidInput)?;
        capstan_vault::normalize_target(&target.host)?;
        if !valid_name(&credential_name) {
            return Err(CoreError::InvalidInput);
        }
        let vault = self.current()?;
        let data = vault.snapshot()?;
        if let Some(host) = saved_host(&data, &target.host, target.port, &username, None) {
            return Ok(QuickConnect {
                host_id: host.id,
                created: false,
            });
        }
        let existing_credential = interactive_credential(&data);
        let credential = existing_credential.is_none().then(|| Credential {
            id: Uuid::new_v4(),
            name: credential_name,
            revision: 0,
            material: CredentialMaterial::Interactive {},
        });
        let credential_id = existing_credential
            .or_else(|| credential.as_ref().map(|credential| credential.id))
            .ok_or(CoreError::InvalidInput)?;
        let host = new_host(
            display_name(&username, &target.host, target.port),
            target.host,
            target.port,
            username,
            credential_id,
            None,
        );
        let host_id = host.id;
        task::spawn_blocking(move || {
            vault.transaction(data.revision, move |data| {
                data.credentials.extend(credential);
                data.hosts.push(host);
                Ok(())
            })
        })
        .await
        .map_err(|_| CoreError::TaskFailed)??;
        Ok(QuickConnect {
            host_id,
            created: true,
        })
    }

    /// Checks parsed SSH config hosts against saved hosts and their key files.
    /// Reads key files, so call it off the async runtime.
    ///
    /// # Errors
    ///
    /// Fails while the Vault is locked.
    pub fn plan_ssh_config_import(
        &self,
        candidates: Vec<Candidate>,
    ) -> Result<(ImportPlan, Vec<ImportPreview>)> {
        let data = self.current()?.snapshot()?;
        let mut keys: HashMap<PathBuf, Option<bool>> = HashMap::new();
        let mut plan = ImportPlan::default();
        let mut previews = Vec::new();
        let count = candidates.len();
        for (index, mut candidate) in candidates.into_iter().enumerate() {
            candidate.jump = candidate
                .jump
                .filter(|jump| *jump < count && *jump != index);
            let encrypted = match &candidate.identity_file {
                Some(path) => {
                    // The first listed key that can be read; OpenSSH would
                    // try the others in turn too.
                    let usable = std::iter::once(path)
                        .chain(&candidate.other_identities)
                        .find_map(|path| {
                            keys.entry(path.clone())
                                .or_insert_with(|| inspect_key(path))
                                .map(|encrypted| (path.clone(), encrypted))
                        });
                    let state = usable.as_ref().map(|(_, encrypted)| *encrypted);
                    candidate.identity_file = usable.map(|(path, _)| path);
                    if state.is_none() {
                        candidate
                            .notes
                            .retain(|note| *note != Note::DefaultIdentity);
                        if !candidate.notes.contains(&Note::IdentityUnavailable) {
                            candidate.notes.push(Note::IdentityUnavailable);
                        }
                    }
                    state.unwrap_or(false)
                }
                None => false,
            };
            previews.push(ImportPreview {
                index: u32::try_from(index).map_err(|_| CoreError::InvalidInput)?,
                alias: candidate.alias.clone(),
                address: candidate.address.clone(),
                port: candidate.port,
                username: candidate.username.clone(),
                identity_file: candidate
                    .identity_file
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
                encrypted,
                notes: candidate.notes.clone(),
                jump: candidate
                    .jump
                    .filter(|jump| *jump < count)
                    .and_then(|jump| u32::try_from(jump).ok()),
                saved: false,
            });
            plan.entries.push((candidate, encrypted));
        }
        for (preview, saved) in previews.iter_mut().zip(plan.saved(&data)) {
            preview.saved = saved.is_some();
        }
        Ok((plan, previews))
    }

    /// Key files that importing `indices` of `plan` needs a passphrase for,
    /// jump hosts included, in first-use order. Hosts that are already saved
    /// and keys the Vault already holds need none.
    ///
    /// # Errors
    ///
    /// Fails while the Vault is locked, or for a plan whose jump chain is cut.
    pub fn ssh_config_passphrase_keys(
        &self,
        plan: &ImportPlan,
        indices: &[u32],
    ) -> Result<Vec<PathBuf>> {
        plan.passphrase_keys(&self.current()?.snapshot()?, indices)
    }

    /// Saves the hosts at `indices` of `plan`, with one credential per key
    /// file and one shared credential that asks at connect time, in a group
    /// named `names.group`. Nothing is written if any key fails to open.
    ///
    /// # Errors
    ///
    /// Fails for an unusable key or wrong passphrase, invalid names, a locked
    /// Vault or a concurrent change.
    pub async fn import_ssh_config_hosts(
        &self,
        plan: &ImportPlan,
        indices: &[u32],
        passphrases: Vec<(PathBuf, SecretText)>,
        names: ImportNames,
    ) -> Result<ImportSummary> {
        self.import_planned(plan, indices, &passphrases, names)
            .await
    }

    async fn import_planned(
        &self,
        plan: &ImportPlan,
        indices: &[u32],
        passphrases: &[(PathBuf, SecretText)],
        names: ImportNames,
    ) -> Result<ImportSummary> {
        if !valid_name(&names.interactive_credential) || !valid_name(&names.group) {
            return Err(CoreError::InvalidInput);
        }
        let vault = self.current()?;
        let data = vault.snapshot()?;
        let Resolution { selected, existing } = plan.resolve(&data, indices)?;
        let mut credentials = Vec::new();
        let mut key_credentials: HashMap<PathBuf, Uuid> = HashMap::new();
        for path in selected
            .iter()
            .filter_map(|(_, candidate, _, _)| candidate.identity_file.as_ref())
        {
            if key_credentials.contains_key(path) {
                continue;
            }
            if let Some(saved) = saved_key_credential(&data, path) {
                key_credentials.insert(path.clone(), saved);
                continue;
            }
            let text = path.to_string_lossy().into_owned();
            let passphrase = passphrases
                .iter()
                .find(|(key, _)| key == path)
                .map(|(_, passphrase)| passphrase.clone());
            let read_path = path.clone();
            let content = task::spawn_blocking(move || read_private_key_file(&read_path))
                .await
                .map_err(|_| CoreError::TaskFailed)??;
            capstan_ssh::validate_private_key(
                content.expose(),
                passphrase.as_ref().map(SecretText::expose),
            )?;
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().chars().take(256).collect::<String>())
                .filter(|name| valid_name(name))
                .unwrap_or_else(|| "SSH key".into());
            let credential = Credential {
                id: Uuid::new_v4(),
                name,
                revision: 0,
                material: CredentialMaterial::KeyFile {
                    path: text,
                    passphrase,
                },
            };
            key_credentials.insert(path.clone(), credential.id);
            credentials.push(credential);
        }
        let mut interactive = interactive_credential(&data);
        if interactive.is_none()
            && selected
                .iter()
                .any(|(_, candidate, _, _)| candidate.identity_file.is_none())
        {
            let credential = Credential {
                id: Uuid::new_v4(),
                name: names.interactive_credential,
                revision: 0,
                material: CredentialMaterial::Interactive {},
            };
            interactive = Some(credential.id);
            credentials.push(credential);
        }
        let mut group = None;
        let group_id = if selected.is_empty() {
            None
        } else if let Some(saved) = data
            .groups
            .iter()
            .find(|group| group.parent_id.is_none() && group.name == names.group)
        {
            Some(saved.id)
        } else {
            let created = Group {
                id: Uuid::new_v4(),
                name: names.group,
                parent_id: None,
                production: false,
            };
            let id = created.id;
            group = Some(created);
            Some(id)
        };
        let mut hosts = Vec::new();
        for (_, candidate, id, jump) in &selected {
            let credential_id = match &candidate.identity_file {
                Some(path) => key_credentials.get(path).copied(),
                None => interactive,
            }
            .ok_or(CoreError::InvalidInput)?;
            let mut host = new_host(
                candidate.alias.clone(),
                candidate.address.clone(),
                candidate.port,
                candidate.username.clone(),
                credential_id,
                group_id,
            );
            host.id = *id;
            host.jump_host_id = *jump;
            hosts.push(host);
        }
        let created = u32::try_from(hosts.len()).map_err(|_| CoreError::InvalidInput)?;
        if created > 0 {
            task::spawn_blocking(move || {
                vault.transaction(data.revision, move |data| {
                    data.credentials.extend(credentials);
                    data.groups.extend(group);
                    data.hosts.extend(hosts);
                    Ok(())
                })
            })
            .await
            .map_err(|_| CoreError::TaskFailed)??;
        }
        Ok(ImportSummary { created, existing })
    }
}

#[cfg(test)]
#[path = "host_import_tests.rs"]
mod tests;
