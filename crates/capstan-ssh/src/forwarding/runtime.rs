//! Owned TCP forwarding on an existing authenticated transport.
use crate::{
    client,
    forwarding::{TunnelErrorCode as Error, TunnelRuntimeSnapshot, TunnelRuntimeState as State},
    ConnectionLease, PasswordConnection,
};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{broadcast, mpsc, Semaphore},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use uuid::Uuid;

const SETUP_TIMEOUT: Duration = Duration::from_secs(15);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(2);
/// Concurrent clients per tunnel; the UI copy for `client_limit` names it.
pub const MAX_CLIENTS: usize = 64;
type SshChannel = russh::Channel<client::Msg>;
type RemoteKey = (String, u32);
type RemoteRegistration = (Uuid, mpsc::Sender<SshChannel>);

#[derive(Default)]
pub(crate) struct RemoteForwards(Mutex<HashMap<RemoteKey, RemoteRegistration>>);
impl RemoteForwards {
    pub(crate) fn dispatch(&self, host: &str, port: u32, channel: SshChannel) -> bool {
        self.0.lock().is_ok_and(|entries| {
            entries
                .get(&(host.to_owned(), port))
                .is_some_and(|(_, sender)| sender.try_send(channel).is_ok())
        })
    }
    fn register(
        self: &Arc<Self>,
        host: String,
        port: u16,
    ) -> Result<(Registration, mpsc::Receiver<SshChannel>), Error> {
        let mut entries = self.0.lock().map_err(|_| Error::SshProtocol)?;
        let key = (host, u32::from(port));
        if entries.contains_key(&key) {
            return Err(Error::RemoteForwardRejected);
        }
        let (sender, receiver) = mpsc::channel(16);
        let id = Uuid::new_v4();
        entries.insert(key.clone(), (id, sender));
        Ok((
            Registration {
                entries: self.clone(),
                key,
                id,
            },
            receiver,
        ))
    }
}
struct Registration {
    entries: Arc<RemoteForwards>,
    key: RemoteKey,
    id: Uuid,
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(mut entries) = self.entries.0.lock() {
            if entries.get(&self.key).is_some_and(|(id, _)| *id == self.id) {
                entries.remove(&self.key);
            }
        }
    }
}

/// Addresses are normalized and validated by core before crossing this boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForwardSpec {
    Local {
        bind_host: String,
        bind_port: u16,
        destination_host: String,
        destination_port: u16,
    },
    Remote {
        remote_bind_host: String,
        remote_bind_port: u16,
        local_destination_host: String,
        local_destination_port: u16,
    },
    Socks5 {
        bind_host: String,
        bind_port: u16,
    },
}

struct Shared {
    snapshot: Mutex<TunnelRuntimeSnapshot>,
    events: broadcast::Sender<TunnelRuntimeSnapshot>,
    changed: tokio::sync::Notify,
    cancel: CancellationToken,
    done: CancellationToken,
    stop_gate: tokio::sync::Mutex<()>,
    pending_remote: Mutex<Option<Arc<PendingRemote>>>,
}

// Keep the lease and endpoint reservation until cancellation is acknowledged or
// the transport ends. Dropping the receiver already prevents forwarding clients.
struct PendingRemote {
    host: String,
    port: u16,
    lease: ConnectionLease<PasswordConnection>,
    _registration: Registration,
    cancellation: tokio::sync::Mutex<Option<CancelRequest>>,
}
type CancelRequest = Pin<Box<dyn Future<Output = bool> + Send>>;

