//! Byte-preserving plans for explicit Bash/Zsh configuration transactions.
//! Configuration bodies stay in Rust; this module grants no connection or I/O.
use crate::sftp::cwd::CwdAuthority;
use crate::sftp::{RemoteName, RemotePath};
use capstan_ssh::sftp::{ClientError, SftpClient};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub const MAX_CONFIG_BYTES: usize = 64 * 1024;
const START: &str = "# >>> capstan-osc7:";
const END: &str = "# <<< capstan-osc7:v1 <<<\n";
const RESERVED: &str = "__capstan_osc7_v1_";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shell {
    Bash,
    Zsh,
}
impl Shell {
    fn name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
        }
    }
    fn template(self) -> &'static str {
        match self {
            Self::Bash => include_str!("bash.sh"),
            Self::Zsh => include_str!("zsh.sh"),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Install,
    Uninstall,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum IntegrationError {
    #[error("shell_config_invalid")]
    Invalid,
    #[error("shell_integration_modified")]
    Modified,
    #[error("shell_integration_conflict")]
    Conflict,
    #[error("resource_limit")]
    ResourceLimit,
}

/// A Rust-only replacement. Never serialize configuration or backup bodies.
pub struct Plan {
    pub replacement: Vec<u8>,
    pub changed: bool,
    pub added_lines: usize,
    pub removed_lines: usize,
}

pub const MAX_PREVIEWS: usize = 128;
pub const PREVIEW_LIFETIME_SECONDS: u64 = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewBinding {
    pub window: String,
    pub terminal: String,
    pub generation: u64,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSummary {
    pub token: Uuid,
    pub shell: Shell,
    pub action: Action,
    pub host: String,
    pub source_hash: String,
    pub replacement_hash: String,
    pub added_lines: usize,
    pub removed_lines: usize,
}

struct StoredPreview {
    binding: PreviewBinding,
    shell: Shell,
    action: Action,
    host: String,
    source_hash: String,
    plan: Plan,
    expires: Instant,
}

#[derive(Default)]
pub struct PreviewRegistry {
    previews: Mutex<HashMap<Uuid, StoredPreview>>,
}
impl PreviewRegistry {
    pub fn create(
        &self,
        binding: PreviewBinding,
        source: &[u8],
        shell: Shell,
        action: Action,
        authenticated_target: &str,
    ) -> Result<PreviewSummary, IntegrationError> {
        let plan = plan(source, shell, action, authenticated_target)?;
        let mut previews = self
            .previews
            .lock()
            .map_err(|_| IntegrationError::Invalid)?;
        previews.retain(|_, preview| preview.expires > Instant::now());
        if previews.len() >= MAX_PREVIEWS {
            return Err(IntegrationError::ResourceLimit);
        }
        let token = Uuid::new_v4();
        let source_hash = digest(source);
        let replacement_hash = digest(&plan.replacement);
        let summary = PreviewSummary {
            token,
            shell,
            action,
            host: authenticated_target.to_owned(),
            source_hash: source_hash.clone(),
            replacement_hash,
            added_lines: plan.added_lines,
            removed_lines: plan.removed_lines,
        };
        previews.insert(
            token,
            StoredPreview {
                binding,
                shell,
                action,
                host: authenticated_target.to_owned(),
                source_hash,
                plan,
                expires: Instant::now() + Duration::from_secs(PREVIEW_LIFETIME_SECONDS),
            },
        );
        Ok(summary)
    }

    /// Consume a preview only after the caller has re-read the current remote
    /// file and proved its hash and binding still match. The replacement bytes
    /// stay inside Rust and are never part of the preview DTO.
    pub fn consume(
        &self,
        binding: &PreviewBinding,
        token: Uuid,
        current_source: &[u8],
    ) -> Result<(Shell, Action, String, Plan), IntegrationError> {
        let mut previews = self
            .previews
            .lock()
            .map_err(|_| IntegrationError::Invalid)?;
        let preview = previews.remove(&token).ok_or(IntegrationError::Invalid)?;
        if preview.expires <= Instant::now()
            || &preview.binding != binding
            || digest(current_source) != preview.source_hash
        {
            return Err(IntegrationError::Modified);
        }
        Ok((preview.shell, preview.action, preview.host, preview.plan))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteApplyResult {
    pub backup_created: bool,
    pub replacement_hash: String,
}

/// Read a bounded remote configuration through the already authenticated SFTP
/// client. The returned bytes stay in Rust and are intended for `plan` or a
/// private native staging file.
pub async fn read_remote_config(
    client: &std::sync::Arc<SftpClient>,
    path: &RemotePath,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, IntegrationError> {
    read_remote_config_checked(client, path, None, cancel).await
}

/// Read a configuration while proving that the browse entry's metadata still
/// describes the same regular file. The second lstat closes the common race
/// where a shell writes the file while the bounded body is being read.
pub async fn read_remote_config_checked(
    client: &std::sync::Arc<SftpClient>,
    path: &RemotePath,
    expected: Option<&crate::sftp::Metadata>,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, IntegrationError> {
    let attributes = client
        .lstat(path.as_str(), cancel)
        .await
        .map_err(map_client_error)?;
    if let Some(expected) = expected {
        if !attributes_match(attributes.clone(), expected) {
            return Err(IntegrationError::Conflict);
        }
    }
    let size = attributes.size.ok_or(IntegrationError::Invalid)? as usize;
    if size > MAX_CONFIG_BYTES {
        return Err(IntegrationError::ResourceLimit);
    }
    let file = client
        .open_file(path.as_str(), false, cancel)
        .await
        .map_err(map_client_error)?;
    let mut offset = 0_u64;
    let mut output = Vec::with_capacity(size);
    while offset < size as u64 {
        let length = (size as u64 - offset).min(32 * 1024) as usize;
        let chunk = file
            .read(offset, length, cancel)
            .await
            .map_err(map_client_error)?;
        if chunk.is_empty() {
            return Err(IntegrationError::Invalid);
        }
        output.extend_from_slice(&chunk);
        offset += chunk.len() as u64;
    }
    file.close(cancel).await.map_err(map_client_error)?;
    if output.len() != size {
        return Err(IntegrationError::Invalid);
    }
    let after = client
        .lstat(path.as_str(), cancel)
        .await
        .map_err(map_client_error)?;
    if after != attributes {
        return Err(IntegrationError::Conflict);
    }
    Ok(output)
}

/// Apply a consumed preview with a private backup and a same-directory
/// temporary. Existing targets require the server's atomic rename extension;
/// a plain rename is used only when the target was absent at preview time.
pub async fn apply_remote_plan(
    client: &std::sync::Arc<SftpClient>,
    path: &RemotePath,
    source: &[u8],
    plan: &Plan,
    expected: &crate::sftp::Metadata,
    commits: &tokio::sync::Mutex<()>,
    cancel: &CancellationToken,
) -> Result<RemoteApplyResult, IntegrationError> {
    if source.len() > MAX_CONFIG_BYTES || plan.replacement.len() > MAX_CONFIG_BYTES {
        return Err(IntegrationError::ResourceLimit);
    }
    let current = read_remote_config_checked(client, path, Some(expected), cancel).await?;
    if current != source {
        return Err(IntegrationError::Conflict);
    }
    if !plan.changed {
        return Ok(RemoteApplyResult {
            backup_created: false,
            replacement_hash: digest(source),
        });
    }
    if !client.capabilities().atomic_replace {
        return Err(IntegrationError::Conflict);
    }
    let parent = path
        .as_str()
        .rsplit_once('/')
        .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
        .ok_or(IntegrationError::Invalid)?;
    let backup = RemotePath::parse(parent)
        .map_err(|_| IntegrationError::Invalid)?
        .join(
            &RemoteName::from_bytes(format!(".capstan-backup-{}", Uuid::new_v4()).as_bytes())
                .map_err(|_| IntegrationError::Invalid)?,
        )
        .map_err(|_| IntegrationError::Invalid)?;
    // Share the hash-verified atomic publication and owned-temp cleanup with
    // remote editors. Backups stay private; configuration mode is preserved.
    crate::remote_edit::apply_remote_bytes(client, &backup, source, None, None, commits, cancel)
        .await
        .map_err(map_edit_error)?;
    crate::remote_edit::apply_remote_bytes(
        client,
        path,
        &plan.replacement,
        Some(expected),
        Some(Sha256::digest(source).into()),
        commits,
        cancel,
    )
    .await
    .map_err(map_edit_error)?;
    Ok(RemoteApplyResult {
        backup_created: true,
        replacement_hash: digest(&plan.replacement),
    })
}

fn map_edit_error(error: crate::remote_edit::EditError) -> IntegrationError {
    match error {
        crate::remote_edit::EditError::Conflict => IntegrationError::Conflict,
        crate::remote_edit::EditError::ResourceLimit => IntegrationError::ResourceLimit,
        _ => IntegrationError::Invalid,
    }
}

fn map_client_error(error: ClientError) -> IntegrationError {
    match error {
        ClientError::PermissionDenied => IntegrationError::Invalid,
        ClientError::Canceled => IntegrationError::Invalid,
        ClientError::Unsupported => IntegrationError::Conflict,
        ClientError::NotFound => IntegrationError::Invalid,
        ClientError::ConnectionLost
        | ClientError::Unavailable
        | ClientError::RemoteIo
        | ClientError::Uncertain
        | ClientError::ResourceLimit
        | ClientError::InvalidPath => IntegrationError::Invalid,
    }
}

fn attributes_match(
    attributes: capstan_ssh::sftp::Attributes,
    expected: &crate::sftp::Metadata,
) -> bool {
    expected.kind == crate::sftp::FileKind::from_permissions(attributes.permissions)
        && expected.size.map(|value| value.0) == attributes.size
        && expected
            .modified_seconds
            .and_then(|value| u32::try_from(value.0).ok())
            == attributes.modified_seconds
        && expected.permissions == attributes.permissions
}

fn authority(target: &str) -> Result<String, IntegrationError> {
    let validator = CwdAuthority::new(target, &[]).map_err(|_| IntegrationError::Invalid)?;
    let unbracketed = target
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(target);
    let name = match unbracketed.parse::<IpAddr>() {
        Ok(IpAddr::V6(ip)) => format!("[{ip}]"),
        Ok(IpAddr::V4(ip)) => ip.to_string(),
        Err(_) => target
            .strip_suffix('.')
            .unwrap_or(target)
            .to_ascii_lowercase(),
    };
    validator
        .parse(&format!("file://{name}/"))
        .map_err(|_| IntegrationError::Invalid)?;
    Ok(name)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:X}", Sha256::digest(bytes))
}

fn block(shell: Shell, host: &str, separator: bool, anchor: &str) -> String {
    format!(
        "{START}v1 {} {host} {} {anchor} >>>\n{}{END}",
        shell.name(),
        u8::from(separator),
        shell.template().replace("__CAPSTAN_AUTHORITY__", host)
    )
}

struct Managed<'a> {
    start: usize,
    end: usize,
    shell: Shell,
    host: &'a str,
    separator: bool,
}
fn managed(source: &str) -> Result<Option<Managed<'_>>, IntegrationError> {
    let starts: Vec<_> = source.match_indices(START).collect();
    let endings: Vec<_> = source.match_indices("# <<< capstan-osc7:").collect();
    if starts.is_empty() && endings.is_empty() {
        return if source.contains(RESERVED) {
            Err(IntegrationError::Conflict)
        } else {
            Ok(None)
        };
    }
    if starts.len() != 1 || endings.len() != 1 {
        return Err(IntegrationError::Modified);
    }
    let start = starts[0].0;
    if start > 0 && source.as_bytes()[start - 1] != b'\n' {
        return Err(IntegrationError::Modified);
    }
    let header_end = source[start..]
        .find('\n')
        .ok_or(IntegrationError::Modified)?
        + start;
    let fields: Vec<_> = source[start..header_end].split(' ').collect();
    if fields.len() != 8 || fields[..3] != ["#", ">>>", "capstan-osc7:v1"] || fields[7] != ">>>" {
        return Err(IntegrationError::Modified);
    }
    let shell = match fields[3] {
        "bash" => Shell::Bash,
        "zsh" => Shell::Zsh,
        _ => return Err(IntegrationError::Modified),
    };
    let host = fields[4];
    if authority(host).ok().as_deref() != Some(host) {
        return Err(IntegrationError::Modified);
    }
    let separator = match fields[5] {
        "0" => false,
        "1" => true,
        _ => return Err(IntegrationError::Modified),
    };
    let anchor = fields[6];
    if anchor.len() != 64
        || !anchor
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'A'..=b'F').contains(&c))
    {
        return Err(IntegrationError::Modified);
    }
    let expected = block(shell, host, separator, anchor);
    let end = start + expected.len();
    if !source[start..].starts_with(&expected)
        || endings[0].0 + END.len() != end
        || source[..start].contains(RESERVED)
        || source[end..].contains(RESERVED)
    {
        return Err(IntegrationError::Modified);
    }
    if separator && (start == 0 || digest(&source.as_bytes()[..start - 1]) != anchor) {
        // Removing an inserted line break is safe only while its original
        // prefix still matches. Changed surrounding content needs manual work.
        return Err(IntegrationError::Modified);
    }
    Ok(Some(Managed {
        start,
        end,
        shell,
        host,
        separator,
    }))
}

pub fn plan(
    source: &[u8],
    shell: Shell,
    action: Action,
    authenticated_target: &str,
) -> Result<Plan, IntegrationError> {
    if source.len() > MAX_CONFIG_BYTES {
        return Err(IntegrationError::ResourceLimit);
    }
    let text = std::str::from_utf8(source).map_err(|_| IntegrationError::Invalid)?;
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n'))
    {
        return Err(IntegrationError::Invalid);
    }
    let host = authority(authenticated_target)?;
    let current = managed(text)?;
    let (replacement, added_lines, removed_lines) = match (action, current) {
        (Action::Install, Some(existing)) => {
            if existing.shell != shell || existing.host != host {
                return Err(IntegrationError::Conflict);
            }
            (source.to_vec(), 0, 0)
        }
        (Action::Uninstall, None) => (source.to_vec(), 0, 0),
        (Action::Uninstall, Some(existing)) => {
            if existing.shell != shell {
                return Err(IntegrationError::Conflict);
            }
            let suffix = &source[existing.end..];
            // Preserve the line boundary of user commands appended after an
            // installed block when the original file had no final newline.
            let keep_separator = !suffix.is_empty() && !suffix.starts_with(b"\n");
            let first = existing.start - usize::from(existing.separator && !keep_separator);
            let mut output = source[..first].to_vec();
            output.extend_from_slice(&source[existing.end..]);
            (
                output,
                0,
                text[existing.start..existing.end].lines().count(),
            )
        }
        (Action::Install, None) => {
            let separator = !source.is_empty() && !source.ends_with(b"\n");
            let addition = block(shell, &host, separator, &digest(source));
            let lines = addition.lines().count();
            let mut output = source.to_vec();
            if separator {
                output.push(b'\n');
            }
            output.extend_from_slice(addition.as_bytes());
            (output, lines, 0)
        }
    };
    if replacement.len() > MAX_CONFIG_BYTES {
        return Err(IntegrationError::ResourceLimit);
    }
    Ok(Plan {
        changed: replacement != source,
        replacement,
        added_lines,
        removed_lines,
    })
}

#[cfg(test)]
mod tests;
