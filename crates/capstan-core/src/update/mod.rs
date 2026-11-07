//! Bounded, fail-closed update policy. No installed files or Vault state are writable here.
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Write, time::Duration};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

pub const ENDPOINT: &str =
    "https://github.com/Chr0nixz/capstan/releases/latest/download/latest.json";
pub const TARGET: &str = "windows-x86_64";
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
pub const MAX_NOTES_BYTES: usize = 16 * 1024;
pub const MAX_PACKAGE_BYTES: u64 = 200 * 1024 * 1024;
const RELEASES: &str = "https://github.com/Chr0nixz/capstan/releases/download/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UpdateError {
    #[error("update_unavailable")]
    Unavailable,
    #[error("update_manifest")]
    Manifest,
    #[error("update_version")]
    Version,
    #[error("update_limit")]
    Limit,
    #[error("update_signature")]
    Signature,
    #[error("update_network")]
    Network,
    #[error("update_storage")]
    Storage,
    #[error("update_canceled")]
    Canceled,
    #[error("update_busy")]
    Busy,
    #[error("update_install")]
    Install,
    #[error("update_confirmation")]
    Confirmation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub url: String,
    pub signature: String,
    pub size: u64,
}
#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: String,
    platforms: BTreeMap<String, Artifact>,
}
#[derive(Debug, Clone)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub artifact: Artifact,
}

impl Release {
    /// The SDK only sees this bounded, policy-checked static manifest.
    pub fn sdk_manifest(&self) -> Result<Vec<u8>, UpdateError> {
        serde_json::to_vec(&serde_json::json!({
            "version": self.version,
            "notes": self.notes,
            "platforms": { TARGET: self.artifact }
        }))
        .map_err(|_| UpdateError::Manifest)
    }
}

fn stable_version(value: &str) -> Result<semver::Version, UpdateError> {
    if value.len() > 64 {
        return Err(UpdateError::Version);
    }
    let version = semver::Version::parse(value).map_err(|_| UpdateError::Version)?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Err(UpdateError::Version);
    }
    Ok(version)
}

pub fn parse_manifest(bytes: &[u8], current: &str) -> Result<Release, UpdateError> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(UpdateError::Limit);
    }
    let manifest: Manifest = serde_json::from_slice(bytes).map_err(|_| UpdateError::Manifest)?;
    if stable_version(&manifest.version)? <= stable_version(current)? {
        return Err(UpdateError::Version);
    }
    if manifest.notes.len() > MAX_NOTES_BYTES {
        return Err(UpdateError::Limit);
    }
    let artifact = manifest
        .platforms
        .get(TARGET)
        .ok_or(UpdateError::Manifest)?
        .clone();
    if artifact.size == 0 || artifact.size > MAX_PACKAGE_BYTES {
        return Err(UpdateError::Limit);
    }
    // Bind the URL to the version as well as the repository, and disallow credentials,
    // arbitrary queries, archives, alternate installers and ambiguous encoded paths.
    let expected = format!("{RELEASES}v{0}/Capstan_{0}_x64-setup.exe", manifest.version);
    if artifact.url != expected {
        return Err(UpdateError::Manifest);
    }
    if artifact.signature.is_empty() || artifact.signature.len() > 4096 {
        return Err(UpdateError::Signature);
    }
    decode_signature(&artifact.signature)?;
    Ok(Release {
        version: manifest.version,
        notes: manifest.notes,
        artifact,
    })
}

fn decode_signature(value: &str) -> Result<minisign_verify::Signature, UpdateError> {
    let decoded = BASE64
        .decode(value.trim())
        .map_err(|_| UpdateError::Signature)?;
    let text = std::str::from_utf8(&decoded).map_err(|_| UpdateError::Signature)?;
    minisign_verify::Signature::decode(text).map_err(|_| UpdateError::Signature)
}

