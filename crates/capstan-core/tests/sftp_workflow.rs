//! Production SFTP service exercised over an in-process russh transport.
#![allow(clippy::pedantic)]

use capstan_core::sftp::{
    DirectoryPage, Mutation, RemotePath, SftpError, SftpService, SftpSessionId,
};
use capstan_ssh::{
    AttemptBinding, ConnectionLease, ConnectionPool, HostPublicKey, PasswordConnection, PoolKey,
    TrustBroker, TrustRepository,
};
use russh::{
    server::{self, Server as _, Session},
    Channel, ChannelId,
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Notify,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use uuid::Uuid;
#[path = "sftp_workflow/cwd.rs"]
mod cwd;
#[path = "sftp_workflow/lifecycle.rs"]
mod lifecycle;
#[cfg(all(windows, feature = "benchmarks"))]
#[path = "sftp_workflow/performance.rs"]
mod performance;
#[cfg(windows)]
#[path = "sftp_workflow/relay.rs"]
mod relay;
#[path = "sftp_workflow/subsystem_pool.rs"]
mod subsystem_pool;
#[path = "sftp_workflow/transfers.rs"]
mod transfers;

#[derive(Default)]
struct Observations {
    auth: AtomicUsize,
    subsystems: AtomicUsize,
    subsystem_requests: AtomicUsize,
    active_subsystems: AtomicUsize,
    peak_subsystems: AtomicUsize,
    session_limit: AtomicUsize,
    rejected_sessions: AtomicUsize,
    reject_subsystem: AtomicBool,
    invalid_sftp_version: AtomicBool,
    stall_init: AtomicBool,
    init_started: Notify,
    init_release: Notify,
    transport: std::sync::OnceLock<server::Handle>,
    shell_channels: std::sync::Mutex<Vec<ChannelId>>,
    mutations: AtomicUsize,
    handles: AtomicUsize,
    stats: AtomicUsize,
    stall_stat: AtomicBool,
    stat_started: Notify,
    stat_release: Notify,
    transfer_mode: AtomicBool,
    transfer: transfers::RemoteState,
}

struct ActiveSubsystem(Arc<Observations>);
impl ActiveSubsystem {
    fn new(observations: Arc<Observations>) -> Self {
        let active = observations
            .active_subsystems
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        observations
            .peak_subsystems
            .fetch_max(active, Ordering::SeqCst);
        Self(observations)
    }
}
impl Drop for ActiveSubsystem {
    fn drop(&mut self) {
        self.0.active_subsystems.fetch_sub(1, Ordering::SeqCst);
    }
}

struct OpenHandles<'a> {
    observations: &'a Observations,
    count: usize,
}
impl<'a> OpenHandles<'a> {
    fn new(observations: &'a Observations) -> Self {
        Self {
            observations,
            count: 0,
        }
    }
    fn opened(&mut self) {
        self.count += 1;
        self.observations.handles.fetch_add(1, Ordering::SeqCst);
    }
    fn closed(&mut self) {
        self.count -= 1;
        self.observations.handles.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Drop for OpenHandles<'_> {
    fn drop(&mut self) {
        self.observations
            .handles
            .fetch_sub(self.count, Ordering::SeqCst);
    }
}

#[derive(Clone)]
struct TestServer {
    password: Arc<zeroize::Zeroizing<String>>,
    observations: Arc<Observations>,
    channels: HashMap<ChannelId, Arc<tokio::sync::Mutex<Option<Channel<server::Msg>>>>>,
    tasks: TaskTracker,
    cancel: CancellationToken,
}
impl server::Server for TestServer {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        let mut handler = self.clone();
        handler.channels.clear();
        handler
    }
}
impl server::Handler for TestServer {
    type Error = russh::Error;
    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        self.observations.auth.fetch_add(1, Ordering::SeqCst);
        Ok(if user == "test" && password == self.password.as_str() {
            server::Auth::Accept
        } else {
            server::Auth::reject()
        })
    }
    async fn channel_open_session(
        &mut self,
        channel: Channel<server::Msg>,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        let _ = self.observations.transport.set(session.handle());
        let limit = self.observations.session_limit.load(Ordering::SeqCst);
        if limit != 0
            && self.channels.len() + self.observations.active_subsystems.load(Ordering::SeqCst)
                >= limit
        {
            self.observations
                .rejected_sessions
                .fetch_add(1, Ordering::SeqCst);
            return Ok(false);
        }
        self.channels.insert(
            channel.id(),
            Arc::new(tokio::sync::Mutex::new(Some(channel))),
        );
        Ok(true)
    }
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _: &str,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
        _: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)
    }
    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.observations
            .shell_channels
            .lock()
            .unwrap()
            .push(channel);
        session.channel_success(channel)
    }
    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.channels.contains_key(&channel) {
            session.data(channel, data.to_vec())?;
        }
        Ok(())
    }
    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name != "sftp" {
            return session.channel_failure(channel);
        }
        self.observations
            .subsystem_requests
            .fetch_add(1, Ordering::SeqCst);
        if self.observations.reject_subsystem.load(Ordering::SeqCst) {
            session.channel_failure(channel)?;
            self.channels.remove(&channel);
            return session.close(channel);
        }
        let channel_owner = self
            .channels
            .remove(&channel)
            .unwrap()
            .lock()
            .await
            .take()
            .unwrap();
        session.channel_success(channel)?;
        self.observations.subsystems.fetch_add(1, Ordering::SeqCst);
        let cancel = self.cancel.clone();
        let observations = self.observations.clone();
        self.tasks.spawn(async move {
            let _active = ActiveSubsystem::new(observations.clone());
            tokio::select! {
                () = cancel.cancelled() => {},
                () = async {
                    let stream = channel_owner.into_stream();
                    if observations.transfer_mode.load(Ordering::SeqCst) {
                        transfers::serve(stream, observations).await;
                    } else { serve_sftp(stream, observations).await; }
                } => {},
            }
        });
        Ok(())
    }
}

