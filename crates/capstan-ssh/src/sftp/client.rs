use super::{SftpWire, MAX_PACKET_BYTES};
use russh_sftp::{
    client::{error::Error, Config, RawSftpSession},
    protocol::{FileAttributes, OpenFlags, Packet, StatusCode},
};
use std::{
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use thiserror::Error as ThisError;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{OwnedSemaphorePermit, Semaphore},
};
use tokio_util::sync::CancellationToken;

const IN_FLIGHT: usize = 208;
const WAITING: usize = 128;
const HANDLES: usize = 64;
const BLOCK: usize = super::wire::MAX_DATA_BYTES;
/// Connection-wide admission, shared by every subsystem on one transport.
pub(crate) struct Admission {
    slots: Arc<Semaphore>,
    callers: Arc<Semaphore>,
    handles: Arc<Semaphore>,
}
impl Default for Admission {
    fn default() -> Self {
        Self {
            slots: Arc::new(Semaphore::new(IN_FLIGHT)),
            callers: Arc::new(Semaphore::new(IN_FLIGHT + WAITING)),
            handles: Arc::new(Semaphore::new(HANDLES)),
        }
    }
}

#[path = "pipeline.rs"]
mod pipeline;
pub use pipeline::PipelinedFile;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ThisError)]
pub enum ClientError {
    #[error("sftp_unavailable")]
    Unavailable,
    #[error("permission_denied")]
    PermissionDenied,
    #[error("not_found")]
    NotFound,
    #[error("unsupported_file_type")]
    Unsupported,
    #[error("remote_io")]
    RemoteIo,
    #[error("connection_lost")]
    ConnectionLost,
    #[error("operation_canceled")]
    Canceled,
    #[error("operation_uncertain")]
    Uncertain,
    #[error("resource_limit")]
    ResourceLimit,
    #[error("path_invalid")]
    InvalidPath,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attributes {
    pub size: Option<u64>,
    pub modified_seconds: Option<u32>,
    pub permissions: Option<u32>,
}
impl From<FileAttributes> for Attributes {
    fn from(value: FileAttributes) -> Self {
        Self {
            size: value.size,
            modified_seconds: value.mtime,
            permissions: value.permissions,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Capabilities {
    pub atomic_replace: bool,
    pub fsync: bool,
    pub request_bytes: usize,
}

pub struct SftpClient {
    raw: Arc<RawSftpSession>,
    wire: SftpWire,
    slots: Arc<Semaphore>,
    callers: Arc<Semaphore>,
    handles: Arc<Semaphore>,
    local_handles: Arc<Semaphore>,
    pub(super) pending: tokio_util::task::TaskTracker,
    remaining: AtomicU64,
    capabilities: Capabilities,
    keepalive: std::sync::Mutex<Option<crate::ConnectionLease<crate::PasswordConnection>>>,
    // Only the primary owns companions. The connection cache is weak, so
    // retaining reusable data clients here cannot form a connection cycle.
    companions: std::sync::Mutex<[Option<Arc<Self>>; 2]>,
}

impl SftpClient {
    pub async fn connect<S>(stream: S, cancel: &CancellationToken) -> Result<Arc<Self>, ClientError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        Self::connect_with_admission(stream, cancel, &Admission::default()).await
    }
    pub(crate) async fn connect_with_admission<S>(
        stream: S,
        cancel: &CancellationToken,
        admission: &Admission,
    ) -> Result<Arc<Self>, ClientError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (stream, wire) = SftpWire::bridge(stream);
        let mut raw = RawSftpSession::new_with_config(
            stream,
            Config {
                request_timeout_secs: 15,
                max_packet_len: MAX_PACKET_BYTES as u32,
                ..Default::default()
            },
        );
        let mut initialization_admission = None;
        let initialized = async {
            let version = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(ClientError::Canceled),
                result = tokio::time::timeout(Duration::from_secs(15), raw.init()) => result.map_err(|_| ClientError::Unavailable)?.map_err(|_| ClientError::Unavailable)?,
            };
            if version.version != 3 {
                return Err(ClientError::Unavailable);
            }
            let has = |extension: &str| {
                version
                    .extensions
                    .get(extension)
                    .is_some_and(|version| version == "1")
            };
            let mut bytes = BLOCK;
            let mut handles = HANDLES;
            let mut used = 0;
            if has("limits@openssh.com") {
                let caller = admission.callers.clone().try_acquire_owned()
                    .map_err(|_| ClientError::ResourceLimit)?;
                let slot = tokio::select! {
                    biased;
                    () = cancel.cancelled() => return Err(ClientError::Canceled),
                    () = wire.cancelled() => return Err(ClientError::ConnectionLost),
                    permit = admission.slots.clone().acquire_owned() => permit.map_err(|_| ClientError::ConnectionLost)?,
                };
                // A canceled limits request keeps both permits until the
                // failed initialization has closed and joined its wire.
                initialization_admission = Some((caller, slot));
                let limits = tokio::select! {
                    biased;
                    () = cancel.cancelled() => return Err(ClientError::Canceled),
                    result = raw.limits() => result.map_err(map_error)?,
                };
                let mut limits = russh_sftp::client::rawsession::Limits::from(limits);
                // Account for the maximum hex handle and packet fields too.
                for limit in [
                    limits.read_len,
                    limits.write_len,
                    limits.packet_len.map(|size| size.saturating_sub(1024)),
                ]
                .into_iter()
                .flatten()
                {
                    bytes = bytes.min(usize::try_from(limit).unwrap_or(usize::MAX));
                }
                if bytes == 0 {
                    return Err(ClientError::Unavailable);
                }
                if let Some(limit) = limits.open_handles {
                    handles = handles.min(usize::try_from(limit).unwrap_or(usize::MAX));
                }
                limits.packet_len = Some(
                    limits
                        .packet_len
                        .unwrap_or(MAX_PACKET_BYTES as u64)
                        .min(MAX_PACKET_BYTES as u64),
                );
                raw.set_limits(limits);
                used = 1;
            }
            Ok((Capabilities {
                atomic_replace: has("posix-rename@openssh.com"),
                fsync: has("fsync@openssh.com"),
                request_bytes: bytes,
            }, handles, used))
        }.await;
        let (capabilities, handles, used) = match initialized {
            Ok(value) => value,
            Err(error) => {
                let _ = raw.close_session();
                wire.close().await;
                return Err(error);
            }
        };
        Ok(Arc::new(Self {
            raw: Arc::new(raw),
            wire,
            slots: admission.slots.clone(),
            callers: admission.callers.clone(),
            handles: admission.handles.clone(),
            local_handles: Arc::new(Semaphore::new(handles)),
            pending: tokio_util::task::TaskTracker::new(),
            remaining: AtomicU64::new(u64::from(u32::MAX) - used),
            capabilities,
            keepalive: std::sync::Mutex::new(None),
            companions: std::sync::Mutex::new([None, None]),
        }))
    }
    pub fn capabilities(&self) -> Capabilities {
        self.capabilities
    }
    pub(crate) fn retain_companion(
        &self,
        index: usize,
        client: Arc<Self>,
    ) -> Result<(), ClientError> {
        let mut companions = self
            .companions
            .lock()
            .map_err(|_| ClientError::ConnectionLost)?;
        let slot = companions
            .get_mut(index)
            .ok_or(ClientError::ResourceLimit)?;
        *slot = Some(client);
        Ok(())
    }
    pub(crate) fn retain_connection(
        &self,
        lease: crate::ConnectionLease<crate::PasswordConnection>,
    ) -> Result<(), ClientError> {
        *self
            .keepalive
            .lock()
            .map_err(|_| ClientError::ConnectionLost)? = Some(lease);
        Ok(())
    }
    pub fn is_closed(&self) -> bool {
        self.wire.is_closed()
    }
    /// Observe EOF, protocol quarantine or explicit close without acquiring a
    /// request slot. Dropping this wait does not cancel the shared subsystem.
    pub async fn closed(&self) {
        self.wire.cancelled().await;
    }
    /// Coordinated exit waits for owned protocol cleanup after admission stops.
    pub async fn wait_pending(&self) {
        self.pending.close();
        self.pending.wait().await;
    }
    pub async fn close(&self) {
        let _ = self.raw.close_session();
        self.wire.close().await;
        if let Ok(mut keepalive) = self.keepalive.lock() {
            keepalive.take();
        }
    }
    pub(crate) fn reaper(&self) -> super::SftpReaper {
        self.wire.reaper()
    }