pub fn public_key_available(value: &str) -> bool {
    if value.is_empty() || value.len() > 4096 {
        return false;
    }
    BASE64
        .decode(value.trim())
        .ok()
        .and_then(|decoded| String::from_utf8(decoded).ok())
        .and_then(|text| minisign_verify::PublicKey::decode(&text).ok())
        .is_some()
}

pub fn verify_package(release: &Release, key: &str, bytes: &[u8]) -> Result<(), UpdateError> {
    if bytes.len() as u64 != release.artifact.size || bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(UpdateError::Limit);
    }
    let decoded = BASE64
        .decode(key.trim())
        .map_err(|_| UpdateError::Signature)?;
    let text = std::str::from_utf8(&decoded).map_err(|_| UpdateError::Signature)?;
    let key = minisign_verify::PublicKey::decode(text).map_err(|_| UpdateError::Signature)?;
    let signature = decode_signature(&release.artifact.signature)?;
    key.verify(bytes, &signature, true)
        .map_err(|_| UpdateError::Signature)?;
    // Only inspect trusted metadata after verifying its global signature.
    let version = signature
        .trusted_comment()
        .split('\t')
        .find_map(|field| field.strip_prefix("version:"));
    if version != Some(release.version.as_str()) {
        return Err(UpdateError::Version);
    }
    Ok(())
}

fn allowed_redirect(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && match url.host_str() {
            Some("github.com") => url.path().starts_with("/Chr0nixz/capstan/releases/"),
            Some("release-assets.githubusercontent.com" | "objects.githubusercontent.com") => true,
            _ => false,
        }
}

fn client() -> Result<reqwest::Client, UpdateError> {
    reqwest::Client::builder()
        .user_agent("Capstan-updater")
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(15 * 60))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() <= 5 && allowed_redirect(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("update_redirect")
            }
        }))
        .build()
        .map_err(|_| UpdateError::Network)
}

pub async fn check(
    current: &str,
    cancel: &CancellationToken,
) -> Result<Option<Release>, UpdateError> {
    let response = tokio::select! {
        biased;
        () = cancel.cancelled() => return Err(UpdateError::Canceled),
        response = client()?.get(ENDPOINT).send() => response.map_err(|_| UpdateError::Network)?,
    };
    if response.status() == reqwest::StatusCode::NOT_FOUND
        || response.status() == reqwest::StatusCode::NO_CONTENT
    {
        return Ok(None);
    }
    let bytes = receive_manifest(response, cancel).await?;
    // An unchanged latest release is a successful check with no update. Older releases
    // are rejected rather than presented as an installable candidate.
    let version: Manifest = serde_json::from_slice(&bytes).map_err(|_| UpdateError::Manifest)?;
    if stable_version(&version.version)? == stable_version(current)? {
        return Ok(None);
    }
    parse_manifest(&bytes, current).map(Some)
}