impl PendingRemote {
    async fn revoke(&self) -> bool {
        let connection = self.lease.connection();
        let mut cancellation = self.cancellation.lock().await;
        if cancellation.is_none() {
            // Only borrow the shared handle to obtain an owned reference. A slow
            // global request must not lock other channels or cancellation requests.
            let handle = connection.handle.lock().await.clone();
            let host = self.host.clone();
            let port = self.port;
            *cancellation = Some(Box::pin(async move {
                handle
                    .cancel_tcpip_forward(host, u32::from(port))
                    .await
                    .is_ok()
            }));
        }
        let result = tokio::select! {
            biased;
            () = connection.closed() => Some(true),
            result = tokio::time::timeout(CANCEL_TIMEOUT, async {
                match cancellation.as_mut() {
                    Some(request) => request.await,
                    None => false,
                }
            }) => result.ok(),
        };
        if let Some(confirmed) = result {
            cancellation.take();
            return confirmed;
        }
        // Keep polling the same request on retry so its late success isn't lost.
        false
    }
}
impl Shared {
    fn snapshot(&self) -> TunnelRuntimeSnapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
    fn update(&self, change: impl FnOnce(&mut TunnelRuntimeSnapshot), publish: bool) {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        change(&mut snapshot);
        if publish {
            let _ = self.events.send(snapshot.clone());
        }
        self.changed.notify_waiters();
    }
    fn state(&self, state: State, error_code: Option<Error>) {
        self.update(
            |snapshot| {
                snapshot.state = state;
                snapshot.error_code = error_code;
            },
            true,
        );
    }
    fn destination_connected(&self) {
        self.update(
            |snapshot| {
                // A successful setup confirms recovery immediately, even for
                // a long-lived stream. Capacity returns only when a client ends.
                if snapshot.state == State::Running
                    && snapshot.error_code != Some(Error::ClientLimit)
                {
                    snapshot.error_code = None;
                }
            },
            true,
        );
    }
}

pub struct TunnelRuntime {
    shared: Arc<Shared>,
}
impl Drop for TunnelRuntime {
    fn drop(&mut self) {
        self.shared.cancel.cancel();
    }
}
impl TunnelRuntime {
    pub fn spawn(
        id: Uuid,
        spec: ForwardSpec,
        lease: ConnectionLease<PasswordConnection>,
        events: broadcast::Sender<TunnelRuntimeSnapshot>,
    ) -> Self {
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(Self::stopped_snapshot(id)),
            events,
            changed: tokio::sync::Notify::new(),
            cancel: CancellationToken::new(),
            done: CancellationToken::new(),
            stop_gate: tokio::sync::Mutex::new(()),
            pending_remote: Mutex::new(None),
        });
        shared.state(State::Starting, None);
        let run = shared.clone();
        lease.connection().forward_tasks.clone().spawn(async move {
            supervise(run, spec, lease).await;
        });
        Self { shared }
    }
    pub fn stopped_snapshot(id: Uuid) -> TunnelRuntimeSnapshot {
        TunnelRuntimeSnapshot {
            id,
            state: State::Stopped,
            error_code: None,
            active_connections: 0,
            bytes_sent: 0,
            bytes_received: 0,
        }
    }
    pub fn snapshot(&self) -> TunnelRuntimeSnapshot {
        self.shared.snapshot()
    }
    pub fn is_finished(&self) -> bool {
        self.shared.done.is_cancelled()
            && self
                .shared
                .pending_remote
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_none()
    }
    pub fn cancel(&self) {
        self.shared.cancel.cancel();
    }
    pub async fn started(&self) -> TunnelRuntimeSnapshot {
        loop {
            let changed = self.shared.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let snapshot = self.snapshot();
            if snapshot.state != State::Starting {
                return snapshot;
            }
            changed.await;
        }
    }
    pub async fn wait_for_shutdown(&self) {
        self.shared.done.cancelled().await;
    }
    pub async fn stop(&self) -> TunnelRuntimeSnapshot {
        let _gate = self.shared.stop_gate.lock().await;
        let already_done = self.shared.done.is_cancelled();
        self.cancel();
        self.shared.done.cancelled().await;
        let pending = self
            .shared
            .pending_remote
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(pending) = pending {
            // The first stop reports its cleanup attempt; a later stop retries.
            if !already_done {
                return self.snapshot();
            }
            self.shared.state(State::Stopping, None);
            if !pending.revoke().await {
                self.shared
                    .state(State::Failed, Some(Error::RemoteForwardRejected));
                return self.snapshot();
            }
            self.shared
                .pending_remote
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
        }
        self.shared.state(State::Stopped, None);
        self.snapshot()
    }
}