    async fn request<T: Send + 'static, F: Future<Output = Result<T, Error>> + Send + 'static>(
        self: &Arc<Self>,
        cancel: &CancellationToken,
        mutation: bool,
        operation: impl FnOnce(Arc<RawSftpSession>) -> F + Send,
    ) -> Result<T, ClientError> {
        self.request_with_completion(cancel, cancel, mutation, operation)
            .await
    }
    async fn request_with_completion<
        T: Send + 'static,
        F: Future<Output = Result<T, Error>> + Send + 'static,
    >(
        self: &Arc<Self>,
        cancel: &CancellationToken,
        completion: &CancellationToken,
        mutation: bool,
        operation: impl FnOnce(Arc<RawSftpSession>) -> F + Send,
    ) -> Result<T, ClientError> {
        let caller = self
            .callers
            .clone()
            .try_acquire_owned()
            .map_err(|_| ClientError::ResourceLimit)?;
        let slot = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(ClientError::Canceled),
            () = self.wire.cancelled() => return Err(ClientError::ConnectionLost),
            permit = self.slots.clone().acquire_owned() => permit.map_err(|_| ClientError::ConnectionLost)?,
        };
        if self
            .remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .is_err()
        {
            self.wire.cancel();
            return Err(ClientError::ResourceLimit);
        }
        // Construct owned arguments only after admission. Cancellation returns
        // promptly but the independent drainer retains both permits until the
        // reply, EOF or protocol timeout, so repeated cancels cannot flood TX.
        let operation = operation(self.raw.clone());
        let owner = self.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        self.pending.spawn(async move {
            let admission = (caller, slot);
            let uncertain = if mutation {
                ClientError::Uncertain
            } else {
                ClientError::ConnectionLost
            };
            let result = tokio::select! {
                biased;
                () = owner.wire.cancelled() => Err(uncertain),
                result = tokio::time::timeout(Duration::from_secs(15), operation) => match result {
                    Ok(Ok(value)) => Ok(value),
                    Ok(Err(Error::Status(status))) => Err(map_error(Error::Status(status))),
                    Ok(Err(Error::Limited(_))) => Err(ClientError::ResourceLimit),
                    _ => { owner.close().await; Err(uncertain) },
                },
            };
            drop(admission);
            let _ = send.send(result);
        });
        tokio::select! {
            biased;
            () = completion.cancelled() => Err(if mutation { ClientError::Uncertain } else { ClientError::Canceled }),
            result = receive => result.map_err(|_| ClientError::ConnectionLost)?,
        }
    }
    pub async fn realpath(
        self: &Arc<Self>,
        path: &str,
        cancel: &CancellationToken,
    ) -> Result<String, ClientError> {
        if path != "." {
            validate_path(path)?;
        }
        let result = self
            .request(cancel, false, |raw| {
                let path = path.to_owned();
                async move { raw.realpath(path).await }
            })
            .await?;
        if result.files.len() != 1 {
            return Err(ClientError::RemoteIo);
        }
        let path = &result.files[0].filename;
        validate_path(path)?;
        if path.contains('\u{fffd}') {
            return Err(ClientError::Unsupported);
        }
        Ok(path.clone())
    }
    pub async fn lstat(
        self: &Arc<Self>,
        path: &str,
        cancel: &CancellationToken,
    ) -> Result<Attributes, ClientError> {
        validate_path(path)?;
        self.request(cancel, false, |raw| {
            let path = path.to_owned();
            async move { raw.lstat(path).await }
        })
        .await
        .map(|attrs| attrs.attrs.into())
    }
    pub async fn open_directory(
        self: &Arc<Self>,
        path: &str,
        cancel: &CancellationToken,
    ) -> Result<RemoteDirectory, ClientError> {
        validate_path(path)?;
        let handle = self.open_handle(path.to_owned(), None, cancel).await?;
        Ok(RemoteDirectory { handle, eof: false })
    }
    pub async fn open_file(
        self: &Arc<Self>,
        path: &str,
        create: bool,
        cancel: &CancellationToken,
    ) -> Result<RemoteFile, ClientError> {
        let flags = if create {
            OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE
        } else {
            OpenFlags::READ
        };
        self.open_file_with_flags(path, flags, create, cancel).await
    }

