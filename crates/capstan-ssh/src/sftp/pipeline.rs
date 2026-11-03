//! Admission around the selected library's existing bounded File pipeline.
use super::*;
use russh_sftp::client::fs::File;
use std::io::SeekFrom;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub(super) const FILE_REQUESTS: usize = 64;

impl RemoteFile {
    pub async fn pipeline(self, cancel: &CancellationToken) -> Result<PipelinedFile, ClientError> {
        let owner = self.handle.owner.clone();
        let callers = owner
            .callers
            .clone()
            .try_acquire_many_owned(FILE_REQUESTS as u32)
            .map_err(|_| ClientError::ResourceLimit)?;
        let slots = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(ClientError::Canceled),
            () = owner.closed() => return Err(ClientError::ConnectionLost),
            slots = owner.slots.clone().acquire_many_owned(FILE_REQUESTS as u32) => slots.map_err(|_| ClientError::ConnectionLost)?,
        };
        let file = File::from_initialized_handle(
            owner.raw.clone(),
            self.handle.value.clone(),
            Config {
                max_concurrent_reads: FILE_REQUESTS,
                max_concurrent_writes: FILE_REQUESTS,
                max_packet_len: MAX_PACKET_BYTES as u32,
                request_timeout_secs: 15,
                ..Default::default()
            },
            owner.capabilities.request_bytes as u32,
            owner.capabilities.fsync,
        );
        Ok(PipelinedFile {
            file: Some(file),
            handle: self.handle,
            _slots: Some(slots),
            _callers: Some(callers),
            payload_guard: None,
            writable: self.writable,
            sent: 0,
        })
    }
}

