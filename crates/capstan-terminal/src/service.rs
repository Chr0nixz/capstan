use std::collections::HashMap;
use std::future::Future;
use std::io::{Read, Write};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, watch, OwnedSemaphorePermit, Semaphore};

use crate::{
    FrameSink, OutputConfig, OutputMetrics, OutputPump, OutputPumpHandle, Result, TerminalError,
};

const INPUT_BYTES: usize = 64 * 1024;
const ACK_TIMEOUT: Duration = Duration::from_secs(30);

/// Backend controls must not share a lock with blocking reads or writes.
/// Closing must terminate when requested and release the endpoint so both
/// independent I/O workers return. This contract is tested with fake endpoints.
pub trait BackendControl: Send + Sync + 'static {
    fn resize(&self, columns: u16, rows: u16) -> Result<()>;
    fn try_wait(&self) -> Result<Option<u32>>;
    fn close(&self, terminate: bool) -> Result<Option<u32>>;
}

pub struct BackendParts {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub control: Arc<dyn BackendControl>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendPoll {
    Running,
    Exited(Option<u32>),
}

pub trait AsyncBackendControl: Send + Sync + 'static {
    fn resize<'a>(
        &'a self,
        columns: u16,
        rows: u16,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
    fn poll<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<BackendPoll>> + Send + 'a>>;
    fn close<'a>(
        &'a self,
        terminate: bool,
    ) -> Pin<Box<dyn Future<Output = Result<Option<u32>>> + Send + 'a>>;
}

pub struct AsyncBackendParts {
    pub reader: Pin<Box<dyn AsyncRead + Send>>,
    pub writer: Pin<Box<dyn AsyncWrite + Send>>,
    pub control: Arc<dyn AsyncBackendControl>,
}
enum BackendRuntime {
    Blocking(BackendParts),
    Async(AsyncBackendParts),
}