fn string(packet: &mut Vec<u8>, value: &[u8]) {
    packet.extend_from_slice(&(value.len() as u32).to_be_bytes());
    packet.extend_from_slice(value);
}
fn get_string(packet: &[u8]) -> String {
    let length = u32::from_be_bytes(packet[5..9].try_into().unwrap()) as usize;
    String::from_utf8(packet[9..9 + length].to_vec()).unwrap()
}
fn reply(kind: u8, id: u32) -> Vec<u8> {
    let mut packet = vec![kind];
    packet.extend_from_slice(&id.to_be_bytes());
    packet
}
fn attributes(packet: &mut Vec<u8>, mode: u32) {
    packet.extend_from_slice(&13_u32.to_be_bytes());
    packet.extend_from_slice(&12_u64.to_be_bytes());
    packet.extend_from_slice(&mode.to_be_bytes());
    packet.extend_from_slice(&100_u32.to_be_bytes());
    packet.extend_from_slice(&100_u32.to_be_bytes());
}
fn names(id: u32, entries: Vec<(Vec<u8>, u32)>) -> Vec<u8> {
    let mut packet = reply(104, id);
    packet.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (name, mode) in entries {
        string(&mut packet, &name);
        string(&mut packet, b"");
        attributes(&mut packet, mode);
    }
    packet
}
fn status(id: u32, code: u32) -> Vec<u8> {
    let mut packet = reply(101, id);
    packet.extend_from_slice(&code.to_be_bytes());
    string(&mut packet, b"untrusted details");
    string(&mut packet, b"en");
    packet
}
async fn serve_sftp<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    mut stream: S,
    observations: Arc<Observations>,
) {
    let mut handles: HashMap<String, (String, u32)> = HashMap::new();
    let mut gauge = OpenHandles::new(&observations);
    let mut serial = 0;
    while let Ok(length) = stream.read_u32().await {
        assert!(length <= 262144);
        let mut packet = vec![0; length as usize];
        if stream.read_exact(&mut packet).await.is_err() {
            break;
        }
        let id = u32::from_be_bytes(packet[1..5].try_into().unwrap());
        let response = match packet[0] {
            1 => {
                if observations.stall_init.load(Ordering::SeqCst) {
                    observations.init_started.notify_one();
                    tokio::select! {
                        () = observations.init_release.notified() => {}
                        result = stream.read_u8() => {
                            assert!(result.is_err(), "no request is valid before VERSION");
                            break;
                        }
                    }
                }
                vec![
                    2,
                    0,
                    0,
                    0,
                    if observations.invalid_sftp_version.load(Ordering::SeqCst) {
                        4
                    } else {
                        3
                    },
                ]
            }
            16 => {
                let path = get_string(&packet);
                if path == "/missing" {
                    status(id, 2)
                } else {
                    names(
                        id,
                        vec![(
                            if path == "." {
                                b"/".to_vec()
                            } else if path == "/link" {
                                b"/empty".to_vec()
                            } else {
                                path.into_bytes()
                            },
                            0o40755,
                        )],
                    )
                }
            }
            11 | 3 => {
                let path = get_string(&packet);
                if path == "/denied" {
                    status(id, 3)
                } else {
                    if packet[0] == 3 {
                        observations.mutations.fetch_add(1, Ordering::SeqCst);
                    }
                    serial += 1;
                    let handle = format!("handle-{serial}");
                    handles.insert(handle.clone(), (path, 0));
                    gauge.opened();
                    let mut response = reply(102, id);
                    string(&mut response, handle.as_bytes());
                    response
                }
            }
            12 => {
                let (path, page) = handles.get_mut(&get_string(&packet)).unwrap();
                let index = *page;
                *page += 1;
                if path == "/read-denied" {
                    status(id, 3)
                } else if index == 0 || path == "/dots" {
                    names(
                        id,
                        vec![(b".".to_vec(), 0o40755), (b"..".to_vec(), 0o40755)],
                    )
                } else if path == "/empty" || index > if path == "/many" { 3 } else { 1 } {
                    status(id, 1)
                } else if path == "/exact" || path == "/many" {
                    names(
                        id,
                        (0..256)
                            .map(|n| (format!("file-{index}-{n}").into_bytes(), 0o100644))
                            .collect(),
                    )
                } else if path == "/duplicate" {
                    names(
                        id,
                        vec![(b"same".to_vec(), 0o100644), (b"same".to_vec(), 0o100644)],
                    )
                } else if path == "/invalid-only" {
                    names(id, vec![(vec![255], 0o100644)])
                } else {
                    names(
                        id,
                        vec![
                            (b"file".to_vec(), 0o100644),
                            (b"link".to_vec(), 0o120777),
                            (vec![255], 0o100644),
                            (b"../escape".to_vec(), 0o100644),
                        ],
                    )
                }
            }
            4 => {
                assert!(handles.remove(&get_string(&packet)).is_some());
                gauge.closed();
                status(id, 0)
            }
            7 => {
                observations.stats.fetch_add(1, Ordering::SeqCst);
                if observations.stall_stat.load(Ordering::SeqCst) {
                    observations.stat_started.notify_one();
                    observations.stat_release.notified().await;
                }
                let mut response = reply(105, id);
                attributes(
                    &mut response,
                    if get_string(&packet).ends_with("/link") {
                        0o120777
                    } else {
                        0o100644
                    },
                );
                response
            }
            9 | 13 | 14 | 15 | 18 => {
                observations.mutations.fetch_add(1, Ordering::SeqCst);
                status(id, 0)
            }
            _ => panic!("unexpected SFTP request type {}", packet[0]),
        };
        if stream.write_u32(response.len() as u32).await.is_err()
            || stream.write_all(&response).await.is_err()
        {
            break;
        }
    }
}