async fn receive_manifest(
    mut response: reqwest::Response,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, UpdateError> {
    if !response.status().is_success() {
        return Err(UpdateError::Network);
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MANIFEST_BYTES as u64)
    {
        return Err(UpdateError::Limit);
    }
    let mut bytes = Vec::new();
    loop {
        let chunk = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(UpdateError::Canceled),
            chunk = response.chunk() => chunk.map_err(|_| UpdateError::Network)?,
        };
        let Some(chunk) = chunk else {
            break;
        };
        if chunk.len() > MAX_MANIFEST_BYTES.saturating_sub(bytes.len()) {
            return Err(UpdateError::Limit);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub async fn download(
    release: &Release,
    key: &str,
    cancel: &CancellationToken,
    progress: impl FnMut(u64, u64),
) -> Result<Vec<u8>, UpdateError> {
    let response = tokio::select! {
        biased;
        () = cancel.cancelled() => return Err(UpdateError::Canceled),
        response = client()?.get(&release.artifact.url).send() => response.map_err(|_| UpdateError::Network)?,
    };
    receive_package(response, release, key, cancel, None, progress).await
}

async fn receive_package(
    mut response: reqwest::Response,
    release: &Release,
    key: &str,
    cancel: &CancellationToken,
    staging_directory: Option<&std::path::Path>,
    mut progress: impl FnMut(u64, u64),
) -> Result<Vec<u8>, UpdateError> {
    if !response.status().is_success() {
        return Err(UpdateError::Network);
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PACKAGE_BYTES || length != release.artifact.size)
    {
        return Err(UpdateError::Limit);
    }
    let temporary = staging_directory
        .map_or_else(
            tempfile::NamedTempFile::new,
            tempfile::NamedTempFile::new_in,
        )
        .map_err(|_| UpdateError::Storage)?;
    let mut writer =
        tokio::fs::File::from_std(temporary.reopen().map_err(|_| UpdateError::Storage)?);
    let mut received = 0_u64;
    loop {
        let chunk = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(UpdateError::Canceled),
            chunk = response.chunk() => chunk.map_err(|_| UpdateError::Network)?,
        };
        let Some(chunk) = chunk else {
            break;
        };
        received = received
            .checked_add(chunk.len() as u64)
            .ok_or(UpdateError::Limit)?;
        if received > release.artifact.size || received > MAX_PACKAGE_BYTES {
            return Err(UpdateError::Limit);
        }
        writer
            .write_all(&chunk)
            .await
            .map_err(|_| UpdateError::Storage)?;
        progress(received, release.artifact.size);
    }
    if received != release.artifact.size {
        return Err(UpdateError::Network);
    }
    writer.sync_all().await.map_err(|_| UpdateError::Storage)?;
    drop(writer);
    if cancel.is_cancelled() {
        return Err(UpdateError::Canceled);
    }
    let release = release.clone();
    let key = key.to_owned();
    let bytes = tokio::task::spawn_blocking(move || {
        let bytes = std::fs::read(temporary.path()).map_err(|_| UpdateError::Storage)?;
        verify_package(&release, &key, &bytes)?;
        Ok(bytes)
    })
    .await
    .map_err(|_| UpdateError::Storage)??;
    if cancel.is_cancelled() {
        return Err(UpdateError::Canceled);
    }
    Ok(bytes)
}

pub fn startup_check_due(enabled: bool, last: Option<u64>, now: u64) -> bool {
    enabled
        && last.is_none_or(|last| {
            now.checked_sub(last)
                .is_some_and(|elapsed| elapsed >= 86_400)
        })
}

pub fn abort_prepared_exit(
    settings: &crate::settings::Settings,
    transfers: &crate::transfer::TransferService,
    sftp: &crate::sftp::SftpService,
) {
    settings.resume();
    transfers.abort_exit("main");
    sftp.abort_tree_delete_exit("main");
}

/// The throttle is independent of settings and Vault data. Claim before networking,
/// including failed checks, so repeated launches cannot poll more than once a day.
pub fn claim_startup_check(
    directory: &std::path::Path,
    enabled: bool,
    now: u64,
) -> Result<bool, UpdateError> {
    if !enabled {
        return Ok(false);
    }
    let path = directory.join("update-check.json");
    let last = match std::fs::metadata(&path) {
        Ok(metadata) if metadata.len() <= 32 && metadata.is_file() => {
            let bytes = std::fs::read(&path).map_err(|_| UpdateError::Storage)?;
            Some(serde_json::from_slice::<u64>(&bytes).map_err(|_| UpdateError::Storage)?)
        }
        Ok(_) => return Err(UpdateError::Storage),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(UpdateError::Storage),
    };
    if !startup_check_due(enabled, last, now) {
        return Ok(false);
    }
    let mut pending =
        tempfile::NamedTempFile::new_in(directory).map_err(|_| UpdateError::Storage)?;
    write!(pending, "{now}").map_err(|_| UpdateError::Storage)?;
    pending
        .as_file()
        .sync_all()
        .map_err(|_| UpdateError::Storage)?;
    pending.persist(&path).map_err(|_| UpdateError::Storage)?;
    Ok(true)
}

#[cfg(test)]
mod tests;