async fn supervise(
    shared: Arc<Shared>,
    spec: ForwardSpec,
    lease: ConnectionLease<PasswordConnection>,
) {
    let tasks = TaskTracker::new();
    let clients = CancellationToken::new();
    let slots = Arc::new(Semaphore::new(MAX_CLIENTS));
    let connection = lease.connection();
    let mut registration = None;
    let mut remote_request = None;
    let run = async {
        let mut incoming_remote = None;
        let listener = match &spec {
            ForwardSpec::Local {
                bind_host,
                bind_port,
                ..
            }
            | ForwardSpec::Socks5 {
                bind_host,
                bind_port,
            } => {
                if *bind_port == 0 || bind_host.is_empty() {
                    return Err(Error::InvalidConfig);
                }
                Some(
                    TcpListener::bind((bind_host.as_str(), *bind_port))
                        .await
                        .map_err(|error| {
                            if error.kind() == std::io::ErrorKind::AddrInUse {
                                Error::BindAddressInUse
                            } else {
                                Error::InvalidConfig
                            }
                        })?,
                )
            }
            ForwardSpec::Remote {
                remote_bind_host,
                remote_bind_port,
                ..
            } => {
                if *remote_bind_port == 0 || remote_bind_host.is_empty() {
                    return Err(Error::InvalidConfig);
                }
                let (guard, receiver) = connection
                    .remote_forwards
                    .register(remote_bind_host.clone(), *remote_bind_port)?;
                registration = Some(guard);
                incoming_remote = Some(receiver);
                // Record before awaiting: timeout/cancellation can leave an accepted request in flight.
                remote_request = Some((remote_bind_host.clone(), *remote_bind_port));
                let handle = connection.handle.lock().await.clone();
                let forwarded = handle
                    .tcpip_forward(remote_bind_host.clone(), u32::from(*remote_bind_port))
                    .await;
                if matches!(forwarded, Err(russh::Error::RequestDenied)) {
                    // A definitive refusal created no listener owned by this request.
                    remote_request = None;
                }
                forwarded.map_err(|_| Error::RemoteForwardRejected)?;
                None
            }
        };
        shared.state(State::Running, None);
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        let mut last = shared.snapshot();
        loop {
            let client = tokio::select! {
                biased;
                () = shared.cancel.cancelled() => break,
                _ = tick.tick() => {
                    let current = shared.snapshot();
                    if current != last { let _ = shared.events.send(current.clone()); last = current; }
                    continue;
                }
                result = async {
                    if let Some(listener) = &listener {
                        match listener.accept().await {
                            Ok((stream, _)) => Ok(Some(Incoming::Local(stream))),
                            Err(error) if client_gone(&error) => Ok(None),
                            Err(_) => Err(Error::SshProtocol),
                        }
                    } else if let Some(receiver) = &mut incoming_remote {
                        receiver.recv().await.map(|channel| Some(Incoming::Remote(channel))).ok_or(Error::ConnectionLost)
                    } else { Err(Error::InvalidConfig) }
                } => result?,
            };
            let Some(client) = client else {
                continue;
            };
            let Ok(permit) = slots.clone().try_acquire_owned() else {
                // The tunnel keeps running; say why this client was turned away.
                shared.update(
                    |snapshot| snapshot.error_code = Some(Error::ClientLimit),
                    true,
                );
                if let Incoming::Remote(channel) = client {
                    drop(channel.into_stream());
                }
                continue;
            };
            let lease = lease.clone();
            let spec = spec.clone();
            let shared = shared.clone();
            let cancel = clients.clone();
            tasks.spawn(async move {
                shared.update(|snapshot| snapshot.active_connections += 1, true);
                let result = tokio::select! {
                    biased;
                    () = cancel.cancelled() => Ok(()),
                    result = serve_client(client, spec, lease.connection(), &shared) => result,
                };
                drop(permit);
                shared.update(
                    |snapshot| {
                        snapshot.active_connections = snapshot.active_connections.saturating_sub(1);
                        if snapshot.state == State::Running {
                            // Ending an old stream or an incomplete SOCKS request
                            // says nothing about the destination's current health.
                            match result {
                                Err(error) => snapshot.error_code = Some(error),
                                Ok(()) if snapshot.error_code == Some(Error::ClientLimit) => {
                                    snapshot.error_code = None
                                }
                                Ok(()) => {}
                            }
                        }
                    },
                    true,
                );
            });
        }
        Ok(())
    };
    let result = tokio::select! {
        biased;
        () = shared.cancel.cancelled() => Ok(()),
        () = connection.closed() => Err(Error::ConnectionLost),
        result = async {
            tokio::pin!(run);
            tokio::select! {
                biased;
                result = &mut run => result,
                () = tokio::time::sleep(SETUP_TIMEOUT) => {
                    if shared.snapshot().state == State::Starting { Err(Error::SshProtocol) } else { run.await }
                }
            }
        } => result,
    };
    if result.is_ok() {
        shared.state(State::Stopping, None);
    }
    // The dropped receiver revokes routing; retain the endpoint reservation while
    // canceling and joining clients, until the server confirms cancellation.
    clients.cancel();
    tasks.close();
    tasks.wait().await;
    let mut result = result;
    if let Some((host, port)) = remote_request {
        if let Some(registration) = registration.take() {
            let pending = Arc::new(PendingRemote {
                host,
                port,
                lease: lease.clone(),
                _registration: registration,
                cancellation: tokio::sync::Mutex::new(None),
            });
            if !pending.revoke().await {
                *shared
                    .pending_remote
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(pending);
                result = Err(Error::RemoteForwardRejected);
            }
        }
    }
    drop(registration);
    match result {
        Ok(()) => shared.state(State::Stopped, None),
        Err(error) => shared.state(State::Failed, Some(error)),
    }
    drop(lease);
    shared.done.cancel();
}