    /// Open an existing regular file for offset writes without truncating it.
    /// This is only used for a previously owned, prefix-verified transfer
    /// temporary; callers must never pass a WebView-provided path here.
    pub async fn open_existing_file(
        self: &Arc<Self>,
        path: &str,
        cancel: &CancellationToken,
    ) -> Result<RemoteFile, ClientError> {
        self.open_file_with_flags(path, OpenFlags::WRITE, true, cancel)
            .await
    }

    async fn open_file_with_flags(
        self: &Arc<Self>,
        path: &str,
        flags: OpenFlags,
        writable: bool,
        cancel: &CancellationToken,
    ) -> Result<RemoteFile, ClientError> {
        validate_path(path)?;
        let handle = self
            .open_handle(path.to_owned(), Some(flags), cancel)
            .await?;
        Ok(RemoteFile { handle, writable })
    }
    /// An admitted OPEN is drained independently of its caller. If cancellation
    /// wins, a late HANDLE is closed exactly once, with admission held through
    /// cleanup. Only an undrainable protocol timeout quarantines the subsystem.
    async fn open_handle(
        self: &Arc<Self>,
        path: String,
        flags: Option<OpenFlags>,
        cancel: &CancellationToken,
    ) -> Result<OwnedHandle, ClientError> {
        if cancel.is_cancelled() {
            return Err(ClientError::Canceled);
        }
        let permit = self
            .handles
            .clone()
            .try_acquire_owned()
            .map_err(|_| ClientError::ResourceLimit)?;
        let local_permit = self
            .local_handles
            .clone()
            .try_acquire_owned()
            .map_err(|_| ClientError::ResourceLimit)?;
        let permit = HandleAdmission {
            _global: permit,
            _local: local_permit,
        };
        let owner = self.clone();
        let admission = cancel.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        self.pending.spawn(async move {
            let completion = CancellationToken::new();
            let result = owner
                .request_with_completion(&admission, &completion, true, |raw| async move {
                    match flags {
                        Some(flags) => {
                            raw.open(
                                path,
                                flags,
                                FileAttributes {
                                    permissions: Some(0o600),
                                    ..Default::default()
                                },
                            )
                            .await
                        }
                        None => raw.opendir(path).await,
                    }
                })
                .await;
            if matches!(result, Err(ClientError::Uncertain)) {
                owner.close().await;
            }
            let result = result.map(|handle| OwnedHandle {
                owner,
                value: handle.handle,
                _permit: Some(permit),
                closed: false,
            });
            let _ = send.send(result);
        });
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(ClientError::Uncertain),
            result = receive => result.map_err(|_| ClientError::ConnectionLost)?,
        }
    }
    pub async fn mkdir(
        self: &Arc<Self>,
        path: &str,
        cancel: &CancellationToken,
    ) -> Result<(), ClientError> {
        validate_path(path)?;
        self.request(cancel, true, |raw| {
            let path = path.to_owned();
            async move {
                raw.mkdir(
                    path,
                    FileAttributes {
                        permissions: Some(0o700),
                        ..Default::default()
                    },
                )
                .await
            }
        })
        .await
        .map(|_| ())
    }
    pub async fn remove_file(
        self: &Arc<Self>,
        path: &str,
        cancel: &CancellationToken,
    ) -> Result<(), ClientError> {
        validate_path(path)?;
        self.request(cancel, true, |raw| {
            let path = path.to_owned();
            async move { raw.remove(path).await }
        })
        .await
        .map(|_| ())
    }
    pub async fn remove_empty_directory(
        self: &Arc<Self>,
        path: &str,
        cancel: &CancellationToken,
    ) -> Result<(), ClientError> {
        validate_path(path)?;
        self.request(cancel, true, |raw| {
            let path = path.to_owned();
            async move { raw.rmdir(path).await }
        })
        .await
        .map(|_| ())
    }
    pub async fn rename(
        self: &Arc<Self>,
        source: &str,
        target: &str,
        cancel: &CancellationToken,
    ) -> Result<(), ClientError> {
        validate_path(source)?;
        validate_path(target)?;
        self.request(cancel, true, |raw| {
            let source = source.to_owned();
            let target = target.to_owned();
            async move { raw.rename(source, target).await }
        })
        .await
        .map(|_| ())
    }
    pub async fn set_permissions(
        self: &Arc<Self>,
        path: &str,
        mode: u32,
        cancel: &CancellationToken,
    ) -> Result<(), ClientError> {
        validate_path(path)?;
        if mode > 0o7777 {
            return Err(ClientError::InvalidPath);
        }
        self.request(cancel, true, |raw| {
            let path = path.to_owned();
            async move {
                raw.setstat(
                    path,
                    FileAttributes {
                        permissions: Some(mode),
                        ..Default::default()
                    },
                )
                .await
            }
        })
        .await
        .map(|_| ())
    }
    /// Never emulate replacement with REMOVE followed by RENAME.
    pub async fn atomic_replace(
        self: &Arc<Self>,
        source: &str,
        target: &str,
        cancel: &CancellationToken,
    ) -> Result<(), ClientError> {
        validate_path(source)?;
        validate_path(target)?;
        if !self.capabilities.atomic_replace {
            return Err(ClientError::Unsupported);
        }
        let mut data = Vec::with_capacity(source.len() + target.len() + 8);
        for path in [source, target] {
            data.extend_from_slice(&(path.len() as u32).to_be_bytes());
            data.extend_from_slice(path.as_bytes());
        }
        self.request(cancel, true, |raw| async move {
            match raw.extended("posix-rename@openssh.com", data).await? {
                Packet::Status(status) if status.status_code == StatusCode::Ok => Ok(()),
                Packet::Status(status) => Err(Error::Status(status)),
                _ => Err(Error::UnexpectedPacket),
            }
        })
        .await
    }
}
fn validate_path(path: &str) -> Result<(), ClientError> {
    if !path.starts_with('/') || path.len() > 4096 || path.chars().any(char::is_control) {
        Err(ClientError::InvalidPath)
    } else {
        Ok(())
    }
}
fn map_error(error: Error) -> ClientError {
    match error {
        Error::Status(status) => match status.status_code {
            StatusCode::PermissionDenied => ClientError::PermissionDenied,
            StatusCode::NoSuchFile => ClientError::NotFound,
            StatusCode::OpUnsupported => ClientError::Unsupported,
            StatusCode::NoConnection | StatusCode::ConnectionLost => ClientError::ConnectionLost,
            _ => ClientError::RemoteIo,
        },
        _ => ClientError::ConnectionLost,
    }
}

