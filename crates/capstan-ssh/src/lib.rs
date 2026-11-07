//! Async SSH trust, authentication, connection leases and channels.
#![deny(unsafe_code)]
#![allow(clippy::pedantic)]
pub mod connection_state;
pub mod exec;
pub mod forwarding;
#[path = "forwarding/runtime.rs"]
pub mod forwarding_runtime;
mod pool;
pub mod sftp;
mod sftp_pool;
pub use pool::{ConnectionLease, ConnectionPool, PoolKey};
pub use sftp_pool::SftpTransferLease;

use async_trait::async_trait;
use russh::{
    client,
    keys::{
        agent::{client::AgentClient, AgentIdentity},
        HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey,
    },
    Disconnect,
};
use secrecy::{ExposeSecret, SecretString};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, ReadBuf},
    sync::{mpsc, oneshot, watch, Semaphore},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use uuid::Uuid;

static NEXT_CONNECTION_GENERATION: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SshError {
    #[error("connection timed out")]
    Timeout,
    #[error("connection failed")]
    Connection,
    #[error("authentication failed")]
    Authentication,
    #[error("host key is not trusted")]
    TrustRequired,
    #[error("host key changed")]
    HostKeyChanged,
    #[error("host key confirmation was rejected")]
    TrustRejected,
    #[error("attempt was canceled")]
    Canceled,
    #[error("terminal channel request was rejected")]
    ChannelRejected,
    #[error("SSH protocol failed")]
    Protocol,
    #[error("invalid connection input")]
    InvalidInput,
    #[error("private key is invalid or its passphrase is incorrect")]
    InvalidPrivateKey,
    #[error("SSH agent is unavailable")]
    AgentUnavailable,
    #[error("selected SSH agent identity is unavailable")]
    AgentIdentityUnavailable,
}
impl SshError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Timeout => "ssh_timeout",
            Self::Connection => "ssh_connection",
            Self::Authentication => "ssh_authentication",
            Self::TrustRequired => "host_key_unknown",
            Self::HostKeyChanged => "host_key_changed",
            Self::TrustRejected => "host_key_rejected",
            Self::Canceled => "canceled",
            Self::ChannelRejected => "ssh_channel_rejected",
            Self::Protocol => "ssh_protocol",
            Self::InvalidInput => "invalid_input",
            Self::InvalidPrivateKey => "ssh_private_key_invalid",
            Self::AgentUnavailable => "ssh_agent_unavailable",
            Self::AgentIdentityUnavailable => "ssh_agent_identity_unavailable",
        }
    }
}
pub type Result<T> = std::result::Result<T, SshError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProvider {
    OpenSsh,
    Pageant,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentIdentityView {
    pub algorithm: String,
    pub fingerprint: String,
}

type DynamicAgentClient =
    AgentClient<Box<dyn russh::keys::agent::client::AgentStream + Send + Unpin>>;
type AgentIo = Box<dyn russh::keys::agent::client::AgentStream + Send + Unpin>;
type AgentConnectorFuture = Pin<Box<dyn Future<Output = Result<AgentIo>> + Send>>;
type AgentConnector = Arc<dyn Fn() -> AgentConnectorFuture + Send + Sync>;
const MAX_AGENT_CHANNELS: usize = 8;

fn agent_identity_view(identity: &AgentIdentity) -> Option<AgentIdentityView> {
    let AgentIdentity::PublicKey { key, .. } = identity else {
        return None;
    };
    Some(AgentIdentityView {
        algorithm: key.algorithm().to_string(),
        fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
    })
}

fn select_agent_identity(identities: &[AgentIdentity], fingerprint: &str) -> Result<PublicKey> {
    identities
        .iter()
        .find_map(|identity| match identity {
            AgentIdentity::PublicKey { key, .. }
                if key.fingerprint(HashAlg::Sha256).to_string() == fingerprint =>
            {
                Some(key.clone())
            }
            AgentIdentity::PublicKey { .. } | AgentIdentity::Certificate { .. } => None,
        })
        .ok_or(SshError::AgentIdentityUnavailable)
}

async fn open_agent_client(provider: AgentProvider) -> Result<DynamicAgentClient> {
    #[cfg(windows)]
    {
        match provider {
            AgentProvider::OpenSsh => {
                AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent")
                    .await
                    .map(AgentClient::dynamic)
                    .map_err(|_| SshError::AgentUnavailable)
            }
            AgentProvider::Pageant => AgentClient::connect_pageant()
                .await
                .map(AgentClient::dynamic)
                .map_err(|_| SshError::AgentUnavailable),
        }
    }
    #[cfg(unix)]
    {
        match provider {
            AgentProvider::OpenSsh => AgentClient::connect_env()
                .await
                .map(AgentClient::dynamic)
                .map_err(|_| SshError::AgentUnavailable),
            AgentProvider::Pageant => Err(SshError::AgentUnavailable),
        }
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = provider;
        Err(SshError::AgentUnavailable)
    }
}

fn system_agent_connector(provider: AgentProvider) -> AgentConnector {
    Arc::new(move || {
        Box::pin(async move {
            open_agent_client(provider)
                .await
                .map(AgentClient::into_inner)
        })
    })
}

async fn relay_agent_channel(
    channel: russh::Channel<client::Msg>,
    connector: Option<AgentConnector>,
) {
    let Some(connector) = connector else {
        let _ = channel.close().await;
        return;
    };
    let agent = match tokio::time::timeout(Duration::from_secs(5), connector()).await {
        Ok(Ok(agent)) => agent,
        Ok(Err(_)) | Err(_) => {
            let _ = channel.close().await;
            return;
        }
    };
    let mut channel = channel.into_stream();
    let mut agent = agent;
    let _ = tokio::io::copy_bidirectional(&mut channel, &mut agent).await;
}

pub async fn list_agent_identities(provider: AgentProvider) -> Result<Vec<AgentIdentityView>> {
    let mut client = tokio::time::timeout(Duration::from_secs(5), open_agent_client(provider))
        .await
        .map_err(|_| SshError::Timeout)??;
    let identities = tokio::time::timeout(Duration::from_secs(5), client.request_identities())
        .await
        .map_err(|_| SshError::Timeout)?
        .map_err(|_| SshError::AgentUnavailable)?;
    let mut views = identities
        .iter()
        .filter_map(agent_identity_view)
        .collect::<Vec<_>>();
    views.sort_by(|left, right| left.fingerprint.cmp(&right.fingerprint));
    views.dedup_by(|left, right| left.fingerprint == right.fingerprint);
    Ok(views)
}

enum AuthenticationMethod {
    Password(SecretString),
    PrivateKey(Arc<PrivateKey>),
    Agent {
        provider: AgentProvider,
        fingerprint: String,
    },
}

pub fn validate_private_key(content: &str, passphrase: Option<&str>) -> Result<()> {
    russh::keys::decode_secret_key(content, passphrase)
        .map(|_| ())
        .map_err(|_| SshError::InvalidPrivateKey)
}

pub fn private_key_requires_passphrase(content: &str) -> bool {
    matches!(
        russh::keys::decode_secret_key(content, None),
        Err(russh::keys::Error::KeyIsEncrypted)
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPublicKey {
    pub algorithm: String,
    pub public_key: String,
    pub fingerprint: String,
}
impl HostPublicKey {
    pub fn from_openssh(content: &str) -> Result<Self> {
        let mut key = PublicKey::from_openssh(content).map_err(|_| SshError::InvalidInput)?;
        // OpenSSH file comments are not transmitted as part of the host key.
        key.set_comment("");
        Self::from_public_key(&key)
    }
    pub fn from_public_key(key: &PublicKey) -> Result<Self> {
        Ok(Self {
            algorithm: key.algorithm().to_string(),
            public_key: key.to_openssh().map_err(|_| SshError::Protocol)?,
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct AttemptBinding {
    pub attempt_id: Uuid,
    pub host_id: Uuid,
    pub host_revision: u64,
    pub vault_epoch: u64,
    pub target: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustChallenge {
    pub challenge_id: Uuid,
    pub attempt_id: Uuid,
    pub host_id: Uuid,
    pub host_revision: u64,
    pub target: String,
    pub port: u16,
    pub algorithm: String,
    pub public_key: String,
    pub fingerprint: String,
    pub known_algorithm: Option<String>,
    pub known_fingerprint: Option<String>,
    pub key_changed: bool,
}

#[async_trait]
pub trait TrustRepository: Send + Sync + 'static {
    async fn lookup(&self, target: &str, port: u16) -> Result<Option<HostPublicKey>>;
    async fn is_current(&self, attempt: &AttemptBinding) -> bool;
    async fn remember(&self, attempt: &AttemptBinding, key: &HostPublicKey) -> Result<()>;
    async fn authenticating(&self, _attempt: &AttemptBinding) {}
}

struct PendingTrust {
    attempt_id: Uuid,
    response: oneshot::Sender<bool>,
}
pub struct TrustBroker {
    events: mpsc::Sender<TrustChallenge>,
    pending: Mutex<HashMap<Uuid, PendingTrust>>,
}
impl TrustBroker {
    pub fn new() -> (Arc<Self>, mpsc::Receiver<TrustChallenge>) {
        let (events, receiver) = mpsc::channel(32);
        (
            Arc::new(Self {
                events,
                pending: Mutex::new(HashMap::new()),
            }),
            receiver,
        )
    }
    pub fn respond(&self, challenge_id: Uuid, accepted: bool) -> Result<()> {
        let pending = self
            .pending
            .lock()
            .map_err(|_| SshError::Protocol)?
            .remove(&challenge_id)
            .ok_or(SshError::Canceled)?;
        let _ = pending.attempt_id;
        pending
            .response
            .send(accepted)
            .map_err(|_| SshError::Canceled)
    }
    pub fn cancel_attempt(&self, attempt_id: Uuid) {
        if let Ok(mut pending) = self.pending.lock() {
            let ids: Vec<_> = pending
                .iter()
                .filter_map(|(id, value)| (value.attempt_id == attempt_id).then_some(*id))
                .collect();
            for id in ids {
                if let Some(value) = pending.remove(&id) {
                    let _ = value.response.send(false);
                }
            }
        }
    }
    async fn verify(
        &self,
        repository: &dyn TrustRepository,
        attempt: &AttemptBinding,
        key: HostPublicKey,
        cancel: &CancellationToken,
    ) -> Result<()> {
        if !repository.is_current(attempt).await {
            return Err(SshError::Canceled);
        }
        if let Some(known) = repository.lookup(&attempt.target, attempt.port).await? {
            if known == key {
                return Ok(());
            }
            self.emit_changed_key(attempt, &known, &key).await;
            return Err(SshError::HostKeyChanged);
        }
        let challenge_id = Uuid::new_v4();
        let (response, answer) = oneshot::channel();
        self.pending.lock().map_err(|_| SshError::Protocol)?.insert(
            challenge_id,
            PendingTrust {
                attempt_id: attempt.attempt_id,
                response,
            },
        );
        let challenge = TrustChallenge {
            challenge_id,
            attempt_id: attempt.attempt_id,
            host_id: attempt.host_id,
            host_revision: attempt.host_revision,
            target: attempt.target.clone(),
            port: attempt.port,
            algorithm: key.algorithm.clone(),
            public_key: key.public_key.clone(),
            fingerprint: key.fingerprint.clone(),
            known_algorithm: None,
            known_fingerprint: None,
            key_changed: false,
        };
        if self.events.send(challenge).await.is_err() {
            self.pending
                .lock()
                .map_err(|_| SshError::Protocol)?
                .remove(&challenge_id);
            return Err(SshError::Canceled);
        }
        let answer = tokio::select! {
            biased;
            () = cancel.cancelled() => Err(SshError::Canceled),
            result = tokio::time::timeout(Duration::from_secs(120), answer) => match result {
                Ok(Ok(answer)) => Ok(answer),
                Ok(Err(_)) => Err(SshError::Canceled),
                Err(_) => Err(SshError::Timeout),
            },
        };
        self.pending
            .lock()
            .map_err(|_| SshError::Protocol)?
            .remove(&challenge_id);
        if !answer? {
            return Err(SshError::TrustRejected);
        }
        if !repository.is_current(attempt).await {
            return Err(SshError::Canceled);
        }
        if let Some(known) = repository.lookup(&attempt.target, attempt.port).await? {
            if known == key {
                return Ok(());
            }
            self.emit_changed_key(attempt, &known, &key).await;
            return Err(SshError::HostKeyChanged);
        }
        repository.remember(attempt, &key).await
    }

    async fn emit_changed_key(
        &self,
        attempt: &AttemptBinding,
        known: &HostPublicKey,
        presented: &HostPublicKey,
    ) {
        let _ = self
            .events
            .send(TrustChallenge {
                challenge_id: Uuid::new_v4(),
                attempt_id: attempt.attempt_id,
                host_id: attempt.host_id,
                host_revision: attempt.host_revision,
                target: attempt.target.clone(),
                port: attempt.port,
                algorithm: presented.algorithm.clone(),
                public_key: presented.public_key.clone(),
                fingerprint: presented.fingerprint.clone(),
                known_algorithm: Some(known.algorithm.clone()),
                known_fingerprint: Some(known.fingerprint.clone()),
                key_changed: true,
            })
            .await;
    }
}

struct ClientHandler {
    remote_forwards: Arc<forwarding_runtime::RemoteForwards>,
    repository: Arc<dyn TrustRepository>,
    broker: Arc<TrustBroker>,
    attempt: AttemptBinding,
    cancel: CancellationToken,
    transport_closed: CancellationToken,
    failure: Arc<Mutex<Option<SshError>>>,
    checking_host_key: watch::Sender<bool>,
    agent_forwarding: Option<AgentConnector>,
    agent_slots: Arc<Semaphore>,
    agent_tasks: TaskTracker,
    agent_cancel: CancellationToken,
    algorithms: Arc<Mutex<Option<NegotiatedAlgorithms>>>,
}
impl Drop for ClientHandler {
    fn drop(&mut self) {
        // Keep liveness separate from startup cancellation so a rejected host
        // key or authentication retains its precise error classification.
        self.transport_closed.cancel();
        self.agent_cancel.cancel();
    }
}
impl ClientHandler {
    fn remember_failure(&self, error: SshError) {
        if let Ok(mut failure) = self.failure.lock() {
            *failure = Some(error);
        }
    }
}
impl client::Handler for ClientHandler {
    type Error = russh::Error;
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        session: &mut client::Session,
    ) -> std::result::Result<(), Self::Error> {
        let id = channel.id();
        if !self
            .remote_forwards
            .dispatch(connected_address, connected_port, channel)
        {
            session.close(id)?;
        }
        Ok(())
    }
    async fn kex_done(
        &mut self,
        _shared_secret: Option<&[u8]>,
        names: &russh::Names,
        _session: &mut client::Session,
    ) -> std::result::Result<(), Self::Error> {
        if let Ok(mut algorithms) = self.algorithms.lock() {
            *algorithms = Some(NegotiatedAlgorithms {
                kex: names.kex.as_ref().to_owned(),
                cipher: names.cipher.as_ref().to_owned(),
                client_mac: names.client_mac.as_ref().to_owned(),
                server_mac: names.server_mac.as_ref().to_owned(),
            });
        }
        Ok(())
    }
    async fn check_server_key(
        &mut self,
        key: &PublicKey,
    ) -> std::result::Result<bool, Self::Error> {
        let key = HostPublicKey::from_public_key(key).map_err(|_| russh::Error::Inconsistent)?;
        self.checking_host_key.send_replace(true);
        let verified = self
            .broker
            .verify(self.repository.as_ref(), &self.attempt, key, &self.cancel)
            .await;
        self.checking_host_key.send_replace(false);
        match verified {
            Ok(()) => Ok(true),
            Err(error) => {
                self.remember_failure(error);
                Ok(false)
            }
        }
    }

    async fn server_channel_open_agent_forward(
        &mut self,
        channel: russh::Channel<client::Msg>,
        session: &mut client::Session,
    ) -> std::result::Result<(), Self::Error> {
        let permit = self.agent_slots.clone().try_acquire_owned();
        if self.agent_forwarding.is_none() || self.agent_cancel.is_cancelled() || permit.is_err() {
            return session.close(channel.id());
        }
        let permit = permit.map_err(|_| russh::Error::Inconsistent)?;
        let connector = self.agent_forwarding.clone();
        let cancel = self.agent_cancel.clone();
        self.agent_tasks.spawn(async move {
            let _permit = permit;
            tokio::select! {
                biased;
                () = cancel.cancelled() => {}
                () = relay_agent_channel(channel, connector) => {}
            }
        });
        Ok(())
    }
}

/// Algorithm identifiers only; never retains key-exchange secrets.
#[derive(Clone, Debug, serde::Serialize)]
pub struct NegotiatedAlgorithms {
    pub kex: String,
    pub cipher: String,
    pub client_mac: String,
    pub server_mac: String,
}

pub struct PasswordConnection {
    remote_forwards: Arc<forwarding_runtime::RemoteForwards>,
    forward_tasks: TaskTracker,
    handle: Arc<tokio::sync::Mutex<Arc<client::Handle<ClientHandler>>>>,
    cancel: CancellationToken,
    transport_closed: CancellationToken,
    connection_id: Uuid,
    connection_generation: u64,
    shell_tasks: TaskTracker,
    exec_gate: Arc<tokio::sync::Mutex<()>>,
    status_channel: Arc<exec::StatusChannelCoordinator>,
    agent_forwarding: bool,
    agent_forwarding_ready: tokio::sync::Mutex<bool>,
    agent_tasks: TaskTracker,
    algorithms: Arc<Mutex<Option<NegotiatedAlgorithms>>>,
    sftp: sftp_pool::SftpPool,
}
impl Drop for PasswordConnection {
    fn drop(&mut self) {
        self.cancel.cancel();
        // Idle pool expiry must close the transport even when an exited tab
        // still owns its channel writer to retain scrollback.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let handle = self.handle.clone();
            runtime.spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(2), async {
                    let handle = handle.lock().await;
                    let _ = handle
                        .disconnect(Disconnect::ByApplication, "session closed", "en")
                        .await;
                })
                .await;
            });
        }
    }
}

async fn channel_request_reply(
    channel: &mut russh::Channel<client::Msg>,
    pending: &mut Vec<Vec<u8>>,
) -> Result<()> {
    loop {
        match channel.wait().await {
            Some(russh::ChannelMsg::Success) => return Ok(()),
            Some(russh::ChannelMsg::WindowAdjusted { .. }) => {}
            Some(
                russh::ChannelMsg::Data { data } | russh::ChannelMsg::ExtendedData { data, .. },
            ) => {
                if pending.iter().map(Vec::len).sum::<usize>() + data.len() > 64 * 1024 {
                    return Err(SshError::Protocol);
                }
                pending.push(data.to_vec());
            }
            _ => return Err(SshError::ChannelRejected),
        }
    }
}

struct ShellLifetime {
    exit: tokio::sync::watch::Sender<ChannelExit>,
    lease: Mutex<Option<ConnectionLease<PasswordConnection>>>,
    cancel: CancellationToken,
}
impl ShellLifetime {
    fn finish(&self, result: ChannelExit) {
        // The lease and terminal transition share a lock with late attachment.
        if let Ok(mut lease) = self.lease.lock() {
            self.exit.send_if_modified(|state| {
                if matches!(state, ChannelExit::Ended(_) | ChannelExit::Failed) {
                    false
                } else {
                    *state = result;
                    true
                }
            });
            lease.take();
        }
    }
}
impl PasswordConnection {
    pub fn connection_id(&self) -> Uuid {
        self.connection_id
    }

    /// A global request on this transport, without opening a shell or replaying input.
    pub async fn probe(&self) -> Result<()> {
        tokio::select! {
            biased;
            () = self.closed() => Err(SshError::Connection),
            result = tokio::time::timeout(Duration::from_secs(5), async {
                self.handle.lock().await.send_ping().await
            }) => result.map_err(|_| SshError::Timeout)?.map_err(|_| SshError::Connection),
        }
    }

    pub async fn open_shell(&self, columns: u16, rows: u16) -> Result<SshShell> {
        if columns == 0 || rows == 0 || columns > i16::MAX as u16 || rows > i16::MAX as u16 {
            return Err(SshError::InvalidInput);
        }
        let mut channel = tokio::select! {
            biased;
            () = self.cancel.cancelled() => return Err(SshError::Canceled),
            () = self.transport_closed.cancelled() => return Err(SshError::Connection),
            result = tokio::time::timeout(Duration::from_secs(15), self.open_user_session()) =>
                result.map_err(|_| SshError::Timeout)?.map_err(|_| SshError::Connection)?,
        };
        let mut pending = Vec::new();
        let setup = async {
            channel
                .request_pty(
                    true,
                    "xterm-256color",
                    u32::from(columns),
                    u32::from(rows),
                    0,
                    0,
                    &[],
                )
                .await
                .map_err(|_| SshError::ChannelRejected)?;
            channel_request_reply(&mut channel, &mut pending).await?;
            if self.agent_forwarding {
                // OpenSSH creates one Agent socket for the entire transport and rejects
                // duplicate forwarding requests, including requests from another shell.
                let mut ready = self.agent_forwarding_ready.lock().await;
                if !*ready {
                    channel
                        .agent_forward(true)
                        .await
                        .map_err(|_| SshError::ChannelRejected)?;
                    channel_request_reply(&mut channel, &mut pending).await?;
                    *ready = true;
                }
            }
            channel
                .request_shell(true)
                .await
                .map_err(|_| SshError::ChannelRejected)?;
            channel_request_reply(&mut channel, &mut pending).await
        };
        let setup_result = tokio::select! {
            biased;
            () = self.cancel.cancelled() => Err(SshError::Canceled),
            () = self.transport_closed.cancelled() => Err(SshError::Connection),
            result = tokio::time::timeout(Duration::from_secs(15), setup) => result.unwrap_or(Err(SshError::Timeout)),
        };
        if let Err(error) = setup_result {
            let _ = tokio::time::timeout(Duration::from_secs(2), channel.close()).await;
            return Err(error);
        }
        let (mut read_half, writer) = channel.split();
        let (exit_tx, exit) = tokio::sync::watch::channel(ChannelExit::Open);
        let (data_tx, data_rx) = mpsc::channel(32);
        let lifetime = Arc::new(ShellLifetime {
            exit: exit_tx,
            lease: Mutex::new(None),
            cancel: CancellationToken::new(),
        });
        let reader_lifetime = lifetime.clone();
        let connection_cancel = self.cancel.clone();
        let transport_closed = self.transport_closed.clone();
        let reader_task = self.shell_tasks.spawn(async move {
            let mut result_code = None;
            let mut data_tx = Some(data_tx);
            let run = async {
                for data in pending {
                    if let Some(sender) = &data_tx {
                        if sender.send(data).await.is_err() {
                            return ChannelExit::Failed;
                        }
                    }
                }
                while let Some(message) = read_half.wait().await {
                    match message {
                        russh::ChannelMsg::Data { data }
                        | russh::ChannelMsg::ExtendedData { data, .. } => {
                            if let Some(sender) = &data_tx {
                                if sender.send(data.to_vec()).await.is_err() {
                                    return ChannelExit::Failed;
                                }
                            } else {
                                return ChannelExit::Failed;
                            }
                        }
                        russh::ChannelMsg::ExitStatus { exit_status } => {
                            result_code = Some(exit_status);
                            reader_lifetime
                                .exit
                                .send_replace(ChannelExit::Status(exit_status));
                        }
                        russh::ChannelMsg::ExitSignal { .. } => {
                            result_code = None;
                            reader_lifetime
                                .exit
                                .send_replace(ChannelExit::StatusUnknown);
                        }
                        russh::ChannelMsg::Eof => {
                            // EOF closes data only. Exit status and channel close can follow it.
                            data_tx.take();
                        }
                        russh::ChannelMsg::Close => return ChannelExit::Ended(result_code),
                        _ => {}
                    }
                }
                ChannelExit::Failed
            };
            let result = tokio::select! {
                biased;
                () = reader_lifetime.cancel.cancelled() => ChannelExit::Ended(None),
                () = connection_cancel.cancelled() => ChannelExit::Failed,
                () = transport_closed.cancelled() => ChannelExit::Failed,
                result = run => result,
            };
            drop(data_tx);
            reader_lifetime.finish(result);
        });
        let control = Arc::new(SshChannelControl {
            writer: Arc::new(writer),
            lifetime,
            connection_cancel: self.cancel.clone(),
            reader_task: tokio::sync::Mutex::new(Some(reader_task)),
        });
        Ok(SshShell {
            reader: SshChannelReader {
                receiver: data_rx,
                current: Vec::new(),
                offset: 0,
            },
            control,
            exit,
            connection_id: self.connection_id,
            algorithms: self.algorithms.clone(),
        })
    }
    pub async fn disconnect(&self) {
        self.cancel.cancel();
        self.forward_tasks.close();
        self.forward_tasks.wait().await;
        self.shell_tasks.close();
        self.shell_tasks.wait().await;
        self.sftp.close().await;
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            let handle = self.handle.lock().await;
            let _ = handle
                .disconnect(Disconnect::ByApplication, "session closed", "en")
                .await;
        })
        .await;
        self.agent_tasks.close();
        self.agent_tasks.wait().await;
    }

    pub fn connection_generation(&self) -> u64 {
        self.connection_generation
    }
    pub fn is_closed(&self) -> bool {
        self.cancel.is_cancelled() || self.transport_closed.is_cancelled()
    }
    pub async fn closed(&self) {
        tokio::select! {
            () = self.cancel.cancelled() => {}
            () = self.transport_closed.cancelled() => {}
        }
    }
    /// Observes loss of the transport without retaining the connection.
    pub fn transport_close_token(&self) -> CancellationToken {
        self.transport_closed.child_token()
    }
    /// Drain every owned subsystem, including canceled data assignments.
    pub async fn wait_sftp_pending(&self) {
        self.sftp.wait_pending().await;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelExit {
    Open,
    Status(u32),
    StatusUnknown,
    Ended(Option<u32>),
    Failed,
}

pub struct SshShell {
    pub reader: SshChannelReader,
    pub control: Arc<SshChannelControl>,
    pub exit: tokio::sync::watch::Receiver<ChannelExit>,
    connection_id: Uuid,
    algorithms: Arc<Mutex<Option<NegotiatedAlgorithms>>>,
}
impl SshShell {
    pub fn negotiated_algorithms(&self) -> Option<NegotiatedAlgorithms> {
        self.algorithms.lock().ok().and_then(|value| value.clone())
    }
    pub fn connection_id(&self) -> Uuid {
        self.connection_id
    }
    pub fn into_terminal_parts(self) -> capstan_terminal::AsyncBackendParts {
        let writer = self.control.writer.make_writer();
        capstan_terminal::AsyncBackendParts {
            reader: Box::pin(self.reader),
            writer: Box::pin(writer),
            control: self.control,
        }
    }
}
pub struct SshChannelReader {
    receiver: mpsc::Receiver<Vec<u8>>,
    current: Vec<u8>,
    offset: usize,
}
impl AsyncRead for SshChannelReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        loop {
            if self.offset < self.current.len() {
                let remaining = &self.current[self.offset..];
                let count = remaining.len().min(buffer.remaining());
                buffer.put_slice(&remaining[..count]);
                self.offset += count;
                if self.offset == self.current.len() {
                    self.current.clear();
                    self.offset = 0;
                }
                return std::task::Poll::Ready(Ok(()));
            }
            match self.receiver.poll_recv(context) {
                std::task::Poll::Ready(Some(data)) => {
                    self.current = data;
                }
                std::task::Poll::Ready(None) => return std::task::Poll::Ready(Ok(())),
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
    }
}
pub struct SshChannelControl {
    writer: Arc<russh::ChannelWriteHalf<russh::client::Msg>>,
    lifetime: Arc<ShellLifetime>,
    connection_cancel: CancellationToken,
    reader_task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl Drop for SshChannelControl {
    fn drop(&mut self) {
        self.lifetime.cancel.cancel();
    }
}
impl SshChannelControl {
    pub async fn write(&self, bytes: Vec<u8>) -> Result<()> {
        self.writer
            .data_bytes(bytes)
            .await
            .map_err(|_| SshError::Protocol)
    }
    pub async fn resize(&self, columns: u16, rows: u16) -> Result<()> {
        if columns == 0 || rows == 0 || columns > i16::MAX as u16 || rows > i16::MAX as u16 {
            return Err(SshError::InvalidInput);
        }
        self.writer
            .window_change(u32::from(columns), u32::from(rows), 0, 0)
            .await
            .map_err(|_| SshError::Protocol)
    }
    pub async fn close(&self) -> Result<()> {
        // Stop the owned reader even if the connection is gone or the send queue is full.
        // Serialize joiners so every close waits until local resources are reclaimed.
        let mut reader_task = self.reader_task.lock().await;
        let state = *self.lifetime.exit.borrow();
        self.lifetime.cancel.cancel();
        if !matches!(state, ChannelExit::Ended(_) | ChannelExit::Failed) {
            let _ = tokio::time::timeout(Duration::from_secs(2), async {
                let _ = self.writer.eof().await;
                let _ = self.writer.close().await;
            })
            .await;
        }
        if let Some(task) = reader_task.take() {
            task.await.map_err(|_| SshError::Protocol)?;
        }
        Ok(())
    }
    pub fn connection_cancel(&self) -> &CancellationToken {
        &self.connection_cancel
    }
}

impl ConnectionLease<PasswordConnection> {
    pub async fn open_shell(&self, columns: u16, rows: u16) -> Result<SshShell> {
        let shell = self.connection().open_shell(columns, rows).await?;
        {
            let mut lease = shell
                .control
                .lifetime
                .lease
                .lock()
                .map_err(|_| SshError::Protocol)?;
            if !matches!(
                *shell.control.lifetime.exit.borrow(),
                ChannelExit::Ended(_) | ChannelExit::Failed
            ) {
                *lease = Some(self.clone());
            }
        }
        Ok(shell)
    }
}

impl capstan_terminal::AsyncBackendControl for SshChannelControl {
    fn resize<'a>(
        &'a self,
        columns: u16,
        rows: u16,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = capstan_terminal::Result<()>> + Send + 'a>,
    > {
        Box::pin(async move {
            SshChannelControl::resize(self, columns, rows)
                .await
                .map_err(|error| capstan_terminal::TerminalError::Backend(error.code().to_owned()))
        })
    }
    fn poll<'a>(
        &'a self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = capstan_terminal::Result<capstan_terminal::BackendPoll>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            match *self.lifetime.exit.borrow() {
                ChannelExit::Open | ChannelExit::Status(_) | ChannelExit::StatusUnknown => {
                    Ok(capstan_terminal::BackendPoll::Running)
                }
                ChannelExit::Ended(code) => Ok(capstan_terminal::BackendPoll::Exited(code)),
                ChannelExit::Failed => Err(capstan_terminal::TerminalError::Backend(
                    "ssh_disconnected".to_owned(),
                )),
            }
        })
    }
    fn close<'a>(
        &'a self,
        _terminate: bool,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = capstan_terminal::Result<Option<u32>>> + Send + 'a>,
    > {
        Box::pin(async move {
            SshChannelControl::close(self).await.map_err(|error| {
                capstan_terminal::TerminalError::Backend(error.code().to_owned())
            })?;
            Ok(match *self.lifetime.exit.borrow() {
                ChannelExit::Ended(code) => code,
                _ => None,
            })
        })
    }
}