struct Trust(HostPublicKey);
#[async_trait::async_trait]
impl TrustRepository for Trust {
    async fn lookup(&self, _: &str, _: u16) -> capstan_ssh::Result<Option<HostPublicKey>> {
        Ok(Some(self.0.clone()))
    }
    async fn is_current(&self, _: &AttemptBinding) -> bool {
        true
    }
    async fn remember(&self, _: &AttemptBinding, _: &HostPublicKey) -> capstan_ssh::Result<()> {
        panic!("the generated key is pinned")
    }
}

struct Fixture {
    service: SftpService,
    shells: [capstan_ssh::SshShell; 2],
    lease: ConnectionLease<PasswordConnection>,
    pool: ConnectionPool<PasswordConnection>,
    observations: Arc<Observations>,
    server: tokio::task::JoinHandle<()>,
    tasks: TaskTracker,
    cancel: CancellationToken,
    connector: Connector,
}
#[derive(Clone)]
struct Connector {
    binding: AttemptBinding,
    pool_key: PoolKey,
    password: Arc<zeroize::Zeroizing<String>>,
    trust: Arc<Trust>,
}
impl Connector {
    async fn acquire(
        &self,
        pool: &ConnectionPool<PasswordConnection>,
    ) -> ConnectionLease<PasswordConnection> {
        let mut binding = self.binding.clone();
        binding.attempt_id = Uuid::new_v4();
        let password = self.password.clone();
        let trust = self.trust.clone();
        let (broker, _events) = TrustBroker::new();
        pool.acquire_checked(
            self.pool_key.clone(),
            CancellationToken::new(),
            |connection| !connection.is_closed(),
            move |cancel| async move {
                capstan_ssh::connect_password(
                    binding.target.clone(),
                    binding.port,
                    "test".into(),
                    secrecy::SecretString::from(password.to_string()),
                    binding,
                    trust,
                    broker,
                    cancel,
                )
                .await
            },
        )
        .await
        .unwrap()
    }
}
impl Fixture {
    async fn new() -> Self {
        let key =
            russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)
                .unwrap();
        let trust = Arc::new(Trust(
            HostPublicKey::from_public_key(key.public_key()).unwrap(),
        ));
        let password = Arc::new(zeroize::Zeroizing::new(Uuid::new_v4().to_string()));
        let observations = Arc::new(Observations::default());
        let tasks = TaskTracker::new();
        let cancel = CancellationToken::new();
        let mut server = TestServer {
            password: password.clone(),
            observations: observations.clone(),
            channels: HashMap::new(),
            tasks: tasks.clone(),
            cancel: cancel.clone(),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let config = Arc::new(server::Config {
            keys: vec![key],
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            ..Default::default()
        });
        let server_task = tokio::spawn(async move {
            server.run_on_socket(config, &listener).await.unwrap();
        });
        let pool = ConnectionPool::new(Duration::from_secs(30));
        let host = Uuid::new_v4();
        let binding = AttemptBinding {
            attempt_id: Uuid::new_v4(),
            host_id: host,
            host_revision: 1,
            vault_epoch: 1,
            target: "127.0.0.1".into(),
            port: address.port(),
        };
        let pool_key = PoolKey {
            target: binding.target.clone(),
            port: binding.port,
            username: "test".into(),
            credential_id: Uuid::new_v4(),
            credential_revision: 1,
            trust_fingerprint: trust.0.fingerprint.clone(),
            vault_epoch: 1,
            agent_identity: None,
            agent_forwarding: false,
            proxy_path: String::new(),
        };
        let connector = Connector {
            binding,
            pool_key,
            password,
            trust,
        };
        let lease = connector.acquire(&pool).await;
        let shells = [
            lease.open_shell(80, 24).await.unwrap(),
            lease.open_shell(80, 24).await.unwrap(),
        ];
        assert_eq!(shells[0].connection_id(), shells[1].connection_id());
        let service = SftpService::default();
        service
            .register(
                "main",
                "one",
                host,
                shells[0].connection_id(),
                lease.clone(),
            )
            .unwrap();
        service
            .register(
                "main",
                "two",
                host,
                shells[1].connection_id(),
                lease.clone(),
            )
            .unwrap();
        Self {
            service,
            shells,
            lease,
            pool,
            observations,
            server: server_task,
            tasks,
            cancel,
            connector,
        }
    }
    async fn attach(&self, terminal: &str) -> SftpSessionId {
        self.service
            .attach("main", terminal, &CancellationToken::new())
            .await
            .unwrap()
            .session_id
    }
    async fn list(&self, session: SftpSessionId, path: &str) -> Result<DirectoryPage, SftpError> {
        self.service
            .list("main", "one", session, RemotePath::parse(path).unwrap())
            .await
    }
    async fn close(self) {
        self.service.revoke_terminal("one");
        self.service.revoke_terminal("two");
        for shell in &self.shells {
            shell.control.close().await.unwrap();
        }
        for connection in self.pool.drain().await {
            connection.disconnect().await;
        }
        self.cancel.cancel();
        self.tasks.close();
        self.tasks.wait().await;
        self.server.abort();
        let _ = self.server.await;
    }
}