enum ReaderEndpoint {
    Blocking(Box<dyn Read + Send>),
    Async(Pin<Box<dyn AsyncRead + Send>>),
}
enum WriterEndpoint {
    Blocking(Box<dyn Write + Send>),
    Async(Pin<Box<dyn AsyncWrite + Send>>),
}
#[derive(Clone)]
enum ControlEndpoint {
    Blocking(Arc<dyn BackendControl>),
    Async(Arc<dyn AsyncBackendControl>),
}
impl ControlEndpoint {
    async fn resize(&self, columns: u16, rows: u16) -> Result<()> {
        match self {
            Self::Blocking(control) => {
                let control = control.clone();
                tokio::task::spawn_blocking(move || control.resize(columns, rows))
                    .await
                    .map_err(|_| TerminalError::TaskFailed)?
            }
            Self::Async(control) => control.resize(columns, rows).await,
        }
    }
    async fn poll(&self) -> Result<BackendPoll> {
        match self {
            Self::Blocking(control) => {
                let control = control.clone();
                match tokio::task::spawn_blocking(move || control.try_wait())
                    .await
                    .map_err(|_| TerminalError::TaskFailed)??
                {
                    Some(code) => Ok(BackendPoll::Exited(Some(code))),
                    None => Ok(BackendPoll::Running),
                }
            }
            Self::Async(control) => control.poll().await,
        }
    }
    async fn close(&self, terminate: bool) -> Result<Option<u32>> {
        match self {
            Self::Blocking(control) => {
                let control = control.clone();
                tokio::task::spawn_blocking(move || control.close(terminate))
                    .await
                    .map_err(|_| TerminalError::TaskFailed)?
            }
            Self::Async(control) => control.close(terminate).await,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TerminalSessionId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalState {
    Running,
    Exited,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSnapshot {
    pub id: TerminalSessionId,
    pub state: TerminalState,
    pub exit_code: Option<u32>,
    pub error_code: Option<String>,
    pub metrics: OutputMetrics,
}

impl TerminalSnapshot {
    pub fn is_finished(&self) -> bool {
        self.state != TerminalState::Running
    }
}

struct InputRequest {
    bytes: Vec<u8>,
    reply: oneshot::Sender<Result<()>>,
    _budget: OwnedSemaphorePermit,
}

pub struct TerminalSession {
    control: ControlEndpoint,
    output: OutputPumpHandle,
    input: mpsc::Sender<InputRequest>,
    input_budget: Arc<Semaphore>,
    cancel: watch::Sender<bool>,
    status: watch::Receiver<TerminalSnapshot>,
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.output.cancel();
        self.cancel.send_replace(true);
    }
}

async fn cancelled(receiver: &mut watch::Receiver<bool>) {
    while !*receiver.borrow_and_update() {
        if receiver.changed().await.is_err() {
            break;
        }
    }
}

impl TerminalSession {
    fn start(
        id: TerminalSessionId,
        backend: BackendRuntime,
        sink: Arc<dyn FrameSink>,
    ) -> Arc<Self> {
        let (reader_endpoint, writer_endpoint, control) = match backend {
            BackendRuntime::Blocking(parts) => (
                ReaderEndpoint::Blocking(parts.reader),
                WriterEndpoint::Blocking(parts.writer),
                ControlEndpoint::Blocking(parts.control),
            ),
            BackendRuntime::Async(parts) => (
                ReaderEndpoint::Async(parts.reader),
                WriterEndpoint::Async(parts.writer),
                ControlEndpoint::Async(parts.control),
            ),
        };
        let (output, mut stream, pump) = OutputPump::start(OutputConfig::default());
        let (input, mut input_rx) = mpsc::channel::<InputRequest>(256);
        let (cancel, mut stop) = watch::channel(false);
        let supervisor_control = control.clone();
        let (status_tx, status) = watch::channel(TerminalSnapshot {
            id,
            state: TerminalState::Running,
            exit_code: None,
            error_code: None,
            metrics: OutputMetrics::default(),
        });
        let (failures, mut failure_rx) = mpsc::channel::<TerminalError>(4);
        let session = Arc::new(Self {
            control,
            output: output.clone(),
            input,
            input_budget: Arc::new(Semaphore::new(INPUT_BYTES)),
            cancel: cancel.clone(),
            status,
        });

        let runtime = tokio::runtime::Handle::current();
        let read_output = output.clone();
        let read_failures = failures.clone();
        let reader = match reader_endpoint {
            ReaderEndpoint::Blocking(mut reader) => tokio::task::spawn_blocking(move || {
                let mut buffer = vec![0; 16 * 1024];
                let mut discard = false;
                loop {
                    if !discard {
                        match runtime.block_on(tokio::time::timeout(
                            ACK_TIMEOUT,
                            read_output.wait_until_readable(),
                        )) {
                            Ok(Ok(())) => {}
                            Ok(Err(_)) => discard = true,
                            Err(_) => {
                                let _ = read_failures.try_send(TerminalError::AckTimeout);
                                read_output.cancel();
                                discard = true;
                            }
                        }
                    }
                    match reader.read(&mut buffer) {
                        Ok(0) => {
                            read_output.finish();
                            return;
                        }
                        Ok(length) if !discard => {
                            if runtime
                                .block_on(read_output.send(buffer[..length].to_vec()))
                                .is_err()
                            {
                                discard = true;
                            }
                        }
                        Ok(_) => {}
                        Err(error) => {
                            if !discard && !read_output.is_cancelled() {
                                let _ = read_failures.try_send(TerminalError::Io(error));
                            }
                            read_output.finish();
                            return;
                        }
                    }
                    // Drain without buffering after cancellation until the backend releases the pipe.
                }
            }),
            ReaderEndpoint::Async(mut reader) => tokio::spawn(async move {
                let mut buffer = vec![0; 16 * 1024];
                let mut discard = false;
                loop {
                    if !discard {
                        match tokio::time::timeout(ACK_TIMEOUT, read_output.wait_until_readable())
                            .await
                        {
                            Ok(Ok(())) => {}
                            Ok(Err(_)) => discard = true,
                            Err(_) => {
                                let _ = read_failures.try_send(TerminalError::AckTimeout);
                                read_output.cancel();
                                discard = true;
                            }
                        }
                    }
                    match reader.as_mut().read(&mut buffer).await {
                        Ok(0) => {
                            read_output.finish();
                            return;
                        }
                        Ok(length) if !discard => {
                            if read_output.send(buffer[..length].to_vec()).await.is_err() {
                                discard = true;
                            }
                        }
                        Ok(_) => {}
                        Err(error) => {
                            if !discard && !read_output.is_cancelled() {
                                let _ = read_failures.try_send(TerminalError::Io(error));
                            }
                            read_output.finish();
                            return;
                        }
                    }
                }
            }),
        };

        let write_failures = failures.clone();
        let (input_stop, mut write_stop) = watch::channel(false);
        let writer = tokio::spawn(async move {
            let mut writer = writer_endpoint;
            loop {
                let request = tokio::select! {
                    biased;
                    () = cancelled(&mut write_stop) => break,
                    request = input_rx.recv() => match request { Some(request) => request, None => break },
                };
                let (returned_writer, request, result) = match writer {
                    WriterEndpoint::Blocking(mut writer) => {
                        match tokio::task::spawn_blocking(move || {
                            let result = writer
                                .write_all(&request.bytes)
                                .and_then(|()| writer.flush())
                                .map_err(TerminalError::Io);
                            (WriterEndpoint::Blocking(writer), request, result)
                        })
                        .await
                        {
                            Ok(result) => result,
                            Err(_) => {
                                let _ = write_failures.try_send(TerminalError::TaskFailed);
                                break;
                            }
                        }
                    }
                    WriterEndpoint::Async(mut writer) => {
                        // Input can be waiting for SSH window/queue admission.
                        // Closing must cancel that wait as well as the reader;
                        // a partially submitted input is never replayed.
                        let result = tokio::select! {
                            biased;
                            () = cancelled(&mut write_stop) => Err(TerminalError::SessionClosed),
                            result = async {
                                writer.as_mut().write_all(&request.bytes).await?;
                                writer.as_mut().flush().await
                            } => result.map_err(TerminalError::Io),
                        };
                        (WriterEndpoint::Async(writer), request, result)
                    }
                };
                writer = returned_writer;
                let failed = result.is_err();
                let _ = request.reply.send(result);
                if failed && !*write_stop.borrow() {
                    let _ =
                        write_failures.try_send(TerminalError::Backend("input_write".to_owned()));
                    break;
                }
            }
            input_rx.close();
            while let Some(request) = input_rx.recv().await {
                let _ = request.reply.send(Err(TerminalError::SessionClosed));
            }
        });

        let forward_output = output.clone();
        let forward_failures = failures;
        let mut forward_stop = cancel.subscribe();
        let forwarder = tokio::spawn(async move {
            while let Some(frame) = stream.recv().await {
                let result = tokio::select! {
                    biased;
                    () = cancelled(&mut forward_stop) => break,
                    result = tokio::time::timeout(ACK_TIMEOUT, sink.send(frame)) => result,
                };
                let error = match result {
                    Ok(Ok(())) => continue,
                    Ok(Err(error)) => error,
                    Err(_) => TerminalError::AckTimeout,
                };
                forward_output.cancel();
                let _ = forward_failures.try_send(error);
                break;
            }
        });

        tokio::spawn(async move {
            let control = supervisor_control;
            let mut ticker = tokio::time::interval(Duration::from_millis(50));
            let mut previous_ack = 0;
            let mut last_progress = Instant::now();
            let (mut state, mut error, polled_exit_code) = loop {
                tokio::select! {
                    biased;
                    () = cancelled(&mut stop) => break (TerminalState::Cancelled, None, None),
                    Some(error) = failure_rx.recv() => break (TerminalState::Error, Some(error), None),
                    _ = ticker.tick() => {
                        let metrics = output.metrics().await;
                        let acknowledged = metrics.output_bytes.saturating_sub(metrics.in_flight_bytes);
                        if acknowledged != previous_ack || metrics.in_flight_bytes == 0 {
                            previous_ack = acknowledged;
                            last_progress = Instant::now();
                        } else if last_progress.elapsed() >= ACK_TIMEOUT {
                            break (TerminalState::Error, Some(TerminalError::AckTimeout), None);
                        }
                        // Poll backend liveness without flooding subscribers
                        // (and their IPC channels) when an idle tab is unchanged.
                        status_tx.send_if_modified(|status| {
                            if status.metrics == metrics {
                                return false;
                            }
                            status.metrics = metrics;
                            true
                        });
                        match control.poll().await {
                            Ok(BackendPoll::Exited(code)) => break (TerminalState::Exited, None, code),
                            Ok(BackendPoll::Running) => {}
                            Err(error) => break (TerminalState::Error, Some(error), None),
                        }
                    }
                }
            };
            if state != TerminalState::Exited {
                output.cancel();
            }
            // Stop input independently; successful EOF still drains output.
            input_stop.send_replace(true);
            if state != TerminalState::Exited {
                cancel.send_replace(true);
            }
            let force = state != TerminalState::Exited;
            let exit_code = match control.close(force).await {
                Ok(code) => code.or(polled_exit_code),
                Err(close_error) => {
                    error.get_or_insert(close_error);
                    None
                }
            };
            if reader.await.is_err() {
                error.get_or_insert(TerminalError::TaskFailed);
                output.cancel();
            }
            if writer.await.is_err() {
                error.get_or_insert(TerminalError::TaskFailed);
            }
            if forwarder.await.is_err() {
                error.get_or_insert(TerminalError::TaskFailed);
                output.cancel();
            }
            match pump.await {
                Ok(Ok(_)) => {}
                Ok(Err(pump_error)) => {
                    error.get_or_insert(pump_error);
                }
                Err(_) => {
                    error.get_or_insert(TerminalError::TaskFailed);
                }
            }
            if let Ok(late_error) = failure_rx.try_recv() {
                error.get_or_insert(late_error);
            }
            if error.is_some() {
                state = TerminalState::Error;
            }
            let metrics = output.metrics().await;
            status_tx.send_modify(|status| {
                status.state = state;
                status.exit_code = exit_code;
                status.error_code = error.map(|error| error.code().to_owned());
                status.metrics = metrics;
            });
        });
        session
    }

    pub fn subscribe(&self) -> watch::Receiver<TerminalSnapshot> {
        self.status.clone()
    }
    pub fn snapshot(&self) -> TerminalSnapshot {
        self.status.borrow().clone()
    }

    /// Includes ACKs received after the terminal's final state was published.
    pub async fn output_metrics(&self) -> OutputMetrics {
        self.output.metrics().await
    }

    pub async fn write(&self, bytes: Vec<u8>) -> Result<()> {
        if self.snapshot().is_finished() || *self.cancel.borrow() {
            return Err(TerminalError::SessionClosed);
        }
        if bytes.len() > INPUT_BYTES {
            return Err(TerminalError::ChunkTooLarge);
        }
        let budget = self
            .input_budget
            .clone()
            .try_acquire_many_owned(bytes.len() as u32)
            .map_err(|_| TerminalError::QueueFull)?;
        let (reply, response) = oneshot::channel();
        self.input
            .try_send(InputRequest {
                bytes,
                reply,
                _budget: budget,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => TerminalError::QueueFull,
                mpsc::error::TrySendError::Closed(_) => TerminalError::SessionClosed,
            })?;
        response.await.map_err(|_| TerminalError::SessionClosed)?
    }

    pub async fn resize(&self, columns: u16, rows: u16) -> Result<()> {
        if columns == 0 || rows == 0 || columns > i16::MAX as u16 || rows > i16::MAX as u16 {
            return Err(TerminalError::InvalidSize);
        }
        if self.snapshot().is_finished() {
            return Err(TerminalError::SessionClosed);
        }
        self.control.resize(columns, rows).await
    }

    pub async fn ack(&self, bytes: usize) -> Result<()> {
        self.output.ack(bytes).await.map(|_| ())
    }

    pub fn cancel(&self) {
        if !self.snapshot().is_finished() {
            self.output.cancel();
            self.cancel.send_replace(true);
        }
    }

    pub async fn close(&self) -> TerminalSnapshot {
        self.cancel();
        self.wait().await
    }

    pub async fn wait(&self) -> TerminalSnapshot {
        let mut status = self.status.clone();
        loop {
            let snapshot = status.borrow_and_update().clone();
            if snapshot.is_finished() {
                return snapshot;
            }
            if status.changed().await.is_err() {
                return TerminalSnapshot {
                    state: TerminalState::Error,
                    error_code: Some("task_failed".to_owned()),
                    ..snapshot
                };
            }
        }
    }
}

#[derive(Default)]
pub struct TerminalService {
    sessions: Mutex<HashMap<TerminalSessionId, Arc<TerminalSession>>>,
    next_id: AtomicU64,
    closing: AtomicBool,
    starts: tokio::sync::RwLock<()>,
}

impl TerminalService {
    /// Counts registrations after close has waited for all session-owned workers.
    pub fn registered_sessions(&self) -> usize {
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
    fn register(
        &self,
        backend: BackendParts,
        sink: Arc<dyn FrameSink>,
    ) -> Result<TerminalSessionId> {
        let id = TerminalSessionId(format!(
            "session-{}",
            self.next_id.fetch_add(1, Ordering::Relaxed)
        ));
        let session = TerminalSession::start(id.clone(), BackendRuntime::Blocking(backend), sink);
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        sessions.insert(id.clone(), session);
        Ok(id)
    }

    pub async fn start(
        &self,
        backend: BackendParts,
        sink: Arc<dyn FrameSink>,
    ) -> Result<TerminalSessionId> {
        let _start = self.starts.read().await;
        let id = self.register(backend, sink)?;
        if self.closing.load(Ordering::Acquire) {
            self.close(&id).await?;
            return Err(TerminalError::SessionClosed);
        }
        Ok(id)
    }

    pub async fn start_async(
        &self,
        backend: AsyncBackendParts,
        sink: Arc<dyn FrameSink>,
    ) -> Result<TerminalSessionId> {
        let _start = self.starts.read().await;
        let id = self.register_runtime(BackendRuntime::Async(backend), sink)?;
        if self.closing.load(Ordering::Acquire) {
            self.close(&id).await?;
            return Err(TerminalError::SessionClosed);
        }
        Ok(id)
    }

    /// Keep startup owned until attachment or cleanup, including window-close
    /// races while a blocking backend factory is still creating a process.
    pub async fn open<T: Send + 'static>(
        self: &Arc<Self>,
        factory: impl FnOnce() -> Result<(BackendParts, T)> + Send + 'static,
        sink: Arc<dyn FrameSink>,
    ) -> Result<(TerminalSessionId, T)> {
        let service = self.clone();
        let (reply, response) = oneshot::channel();
        tokio::spawn(async move {
            // This lease belongs to the worker, not the caller's future. A
            // cancelled IPC request cannot orphan a still-running factory.
            let _start = service.starts.read().await;
            let result = async {
                if service.closing.load(Ordering::Acquire) {
                    return Err(TerminalError::SessionClosed);
                }
                let (backend, metadata) = tokio::task::spawn_blocking(factory)
                    .await
                    .map_err(|_| TerminalError::TaskFailed)??;
                let id = service.register(backend, sink)?;
                if service.closing.load(Ordering::Acquire) {
                    service.close(&id).await?;
                    return Err(TerminalError::SessionClosed);
                }
                Ok((id, metadata))
            }
            .await;
            if let Err(Ok((id, _))) = reply.send(result) {
                let _ = service.close(&id).await;
            }
        });
        response.await.map_err(|_| TerminalError::TaskFailed)?
    }

    pub async fn open_async<T, F, Fut>(
        self: &Arc<Self>,
        factory: F,
        cancellation: tokio_util::sync::CancellationToken,
        sink: Arc<dyn FrameSink>,
    ) -> Result<(TerminalSessionId, T)>
    where
        T: Send + 'static,
        F: FnOnce(tokio_util::sync::CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(AsyncBackendParts, T)>> + Send + 'static,
    {
        let service = self.clone();
        let (mut reply, response) = oneshot::channel();
        tokio::spawn(async move {
            let _start = service.starts.read().await;
            let result = async {
                if service.closing.load(Ordering::Acquire) || cancellation.is_cancelled() {
                    return Err(TerminalError::SessionClosed);
                }
                let start_result = tokio::select! {
                    biased;
                    () = reply.closed() => { cancellation.cancel(); return Err(TerminalError::SessionClosed); },
                    () = cancellation.cancelled() => return Err(TerminalError::SessionClosed),
                    result = factory(cancellation.clone()) => result,
                };
                let started = start_result?;
                let (backend, metadata) = started;
                let id = service.register_runtime(BackendRuntime::Async(backend), sink)?;
                if service.closing.load(Ordering::Acquire) || cancellation.is_cancelled() {
                    service.close(&id).await?;
                    return Err(TerminalError::SessionClosed);
                }
                Ok((id, metadata))
            }.await;
            if let Err(Ok((id, _))) = reply.send(result) {
                let _ = service.close(&id).await;
            }
        });
        response.await.map_err(|_| TerminalError::TaskFailed)?
    }

    fn register_runtime(
        &self,
        backend: BackendRuntime,
        sink: Arc<dyn FrameSink>,
    ) -> Result<TerminalSessionId> {
        let id = TerminalSessionId(format!(
            "session-{}",
            self.next_id.fetch_add(1, Ordering::Relaxed)
        ));
        let session = TerminalSession::start(id.clone(), backend, sink);
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.clone(), session);
        Ok(id)
    }

    pub fn get(&self, id: &TerminalSessionId) -> Result<Arc<TerminalSession>> {
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .cloned()
            .ok_or(TerminalError::SessionClosed)
    }

    pub async fn close(&self, id: &TerminalSessionId) -> Result<()> {
        let session = self
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .cloned();
        if let Some(session) = session {
            let result = session.close().await;
            self.sessions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(id);
            if result.state == TerminalState::Error {
                return Err(TerminalError::Backend(
                    result.error_code.unwrap_or_default(),
                ));
            }
        }
        Ok(())
    }

    pub async fn close_all(&self) {
        self.closing.store(true, Ordering::Release);
        {
            let sessions = self
                .sessions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for session in sessions.values() {
                session.cancel();
            }
        }
        let _starts = self.starts.write().await;
        let sessions = {
            let mut sessions = self
                .sessions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.closing.store(true, Ordering::Release);
            sessions
                .drain()
                .map(|(_, session)| session)
                .collect::<Vec<_>>()
        };
        for session in &sessions {
            session.cancel();
        }
        for session in sessions {
            session.wait().await;
        }
    }

    /// Wait until all currently owned backend factories have either attached or cleaned up.
    /// Unlike `close_all`, this does not stop future or local terminal sessions.
    pub async fn wait_for_starts(&self) {
        let _starts = self.starts.write().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BinaryFrame;
    use std::io::Cursor;
    use std::sync::Condvar;

    struct AsyncFinished(watch::Sender<BackendPoll>);
    impl AsyncBackendControl for AsyncFinished {
        fn resize<'a>(
            &'a self,
            _: u16,
            _: u16,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            Box::pin(async { Ok(()) })
        }
        fn poll<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<BackendPoll>> + Send + 'a>> {
            Box::pin(async move { Ok(*self.0.borrow()) })
        }
        fn close<'a>(
            &'a self,
            terminate: bool,
        ) -> Pin<Box<dyn Future<Output = Result<Option<u32>>> + Send + 'a>> {
            Box::pin(async move {
                if terminate {
                    self.0.send_replace(BackendPoll::Exited(None));
                }
                match *self.0.borrow() {
                    BackendPoll::Exited(code) => Ok(code),
                    BackendPoll::Running => Ok(None),
                }
            })
        }
    }