struct HandleAdmission {
    _global: OwnedSemaphorePermit,
    _local: OwnedSemaphorePermit,
}
struct OwnedHandle {
    owner: Arc<SftpClient>,
    value: String,
    _permit: Option<HandleAdmission>,
    closed: bool,
}
impl OwnedHandle {
    async fn close(mut self, cancel: &CancellationToken) -> Result<(), ClientError> {
        let owner = self.owner.clone();
        let result = owner
            .request(cancel, true, |raw| {
                let value = self.value.clone();
                let permit = self._permit.take();
                async move {
                    let _permit = permit;
                    raw.close(value).await
                }
            })
            .await;
        self.closed = result.is_ok()
            || matches!(
                result,
                Err(ClientError::Uncertain | ClientError::ConnectionLost)
            );
        result.map(|_| ())
    }
}
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // Drain this handle independently. Dropping one request must not
        // quarantine the shared SFTP subsystem or sibling shell operations.
        if !self.closed && !self.owner.is_closed() {
            let owner = self.owner.clone();
            let value = self.value.clone();
            let permit = self._permit.take();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                let pending = owner.pending.clone();
                runtime.spawn(pending.track_future(async move {
                    let _permit = permit;
                    let cancel = CancellationToken::new();
                    if matches!(
                        owner
                            .request(&cancel, true, |raw| async move { raw.close(value).await })
                            .await,
                        Err(ClientError::Uncertain | ClientError::ResourceLimit)
                    ) {
                        owner.close().await;
                    }
                }));
            }
        }
    }
}
pub struct RemoteDirectory {
    handle: OwnedHandle,
    eof: bool,
}
impl RemoteDirectory {
    /// One server packet only; never accumulates an unbounded directory.
    pub async fn next(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<Vec<(String, Attributes)>, ClientError> {
        if self.eof {
            return Ok(Vec::new());
        }
        let result = self
            .handle
            .owner
            .request(cancel, false, |raw| {
                let value = self.handle.value.clone();
                async move {
                    match raw.readdir(value).await {
                        Err(Error::Status(status)) if status.status_code == StatusCode::Eof => {
                            Ok(None)
                        }
                        result => result.map(Some),
                    }
                }
            })
            .await?;
        match result {
            Some(page) => Ok(page
                .files
                .into_iter()
                .map(|file| (file.filename, file.attrs.into()))
                .collect()),
            None => {
                self.eof = true;
                Ok(Vec::new())
            }
        }
    }
    pub async fn close(self, cancel: &CancellationToken) -> Result<(), ClientError> {
        self.handle.close(cancel).await
    }
}
pub struct RemoteFile {
    handle: OwnedHandle,
    writable: bool,
}

impl RemoteFile {
    pub async fn metadata(&self, cancel: &CancellationToken) -> Result<Attributes, ClientError> {
        self.handle
            .owner
            .request(cancel, false, |raw| {
                let value = self.handle.value.clone();
                async move { raw.fstat(value).await }
            })
            .await
            .map(|attrs| attrs.attrs.into())
    }
    /// A successful file fsync does not imply a durable parent-directory rename.
    pub async fn sync(&self, cancel: &CancellationToken) -> Result<(), ClientError> {
        if !self.handle.owner.capabilities.fsync {
            return Err(ClientError::Unsupported);
        }
        self.handle
            .owner
            .request(cancel, true, |raw| {
                let value = self.handle.value.clone();
                async move { raw.fsync(value).await }
            })
            .await
            .map(|_| ())
    }
    pub async fn read(
        &self,
        offset: u64,
        length: usize,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, ClientError> {
        if self.writable
            || length == 0
            || length > self.handle.owner.capabilities.request_bytes
            || offset.checked_add(length as u64).is_none()
        {
            return Err(ClientError::InvalidPath);
        }
        let data = self
            .handle
            .owner
            .request(cancel, false, |raw| {
                let value = self.handle.value.clone();
                async move {
                    match raw.read(value, offset, length as u32).await {
                        Err(Error::Status(status)) if status.status_code == StatusCode::Eof => {
                            Ok(Vec::new())
                        }
                        result => result.map(|data| data.data),
                    }
                }
            })
            .await?;
        if data.len() > length {
            return Err(ClientError::RemoteIo);
        }
        Ok(data)
    }
    pub async fn write(
        &self,
        offset: u64,
        data: &[u8],
        cancel: &CancellationToken,
    ) -> Result<(), ClientError> {
        if !self.writable
            || data.len() > self.handle.owner.capabilities.request_bytes
            || offset.checked_add(data.len() as u64).is_none()
        {
            return Err(ClientError::InvalidPath);
        }
        self.handle
            .owner
            .request(cancel, true, |raw| {
                // Allocate only after bounded request admission.
                let value = self.handle.value.clone();
                let data = data.to_vec();
                async move { raw.write(value, offset, data).await }
            })
            .await
            .map(|_| ())
    }
    pub async fn close(self, cancel: &CancellationToken) -> Result<(), ClientError> {
        self.handle.close(cancel).await
    }
}

#[cfg(test)]
#[path = "capability_tests.rs"]
mod capability_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

