use capstan_core::VaultManager;
use capstan_ssh::{
    connection_state::{ConnectionSnapshot, ConnectionState},
    ConnectionLease, PasswordConnection, SshShell, TrustBroker,
};
use capstan_vault::{Host, KdfParameters, SecretText, TunnelConfig, TunnelForward};
use russh::{
    keys::{Algorithm, PrivateKey},
    server::{self, Session},
    Channel, ChannelId,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use uuid::Uuid;

#[derive(Default)]
pub struct Observed {
    pub accepts: AtomicUsize,
    pub reject_remote: AtomicBool,
    pub reject_direct: AtomicBool,
    pub cancel_denied: AtomicBool,
    pub cancel_paused: AtomicBool,
    pub cancel_resume: tokio::sync::Notify,
    pub cancel_requests: AtomicUsize,
    pub pty_denied: AtomicBool,
    pub pty_paused: AtomicBool,
    pub pty_resume: tokio::sync::Notify,
    pub pty_requests: AtomicUsize,
    pub remote_paused: AtomicBool,
    pub remote_resume: tokio::sync::Notify,
    pub remote_requests_started: AtomicUsize,
    pub destinations: Mutex<Vec<(String, u32)>>,
    pub handles: Mutex<Vec<server::Handle>>,
}
struct Peer {
    observed: Arc<Observed>,
    tasks: TaskTracker,
    stop: CancellationToken,
    shells: HashSet<ChannelId>,
    listeners: HashMap<(String, u32), (CancellationToken, CancellationToken)>,
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
impl server::Handler for Peer {
    type Error = russh::Error;
    async fn auth_password(&mut self, _: &str, _: &str) -> Result<server::Auth, Self::Error> {
        Ok(server::Auth::Accept)
    }
    async fn channel_open_session(
        &mut self,
        channel: Channel<server::Msg>,
        _: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.shells.insert(channel.id());
        Ok(true)
    }
    async fn pty_request(
        &mut self,
        id: ChannelId,
        _: &str,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
        _: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.observed.pty_requests.fetch_add(1, Ordering::SeqCst);
        if self.observed.pty_paused.swap(false, Ordering::SeqCst) {
            self.observed.pty_resume.notified().await;
        }
        if self.observed.pty_denied.load(Ordering::SeqCst) {
            session.channel_failure(id)
        } else {
            session.channel_success(id)
        }
    }
    async fn shell_request(
        &mut self,
        id: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(id)
    }
    async fn data(
        &mut self,
        id: ChannelId,
        bytes: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.shells.contains(&id) {
            session.data(id, bytes.to_vec())?;
        }
        Ok(())
    }
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<server::Msg>,
        host: &str,
        port: u32,
        _: &str,
        _: u32,
        _: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.observed
            .destinations
            .lock()
            .unwrap()
            .push((host.into(), port));
        if self.observed.reject_direct.load(Ordering::SeqCst) {
            return Ok(false);
        }
        let Ok(mut stream) = TcpStream::connect((host, u16::try_from(port).unwrap())).await else {
            return Ok(false);
        };
        let stop = self.stop.clone();
        self.tasks.spawn(async move {
            let mut channel = channel.into_stream();
            tokio::select! { _ = stop.cancelled() => {}, _ = tokio::io::copy_bidirectional(&mut stream, &mut channel) => {} }
        });
        Ok(true)
    }
    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.observed
            .remote_requests_started
            .fetch_add(1, Ordering::SeqCst);
        if self.observed.remote_paused.swap(false, Ordering::SeqCst) {
            self.observed.remote_resume.notified().await;
        }
        if self.observed.reject_remote.load(Ordering::SeqCst) {
            return Ok(false);
        }
        let Ok(listener) = TcpListener::bind((address, u16::try_from(*port).unwrap())).await else {
            return Ok(false);
        };
        let stop = self.stop.child_token();
        let done = CancellationToken::new();
        self.listeners
            .insert((address.into(), *port), (stop.clone(), done.clone()));
        let handle = session.handle();
        let host = address.to_owned();
        let port = *port;
        let tasks = self.tasks.clone();
        let completed = done.clone();
        self.tasks.spawn(async move {
            loop {
                let (mut stream, origin) = tokio::select! { biased; () = stop.cancelled() => break, result = listener.accept() => result.unwrap() };
                let handle = handle.clone(); let host = host.clone(); let stop = stop.clone();
                tasks.spawn(async move {
                    tokio::select! {
                        () = stop.cancelled() => {},
                        _ = async {
                            if let Ok(channel) = handle.channel_open_forwarded_tcpip(host, port, origin.ip().to_string(), u32::from(origin.port())).await {
                                let _ = tokio::io::copy_bidirectional(&mut stream, &mut channel.into_stream()).await;
                            }
                        } => {}
                    }
                });
            }
            drop(listener);
            completed.cancel();
        });
        Ok(true)
    }
    async fn cancel_tcpip_forward(
        &mut self,
        address: &str,
        port: u32,
        _: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.observed.cancel_requests.fetch_add(1, Ordering::SeqCst);
        if self.observed.cancel_paused.swap(false, Ordering::SeqCst) {
            self.observed.cancel_resume.notified().await;
        }
        if self.observed.cancel_denied.load(Ordering::SeqCst) {
            return Ok(false);
        }
        if let Some((stop, done)) = self.listeners.remove(&(address.into(), port)) {
            stop.cancel();
            done.cancelled().await;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
pub struct Fixture {
    pub directory: tempfile::TempDir,
    pub vault: VaultManager,
    pub host: Host,
    pub observed: Arc<Observed>,
    pub master: SecretText,
    pub broker: Arc<TrustBroker>,
    stop: CancellationToken,
    tasks: TaskTracker,
}
impl Fixture {
    pub async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let observed = Arc::new(Observed::default());
        let stop = CancellationToken::new();
        let tasks = TaskTracker::new();
        let serve_observed = observed.clone();
        let serve_stop = stop.clone();
        let serve_tasks = tasks.clone();
        let config = Arc::new(server::Config {
            keys: vec![PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()],
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            ..server::Config::default()
        });
        tasks.spawn(async move {
            loop {
                let stream = tokio::select! { biased; () = serve_stop.cancelled() => break, result = listener.accept() => result.unwrap().0 };
                serve_observed.accepts.fetch_add(1, Ordering::SeqCst);
                let peer = Peer { observed: serve_observed.clone(), tasks: serve_tasks.clone(), stop: serve_stop.child_token(), shells: HashSet::new(), listeners: HashMap::new() };
                let config = config.clone(); let observed = serve_observed.clone(); let stop = serve_stop.clone();
                serve_tasks.spawn(async move {
                    let session = server::run_stream(config, stream, peer).await.unwrap();
                    observed.handles.lock().unwrap().push(session.handle());
                    tokio::select! { _ = session => {}, () = stop.cancelled() => {} }
                });
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let vault = VaultManager::new(directory.path().join("vault.cav"));
        let master = SecretText::new(Uuid::new_v4().to_string());
        vault
            .initialize(
                master.clone(),
                KdfParameters {
                    memory_kib: 19_456,
                    iterations: 2,
                    parallelism: 1,
                },
            )
            .await
            .unwrap();
        let credential_id = vault
            .add_password_credential("Forwarding test".into(), Uuid::new_v4().to_string())
            .await
            .unwrap();
        let host = Host {
            id: Uuid::new_v4(),
            name: "Forwarding host".into(),
            address: "127.0.0.1".into(),
            port,
            username: "test".into(),
            credential_id,
            group_id: None,
            tags: vec![],
            favorite: false,
            revision: 0,
            working_directory: None,
            startup_command: None,
            encoding: "utf-8".into(),
            theme: "graphite".into(),
            agent_forwarding: false,
            production: false,
        };
        vault
            .save_host(host.clone(), vault.revision().unwrap())
            .await
            .unwrap();
        let (broker, mut challenges) = TrustBroker::new();
        let decisions = broker.clone();
        let decision_stop = stop.clone();
        tasks.spawn(async move {
            loop { tokio::select! {
                () = decision_stop.cancelled() => break,
                challenge = challenges.recv() => { let Some(challenge) = challenge else { break; }; assert!(!challenge.key_changed); decisions.respond(challenge.challenge_id, true).unwrap(); }
            } }
        });
        Self {
            directory,
            vault,
            host,
            observed,
            master,
            broker,
            stop,
            tasks,
        }
    }
    pub async fn connect(
        &self,
        owner: Uuid,
    ) -> (
        SshShell,
        ConnectionLease<PasswordConnection>,
        ConnectionSnapshot,
    ) {
        let (_, shell, lease) = self
            .vault
            .connect_password_host_with_lease(
                self.host.id,
                owner,
                self.broker.clone(),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        (
            shell,
            lease,
            self.vault.connection_for_owner(owner).unwrap(),
        )
    }
    pub async fn save(&self, forward: TunnelForward, auto_start: bool) -> TunnelConfig {
        let config = TunnelConfig {
            id: Uuid::new_v4(),
            host_id: self.host.id,
            auto_start,
            forward,
        };
        self.vault
            .save_tunnel(config.clone(), self.vault.revision().unwrap())
            .await
            .unwrap();
        config
    }
    pub async fn state(
        &self,
        id: Uuid,
        state: ConnectionState,
        generation: u64,
    ) -> ConnectionSnapshot {
        timeout(Duration::from_secs(12), async {
            loop {
                if let Some(snapshot) =
                    self.vault
                        .connection_snapshots()
                        .unwrap()
                        .into_iter()
                        .find(|snapshot| {
                            snapshot.connection_id == id
                                && snapshot.state == state
                                && snapshot.generation >= generation
                        })
                {
                    return snapshot;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }
    pub async fn lose_transport(&self) {
        let handles = self.observed.handles.lock().unwrap().clone();
        for handle in handles {
            let _ = handle
                .disconnect(
                    russh::Disconnect::ByApplication,
                    "test loss".into(),
                    "en".into(),
                )
                .await;
        }
    }
    pub async fn echo_server(&self) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = self.stop.clone();
        let tasks = self.tasks.clone();
        self.tasks.spawn(async move {
            loop {
                let (mut stream, _) = tokio::select! { biased; () = stop.cancelled() => break, result = listener.accept() => result.unwrap() };
                let stop = stop.clone(); tasks.spawn(async move {
                    let (mut read, mut write) = stream.split();
                    tokio::select! { () = stop.cancelled() => {}, _ = async { let _ = tokio::io::copy(&mut read, &mut write).await; let _ = write.shutdown().await; } => {} }
                });
            }
        });
        port
    }
    pub async fn close(self) {
        self.vault.disconnect_ssh().await;
        self.stop.cancel();
        self.tasks.close();
        timeout(Duration::from_secs(5), self.tasks.wait())
            .await
            .unwrap();
    }
}
pub async fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
pub fn local(port: u16, destination: u16) -> TunnelForward {
    TunnelForward::Local {
        bind_host: "127.0.0.1".into(),
        bind_port: port,
        destination_host: "127.0.0.1".into(),
        destination_port: destination,
    }
}
pub fn remote(port: u16, destination: u16) -> TunnelForward {
    TunnelForward::Remote {
        remote_bind_host: "127.0.0.1".into(),
        remote_bind_port: port,
        local_destination_host: "127.0.0.1".into(),
        local_destination_port: destination,
    }
}
pub fn socks(port: u16) -> TunnelForward {
    TunnelForward::Socks5 {
        bind_host: "127.0.0.1".into(),
        bind_port: port,
    }
}
pub async fn echo(port: u16, payload: &[u8]) {
    timeout(Duration::from_secs(5), async {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream.write_all(payload).await.unwrap();
        stream.shutdown().await.unwrap();
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply).await.unwrap();
        assert_eq!(reply, payload);
    })
    .await
    .unwrap();
}