    #[derive(Default)]
    struct Collect(Mutex<Vec<u8>>);
    impl FrameSink for Collect {
        fn send<'a>(
            &'a self,
            frame: BinaryFrame,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
            Box::pin(async move {
                self.0.lock().unwrap().extend(frame.payload);
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn idle_status_does_not_wake_subscribers_but_output_and_exit_do() {
        let service = TerminalService::default();
        let (backend, remote) = tokio::io::duplex(1024);
        let (reader, writer) = tokio::io::split(backend);
        let (_, mut remote_writer) = tokio::io::split(remote);
        let (exit, _) = watch::channel(BackendPoll::Running);
        let id = service
            .start_async(
                AsyncBackendParts {
                    reader: Box::pin(reader),
                    writer: Box::pin(writer),
                    control: Arc::new(AsyncFinished(exit.clone())),
                },
                Arc::new(Collect::default()),
            )
            .await
            .unwrap();
        let session = service.get(&id).unwrap();
        let mut status = session.subscribe();
        status.borrow_and_update();
        assert!(
            tokio::time::timeout(Duration::from_millis(200), status.changed())
                .await
                .is_err(),
            "idle backend polling must not generate repeated IPC status messages"
        );

        remote_writer.write_all(b"output").await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                status.changed().await.unwrap();
                if status.borrow_and_update().metrics.output_bytes == 6 {
                    break;
                }
            }
        })
        .await
        .unwrap();
        session.ack(6).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                status.changed().await.unwrap();
                if status.borrow_and_update().metrics.in_flight_bytes == 0 {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(200), status.changed())
                .await
                .is_err()
        );
        remote_writer.shutdown().await.unwrap();
        exit.send_replace(BackendPoll::Exited(Some(0)));
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                status.changed().await.unwrap();
                if status.borrow_and_update().is_finished() {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(session.snapshot().state, TerminalState::Exited);
        service.close(&id).await.unwrap();
    }