#[allow(clippy::too_many_arguments)] // Keep the attempt, secret and trust services explicit at the library boundary.
pub async fn connect_password(
    address: String,
    port: u16,
    username: String,
    password: SecretString,
    attempt: AttemptBinding,
    repository: Arc<dyn TrustRepository>,
    broker: Arc<TrustBroker>,
    cancel: CancellationToken,
) -> Result<PasswordConnection> {
    connect_with_authentication(
        address,
        port,
        username,
        AuthenticationMethod::Password(password),
        attempt,
        repository,
        broker,
        cancel,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn connect_private_key(
    address: String,
    port: u16,
    username: String,
    content: &str,
    passphrase: Option<&str>,
    attempt: AttemptBinding,
    repository: Arc<dyn TrustRepository>,
    broker: Arc<TrustBroker>,
    cancel: CancellationToken,
) -> Result<PasswordConnection> {
    let key = russh::keys::decode_secret_key(content, passphrase)
        .map_err(|_| SshError::InvalidPrivateKey)?;
    connect_with_authentication(
        address,
        port,
        username,
        AuthenticationMethod::PrivateKey(Arc::new(key)),
        attempt,
        repository,
        broker,
        cancel,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn connect_agent(
    address: String,
    port: u16,
    username: String,
    provider: AgentProvider,
    identity_fingerprint: String,
    agent_forwarding: bool,
    attempt: AttemptBinding,
    repository: Arc<dyn TrustRepository>,
    broker: Arc<TrustBroker>,
    cancel: CancellationToken,
) -> Result<PasswordConnection> {
    if !identity_fingerprint.starts_with("SHA256:") || identity_fingerprint.len() > 256 {
        return Err(SshError::InvalidInput);
    }
    connect_with_authentication(
        address,
        port,
        username,
        AuthenticationMethod::Agent {
            provider,
            fingerprint: identity_fingerprint,
        },
        attempt,
        repository,
        broker,
        cancel,
        agent_forwarding.then(|| system_agent_connector(provider)),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn connect_with_authentication(
    address: String,
    port: u16,
    username: String,
    authentication: AuthenticationMethod,
    attempt: AttemptBinding,
    repository: Arc<dyn TrustRepository>,
    broker: Arc<TrustBroker>,
    cancel: CancellationToken,
    agent_forwarding: Option<AgentConnector>,
) -> Result<PasswordConnection> {
    if port == 0
        || username.is_empty()
        || username.len() > 256
        || address.is_empty()
        || address.len() > 253
    {
        return Err(SshError::InvalidInput);
    }
    // Cancel the handler's pending trust request on every failed or dropped
    // startup, including a network timeout. A late click must not save trust
    // for a connection attempt which no longer has a caller.
    let cancel = cancel.child_token();
    let connection_guard = cancel.clone().drop_guard();
    let target = tokio::select! {
        biased;
        () = cancel.cancelled() => return Err(SshError::Canceled),
        result = tokio::time::timeout(Duration::from_secs(15), tokio::net::lookup_host((address.as_str(), port))) => match result {
            Ok(Ok(mut addresses)) => addresses.next().ok_or(SshError::Connection)?,
            Ok(Err(_)) => return Err(SshError::Connection),
            Err(_) => return Err(SshError::Timeout),
        },
    };
    let failure = Arc::new(Mutex::new(None));
    let agent_tasks = TaskTracker::new();
    let (checking_host_key, host_key_checks) = watch::channel(false);
    let algorithms = Arc::new(Mutex::new(None));
    let transport_closed = CancellationToken::new();
    let remote_forwards = Arc::new(forwarding_runtime::RemoteForwards::default());
    let handler = ClientHandler {
        remote_forwards: remote_forwards.clone(),
        repository: repository.clone(),
        broker,
        attempt: attempt.clone(),
        cancel: cancel.clone(),
        transport_closed: transport_closed.clone(),
        failure: failure.clone(),
        checking_host_key,
        agent_forwarding: agent_forwarding.clone(),
        agent_slots: Arc::new(Semaphore::new(MAX_AGENT_CHANNELS)),
        agent_tasks: agent_tasks.clone(),
        agent_cancel: cancel.child_token(),
        algorithms: algorithms.clone(),
    };
    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(15)),
        keepalive_max: 2,
        channel_buffer_size: 100,
        // Keystrokes and channel control messages must not wait for Nagle/delayed ACK.
        nodelay: true,
        ..client::Config::default()
    });
    let mut handle = tokio::select! {
        biased;
        () = cancel.cancelled() => return Err(SshError::Canceled),
        result = handshake_with_timeout(async {
            client::connect(config, target, handler).await.map_err(|_| {
                failure.lock().ok().and_then(|mut failure| failure.take()).unwrap_or(SshError::Connection)
            })
        }, host_key_checks) => result?,
    };
    if !repository.is_current(&attempt).await {
        let _ = handle
            .disconnect(Disconnect::ByApplication, "canceled", "en")
            .await;
        return Err(SshError::Canceled);
    }
    repository.authenticating(&attempt).await;
    let auth = tokio::select! {
        biased;
        () = cancel.cancelled() => { let _ = handle.disconnect(Disconnect::ByApplication, "canceled", "en").await; return Err(SshError::Canceled); },
        result = tokio::time::timeout(Duration::from_secs(30), async {
            match authentication {
                AuthenticationMethod::Password(password) => handle.authenticate_password(username, password.expose_secret().to_owned()).await
                    .map(|result| result.success())
                    .map_err(|_| SshError::Authentication),
                AuthenticationMethod::PrivateKey(key) => {
                    let hash = handle
                        .best_supported_rsa_hash()
                        .await
                        .ok()
                        .flatten()
                        .flatten()
                        .or(Some(HashAlg::Sha512));
                    handle.authenticate_publickey(username, PrivateKeyWithHashAlg::new(key, hash)).await
                        .map(|result| result.success())
                        .map_err(|_| SshError::Authentication)
                }
                AuthenticationMethod::Agent { provider, fingerprint } => {
                    let mut agent = open_agent_client(provider).await?;
                    let identities = agent.request_identities().await
                        .map_err(|_| SshError::AgentUnavailable)?;
                    let key = select_agent_identity(&identities, &fingerprint)?;
                    let hash = handle
                        .best_supported_rsa_hash()
                        .await
                        .ok()
                        .flatten()
                        .flatten()
                        .or(Some(HashAlg::Sha512));
                    handle.authenticate_publickey_with(username, key, hash, &mut agent).await
                        .map(|result| result.success())
                        .map_err(|_| SshError::Authentication)
                }
            }
        }) => result,
    };
    // russh also returns an unsuccessful AuthResult when its reply stream ends.
    // Preserve trust failures, then distinguish transport loss from a server's
    // authentication rejection so a transient auth-phase outage can retry.
    if let Some(error) = failure.lock().ok().and_then(|mut failure| failure.take()) {
        return Err(error);
    }
    if handle.is_closed() || transport_closed.is_cancelled() {
        return Err(SshError::Connection);
    }
    let authenticated = match auth {
        Ok(Ok(authenticated)) => authenticated,
        Ok(Err(error)) => return Err(error),
        Err(_) => return Err(SshError::Timeout),
    };
    if !repository.is_current(&attempt).await || cancel.is_cancelled() {
        let _ = handle
            .disconnect(Disconnect::ByApplication, "canceled", "en")
            .await;
        return Err(SshError::Canceled);
    }
    if !authenticated {
        let _ = handle
            .disconnect(Disconnect::ByApplication, "authentication failed", "en")
            .await;
        return Err(SshError::Authentication);
    }
    Ok(PasswordConnection {
        remote_forwards,
        forward_tasks: TaskTracker::new(),
        handle: Arc::new(tokio::sync::Mutex::new(Arc::new(handle))),
        cancel: connection_guard.disarm(),
        transport_closed,
        connection_id: Uuid::new_v4(),
        connection_generation: NEXT_CONNECTION_GENERATION
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        shell_tasks: TaskTracker::new(),
        exec_gate: Arc::new(tokio::sync::Mutex::new(())),
        status_channel: Arc::new(exec::StatusChannelCoordinator::default()),
        agent_forwarding: agent_forwarding.is_some(),
        agent_forwarding_ready: tokio::sync::Mutex::new(false),
        agent_tasks,
        algorithms,
        sftp: sftp_pool::SftpPool::default(),
    })
}

/// Charge only network activity to the handshake deadline. Host-key checking
/// includes the independently bounded human decision and saving trust before
/// authentication. Preserve the unused network budget after that check.
async fn handshake_with_timeout<T>(
    handshake: impl Future<Output = Result<T>>,
    mut host_key_checks: watch::Receiver<bool>,
) -> Result<T> {
    let mut remaining = Duration::from_secs(15);
    let mut active_since = tokio::time::Instant::now();
    let deadline = tokio::time::sleep(remaining);
    tokio::pin!(handshake, deadline);
    let mut paused = false;
    let mut updates_open = true;
    loop {
        tokio::select! {
            biased;
            result = &mut handshake => return result,
            changed = host_key_checks.changed(), if updates_open => {
                updates_open = changed.is_ok();
                let checking = updates_open && *host_key_checks.borrow_and_update();
                if checking && !paused {
                    remaining = remaining.saturating_sub(active_since.elapsed());
                    if remaining.is_zero() {
                        return Err(SshError::Timeout);
                    }
                } else if !checking && paused {
                    active_since = tokio::time::Instant::now();
                    deadline.as_mut().reset(active_since + remaining);
                }
                paused = checking;
            }
            () = &mut deadline, if !paused => return Err(SshError::Timeout),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::{
        keys::{Algorithm, PrivateKey},
        server::{self, Auth, Msg, Server as _, Session},
    };
    use std::{
        net::SocketAddr,
        sync::atomic::{AtomicU64, AtomicUsize, Ordering},
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[derive(Default)]
    struct MemoryTrust {
        key: Mutex<Option<HostPublicKey>>,
        epoch: AtomicU64,
        host_id: Uuid,
        host_revision: u64,
    }
    #[async_trait]
    impl TrustRepository for MemoryTrust {
        async fn lookup(&self, _: &str, _: u16) -> Result<Option<HostPublicKey>> {
            Ok(self.key.lock().map_err(|_| SshError::Protocol)?.clone())
        }
        async fn is_current(&self, attempt: &AttemptBinding) -> bool {
            attempt.vault_epoch == self.epoch.load(Ordering::Acquire)
                && attempt.host_id == self.host_id
                && attempt.host_revision == self.host_revision
        }
        async fn remember(&self, attempt: &AttemptBinding, key: &HostPublicKey) -> Result<()> {
            if !self.is_current(attempt).await {
                return Err(SshError::Canceled);
            }
            *self.key.lock().map_err(|_| SshError::Protocol)? = Some(key.clone());
            Ok(())
        }
    }

    #[derive(Clone)]
    struct TestServer {
        auth_count: Arc<AtomicUsize>,
        agent_forward_count: Arc<AtomicUsize>,
        agent_forward_result: mpsc::UnboundedSender<Option<Vec<u8>>>,
        accept_password: bool,
        accepted_public_key: Option<String>,
        close_after_echo: bool,
        saturate_on_request: bool,
        agent_forward_on_shell: bool,
        agent_forward_on_request: bool,
        agent_forward_requested: bool,
        request_failure: Arc<AtomicUsize>,
    }
    impl server::Server for TestServer {
        type Handler = Self;
        fn new_client(&mut self, _: Option<SocketAddr>) -> Self {
            self.clone()
        }
    }
    impl server::Handler for TestServer {
        type Error = russh::Error;
        async fn auth_password(
            &mut self,
            user: &str,
            password: &str,
        ) -> std::result::Result<Auth, Self::Error> {
            self.auth_count.fetch_add(1, Ordering::SeqCst);
            if self.accept_password && user == "alice" && password == "correct-password" {
                Ok(Auth::Accept)
            } else {
                Ok(Auth::reject())
            }
        }
        async fn auth_publickey(
            &mut self,
            user: &str,
            public_key: &PublicKey,
        ) -> std::result::Result<Auth, Self::Error> {
            self.auth_count.fetch_add(1, Ordering::SeqCst);
            let fingerprint = public_key.fingerprint(HashAlg::Sha256).to_string();
            if user == "alice" && self.accepted_public_key.as_ref() == Some(&fingerprint) {
                Ok(Auth::Accept)
            } else {
                Ok(Auth::reject())
            }
        }
        async fn channel_open_session(
            &mut self,
            _: russh::Channel<Msg>,
            _: &mut Session,
        ) -> std::result::Result<bool, Self::Error> {
            Ok(true)
        }
        async fn pty_request(
            &mut self,
            channel: russh::ChannelId,
            _: &str,
            _: u32,
            _: u32,
            _: u32,
            _: u32,
            _: &[(russh::Pty, u32)],
            session: &mut Session,
        ) -> std::result::Result<(), Self::Error> {
            if self.request_failure.load(Ordering::SeqCst) == 1 {
                return session.channel_failure(channel);
            }
            session.channel_success(channel)
        }
        async fn shell_request(
            &mut self,
            channel: russh::ChannelId,
            session: &mut Session,
        ) -> std::result::Result<(), Self::Error> {
            if self.request_failure.load(Ordering::SeqCst) == 2 {
                return session.channel_failure(channel);
            }
            session.channel_success(channel)?;
            if self.agent_forward_on_shell {
                self.open_agent_channel(session);
            }
            Ok(())
        }
        async fn agent_request(
            &mut self,
            channel: russh::ChannelId,
            session: &mut Session,
        ) -> std::result::Result<bool, Self::Error> {
            self.agent_forward_count.fetch_add(1, Ordering::SeqCst);
            if self.agent_forward_requested {
                session.channel_failure(channel)?;
                return Ok(false);
            }
            self.agent_forward_requested = true;
            // russh's server dispatcher emits a global reply for this extension;
            // explicitly provide the channel reply requested by the client.
            session.channel_success(channel)?;
            if self.agent_forward_on_request {
                self.open_agent_channel(session);
            }
            Ok(true)
        }
        async fn data(
            &mut self,
            channel: russh::ChannelId,
            data: &[u8],
            session: &mut Session,
        ) -> std::result::Result<(), Self::Error> {
            if data == b"agent-flood" {
                for _ in 0..MAX_AGENT_CHANNELS + 3 {
                    self.open_agent_channel(session);
                }
                return Ok(());
            }
            if data == b"eof-before-status" {
                session.data(channel, b"tail".to_vec())?;
                session.eof(channel)?;
                session.exit_status_request(channel, 19)?;
                return session.close(channel);
            }
            if data == b"close-without-status" {
                return session.close(channel);
            }
            if self.saturate_on_request && data == b"start-flood" {
                for _ in 0..256 {
                    session.data(channel, vec![b'x'; 32 * 1024])?;
                }
            } else {
                session.data(channel, data.to_vec())?;
            }
            if self.close_after_echo {
                session.exit_status_request(channel, 7)?;
                session.eof(channel)?;
                session.close(channel)?;
            }
            Ok(())
        }
    }
    struct TestServerGuard {
        task: tokio::task::JoinHandle<std::io::Result<()>>,
        agent_forward_count: Arc<AtomicUsize>,
        agent_forward_result: mpsc::UnboundedReceiver<Option<Vec<u8>>>,
    }
    impl TestServer {
        fn open_agent_channel(&self, session: &Session) {
            let handle = session.handle();
            let result = self.agent_forward_result.clone();
            tokio::spawn(async move {
                let forwarded = async {
                    let channel = handle.channel_open_agent().await.map_err(|_| ())?;
                    let mut stream = channel.into_stream();
                    stream.write_all(&[0, 0, 0, 1, 11]).await.map_err(|_| ())?;
                    let mut response = [0; 9];
                    stream.read_exact(&mut response).await.map_err(|_| ())?;
                    Ok::<_, ()>(response.to_vec())
                }
                .await
                .ok();
                let _ = result.send(forwarded);
            });
        }
    }
    impl Drop for TestServerGuard {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn start_server(
        accept_password: bool,
    ) -> (SocketAddr, PrivateKey, Arc<AtomicUsize>, TestServerGuard) {
        start_server_mode(accept_password, true, false).await
    }

    async fn start_server_mode(
        accept_password: bool,
        close_after_echo: bool,
        saturate_on_request: bool,
    ) -> (SocketAddr, PrivateKey, Arc<AtomicUsize>, TestServerGuard) {
        start_server_config(accept_password, close_after_echo, saturate_on_request, None).await
    }

    async fn start_server_config(
        accept_password: bool,
        close_after_echo: bool,
        saturate_on_request: bool,
        accepted_public_key: Option<String>,
    ) -> (SocketAddr, PrivateKey, Arc<AtomicUsize>, TestServerGuard) {
        start_server_config_with_agent(
            accept_password,
            close_after_echo,
            saturate_on_request,
            accepted_public_key,
            false,
            false,
        )
        .await
    }

    async fn start_server_config_with_agent(
        accept_password: bool,
        close_after_echo: bool,
        saturate_on_request: bool,
        accepted_public_key: Option<String>,
        agent_forward_on_shell: bool,
        agent_forward_on_request: bool,
    ) -> (SocketAddr, PrivateKey, Arc<AtomicUsize>, TestServerGuard) {
        let host_key =
            PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("test server key");
        let config = server::Config {
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![host_key.clone()],
            ..server::Config::default()
        };
        let config = Arc::new(config);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        let auth_count = Arc::new(AtomicUsize::new(0));
        let agent_forward_count = Arc::new(AtomicUsize::new(0));
        let request_failure = Arc::new(AtomicUsize::new(0));
        let (agent_forward_result, agent_forward_result_rx) = mpsc::unbounded_channel();
        let server_state = TestServer {
            auth_count: auth_count.clone(),
            agent_forward_count: agent_forward_count.clone(),
            agent_forward_result,
            accept_password,
            accepted_public_key,
            close_after_echo,
            saturate_on_request,
            agent_forward_on_shell,
            agent_forward_on_request,
            agent_forward_requested: false,
            request_failure: request_failure.clone(),
        };
        let task = tokio::spawn(async move {
            let mut server = server_state;
            server.run_on_socket(config, &listener).await
        });
        (
            address,
            host_key,
            auth_count,
            TestServerGuard {
                task,
                agent_forward_count,
                agent_forward_result: agent_forward_result_rx,
            },
        )
    }
    fn attempt(
        address: SocketAddr,
        host_id: Uuid,
        host_revision: u64,
        vault_epoch: u64,
    ) -> AttemptBinding {
        AttemptBinding {
            attempt_id: Uuid::new_v4(),
            host_id,
            host_revision,
            vault_epoch,
            target: address.ip().to_string(),
            port: address.port(),
        }
    }
    fn repository(host_id: Uuid, key: Option<HostPublicKey>) -> Arc<MemoryTrust> {
        Arc::new(MemoryTrust {
            key: Mutex::new(key),
            epoch: AtomicU64::new(5),
            host_id,
            host_revision: 3,
        })
    }
    fn server_public_key(key: &PrivateKey) -> HostPublicKey {
        HostPublicKey::from_public_key(key.public_key()).expect("server key formatting")
    }
    fn token() -> CancellationToken {
        CancellationToken::new()
    }

    #[tokio::test]
    async fn unknown_key_requires_acceptance_before_any_authentication() {
        let (address, host_key, auth_count, _server) = start_server(true).await;
        let host_id = Uuid::new_v4();
        let repository = repository(host_id, None);
        let (broker, mut events) = TrustBroker::new();
        let binding = attempt(address, host_id, 3, 5);
        let attempt_id = binding.attempt_id;
        let broker_for_connect = broker.clone();
        let repo_for_connect = repository.clone();
        let cancel = token();
        let task = tokio::spawn(connect_password(
            address.ip().to_string(),
            address.port(),
            "alice".into(),
            SecretString::from("correct-password".to_owned()),
            binding,
            repo_for_connect,
            broker_for_connect,
            cancel,
        ));
        let challenge = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("challenge timeout")
            .expect("challenge event");
        assert_eq!(
            challenge.fingerprint,
            server_public_key(&host_key).fingerprint
        );
        assert_eq!(auth_count.load(Ordering::SeqCst), 0);
        broker
            .respond(challenge.challenge_id, false)
            .expect("reject challenge");
        let result = task.await.expect("connect task");
        assert!(matches!(result, Err(SshError::TrustRejected)));
        broker.cancel_attempt(attempt_id);
        assert_eq!(auth_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn rsa_sha2_keys_authenticate_after_format_roundtrips() {
        use russh::keys::ssh_key::{
            private::{KeypairData, RsaKeypair},
            Algorithm,
        };
        use signature::{Signer, Verifier};
        let pair = RsaKeypair::random(&mut rand::rng(), 2048).unwrap();
        for hash in [HashAlg::Sha256, HashAlg::Sha512] {
            let signed = (&pair, Some(hash))
                .try_sign(b"synthetic RSA probe")
                .unwrap();
            pair.public()
                .verify(b"synthetic RSA probe", &signed)
                .unwrap();
            assert!(pair.public().verify(b"modified probe", &signed).is_err());
        }
        assert!((&pair, None)
            .try_sign(b"legacy signing is disabled")
            .is_err());
        let private = PrivateKey::new(KeypairData::Rsa(pair), "").unwrap();
        let pem = private
            .to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .unwrap();
        let mut pkcs8 = zeroize::Zeroizing::new(Vec::new());
        russh::keys::encode_pkcs8_pem(&private, &mut *pkcs8).unwrap();
        let pkcs8 =
            zeroize::Zeroizing::new(String::from_utf8(std::mem::take(&mut *pkcs8)).unwrap());
        for content in [pem.as_str(), pkcs8.as_str()] {
            validate_private_key(content, None).unwrap();
            let (address, _key, auth_count, _server) = start_server_config(
                false,
                false,
                false,
                Some(
                    private
                        .public_key()
                        .fingerprint(HashAlg::Sha256)
                        .to_string(),
                ),
            )
            .await;
            let host_id = Uuid::new_v4();
            let repository = repository(host_id, None);
            let (broker, mut events) = TrustBroker::new();
            let content = content.to_owned();
            let connect_broker = broker.clone();
            let task = tokio::spawn(async move {
                connect_private_key(
                    address.ip().to_string(),
                    address.port(),
                    "alice".into(),
                    &content,
                    None,
                    attempt(address, host_id, 3, 5),
                    repository,
                    connect_broker,
                    token(),
                )
                .await
            });
            let challenge = tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap();
            broker.respond(challenge.challenge_id, true).unwrap();
            let connection = task.await.unwrap().unwrap();
            let shell = connection.open_shell(80, 30).await.unwrap();
            shell.control.close().await.unwrap();
            assert_eq!(auth_count.load(Ordering::SeqCst), 1);
        }
        assert!(private.algorithm().is_rsa());
        assert!(matches!(private.algorithm(), Algorithm::Rsa { .. }));
    }

    #[test]
    fn inconsistent_rsa_private_material_is_rejected_at_import() {
        use russh::keys::ssh_key::{
            private::{RsaKeypair, RsaPrivateKey},
            Mpint,
        };
        let pair = RsaKeypair::random(&mut rand::rng(), 2048).unwrap();
        let bad = RsaPrivateKey::new(
            Mpint::from_positive_bytes(&[3]),
            pair.private().iqmp().clone(),
            pair.private().p().clone(),
            pair.private().q().clone(),
        )
        .unwrap();
        assert!(RsaKeypair::new(pair.public().clone(), bad).is_err());
        assert!(RsaKeypair::random(&mut rand::rng(), 1024).is_err());
    }

    #[tokio::test]
    async fn confirming_host_trust_after_network_timeout_still_connects() {
        let (address, _host_key, auth_count, _server) = start_server(true).await;
        let host_id = Uuid::new_v4();
        let repository = repository(host_id, None);
        let (broker, mut events) = TrustBroker::new();
        let task = tokio::spawn(connect_password(
            address.ip().to_string(),
            address.port(),
            "alice".into(),
            SecretString::from("correct-password".to_owned()),
            attempt(address, host_id, 3, 5),
            repository.clone(),
            broker.clone(),
            token(),
        ));
        let challenge = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("challenge timeout")
            .expect("challenge event");
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(20)).await;
        tokio::task::yield_now().await;
        let still_waiting = !task.is_finished();
        tokio::time::resume();
        assert_eq!(auth_count.load(Ordering::SeqCst), 0);
        assert!(repository.key.lock().unwrap().is_none());
        broker.respond(challenge.challenge_id, true).unwrap();
        let result = task.await.expect("connect task");
        assert!(
            still_waiting,
            "network timeout consumed time spent verifying the fingerprint"
        );
        let connection = result.expect("delayed trust confirmation should connect");
        assert_eq!(auth_count.load(Ordering::SeqCst), 1);
        let shell = connection.open_shell(80, 30).await.unwrap();
        shell.control.close().await.unwrap();
    }

    #[tokio::test]
    async fn host_trust_confirmation_still_expires_without_authentication_or_late_saves() {
        let (address, _host_key, auth_count, _server) = start_server(true).await;
        let host_id = Uuid::new_v4();
        let repository = repository(host_id, None);
        let (broker, mut events) = TrustBroker::new();
        let task = tokio::spawn(connect_password(
            address.ip().to_string(),
            address.port(),
            "alice".into(),
            SecretString::from("correct-password".to_owned()),
            attempt(address, host_id, 3, 5),
            repository.clone(),
            broker.clone(),
            token(),
        ));
        let challenge = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap();
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(121)).await;
        tokio::time::resume();
        let result = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(result, Err(SshError::Timeout)));
        assert_eq!(
            broker.respond(challenge.challenge_id, true),
            Err(SshError::Canceled)
        );
        assert!(repository.key.lock().unwrap().is_none());
        assert_eq!(auth_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn dropping_a_startup_cancels_its_pending_trust_confirmation() {
        let (address, _host_key, auth_count, _server) = start_server(true).await;
        let host_id = Uuid::new_v4();
        let repository = repository(host_id, None);
        let (broker, mut events) = TrustBroker::new();
        let task = tokio::spawn(connect_password(
            address.ip().to_string(),
            address.port(),
            "alice".into(),
            SecretString::from("correct-password".to_owned()),
            attempt(address, host_id, 3, 5),
            repository.clone(),
            broker.clone(),
            token(),
        ));
        let challenge = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap();
        task.abort();
        assert!(task.await.err().unwrap().is_cancelled());
        // The separately spawned russh handler observes cancellation and removes
        // its pending prompt; it must not outlive the dropped startup indefinitely.
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if broker.pending.lock().unwrap().is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            broker.respond(challenge.challenge_id, true),
            Err(SshError::Canceled)
        );
        assert!(repository.key.lock().unwrap().is_none());
        assert_eq!(auth_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn handshake_network_budget_remains_bounded_before_and_after_host_key_checks() {
        let (checks, receiver) = watch::channel(false);
        let task = tokio::spawn(handshake_with_timeout(
            std::future::pending::<Result<()>>(),
            receiver,
        ));
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(10)).await;
        checks.send_replace(true);
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(100)).await;
        assert!(!task.is_finished());
        checks.send_replace(false);
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(4)).await;
        assert!(!task.is_finished());
        tokio::time::advance(Duration::from_secs(2)).await;
        assert_eq!(task.await.unwrap(), Err(SshError::Timeout));
    }

    #[tokio::test]
    async fn accepted_trust_is_persisted_before_password_auth_and_shell_is_interactive() {
        let (address, host_key, auth_count, _server) = start_server(true).await;
        let host_id = Uuid::new_v4();
        let repository = repository(host_id, None);
        let (broker, mut events) = TrustBroker::new();
        let binding = attempt(address, host_id, 3, 5);
        let task = tokio::spawn(connect_password(
            address.ip().to_string(),
            address.port(),
            "alice".into(),
            SecretString::from("correct-password".to_owned()),
            binding,
            repository.clone(),
            broker.clone(),
            token(),
        ));
        let challenge = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("challenge timeout")
            .expect("challenge event");
        broker
            .respond(challenge.challenge_id, true)
            .expect("accept challenge");
        let connection = task
            .await
            .expect("connect task")
            .expect("accepted connection");
        assert_eq!(
            repository
                .lookup(&address.ip().to_string(), address.port())
                .await
                .expect("lookup")
                .expect("persisted key"),
            server_public_key(&host_key)
        );
        assert_eq!(auth_count.load(Ordering::SeqCst), 1);
        let mut shell = connection.open_shell(90, 30).await.expect("open shell");
        shell.control.write(b"probe".to_vec()).await.expect("write");
        let mut received = [0_u8; 5];
        tokio::time::timeout(
            Duration::from_secs(3),
            shell.reader.read_exact(&mut received),
        )
        .await
        .expect("shell output timeout")
        .expect("shell output");
        assert_eq!(&received, b"probe");
        let mut extra = [0_u8; 1];
        let _ = tokio::time::timeout(Duration::from_secs(3), shell.reader.read(&mut extra))
            .await
            .expect("EOF timeout")
            .expect("EOF read");
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if matches!(*shell.exit.borrow(), ChannelExit::Ended(Some(7))) {
                    break;
                }
                shell.exit.changed().await.expect("exit status sender");
            }
        })
        .await
        .expect("exit status timeout");
        connection.disconnect().await;
    }

    #[tokio::test]
    async fn natural_shell_exit_releases_pool_lease() {
        let (address, host_key, auth_count, _server) = start_server(true).await;
        let host_id = Uuid::new_v4();
        let server_key = server_public_key(&host_key);
        let repository = repository(host_id, Some(server_key.clone()));
        let (broker, _) = TrustBroker::new();
        let pool = ConnectionPool::new(Duration::from_millis(50));
        let key = PoolKey {
            target: address.ip().to_string(),
            port: address.port(),
            username: "alice".into(),
            credential_id: Uuid::new_v4(),
            credential_revision: 0,
            trust_fingerprint: server_key.fingerprint,
            vault_epoch: 5,
            agent_identity: None,
            agent_forwarding: false,
            proxy_path: String::new(),
        };
        let lease = pool
            .acquire(key, token(), move |cancel| async move {
                connect_password(
                    address.ip().to_string(),
                    address.port(),
                    "alice".into(),
                    SecretString::from("correct-password".to_owned()),
                    attempt(address, host_id, 3, 5),
                    repository,
                    broker,
                    cancel,
                )
                .await
            })
            .await
            .expect("pooled connection");
        let mut shell = lease.open_shell(80, 24).await.expect("open shell");
        shell
            .control
            .write(b"natural-exit".to_vec())
            .await
            .expect("write natural exit marker");
        let mut output = Vec::new();
        shell
            .reader
            .read_to_end(&mut output)
            .await
            .expect("read natural exit output");
        assert_eq!(output, b"natural-exit");
        assert!(matches!(*shell.exit.borrow(), ChannelExit::Ended(Some(7))));
        drop(shell);
        drop(lease);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(pool.entry_count().await, 0);
        assert_eq!(auth_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn closing_after_transport_disconnect_is_idempotent() {
        let (address, host_key, _, _server) = start_server(true).await;
        let host_id = Uuid::new_v4();
        let repository = repository(host_id, Some(server_public_key(&host_key)));
        let (broker, _) = TrustBroker::new();
        let connection = connect_password(
            address.ip().to_string(),
            address.port(),
            "alice".into(),
            SecretString::from("correct-password".to_owned()),
            attempt(address, host_id, 3, 5),
            repository,
            broker,
            token(),
        )
        .await
        .expect("connected server");
        let shell = connection.open_shell(80, 24).await.expect("open shell");
        let control = shell.control.clone();
        connection.disconnect().await;
        control
            .close()
            .await
            .expect("close succeeds after transport disconnect");
        control
            .close()
            .await
            .expect("second close remains idempotent");
    }

    #[tokio::test]
    async fn authorized_agent_forwarding_relays_remote_requests_to_the_selected_agent() {
        let agent_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test agent");
        let agent_address = agent_listener.local_addr().expect("agent address");
        let agent_service = tokio::spawn(async move {
            let (mut stream, _) = agent_listener.accept().await.expect("agent connection");
            let mut request = [0; 5];
            stream
                .read_exact(&mut request)
                .await
                .expect("agent identity request");
            assert_eq!(request, [0, 0, 0, 1, 11]);
            stream
                .write_all(&[0, 0, 0, 5, 12, 0, 0, 0, 0])
                .await
                .expect("agent identity response");
        });
        let connector: AgentConnector = Arc::new(move || {
            Box::pin(async move {
                let stream = tokio::net::TcpStream::connect(agent_address)
                    .await
                    .map_err(|_| SshError::AgentUnavailable)?;
                Ok(Box::new(stream) as AgentIo)
            })
        });
        let (address, host_key, auth_count, mut server) =
            start_server_config_with_agent(true, false, false, None, false, true).await;
        let host_id = Uuid::new_v4();
        let repository = repository(host_id, Some(server_public_key(&host_key)));
        let (broker, _) = TrustBroker::new();
        let connection = connect_with_authentication(
            address.ip().to_string(),
            address.port(),
            "alice".into(),
            AuthenticationMethod::Password(SecretString::from("correct-password".to_owned())),
            attempt(address, host_id, 3, 5),
            repository,
            broker,
            token(),
            Some(connector),
        )
        .await
        .expect("connected with forwarding enabled");
        let (first, second) =
            tokio::join!(connection.open_shell(80, 24), connection.open_shell(90, 30));
        let shell = first.expect("first concurrent forwarding shell");
        let second = second.expect("second concurrent forwarding shell");
        assert_eq!(shell.connection_id(), second.connection_id());
        let response =
            tokio::time::timeout(Duration::from_secs(5), server.agent_forward_result.recv())
                .await
                .expect("remote agent channel timed out")
                .expect("remote test server still running")
                .expect("forwarded channel returned an agent response");
        assert_eq!(response, [0, 0, 0, 5, 12, 0, 0, 0, 0]);
        assert_eq!(server.agent_forward_count.load(Ordering::SeqCst), 1);
        assert_eq!(auth_count.load(Ordering::SeqCst), 1);
        shell.control.close().await.expect("close forwarding shell");
        second
            .control
            .close()
            .await
            .expect("close second forwarding shell");
        connection.disconnect().await;
        agent_service.await.expect("test agent service");
    }

    #[tokio::test]
    async fn disabled_agent_forwarding_closes_unsolicited_agent_channels() {
        let agent_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test agent");
        let (address, host_key, _, mut server) =
            start_server_config_with_agent(true, false, false, None, true, false).await;
        let host_id = Uuid::new_v4();
        let repository = repository(host_id, Some(server_public_key(&host_key)));
        let (broker, _) = TrustBroker::new();
        let connection = connect_password(
            address.ip().to_string(),
            address.port(),
            "alice".into(),
            SecretString::from("correct-password".to_owned()),
            attempt(address, host_id, 3, 5),
            repository,
            broker,
            token(),
        )
        .await
        .expect("connected with forwarding disabled");
        let shell = connection
            .open_shell(80, 24)
            .await
            .expect("non-forwarding shell");
        let response =
            tokio::time::timeout(Duration::from_secs(5), server.agent_forward_result.recv())
                .await
                .expect("unsolicited agent channel timed out")
                .expect("remote test server still running");
        assert!(
            response.is_none(),
            "the unsolicited channel must close without reaching an agent"
        );
        assert_eq!(server.agent_forward_count.load(Ordering::SeqCst), 0);
        assert!(
            tokio::time::timeout(Duration::from_millis(200), agent_listener.accept())
                .await
                .is_err()
        );
        shell
            .control
            .close()
            .await
            .expect("close non-forwarding shell");
        connection.disconnect().await;
    }

    #[tokio::test]
    async fn changed_keys_block_and_wrong_password_fails_after_trust() {
        let (address, host_key, auth_count, _server) = start_server(false).await;
        let host_id = Uuid::new_v4();
        let trusted_key = server_public_key(&host_key);
        let repository = repository(host_id, Some(trusted_key.clone()));
        let (broker, mut events) = TrustBroker::new();
        let changed = HostPublicKey {
            fingerprint: "SHA256:changed".into(),
            ..server_public_key(&host_key)
        };
        *repository.key.lock().expect("repository") = Some(changed);
        let result = connect_password(
            address.ip().to_string(),
            address.port(),
            "alice".into(),
            SecretString::from("correct-password".to_owned()),
            attempt(address, host_id, 3, 5),
            repository.clone(),
            broker.clone(),
            token(),
        )
        .await;
        assert!(matches!(result, Err(SshError::HostKeyChanged)));
        assert_eq!(auth_count.load(Ordering::SeqCst), 0);
        let notice = events
            .try_recv()
            .expect("changed-key comparison is reported");
        assert!(notice.key_changed);
        assert_eq!(notice.known_algorithm.as_deref(), Some("ssh-ed25519"));
        assert_eq!(notice.known_fingerprint.as_deref(), Some("SHA256:changed"));
        assert_eq!(notice.fingerprint, trusted_key.fingerprint);
        assert!(events.try_recv().is_err());
        *repository.key.lock().expect("repository") = Some(server_public_key(&host_key));
        let result = connect_password(
            address.ip().to_string(),
            address.port(),
            "alice".into(),
            SecretString::from("wrong-password".to_owned()),
            attempt(address, host_id, 3, 5),
            repository,
            broker,
            token(),
        )
        .await;
        assert!(matches!(result, Err(SshError::Authentication)));
        assert_eq!(auth_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn stale_attempt_cannot_persist_a_late_confirmation() {
        let (address, _, auth_count, _server) = start_server(true).await;
        let host_id = Uuid::new_v4();
        let repository = repository(host_id, None);
        let (broker, mut events) = TrustBroker::new();
        let binding = attempt(address, host_id, 3, 5);
        let task = tokio::spawn(connect_password(
            address.ip().to_string(),
            address.port(),
            "alice".into(),
            SecretString::from("correct-password".to_owned()),
            binding,
            repository.clone(),
            broker.clone(),
            token(),
        ));
        let challenge = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("challenge timeout")
            .expect("challenge event");
        repository.epoch.fetch_add(1, Ordering::AcqRel);
        broker
            .respond(challenge.challenge_id, true)
            .expect("answer old challenge");
        let result = task.await.expect("connect task");
        assert!(matches!(result, Err(SshError::Canceled)));
        assert!(repository.key.lock().expect("repository").is_none());
        assert_eq!(auth_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn concurrent_compatible_shells_share_one_authenticated_transport() {
        let (address, host_key, auth_count, _server) = start_server(true).await;
        let host_id = Uuid::new_v4();
        let repository = repository(host_id, Some(server_public_key(&host_key)));
        let (broker, _events) = TrustBroker::new();
        let pool = ConnectionPool::new(Duration::from_secs(30));
        let key = PoolKey {
            target: address.ip().to_string(),
            port: address.port(),
            username: "alice".into(),
            credential_id: Uuid::new_v4(),
            credential_revision: 0,
            trust_fingerprint: server_public_key(&host_key).fingerprint,
            vault_epoch: 5,
            agent_identity: None,
            agent_forwarding: false,
            proxy_path: String::new(),
        };
        let build_count = Arc::new(AtomicUsize::new(0));
        let open = |attempt_id| {
            let pool = pool.clone();
            let key = key.clone();
            let repository: Arc<dyn TrustRepository> = repository.clone();
            let broker = broker.clone();
            let build_count = build_count.clone();
            let mut binding = attempt(address, host_id, 3, 5);
            binding.attempt_id = attempt_id;
            async move {
                pool.acquire(key, token(), move |cancel| async move {
                    build_count.fetch_add(1, Ordering::SeqCst);
                    connect_password(
                        address.ip().to_string(),
                        address.port(),
                        "alice".into(),
                        SecretString::from("correct-password".to_owned()),
                        binding,
                        repository,
                        broker,
                        cancel,
                    )
                    .await
                })
                .await
            }
        };
        let (first, second) = tokio::join!(open(Uuid::new_v4()), open(Uuid::new_v4()));
        let first = first.expect("first pooled lease");
        let second = second.expect("second pooled lease");
        assert_eq!(build_count.load(Ordering::SeqCst), 1);
        assert_eq!(auth_count.load(Ordering::SeqCst), 1);
        let first_shell = first.open_shell(80, 24).await.expect("first shell");
        let second_shell = second.open_shell(100, 36).await.expect("second shell");
        assert_eq!(first_shell.connection_id(), second_shell.connection_id());
        assert_eq!(auth_count.load(Ordering::SeqCst), 1);
        first_shell
            .control
            .close()
            .await
            .expect("close first channel");
        second_shell
            .control
            .close()
            .await
            .expect("close second channel");
    }

    #[tokio::test]
    async fn a_saturated_shell_does_not_block_another_shell_on_the_same_transport() {
        let (address, host_key, auth_count, _server) = start_server_mode(true, false, true).await;
        let host_id = Uuid::new_v4();
        let server_key = server_public_key(&host_key);
        let repository = repository(host_id, Some(server_key.clone()));
        let (broker, _events) = TrustBroker::new();
        let connection = connect_password(
            address.ip().to_string(),
            address.port(),
            "alice".into(),
            SecretString::from("correct-password".to_owned()),
            attempt(address, host_id, 3, 5),
            repository,
            broker,
            token(),
        )
        .await
        .expect("trusted connection");
        let slow = connection.open_shell(80, 24).await.expect("slow shell");
        let fast = connection.open_shell(80, 24).await.expect("fast shell");
        slow.control
            .write(b"start-flood".to_vec())
            .await
            .expect("start slow channel output");
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut fast_reader = fast.reader;
        let response = async {
            fast.control.write(b"responsive-channel".to_vec()).await?;
            let mut bytes = [0_u8; 18];
            fast_reader
                .read_exact(&mut bytes)
                .await
                .map_err(|_| SshError::Protocol)?;
            Ok::<_, SshError>(bytes)
        };
        let result = tokio::time::timeout(Duration::from_secs(3), response).await;
        assert_eq!(auth_count.load(Ordering::SeqCst), 1);
        assert!(
            result.is_ok(),
            "another shell should receive data while the first shell is backpressured"
        );
        let bytes = result
            .expect("fast-shell timeout result")
            .expect("fast-shell response");
        assert_eq!(&bytes, b"responsive-channel");
        connection.disconnect().await;
    }

    #[tokio::test]
    async fn imported_ed25519_key_authenticates_and_opens_a_shell() {
        let client_key =
            PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("generate client key");
        let client_fingerprint = client_key
            .public_key()
            .fingerprint(HashAlg::Sha256)
            .to_string();
        let (address, host_key, auth_count, _server) =
            start_server_config(false, true, false, Some(client_fingerprint)).await;
        let host_id = Uuid::new_v4();
        let repository = repository(host_id, Some(server_public_key(&host_key)));
        let (broker, _) = TrustBroker::new();
        let content = client_key
            .to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .expect("encode test client key")
            .to_string();
        validate_private_key(&content, None).expect("test key validates");
        let connection = connect_private_key(
            address.ip().to_string(),
            address.port(),
            "alice".into(),
            &content,
            None,
            attempt(address, host_id, 3, 5),
            repository,
            broker,
            token(),
        )
        .await
        .expect("public key authentication");
        assert_eq!(auth_count.load(Ordering::SeqCst), 1);
        let shell = connection
            .open_shell(80, 24)
            .await
            .expect("public-key shell");
        shell
            .control
            .write(b"key-authenticated".to_vec())
            .await
            .expect("write key-authenticated marker");
        let mut reader = shell.reader;
        let mut received = vec![0; b"key-authenticated".len()];
        reader
            .read_exact(&mut received)
            .await
            .expect("read public-key shell marker");
        assert_eq!(received, b"key-authenticated");
        shell.control.close().await.expect("close key shell");
        connection.disconnect().await;
    }

    #[test]
    fn agent_identity_selection_uses_fingerprints_and_omits_agent_comments() {
        let private = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
            .expect("generate agent identity");
        let public = private.public_key().clone();
        let fingerprint = public.fingerprint(HashAlg::Sha256).to_string();
        let identity = AgentIdentity::PublicKey {
            key: public.clone(),
            comment: "sensitive-agent-comment".to_owned(),
        };

        let view = agent_identity_view(&identity).expect("public-key identity is selectable");
        assert_eq!(view.algorithm, public.algorithm().to_string());
        assert_eq!(view.fingerprint, fingerprint);
        assert!(!serde_json::to_string(&view)
            .expect("serialize public identity view")
            .contains("sensitive-agent-comment"));
        assert_eq!(
            select_agent_identity(std::slice::from_ref(&identity), &fingerprint).unwrap(),
            public
        );
        assert_eq!(
            select_agent_identity(&[identity], "SHA256:another-key")
                .unwrap_err()
                .code(),
            "ssh_agent_identity_unavailable"
        );
    }

    #[test]
    fn private_key_validation_rejects_corrupt_data_without_echoing_it() {
        let marker = "PRIVATE-KEY-SECRET-MARKER";
        let error = validate_private_key(marker, None).expect_err("corrupt key rejected");
        assert_eq!(error.code(), "ssh_private_key_invalid");
        assert!(!format!("{error:?}").contains(marker));
        assert!(!private_key_requires_passphrase(marker));
    }

    #[test]
    fn private_key_validation_covers_generated_openssh_and_pkcs8_encodings() {
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
            .expect("generate an ephemeral test key");
        let openssh = key
            .to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .expect("encode unencrypted OpenSSH key");
        validate_private_key(&openssh, None).expect("accept unencrypted OpenSSH key");

        let encrypted = key
            .encrypt(&mut rand::rng(), "temporary test passphrase")
            .expect("encrypt ephemeral test key");
        let encrypted_openssh = encrypted
            .to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .expect("encode encrypted OpenSSH key");
        validate_private_key(&encrypted_openssh, Some("temporary test passphrase"))
            .expect("accept encrypted OpenSSH key with its passphrase");
        assert_eq!(
            validate_private_key(&encrypted_openssh, Some("wrong test passphrase"))
                .expect_err("reject a wrong passphrase")
                .code(),
            "ssh_private_key_invalid"
        );

        let mut pkcs8 = Vec::new();
        russh::keys::encode_pkcs8_pem(&key, &mut pkcs8).expect("encode unencrypted PKCS#8 PEM");
        let pkcs8 =
            zeroize::Zeroizing::new(String::from_utf8(pkcs8).expect("PKCS#8 PEM is valid UTF-8"));
        validate_private_key(&pkcs8, None).expect("accept unencrypted PKCS#8 PEM");

        let mut encrypted_pkcs8 = Vec::new();
        russh::keys::encode_pkcs8_pem_encrypted(
            &key,
            b"temporary test passphrase",
            1_000,
            &mut encrypted_pkcs8,
        )
        .expect("encode encrypted PKCS#8 PEM");
        let encrypted_pkcs8 = zeroize::Zeroizing::new(
            String::from_utf8(encrypted_pkcs8).expect("encrypted PKCS#8 PEM is valid UTF-8"),
        );
        validate_private_key(&encrypted_pkcs8, Some("temporary test passphrase"))
            .expect("accept encrypted PKCS#8 PEM with its passphrase");
        assert_eq!(
            validate_private_key(&encrypted_pkcs8, None)
                .expect_err("reject an encrypted key without its passphrase")
                .code(),
            "ssh_private_key_invalid"
        );
    }
}