#[tokio::test]
async fn dot_packets_invalid_names_and_exact_last_pages_do_not_hide_supported_files() {
    let fixture = Fixture::new().await;
    let session = fixture.attach("one").await;
    let empty = fixture.list(session, "/empty").await.unwrap();
    assert!(empty.entries.is_empty() && !empty.has_more);
    let page = fixture.list(session, "/").await.unwrap();
    assert_eq!(
        page.entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["file", "link"]
    );
    assert_eq!(page.omitted_entries, 2);
    assert!(!page.has_more);
    let unsupported = fixture.list(session, "/invalid-only").await.unwrap();
    assert!(unsupported.entries.is_empty() && !unsupported.has_more);
    assert_eq!(unsupported.omitted_entries, 1);
    let exact = fixture.list(session, "/exact").await.unwrap();
    assert_eq!(exact.entries.len(), 256);
    assert!(!exact.has_more);
    let first = fixture.list(session, "/many").await.unwrap();
    let second = fixture
        .service
        .next_page("main", "one", session, first.request_revision)
        .await
        .unwrap();
    let third = fixture
        .service
        .next_page("main", "one", session, first.request_revision)
        .await
        .unwrap();
    assert!(first.has_more && second.has_more && !third.has_more);
    assert_eq!(third.entries.len(), 256);
    assert_eq!(fixture.observations.handles.load(Ordering::SeqCst), 0);
    fixture.close().await;
}