    #[test]
    fn protocol_admission_matches_frozen_transfer_and_control_headroom() {
        let policy: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../docs/agent/baselines/M2/performance-policy.json"
        ))
        .unwrap();
        let limits = &policy["transferResourceLimits"];
        for (key, actual) in [
            ("protocolInflight", IN_FLIGHT),
            ("protocolWaiting", WAITING),
            ("remoteHandles", HANDLES),
            ("requestPayloadBytes", BLOCK),
            ("requestsPerFile", pipeline::FILE_REQUESTS),
        ] {
            assert_eq!(limits[key].as_u64(), Some(actual as u64), "{key}");
        }
        let data =
            limits["globalWorkers"].as_u64().unwrap() * limits["requestsPerFile"].as_u64().unwrap();
        assert_eq!(IN_FLIGHT as u64, data + 16);
    }

    async fn handshake(mut server: DuplexStream) -> DuplexStream {
        assert_eq!(server.read_u32().await.unwrap(), 5);
        let mut init = [0; 5];
        server.read_exact(&mut init).await.unwrap();
        assert_eq!(init, [1, 0, 0, 0, 3]);
        server.write_u32(5).await.unwrap();
        server.write_all(&[2, 0, 0, 0, 3]).await.unwrap();
        server
    }
    async fn handshake_and_hold(server: DuplexStream) {
        let _stream = handshake(server).await;
        std::future::pending::<()>().await;
    }

    #[tokio::test]
    async fn cancellation_before_a_request_is_bounded_and_does_not_send_a_packet() {
        let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(handshake_and_hold(server_stream));
        let cancel = CancellationToken::new();
        let client = SftpClient::connect(client_stream, &CancellationToken::new())
            .await
            .unwrap();
        cancel.cancel();
        let result = client.lstat("/safe", &cancel).await;
        assert_eq!(result, Err(ClientError::Canceled));
        assert!(!client.is_closed());
        client.close().await;
        assert!(client.is_closed());
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn canceled_request_does_not_quarantine_a_shared_subsystem() {
        let (client_stream, mut server_stream) = tokio::io::duplex(64 * 1024);
        let (first_seen, first_wait) = tokio::sync::oneshot::channel();
        let (release, release_wait) = tokio::sync::oneshot::channel();
        let (done, done_wait) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            server_stream = handshake(server_stream).await;
            let request = {
                let length = server_stream.read_u32().await.unwrap() as usize;
                let mut packet = vec![0; length];
                server_stream.read_exact(&mut packet).await.unwrap();
                packet
            };
            first_seen.send(()).unwrap();
            release_wait.await.unwrap();
            let request_id = u32::from_be_bytes(request[1..5].try_into().unwrap());
            let mut reply = vec![105];
            reply.extend_from_slice(&request_id.to_be_bytes());
            reply.extend_from_slice(&1_u32.to_be_bytes());
            reply.extend_from_slice(&2_u64.to_be_bytes());
            server_stream.write_u32(reply.len() as u32).await.unwrap();
            server_stream.write_all(&reply).await.unwrap();
            let length = server_stream.read_u32().await.unwrap() as usize;
            let mut packet = vec![0; length];
            server_stream.read_exact(&mut packet).await.unwrap();
            let request_id = u32::from_be_bytes(packet[1..5].try_into().unwrap());
            let mut reply = vec![105];
            reply.extend_from_slice(&request_id.to_be_bytes());
            reply.extend_from_slice(&1_u32.to_be_bytes());
            reply.extend_from_slice(&3_u64.to_be_bytes());
            server_stream.write_u32(reply.len() as u32).await.unwrap();
            server_stream.write_all(&reply).await.unwrap();
            done_wait.await.unwrap();
        });
        let client = SftpClient::connect(client_stream, &CancellationToken::new())
            .await
            .unwrap();
        let cancel = CancellationToken::new();
        let first = {
            let client = client.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move { client.lstat("/one", &cancel).await })
        };
        first_wait.await.unwrap();
        cancel.cancel();
        assert_eq!(first.await.unwrap(), Err(ClientError::Canceled));
        assert_eq!(client.slots.available_permits(), IN_FLIGHT - 1);
        assert!(!client.is_closed());
        release.send(()).unwrap();
        assert_eq!(
            client
                .lstat("/two", &CancellationToken::new())
                .await
                .unwrap()
                .size,
            Some(3)
        );
        done.send(()).unwrap();
        client.close().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn invalid_paths_are_rejected_before_request_admission() {
        let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(handshake_and_hold(server_stream));
        let client = SftpClient::connect(client_stream, &CancellationToken::new())
            .await
            .unwrap();
        for path in [
            "relative",
            "/contains\u{0000}control",
            &format!("/{}", "x".repeat(4096)),
        ] {
            assert_eq!(
                client.lstat(path, &CancellationToken::new()).await,
                Err(ClientError::InvalidPath)
            );
        }
        client.close().await;
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn canceled_open_drains_a_late_handle_exactly_once_and_preserves_other_requests() {
        async fn packet(server: &mut DuplexStream) -> Vec<u8> {
            let length = server.read_u32().await.unwrap() as usize;
            let mut packet = vec![0; length];
            server.read_exact(&mut packet).await.unwrap();
            packet
        }
        async fn send(server: &mut DuplexStream, reply: &[u8]) {
            server.write_u32(reply.len() as u32).await.unwrap();
            server.write_all(reply).await.unwrap();
        }
        let (stream, server_stream) = tokio::io::duplex(65536);
        let (seen, seen_wait) = tokio::sync::oneshot::channel();
        let (release, release_wait) = tokio::sync::oneshot::channel();
        let (done, done_wait) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let mut server = handshake(server_stream).await;
            let open = packet(&mut server).await;
            assert_eq!(open[0], 3);
            seen.send(()).unwrap();
            let stat = packet(&mut server).await;
            assert_eq!(stat[0], 7);
            let mut attrs = vec![105];
            attrs.extend_from_slice(&stat[1..5]);
            attrs.extend_from_slice(&0_u32.to_be_bytes());
            send(&mut server, &attrs).await;
            release_wait.await.unwrap();
            let mut handle = vec![102];
            handle.extend_from_slice(&open[1..5]);
            handle.extend_from_slice(&4_u32.to_be_bytes());
            handle.extend_from_slice(b"late");
            send(&mut server, &handle).await;
            let close = packet(&mut server).await;
            assert_eq!(close[0], 4);
            let mut status = vec![101];
            status.extend_from_slice(&close[1..5]);
            status.extend_from_slice(&[0; 12]);
            send(&mut server, &status).await;
            // A duplicate close would be the next packet and fail this assertion.
            let stat = packet(&mut server).await;
            assert_eq!(stat[0], 7);
            let mut attrs = vec![105];
            attrs.extend_from_slice(&stat[1..5]);
            attrs.extend_from_slice(&0_u32.to_be_bytes());
            send(&mut server, &attrs).await;
            done_wait.await.unwrap();
        });
        let client = SftpClient::connect(stream, &CancellationToken::new())
            .await
            .unwrap();
        let cancel = CancellationToken::new();
        let opening = {
            let client = client.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move { client.open_file("/file", true, &cancel).await.map(|_| ()) })
        };
        seen_wait.await.unwrap();
        cancel.cancel();
        assert_eq!(opening.await.unwrap(), Err(ClientError::Uncertain));
        client
            .lstat("/sibling", &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(client.handles.available_permits(), HANDLES - 1);
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while client.handles.available_permits() != HANDLES {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        client
            .lstat("/after-close", &CancellationToken::new())
            .await
            .unwrap();
        assert!(!client.is_closed());
        done.send(()).unwrap();
        client.close().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn exhausted_request_id_budget_closes_the_subsystem_without_reuse() {
        let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(handshake_and_hold(server_stream));
        let client = SftpClient::connect(client_stream, &CancellationToken::new())
            .await
            .unwrap();
        client.remaining.store(0, Ordering::SeqCst);

        assert_eq!(
            client.lstat("/safe", &CancellationToken::new()).await,
            Err(ClientError::ResourceLimit)
        );
        assert!(client.is_closed());

        server.abort();
        let _ = server.await;
    }
}
