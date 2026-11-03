//! Narrow, already-authorized local I/O. Implementations never accept WebView paths.
use crate::sftp::SftpError;
#[cfg(windows)]
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalVersion {
    pub size: u64,
    pub modified: u64,
    pub identity: u128,
    pub attributes: u32,
}
pub trait LocalSource: Send + Sync {
    fn name(&self) -> Result<String, SftpError>;
    fn size(&self) -> u64;
    fn read(&self, offset: u64, length: usize) -> Result<Vec<u8>, SftpError>;
    fn verify(&self) -> Result<(), SftpError>;
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
#[cfg(windows)]
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

#[cfg(windows)]
impl LocalSource for capstan_native_windows::transfer::Source {
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
}
#[cfg(windows)]
pub struct WindowsDestination(std::sync::Mutex<capstan_native_windows::transfer::Destination>);
#[cfg(windows)]
impl WindowsDestination {
    pub fn new(destination: capstan_native_windows::transfer::Destination) -> Self {
        Self(std::sync::Mutex::new(destination))
    }
    fn with<T>(
        &self,
        operation: impl FnOnce(&mut capstan_native_windows::transfer::Destination) -> io::Result<T>,
    ) -> Result<T, SftpError> {
        operation(&mut *self.0.lock().map_err(|_| SftpError::LocalIo)?).map_err(local_error)
    }
}
#[cfg(windows)]
impl From<capstan_native_windows::transfer::Version> for LocalVersion {
    fn from(value: capstan_native_windows::transfer::Version) -> Self {
        Self {
            size: value.size,
            modified: value.modified,
            identity: value.identity,
            attributes: value.attributes,
        }
    }
}
#[cfg(windows)]
impl LocalDestination for WindowsDestination {
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
            file.commit(
                expected.map(|value| capstan_native_windows::transfer::Version {
                    size: value.size,
                    modified: value.modified,
                    identity: value.identity,
                    attributes: value.attributes,
                }),
            )
        })
    }
    fn discard(&self) -> Result<(), SftpError> {
        self.with(|file| file.discard())
    }
}