#[tokio::test]
async fn failed_navigation_preserves_old_entries_confirmation_and_cursor() {
    let fixture = Fixture::new().await;
    let session = fixture.attach("one").await;
    let page = fixture.list(session, "/many").await.unwrap();
    let entry = page.entries[0].id;
    for (path, error) in [
        ("/missing", SftpError::NotFound),
        ("/denied", SftpError::PermissionDenied),
        ("/read-denied", SftpError::PermissionDenied),
        ("/duplicate", SftpError::TargetConflict),
        ("/dots", SftpError::ResourceLimit),
    ] {
        assert_eq!(fixture.list(session, path).await, Err(error));
        assert!(fixture
            .service
            .prepare_mutation("main", "one", session, entry, Mutation::Delete)
            .await
            .is_ok());
        assert_eq!(fixture.observations.handles.load(Ordering::SeqCst), 1);
    }
    let next = fixture
        .service
        .next_page("main", "one", session, page.request_revision)
        .await
        .unwrap();
    assert_eq!(next.page_sequence, 1);
    fixture.close().await;
}

#[tokio::test]
async fn create_requires_the_current_successfully_browsed_directory() {
    let fixture = Fixture::new().await;
    let session = fixture.attach("one").await;
    assert_eq!(
        fixture
            .service
            .create(
                "main",
                "one",
                session,
                RemotePath::parse("/").unwrap(),
                "new",
                true
            )
            .await,
        Err(SftpError::ScopeExpired)
    );
    fixture.list(session, "/empty").await.unwrap();
    for path in ["/", "/empty/..", "/unrelated"] {
        assert_eq!(
            fixture
                .service
                .create(
                    "main",
                    "one",
                    session,
                    RemotePath::parse(path).unwrap(),
                    "new",
                    true
                )
                .await,
            Err(SftpError::ScopeExpired)
        );
    }
    assert_eq!(fixture.observations.mutations.load(Ordering::SeqCst), 0);
    fixture
        .service
        .create(
            "main",
            "one",
            session,
            RemotePath::parse("/empty").unwrap(),
            "new",
            true,
        )
        .await
        .unwrap();
    assert_eq!(fixture.observations.mutations.load(Ordering::SeqCst), 1);
    fixture.list(session, "/").await.unwrap();
    assert_eq!(
        fixture
            .service
            .create(
                "main",
                "one",
                session,
                RemotePath::parse("/empty").unwrap(),
                "stale",
                false
            )
            .await,
        Err(SftpError::ScopeExpired)
    );
    fixture.close().await;
}

