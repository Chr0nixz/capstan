use super::{SftpError, MAX_REMOTE_NAME_BYTES, MAX_REMOTE_PATH_BYTES};
use serde::{Deserialize, Deserializer, Serialize};

/// A POSIX protocol path. Never pass it through host-platform path utilities.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct RemotePath(String);

impl RemotePath {
    pub fn parse(value: &str) -> Result<Self, SftpError> {
        if !value.starts_with('/')
            || value.len() > MAX_REMOTE_PATH_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(SftpError::PathInvalid);
        }
        // Keep dot segments intact. A preceding component may be a symlink;
        // lexical normalization would change the server's traversal semantics.
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn join(&self, name: &RemoteName) -> Result<Self, SftpError> {
        Self::parse(&format!(
            "{}{}{}",
            self.0,
            if self.0.ends_with('/') { "" } else { "/" },
            name.as_str()
        ))
    }
}

impl<'de> Deserialize<'de> for RemotePath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// A supported, unambiguous UTF-8 directory component, before library decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteName(String);
impl RemoteName {
    pub fn from_bytes(value: &[u8]) -> Result<Self, SftpError> {
        let value = std::str::from_utf8(value).map_err(|_| SftpError::UnsupportedFileType)?;
        if value.is_empty()
            || value.len() > MAX_REMOTE_NAME_BYTES
            || matches!(value, "." | "..")
            || value.contains('/')
            || value.chars().any(char::is_control)
        {
            return Err(SftpError::PathInvalid);
        }
        Ok(Self(value.to_owned()))
    }

    /// The selected library decodes strings lossily. Until a raw-byte adapter
    /// exists, reject replacement characters too; their provenance is unknown.
    pub fn from_library_string(value: &str) -> Result<Self, SftpError> {
        if value.contains('\u{fffd}') {
            return Err(SftpError::UnsupportedFileType);
        }
        Self::from_bytes(value.as_bytes())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