pub struct PipelinedFile {
    // Drop the library's buffered payload before returning request admission.
    file: Option<File>,
    handle: OwnedHandle,
    _slots: Option<OwnedSemaphorePermit>,
    _callers: Option<OwnedSemaphorePermit>,
    payload_guard: Option<Box<dyn Send + Sync>>,
    writable: bool,
    sent: u64,
}
impl Drop for PipelinedFile {
    fn drop(&mut self) {
        // The vendored File sends an unawaited CLOSE from its own Drop. Mark
        // the facade handle as covered by that close so OwnedHandle does not
        // enqueue a duplicate CLOSE request.
        if let Some(file) = self.file.take() {
            self.handle.closed = true;
            let _ = drain_owned_file(
                self.handle.owner.clone(),
                file,
                self._slots.take(),
                self._callers.take(),
                self.handle._permit.take(),
                self.payload_guard.take(),
            );
        }
    }
}
impl PipelinedFile {
    /// Keeps the caller's payload reservation alive through asynchronous
    /// cancellation cleanup. This opaque guard introduces no core dependency.
    pub fn retain_payload_guard(&mut self, guard: Box<dyn Send + Sync>) {
        self.payload_guard = Some(guard);
    }
    pub fn confirmed_bytes(&self) -> u64 {
        self.file.as_ref().map_or(0, File::confirmed_write_offset)
    }
    pub async fn read(
        &mut self,
        length: usize,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, ClientError> {
        let owner = self.handle.owner.clone();
        if self.writable || length == 0 || length > owner.capabilities.request_bytes {
            return Err(ClientError::InvalidPath);
        }
        let file = self.file.as_mut().ok_or(ClientError::ConnectionLost)?;
        owner
            .file_operation(cancel, false, async {
                let mut buffer = vec![0; length];
                let count = file.read(&mut buffer).await?;
                buffer.truncate(count);
                Ok(buffer)
            })
            .await
    }
    /// Returns only the successful contiguous ACK prefix, never queued bytes.
    pub async fn write(
        &mut self,
        data: &[u8],
        cancel: &CancellationToken,
    ) -> Result<u64, ClientError> {
        let owner = self.handle.owner.clone();
        let next = self
            .sent
            .checked_add(data.len() as u64)
            .ok_or(ClientError::InvalidPath)?;
        if !self.writable || data.len() > owner.capabilities.request_bytes {
            return Err(ClientError::InvalidPath);
        }
        let file = self.file.as_mut().ok_or(ClientError::ConnectionLost)?;
        owner
            .file_operation(cancel, true, file.write_all(data))
            .await?;
        self.sent = next;
        Ok(file.confirmed_write_offset())
    }
    /// Move the pipeline's sequential cursor to an already verified prefix.
    /// No request is issued until the next read/write operation.
    pub async fn seek_to(
        &mut self,
        offset: u64,
        cancel: &CancellationToken,
    ) -> Result<(), ClientError> {
        let owner = self.handle.owner.clone();
        let file = self.file.as_mut().ok_or(ClientError::ConnectionLost)?;
        owner
            .file_operation(cancel, false, file.seek(SeekFrom::Start(offset)))
            .await
            .map(|_| {
                self.sent = offset;
            })
    }
    pub async fn drain(&mut self, cancel: &CancellationToken) -> Result<(), ClientError> {
        let owner = self.handle.owner.clone();
        let file = self.file.as_mut().ok_or(ClientError::ConnectionLost)?;
        owner
            .file_operation(cancel, self.writable, file.drain_requests())
            .await
    }
    pub async fn metadata(&self, cancel: &CancellationToken) -> Result<Attributes, ClientError> {
        let owner = &self.handle.owner;
        owner
            .request(cancel, false, |raw| {
                let value = self.handle.value.clone();
                async move { raw.fstat(value).await }
            })
            .await
            .map(|reply| reply.attrs.into())
    }
    pub async fn sync(&mut self, cancel: &CancellationToken) -> Result<(), ClientError> {
        self.drain(cancel).await?;
        let owner = &self.handle.owner;
        if !self.writable || !owner.capabilities.fsync {
            return Err(ClientError::Unsupported);
        }
        owner
            .request(cancel, true, |raw| {
                let value = self.handle.value.clone();
                async move { raw.fsync(value).await }
            })
            .await
            .map(|_| ())
    }
    pub async fn close(mut self, cancel: &CancellationToken) -> Result<(), ClientError> {
        let drained = self.drain(cancel).await;
        let owner = self.handle.owner.clone();
        if owner.is_closed() {
            return drained.and(Err(ClientError::ConnectionLost));
        }
        let file = self.file.take().ok_or(ClientError::ConnectionLost)?;
        // File::close/Drop owns the one CLOSE, including cancellation before
        // its acknowledgement. The facade must not enqueue another CLOSE.
        self.handle.closed = true;
        let receive = drain_owned_file(
            owner.clone(),
            file,
            self._slots.take(),
            self._callers.take(),
            self.handle._permit.take(),
            self.payload_guard.take(),
        )?;
        let closed = tokio::select! {
            biased;
            () = cancel.cancelled() => Err(ClientError::Uncertain),
            () = owner.closed() => Err(ClientError::ConnectionLost),
            result = receive => result.map_err(|_| ClientError::ConnectionLost)?,
        };
        closed?;
        drained
    }
}

fn drain_owned_file(
    owner: Arc<SftpClient>,
    file: File,
    slots: Option<OwnedSemaphorePermit>,
    callers: Option<OwnedSemaphorePermit>,
    handle: Option<super::HandleAdmission>,
    guard: Option<Box<dyn Send + Sync>>,
) -> Result<tokio::sync::oneshot::Receiver<Result<(), ClientError>>, ClientError> {
    let runtime = tokio::runtime::Handle::try_current().map_err(|_| ClientError::ConnectionLost)?;
    let (send, receive) = tokio::sync::oneshot::channel();
    let pending = owner.pending.clone();
    runtime.spawn(pending.track_future(async move {
        let admission = (slots, callers, handle, guard);
        let result = owner
            .file_operation(&CancellationToken::new(), true, file.close())
            .await;
        if matches!(
            result,
            Err(ClientError::Uncertain | ClientError::ConnectionLost)
        ) {
            owner.close().await;
        }
        drop(admission);
        let _ = send.send(result);
    }));
    Ok(receive)
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;

impl SftpClient {
    async fn file_operation<T>(
        &self,
        cancel: &CancellationToken,
        mutation: bool,
        operation: impl Future<Output = std::io::Result<T>>,
    ) -> Result<T, ClientError> {
        if cancel.is_cancelled() {
            return Err(ClientError::Canceled);
        }
        if self.is_closed() {
            return Err(ClientError::ConnectionLost);
        }
        let result = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(if mutation { ClientError::Uncertain } else { ClientError::Canceled }),
            () = self.closed() => return Err(if mutation { ClientError::Uncertain } else { ClientError::ConnectionLost }),
            result = tokio::time::timeout(Duration::from_secs(15), operation) => match result {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(error)) => match error.get_ref().and_then(|error| error.downcast_ref::<Error>()) {
                    Some(Error::Status(status)) => Err(map_error(Error::Status(status.clone()))),
                    Some(Error::Limited(_)) => return Err(ClientError::ResourceLimit),
                    _ => { self.close().await; return Err(if mutation { ClientError::Uncertain } else { ClientError::ConnectionLost }); },
                },
                _ => { self.close().await; return Err(if mutation { ClientError::Uncertain } else { ClientError::ConnectionLost }); },
            },
        };
        result
    }
}