#[tokio::test]
async fn symlink_permissions_are_rejected_before_confirmation_but_navigation_and_unlink_work() {
    let fixture = Fixture::new().await;
    let session = fixture.attach("one").await;
    let page = fixture.list(session, "/").await.unwrap();
    let entry = page.entries[1].id;
    assert_eq!(
        fixture
            .service
            .prepare_mutation(
                "main",
                "one",
                session,
                entry,
                Mutation::Permissions { mode: 0o700 }
            )
            .await
            .unwrap_err(),
        SftpError::UnsupportedFileType
    );
    assert_eq!(fixture.observations.stats.load(Ordering::SeqCst), 0);
    let confirmation = fixture
        .service
        .prepare_mutation("main", "one", session, entry, Mutation::Delete)
        .await
        .unwrap();
    fixture
        .service
        .apply_mutation("main", "one", session, confirmation.token)
        .await
        .unwrap();
    assert_eq!(fixture.observations.mutations.load(Ordering::SeqCst), 1);
    let linked = fixture.list(session, "/link").await.unwrap();
    assert_eq!(linked.path.as_str(), "/empty");
    fixture.close().await;
}

#[tokio::test]
async fn concurrent_views_share_one_subsystem_and_cannot_consume_each_others_confirmations() {
    let fixture = Fixture::new().await;
    let (first, second) = tokio::join!(fixture.attach("one"), fixture.attach("two"));
    let page = fixture.list(first, "/").await.unwrap();
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 1);
    let confirmation = fixture
        .service
        .prepare_mutation("main", "one", first, page.entries[0].id, Mutation::Delete)
        .await
        .unwrap();
    assert_eq!(
        fixture
            .service
            .apply_mutation("main", "two", second, confirmation.token)
            .await,
        Err(SftpError::ScopeExpired)
    );
    fixture
        .service
        .apply_mutation("main", "one", first, confirmation.token)
        .await
        .unwrap();
    assert_eq!(
        fixture
            .service
            .apply_mutation("main", "one", first, confirmation.token)
            .await,
        Err(SftpError::ScopeExpired)
    );
    fixture.service.detach("main", "one", first).await.unwrap();
    fixture
        .service
        .list("main", "two", second, RemotePath::parse("/empty").unwrap())
        .await
        .unwrap();
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 1);
    fixture.close().await;
}

#[tokio::test]
async fn revoking_a_terminal_cancels_a_pending_check_before_it_can_mutate() {
    let fixture = Fixture::new().await;
    let session = fixture.attach("one").await;
    let page = fixture.list(session, "/").await.unwrap();
    let confirmation = fixture
        .service
        .prepare_mutation("main", "one", session, page.entries[0].id, Mutation::Delete)
        .await
        .unwrap();
    fixture
        .observations
        .stall_stat
        .store(true, Ordering::SeqCst);
    let apply = fixture
        .service
        .apply_mutation("main", "one", session, confirmation.token);
    let revoke = async {
        fixture.observations.stat_started.notified().await;
        fixture.service.revoke_terminal("one");
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(apply, revoke)
    })
    .await
    .unwrap();
    assert_eq!(result, Err(SftpError::OperationCanceled));
    fixture.observations.stat_release.notify_one();
    assert_eq!(fixture.observations.mutations.load(Ordering::SeqCst), 0);
    // Canceling SFTP must not disconnect the SSH transport or a new shell.
    let shell = fixture.lease.open_shell(80, 24).await.unwrap();
    shell.control.close().await.unwrap();
    fixture.close().await;
}
