//! Narrow, already-authorized local I/O. Implementations never accept WebView paths.
use crate::sftp::SftpError;
#[cfg(any(windows, unix))]
use std::io;

/// A local file's identity as the platform reports it: on Windows `modified`
/// is a FILETIME and `attributes` the file attributes; on Unix they are
/// nanoseconds since the epoch and `st_mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalVersion {
    pub size: u64,
    pub modified: u64,
    pub identity: u128,
    pub attributes: u32,
}
impl LocalVersion {
    pub fn is_directory(&self) -> bool {
        if cfg!(windows) {
            // FILE_ATTRIBUTE_DIRECTORY
            self.attributes & 0x10 != 0
        } else {
            // S_IFMT and S_IFDIR. 0x10 here is the group write bit.
            self.attributes & 0o170_000 == 0o040_000
        }
    }
    /// Whole seconds since the Unix epoch.
    pub fn modified_seconds(&self) -> u64 {
        if cfg!(windows) {
            // FILETIME counts 100 ns ticks since 1601.
            (self.modified / 10_000_000).saturating_sub(11_644_473_600)
        } else {
            self.modified / 1_000_000_000
        }
    }
}
pub trait LocalSource: Send + Sync {
    fn name(&self) -> Result<String, SftpError>;
    fn size(&self) -> u64;
    fn read(&self, offset: u64, length: usize) -> Result<Vec<u8>, SftpError>;
    fn verify(&self) -> Result<(), SftpError>;
    /// Unix permission bits a new remote copy should start from, or `None`
    /// where local files have none and the server's defaults apply.
    fn permissions(&self) -> Option<u32> {
        None
    }
}
pub trait LocalDestination: Send + Sync {
    fn name(&self) -> Result<String, SftpError>;
    fn target_path(&self) -> Result<Option<std::path::PathBuf>, SftpError> {
        Ok(None)
    }
    fn snapshot(&self) -> Result<Option<LocalVersion>, SftpError>;
    fn retarget(&self, name: &str) -> Result<(), SftpError>;
    fn begin(&self, name: &str) -> Result<(), SftpError>;
    /// Return the size of the currently owned temporary, if one exists.
    /// Implementations that cannot expose an atomic temporary size return
    /// `None`, which disables local partial continuation safely.
    fn partial_size(&self) -> Result<Option<u64>, SftpError> {
        Ok(None)
    }
    fn partial_path(&self) -> Result<Option<std::path::PathBuf>, SftpError> {
        Ok(None)
    }
    fn partial_identity(&self) -> Result<Option<u128>, SftpError> {
        Ok(None)
    }
    fn preserve_partial(&self) -> Result<(), SftpError> {
        Ok(())
    }
    fn recover_partial(
        &self,
        _path: &std::path::Path,
        _identity: u128,
        _minimum: u64,
        _maximum: u64,
    ) -> Result<(), SftpError> {
        Err(SftpError::ScopeExpired)
    }
    fn write(&self, offset: u64, data: &[u8]) -> Result<(), SftpError>;
    fn read(&self, offset: u64, length: usize) -> Result<Vec<u8>, SftpError>;
    fn sync(&self) -> Result<(), SftpError>;
    fn commit(&self, expected: Option<LocalVersion>) -> Result<(), SftpError>;
    fn discard(&self) -> Result<(), SftpError>;
}
#[cfg(any(windows, unix))]
pub(crate) fn local_error(error: io::Error) -> SftpError {
    match error.kind() {
        io::ErrorKind::PermissionDenied => SftpError::PermissionDenied,
        io::ErrorKind::AlreadyExists => SftpError::TargetConflict,
        io::ErrorKind::InvalidInput => SftpError::PathInvalid,
        _ => SftpError::LocalIo,
    }
}
pub(crate) async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, SftpError> + Send + 'static,
) -> Result<T, SftpError> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|_| SftpError::LocalIo)?
}