enum Incoming {
    Local(TcpStream),
    Remote(SshChannel),
}

async fn open_direct(
    connection: &PasswordConnection,
    host: String,
    port: u16,
    origin: std::net::SocketAddr,
) -> Result<SshChannel, Error> {
    if host.is_empty() || port == 0 {
        return Err(Error::InvalidConfig);
    }
    tokio::time::timeout(SETUP_TIMEOUT, async {
        let handle = connection.handle.lock().await.clone();
        handle
            .channel_open_direct_tcpip(
                host,
                u32::from(port),
                origin.ip().to_string(),
                u32::from(origin.port()),
            )
            .await
    })
    .await
    .map_err(|_| Error::DestinationUnreachable)?
    .map_err(|error| match error {
        russh::Error::ChannelOpenFailure(_) => Error::SshChannelRejected,
        _ => Error::SshProtocol,
    })
}

/// Only failures to reach the destination through the tunnel are reported.
/// A client that leaves, resets or sends an unusable SOCKS request ends just
/// that client and leaves the tunnel's error state unchanged.
async fn serve_client(
    client: Incoming,
    spec: ForwardSpec,
    connection: &PasswordConnection,
    shared: &Shared,
) -> Result<(), Error> {
    match (client, spec) {
        (
            Incoming::Local(mut stream),
            ForwardSpec::Local {
                destination_host,
                destination_port,
                ..
            },
        ) => {
            let Ok(origin) = stream.peer_addr() else {
                return Ok(());
            };
            let mut channel = open_direct(connection, destination_host, destination_port, origin)
                .await?
                .into_stream();
            shared.destination_connected();
            relay(&mut stream, &mut channel, shared).await;
            Ok(())
        }
        (Incoming::Local(mut stream), ForwardSpec::Socks5 { .. }) => {
            let Ok(origin) = stream.peer_addr() else {
                return Ok(());
            };
            let Ok(Ok((host, port))) =
                tokio::time::timeout(SETUP_TIMEOUT, socks_request(&mut stream)).await
            else {
                return Ok(());
            };
            match open_direct(connection, host, port, origin).await {
                Ok(channel) => {
                    shared.destination_connected();
                    if stream
                        .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                        .await
                        .is_ok()
                    {
                        relay(&mut stream, &mut channel.into_stream(), shared).await;
                    }
                    Ok(())
                }
                Err(error) => {
                    let _ = stream.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await;
                    Err(error)
                }
            }
        }
        (
            Incoming::Remote(channel),
            ForwardSpec::Remote {
                local_destination_host,
                local_destination_port,
                ..
            },
        ) => {
            let mut channel = channel.into_stream();
            let mut stream = tokio::time::timeout(
                SETUP_TIMEOUT,
                TcpStream::connect((local_destination_host.as_str(), local_destination_port)),
            )
            .await
            .map_err(|_| Error::DestinationUnreachable)?
            .map_err(|_| Error::DestinationUnreachable)?;
            shared.destination_connected();
            // Counters are always local -> SSH and SSH -> local, including remote forwards.
            relay(&mut stream, &mut channel, shared).await;
            Ok(())
        }
        _ => Err(Error::InvalidConfig),
    }
}

/// Accept errors for one pending client must not take the listener down.
fn client_gone(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::Interrupted
    )
}