    #[tokio::test]
    async fn async_backend_uses_shared_output_input_resize_and_exit_contract() {
        let service = TerminalService::default();
        let sink = Arc::new(Collect::default());
        let (backend, remote) = tokio::io::duplex(64 * 1024);
        let (backend_reader, backend_writer) = tokio::io::split(backend);
        let (remote_reader, mut remote_writer) = tokio::io::split(remote);
        let (exit, _) = watch::channel(BackendPoll::Running);
        let id = service
            .start_async(
                AsyncBackendParts {
                    reader: Box::pin(backend_reader),
                    writer: Box::pin(backend_writer),
                    control: Arc::new(AsyncFinished(exit.clone())),
                },
                sink.clone(),
            )
            .await
            .expect("async session start");
        let session = service.get(&id).expect("async session");
        session.resize(100, 40).await.expect("async resize");
        session.write(b"input".to_vec()).await.expect("async input");
        let mut input = [0_u8; 5];
        tokio::time::timeout(
            Duration::from_secs(1),
            remote_reader.take(5).read_exact(&mut input),
        )
        .await
        .expect("input timeout")
        .expect("input bytes");
        assert_eq!(&input, b"input");
        remote_writer
            .write_all(b"async output")
            .await
            .expect("output write");
        remote_writer.flush().await.expect("output flush");
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if sink.0.lock().expect("sink lock").as_slice() == b"async output" {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("output delivery");
        remote_writer.shutdown().await.expect("remote EOF");
        exit.send_replace(BackendPoll::Exited(Some(42)));
        let snapshot = tokio::time::timeout(Duration::from_secs(2), session.wait())
            .await
            .expect("async session exit");
        assert_eq!(snapshot.state, TerminalState::Exited);
        assert_eq!(snapshot.exit_code, Some(42));
        service.close(&id).await.expect("async close");
    }

    struct StalledInput {
        stall_flush: bool,
        attempted: Arc<tokio::sync::Notify>,
        writes: Arc<AtomicU64>,
    }
    impl AsyncWrite for StalledInput {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            bytes: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            self.writes.fetch_add(1, Ordering::Relaxed);
            if self.stall_flush {
                std::task::Poll::Ready(Ok(bytes.len()))
            } else {
                self.attempted.notify_one();
                std::task::Poll::Pending
            }
        }
        fn poll_flush(
            self: Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            self.attempted.notify_one();
            std::task::Poll::Pending
        }
        fn poll_shutdown(
            self: Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn close_cancels_a_stalled_async_input_without_replaying_queued_input() {
        // A backend may accept bytes and then stall in flush, or stall before
        // its first byte. Neither path may keep terminal shutdown pending.
        for stall_flush in [false, true] {
            let service = TerminalService::default();
            let attempted = Arc::new(tokio::sync::Notify::new());
            let writes = Arc::new(AtomicU64::new(0));
            let (exit, _) = watch::channel(BackendPoll::Running);
            let id = service
                .start_async(
                    AsyncBackendParts {
                        reader: Box::pin(tokio::io::empty()),
                        writer: Box::pin(StalledInput {
                            stall_flush,
                            attempted: attempted.clone(),
                            writes: writes.clone(),
                        }),
                        control: Arc::new(AsyncFinished(exit)),
                    },
                    Arc::new(Collect::default()),
                )
                .await
                .unwrap();
            let session = service.get(&id).unwrap();
            let first = {
                let session = session.clone();
                tokio::spawn(async move { session.write(vec![1]).await })
            };
            tokio::time::timeout(Duration::from_secs(1), attempted.notified())
                .await
                .unwrap();
            let second = session.write(vec![2]);
            tokio::pin!(second);
            tokio::select! {
                biased;
                result = &mut second => panic!("queued input unexpectedly completed: {result:?}"),
                () = tokio::task::yield_now() => {}
            }
            let attempts_before_close = writes.load(Ordering::Relaxed);
            tokio::time::timeout(Duration::from_secs(1), service.close(&id))
                .await
                .expect("closing must cancel both pending input stages")
                .unwrap();
            assert!(matches!(
                first.await.unwrap(),
                Err(TerminalError::SessionClosed)
            ));
            assert!(matches!(second.await, Err(TerminalError::SessionClosed)));
            assert_eq!(writes.load(Ordering::Relaxed), attempts_before_close);
            assert_eq!(session.snapshot().state, TerminalState::Cancelled);
            assert_eq!(session.input_budget.available_permits(), INPUT_BYTES);
        }
    }

    struct Finished(AtomicBool);
    impl BackendControl for Finished {
        fn resize(&self, _: u16, _: u16) -> Result<()> {
            Ok(())
        }
        fn try_wait(&self) -> Result<Option<u32>> {
            Ok(Some(7))
        }
        fn close(&self, force: bool) -> Result<Option<u32>> {
            assert!(!force);
            self.0.store(true, Ordering::Release);
            Ok(Some(7))
        }
    }

    #[tokio::test]
    async fn natural_exit_preserves_output_and_joins_before_reporting_exit() {
        let service = TerminalService::default();
        let sink = Arc::new(Collect::default());
        let control = Arc::new(Finished(AtomicBool::new(false)));
        let expected = vec![42; 64 * 1024];
        let id = service
            .start(
                BackendParts {
                    reader: Box::new(Cursor::new(expected.clone())),
                    writer: Box::new(std::io::sink()),
                    control: control.clone(),
                },
                sink.clone(),
            )
            .await
            .expect("start");
        let session = service.get(&id).expect("session");
        let status = tokio::time::timeout(Duration::from_secs(2), session.wait())
            .await
            .expect("exit");
        assert_eq!(status.state, TerminalState::Exited);
        assert_eq!(status.exit_code, Some(7));
        assert_eq!(*sink.0.lock().unwrap(), expected);
        assert!(control.0.load(Ordering::Acquire));
        assert!(matches!(
            session.write(vec![1]).await,
            Err(TerminalError::SessionClosed)
        ));
        service.close(&id).await.expect("close");
        service.close(&id).await.expect("idempotent close");
        assert_eq!(service.registered_sessions(), 0);
    }

    #[derive(Default)]
    struct Blocked {
        closed: Mutex<bool>,
        wake: Condvar,
        writing: AtomicBool,
    }
    impl BackendControl for Blocked {
        fn resize(&self, _: u16, _: u16) -> Result<()> {
            Ok(())
        }
        fn try_wait(&self) -> Result<Option<u32>> {
            Ok(None)
        }
        fn close(&self, _: bool) -> Result<Option<u32>> {
            *self.closed.lock().unwrap() = true;
            self.wake.notify_all();
            Ok(Some(0))
        }
    }
    struct BlockedIo(Arc<Blocked>);
    impl Read for BlockedIo {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            let mut closed = self.0.closed.lock().unwrap();
            while !*closed {
                closed = self.0.wake.wait(closed).unwrap();
            }
            Ok(0)
        }
    }
    impl Write for BlockedIo {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.writing.store(true, Ordering::Release);
            let mut closed = self.0.closed.lock().unwrap();
            while !*closed {
                closed = self.0.wake.wait(closed).unwrap();
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn window_close_waits_for_cancelled_startup_and_cleans_its_backend() {
        let service = Arc::new(TerminalService::default());
        let control = Arc::new(Blocked::default());
        let factory_control = control.clone();
        let (started, started_rx) = oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        let opener = {
            let service = service.clone();
            tokio::spawn(async move {
                service
                    .open(
                        move || {
                            let _ = started.send(());
                            released.recv().expect("release startup");
                            Ok((
                                BackendParts {
                                    reader: Box::new(BlockedIo(factory_control.clone())),
                                    writer: Box::new(BlockedIo(factory_control.clone())),
                                    control: factory_control,
                                },
                                (),
                            ))
                        },
                        Arc::new(Collect::default()),
                    )
                    .await
            })
        };
        started_rx.await.expect("factory started");
        opener.abort();
        let closing = {
            let service = service.clone();
            tokio::spawn(async move {
                service.close_all().await;
            })
        };
        while !service.closing.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
        assert!(!closing.is_finished());
        release.send(()).expect("release");
        tokio::time::timeout(Duration::from_secs(2), closing)
            .await
            .expect("close completes")
            .expect("join");
        assert!(*control.closed.lock().expect("closed"));
        assert!(service.sessions.lock().expect("registry").is_empty());
    }

    #[tokio::test]
    async fn sink_failure_closes_only_its_own_session_and_reports_error() {
        struct FailedSink;
        impl FrameSink for FailedSink {
            fn send<'a>(
                &'a self,
                _: BinaryFrame,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>>
            {
                Box::pin(async { Err(TerminalError::OutputSend("closed".to_owned())) })
            }
        }
        let service = TerminalService::default();
        let healthy = Arc::new(Blocked::default());
        let failed = Arc::new(Blocked::default());
        let other = service
            .start(
                BackendParts {
                    reader: Box::new(BlockedIo(healthy.clone())),
                    writer: Box::new(std::io::sink()),
                    control: healthy.clone(),
                },
                Arc::new(Collect::default()),
            )
            .await
            .expect("healthy session");
        let id = service
            .start(
                BackendParts {
                    reader: Box::new(Cursor::new(vec![0; 1024])),
                    writer: Box::new(std::io::sink()),
                    control: failed.clone(),
                },
                Arc::new(FailedSink),
            )
            .await
            .expect("failed session");
        let status = tokio::time::timeout(
            Duration::from_secs(2),
            service.get(&id).expect("session").wait(),
        )
        .await
        .expect("failure completes");
        assert_eq!(status.state, TerminalState::Error);
        assert_eq!(status.error_code.as_deref(), Some("output_send"));
        assert!(*failed.closed.lock().expect("failed closed"));
        assert!(!*healthy.closed.lock().expect("healthy still running"));
        service.close(&other).await.expect("close healthy session");
    }

    #[tokio::test]
    async fn closing_unblocks_io_and_bounds_backend_input() {
        let service = TerminalService::default();
        let control = Arc::new(Blocked::default());
        let id = service
            .start(
                BackendParts {
                    reader: Box::new(BlockedIo(control.clone())),
                    writer: Box::new(BlockedIo(control.clone())),
                    control: control.clone(),
                },
                Arc::new(Collect::default()),
            )
            .await
            .expect("start");
        let session = service.get(&id).expect("session");
        let writer = {
            let session = session.clone();
            tokio::spawn(async move { session.write(vec![0; INPUT_BYTES]).await })
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            while !control.writing.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("writer blocks");
        assert!(matches!(
            session.write(vec![1]).await,
            Err(TerminalError::QueueFull)
        ));
        session
            .resize(100, 30)
            .await
            .expect("resize independent of input");
        tokio::time::timeout(Duration::from_secs(2), service.close(&id))
            .await
            .expect("close unblocks all I/O")
            .expect("close");
        writer
            .await
            .expect("writer joins")
            .expect("write completes");
        assert_eq!(session.snapshot().state, TerminalState::Cancelled);
    }
}