#[cfg(any(windows, unix))]
impl LocalSource for crate::native_fs::Source {
    fn name(&self) -> Result<String, SftpError> {
        self.name().map_err(local_error)
    }
    fn size(&self) -> u64 {
        self.size()
    }
    fn read(&self, offset: u64, length: usize) -> Result<Vec<u8>, SftpError> {
        self.read(offset, length).map_err(local_error)
    }
    fn verify(&self) -> Result<(), SftpError> {
        self.verify().map_err(local_error)
    }
    #[cfg(unix)]
    fn permissions(&self) -> Option<u32> {
        Some(self.permissions())
    }
}
#[cfg(any(windows, unix))]
pub struct NativeDestination(std::sync::Mutex<crate::native_fs::Destination>);
#[cfg(any(windows, unix))]
impl NativeDestination {
    pub fn new(destination: crate::native_fs::Destination) -> Self {
        Self(std::sync::Mutex::new(destination))
    }
    fn with<T>(
        &self,
        operation: impl FnOnce(&mut crate::native_fs::Destination) -> io::Result<T>,
    ) -> Result<T, SftpError> {
        operation(&mut *self.0.lock().map_err(|_| SftpError::LocalIo)?).map_err(local_error)
    }
}
#[cfg(any(windows, unix))]
impl From<crate::native_fs::Version> for LocalVersion {
    fn from(value: crate::native_fs::Version) -> Self {
        Self {
            size: value.size,
            modified: value.modified,
            identity: value.identity,
            attributes: value.attributes,
        }
    }
}
#[cfg(any(windows, unix))]
impl LocalDestination for NativeDestination {
    fn name(&self) -> Result<String, SftpError> {
        self.with(|file| file.name())
    }
    fn target_path(&self) -> Result<Option<std::path::PathBuf>, SftpError> {
        self.with(|file| Ok(Some(file.target_path())))
    }
    fn snapshot(&self) -> Result<Option<LocalVersion>, SftpError> {
        self.with(|file| file.snapshot())
            .map(|value| value.map(Into::into))
    }
    fn retarget(&self, name: &str) -> Result<(), SftpError> {
        self.with(|file| file.retarget(name))
    }
    fn begin(&self, name: &str) -> Result<(), SftpError> {
        self.with(|file| file.begin(name))
    }
    fn partial_size(&self) -> Result<Option<u64>, SftpError> {
        self.with(|file| file.partial_size())
    }
    fn partial_path(&self) -> Result<Option<std::path::PathBuf>, SftpError> {
        self.with(|file| Ok(file.partial_path()))
    }
    fn partial_identity(&self) -> Result<Option<u128>, SftpError> {
        self.with(|file| file.partial_identity())
    }
    fn preserve_partial(&self) -> Result<(), SftpError> {
        self.with(|file| {
            file.preserve_partial();
            Ok(())
        })
    }
    fn recover_partial(
        &self,
        path: &std::path::Path,
        identity: u128,
        minimum: u64,
        maximum: u64,
    ) -> Result<(), SftpError> {
        self.with(|file| file.recover_partial(path, identity, minimum, maximum))
    }
    fn write(&self, offset: u64, data: &[u8]) -> Result<(), SftpError> {
        self.with(|file| file.write(offset, data))
    }
    fn read(&self, offset: u64, length: usize) -> Result<Vec<u8>, SftpError> {
        self.with(|file| file.read(offset, length))
    }
    fn sync(&self) -> Result<(), SftpError> {
        self.with(|file| file.sync())
    }
    fn commit(&self, expected: Option<LocalVersion>) -> Result<(), SftpError> {
        self.with(|file| {
            file.commit(expected.map(|value| crate::native_fs::Version {
                size: value.size,
                modified: value.modified,
                identity: value.identity,
                attributes: value.attributes,
            }))
        })
    }
    fn discard(&self) -> Result<(), SftpError> {
        self.with(|file| file.discard())
    }
}

#[cfg(test)]
mod version_tests {
    use super::LocalVersion;
    use std::{
        path::Path,
        time::{SystemTime, UNIX_EPOCH},
    };

    /// The values `native_fs` reports, read from the real file system.
    fn version(path: &Path) -> LocalVersion {
        let metadata = std::fs::metadata(path).unwrap();
        #[cfg(unix)]
        let (modified, attributes) = {
            use std::os::unix::fs::MetadataExt;
            let seconds = u64::try_from(metadata.mtime()).unwrap();
            let nanos = u64::try_from(metadata.mtime_nsec()).unwrap();
            (seconds * 1_000_000_000 + nanos, metadata.mode())
        };
        #[cfg(windows)]
        let (modified, attributes) = {
            use std::os::windows::fs::MetadataExt;
            (metadata.last_write_time(), metadata.file_attributes())
        };
        LocalVersion {
            size: metadata.len(),
            modified,
            identity: 0,
            attributes,
        }
    }

    #[test]
    fn local_versions_read_kind_and_time_the_platform_way() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("shared.txt");
        std::fs::write(&file, b"x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Windows reads 0x10 as "directory"; in st_mode it is group write.
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o664)).unwrap();
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        assert!(!version(&file).is_directory());
        assert!(version(directory.path()).is_directory());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(version(&file).modified_seconds().abs_diff(now) < 300);
    }
}