async fn socks_request(stream: &mut TcpStream) -> Result<(String, u16), Error> {
    let mut header = [0; 2];
    stream
        .read_exact(&mut header)
        .await
        .map_err(|_| Error::SshProtocol)?;
    if header[0] != 5 || header[1] == 0 {
        return Err(Error::InvalidConfig);
    }
    let mut methods = vec![0; usize::from(header[1])];
    stream
        .read_exact(&mut methods)
        .await
        .map_err(|_| Error::SshProtocol)?;
    if !methods.contains(&0) {
        let _ = stream.write_all(&[5, 255]).await;
        return Err(Error::InvalidConfig);
    }
    stream
        .write_all(&[5, 0])
        .await
        .map_err(|_| Error::SshProtocol)?;
    let mut request = [0; 4];
    stream
        .read_exact(&mut request)
        .await
        .map_err(|_| Error::SshProtocol)?;
    let reply = if request[0] != 5 || request[2] != 0 {
        Some(1)
    } else if request[1] != 1 {
        Some(7)
    } else if !matches!(request[3], 1 | 3 | 4) {
        Some(8)
    } else {
        None
    };
    if let Some(code) = reply {
        let _ = stream.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0]).await;
        return Err(Error::InvalidConfig);
    }
    let host = match request[3] {
        1 => {
            let mut bytes = [0; 4];
            stream
                .read_exact(&mut bytes)
                .await
                .map_err(|_| Error::SshProtocol)?;
            std::net::Ipv4Addr::from(bytes).to_string()
        }
        4 => {
            let mut bytes = [0; 16];
            stream
                .read_exact(&mut bytes)
                .await
                .map_err(|_| Error::SshProtocol)?;
            std::net::Ipv6Addr::from(bytes).to_string()
        }
        _ => {
            let count = stream.read_u8().await.map_err(|_| Error::SshProtocol)?;
            let mut bytes = vec![0; usize::from(count)];
            stream
                .read_exact(&mut bytes)
                .await
                .map_err(|_| Error::SshProtocol)?;
            let value = String::from_utf8(bytes).map_err(|_| Error::InvalidConfig)?;
            if value.is_empty()
                || value.len() > 253
                || !value.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && label
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                })
            {
                return Err(Error::InvalidConfig);
            }
            value
        }
    };
    let port = stream.read_u16().await.map_err(|_| Error::SshProtocol)?;
    if port == 0 {
        return Err(Error::InvalidConfig);
    }
    Ok((host, port))
}

/// Ends when both directions finish or either side fails; an established
/// stream breaking is a normal end for that client, not a tunnel failure.
async fn relay<A: AsyncRead + AsyncWrite + Unpin, B: AsyncRead + AsyncWrite + Unpin>(
    local: &mut A,
    ssh: &mut B,
    shared: &Shared,
) {
    let (local_read, local_write) = tokio::io::split(local);
    let (ssh_read, ssh_write) = tokio::io::split(ssh);
    let _ = tokio::try_join!(
        pump(local_read, ssh_write, shared, true),
        pump(ssh_read, local_write, shared, false)
    );
}
async fn pump<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut read: R,
    mut write: W,
    shared: &Shared,
    sent: bool,
) -> std::io::Result<()> {
    let mut buffer = [0; 16 * 1024];
    loop {
        let count = read.read(&mut buffer).await?;
        if count == 0 {
            return write.shutdown().await;
        }
        let mut offset = 0;
        while offset < count {
            let written = write.write(&buffer[offset..count]).await?;
            if written == 0 {
                return Err(std::io::ErrorKind::WriteZero.into());
            }
            shared.update(
                |snapshot| {
                    let counter = if sent {
                        &mut snapshot.bytes_sent
                    } else {
                        &mut snapshot.bytes_received
                    };
                    *counter = counter.saturating_add(written as u64);
                },
                false,
            );
            offset += written;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::client_gone;
    use std::io::{Error, ErrorKind};

    #[test]
    fn only_per_client_accept_errors_keep_the_listener() {
        for kind in [
            ErrorKind::ConnectionReset,
            ErrorKind::ConnectionAborted,
            ErrorKind::Interrupted,
        ] {
            assert!(client_gone(&Error::from(kind)));
        }
        for kind in [ErrorKind::PermissionDenied, ErrorKind::Other] {
            assert!(!client_gone(&Error::from(kind)));
        }
    }
}
