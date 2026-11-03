//! Connection lifecycle exercised with a fault-injectable in-process SSH peer.
#![allow(clippy::pedantic)]
use capstan_core::{sftp::ExactU64, VaultManager};
use capstan_ssh::{
    connection_state::{
        validate_successor, ConnectionErrorCode, ConnectionSnapshot, ConnectionState,
    },
    ConnectionLease, PasswordConnection, SshShell, TrustBroker,
};
use capstan_vault::{Host, KdfParameters, SecretText};
use russh::{
    keys::{Algorithm, PrivateKey},
    server::{self, Session},
    Channel, ChannelId,
};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{net::TcpListener, time::timeout};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use uuid::Uuid;

#[derive(Default)]
struct Observed {
    accepts: AtomicUsize,
    auth: AtomicUsize,
    reject: AtomicBool,
    abort_auth: AtomicBool,
    drop_handshake: AtomicBool,
    stall_auth: AtomicBool,
    auth_started: tokio::sync::Notify,
    auth_release: tokio::sync::Notify,
    inputs: Mutex<Vec<Vec<u8>>>,
    handles: Mutex<Vec<server::Handle>>,
}
#[derive(Clone)]
struct Peer(Arc<Observed>);
impl server::Handler for Peer {
    type Error = russh::Error;
    async fn auth_password(&mut self, _: &str, _: &str) -> Result<server::Auth, Self::Error> {
        self.0.auth.fetch_add(1, Ordering::SeqCst);
        if self.0.abort_auth.load(Ordering::SeqCst) {
            return Err(russh::Error::Disconnect);
        }
        if self.0.stall_auth.load(Ordering::SeqCst) {
            self.0.auth_started.notify_one();
            self.0.auth_release.notified().await;
        }
        Ok(if self.0.reject.load(Ordering::SeqCst) {
            server::Auth::reject()
        } else {
            server::Auth::Accept
        })
    }
    async fn channel_open_session(
        &mut self,
        _: Channel<server::Msg>,
        _: &mut Session,
    ) -> Result<bool, Self::Error> {
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
        session.channel_success(channel)
    }
    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.0.inputs.lock().unwrap().push(data.to_vec());
        session.data(channel, data.to_vec())
    }
}
struct Fixture {
    _directory: tempfile::TempDir,
    vault: VaultManager,
    master: SecretText,
    host: Host,
    observed: Arc<Observed>,
    config: Arc<Mutex<Arc<server::Config>>>,
    broker: Arc<TrustBroker>,
    stop: CancellationToken,
    tasks: TaskTracker,
}
fn configuration() -> Arc<server::Config> {
    Arc::new(server::Config {
        keys: vec![PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()],
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..server::Config::default()
    })
}
impl Fixture {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let config = Arc::new(Mutex::new(configuration()));
        let observed = Arc::new(Observed::default());
        let stop = CancellationToken::new();
        let tasks = TaskTracker::new();
        let peer_config = config.clone();
        let peer_observed = observed.clone();
        let peer_stop = stop.clone();
        let sessions = tasks.clone();
        tasks.spawn(async move {
            loop {
                let socket = tokio::select! {
                    biased;
                    () = peer_stop.cancelled() => break,
                    result = listener.accept() => result.unwrap().0,
                };
                peer_observed.accepts.fetch_add(1, Ordering::SeqCst);
                if peer_observed.drop_handshake.load(Ordering::SeqCst) {
                    drop(socket);
                    continue;
                }
                let config = peer_config.lock().unwrap().clone();
                let observed = peer_observed.clone();
                let cancel = peer_stop.clone();
                sessions.spawn(async move {
                    let Ok(session) =
                        server::run_stream(config, socket, Peer(observed.clone())).await
                    else {
                        return;
                    };
                    observed.handles.lock().unwrap().push(session.handle());
                    tokio::select! { _ = session => {}, () = cancel.cancelled() => {} }
                });
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let vault = VaultManager::new(directory.path().join("vault.capstan"));
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
        let credential = vault
            .add_password_credential("Test credential".into(), Uuid::new_v4().to_string())
            .await
            .unwrap();
        let host = Host {
            id: Uuid::new_v4(),
            name: "Test host".into(),
            address: "127.0.0.1".into(),
            port: address.port(),
            username: "test".into(),
            credential_id: credential,
            group_id: None,
            tags: vec![],
            favorite: false,
            revision: 0,
            working_directory: None,
            startup_command: Some("configured-startup".into()),
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
        let decisions_stop = stop.clone();
        tasks.spawn(async move {
            loop {
                tokio::select! {
                    () = decisions_stop.cancelled() => break,
                    challenge = challenges.recv() => {
                        let Some(challenge) = challenge else { break; };
                        if !challenge.key_changed { decisions.respond(challenge.challenge_id, true).unwrap(); }
                    }
                }
            }
        });
        Self {
            _directory: directory,
            vault,
            master,
            host,
            observed,
            config,
            broker,
            stop,
            tasks,
        }
    }
    async fn connect(
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
        let snapshot = self.vault.connection_for_owner(owner).unwrap();
        (shell, lease, snapshot)
    }
    async fn drop_transport(&self) {
        let handles = self.observed.handles.lock().unwrap().clone();
        for handle in handles {
            let _ = handle
                .disconnect(
                    russh::Disconnect::ByApplication,
                    "test network loss".into(),
                    "en".into(),
                )
                .await;
        }
    }
    async fn state(
        &self,
        id: Uuid,
        state: ConnectionState,
        minimum_generation: u64,
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
                                && snapshot.generation >= minimum_generation
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
    async fn close(self) {
        self.observed.auth_release.notify_waiters();
        self.vault.disconnect_ssh().await;
        self.stop.cancel();
        self.tasks.close();
        self.tasks.wait().await;
    }
}

#[tokio::test]
async fn restoring_a_vault_backup_reenables_ssh_connections() {
    let fixture = Fixture::new().await;
    let backup_revision = fixture.vault.revision().unwrap();
    fixture
        .vault
        .save_host(fixture.host.clone(), backup_revision)
        .await
        .unwrap();
    fixture.vault.lock().await.unwrap();
    std::fs::write(fixture.vault.path(), b"corrupt main vault").unwrap();

    let candidates = fixture
        .vault
        .list_recovery_candidates(fixture.master.clone())
        .await
        .unwrap();
    let backup = candidates
        .iter()
        .find(|candidate| candidate.revision == backup_revision)
        .unwrap();
    fixture
        .vault
        .restore_recovery_candidate(backup.token, fixture.master.clone())
        .await
        .unwrap();

    assert!(fixture.vault.status().unlocked);
    assert!(fixture
        .vault
        .hosts()
        .unwrap()
        .iter()
        .any(|host| host.id == fixture.host.id));
    let owner = Uuid::new_v4();
    let (shell, _lease, connection) = fixture.connect(owner).await;
    assert_eq!(connection.state, ConnectionState::Online);
    shell.control.close().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn canceling_one_start_does_not_cancel_another_owner_of_the_same_policy() {
    let fixture = Fixture::new().await;
    let (_, _, first) = fixture.connect(Uuid::new_v4()).await;
    fixture
        .vault
        .disconnect_connection(first.connection_id, first.generation)
        .await
        .unwrap();
    fixture.observed.stall_auth.store(true, Ordering::SeqCst);
    let canceled_owner = Uuid::new_v4();
    let cancel = CancellationToken::new();
    let first_vault = fixture.vault.clone();
    let first_broker = fixture.broker.clone();
    let first_cancel = cancel.clone();
    let host = fixture.host.id;
    let canceled = tokio::spawn(async move {
        first_vault
            .connect_password_host(host, canceled_owner, first_broker, first_cancel)
            .await
    });
    timeout(
        Duration::from_secs(3),
        fixture.observed.auth_started.notified(),
    )
    .await
    .unwrap();
    let surviving_owner = Uuid::new_v4();
    let second_vault = fixture.vault.clone();
    let second_broker = fixture.broker.clone();
    let survivor = tokio::spawn(async move {
        second_vault
            .connect_password_host(
                host,
                surviving_owner,
                second_broker,
                CancellationToken::new(),
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let cancel_vault = fixture.vault.clone();
    let mut cancellation = tokio::spawn(async move {
        cancel_vault.cancel_connection_attempt(canceled_owner).await;
    });
    fixture.observed.stall_auth.store(false, Ordering::SeqCst);
    fixture.observed.auth_release.notify_waiters();
    assert_eq!(canceled.await.unwrap().err().unwrap().code(), "canceled");
    let (_, shell) = timeout(Duration::from_secs(3), survivor)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let snapshot = fixture.vault.connection_for_owner(surviving_owner).unwrap();
    assert_eq!(snapshot.state, ConnectionState::Online);
    assert_eq!(snapshot.generation, 2);
    shell
        .control
        .write(b"surviving-owner".to_vec())
        .await
        .unwrap();
    let completed_before_survivor_monitor_stopped =
        match timeout(Duration::from_millis(300), &mut cancellation).await {
            Ok(result) => {
                result.unwrap();
                true
            }
            Err(_) => false,
        };
    if !completed_before_survivor_monitor_stopped {
        fixture.vault.release_connection_owner(surviving_owner);
        timeout(Duration::from_secs(3), &mut cancellation)
            .await
            .unwrap()
            .unwrap();
    }
    assert!(completed_before_survivor_monitor_stopped);
    fixture.close().await;
}

#[tokio::test]
async fn locking_during_authentication_cancels_and_joins_the_retry() {
    let fixture = Fixture::new().await;
    let (_, _, first) = fixture.connect(Uuid::new_v4()).await;
    fixture.observed.stall_auth.store(true, Ordering::SeqCst);
    fixture.drop_transport().await;
    timeout(
        Duration::from_secs(4),
        fixture.observed.auth_started.notified(),
    )
    .await
    .unwrap();
    timeout(Duration::from_secs(3), fixture.vault.lock())
        .await
        .unwrap()
        .unwrap();
    let failed = fixture
        .state(first.connection_id, ConnectionState::Failed, 2)
        .await;
    assert_eq!(failed.error_code, Some(ConnectionErrorCode::VaultLocked));
    let accepts = fixture.observed.accepts.load(Ordering::SeqCst);
    fixture.vault.probe_connections();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(fixture.observed.accepts.load(Ordering::SeqCst), accepts);
    fixture.close().await;
}

#[tokio::test]
async fn disconnect_stops_all_logical_lifetimes_that_rebind_to_one_transport_after_unlock() {
    let fixture = Fixture::new().await;
    let (_, old_lease, old) = fixture.connect(Uuid::new_v4()).await;
    fixture.vault.lock().await.unwrap();
    fixture.vault.unlock(fixture.master.clone()).await.unwrap();
    let (_, new_lease, new) = fixture.connect(Uuid::new_v4()).await;
    assert_ne!(old.connection_id, new.connection_id);
    assert_ne!(
        old_lease.connection().connection_generation(),
        new_lease.connection().connection_generation()
    );
    old_lease.connection().disconnect().await;
    let rebound = fixture
        .state(old.connection_id, ConnectionState::Online, 2)
        .await;
    assert_eq!(fixture.observed.accepts.load(Ordering::SeqCst), 2);
    fixture
        .vault
        .disconnect_connection(rebound.connection_id, rebound.generation)
        .await
        .unwrap();
    let other = fixture
        .state(
            new.connection_id,
            ConnectionState::Disconnected,
            new.generation,
        )
        .await;
    assert_eq!(other.error_code, Some(ConnectionErrorCode::Canceled));
    assert!(new_lease.connection().is_closed());
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(fixture.observed.accepts.load(Ordering::SeqCst), 2);
    fixture.close().await;
}

#[tokio::test]
async fn reconnect_is_generation_isolated_shares_transport_and_never_replays_input_or_startup() {
    let fixture = Fixture::new().await;
    let mut events = fixture.vault.subscribe_connections();
    let owner = Uuid::new_v4();
    let (old_shell, old_lease, first) = fixture.connect(owner).await;
    let other_owner = Uuid::new_v4();
    let (other_shell, _, other) = fixture.connect(other_owner).await;
    assert_eq!(first, other);
    old_shell
        .control
        .write(b"user-command\r".to_vec())
        .await
        .unwrap();
    timeout(Duration::from_secs(2), async {
        while fixture.observed.inputs.lock().unwrap().len() < 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let inputs = fixture.observed.inputs.lock().unwrap().clone();
    for _ in 0..12 {
        fixture.vault.probe_connections();
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fixture.observed.accepts.load(Ordering::SeqCst), 1);
    fixture.drop_transport().await;
    let next = fixture
        .state(first.connection_id, ConnectionState::Online, 2)
        .await;
    assert_eq!(next.generation, 2);
    assert!(old_lease.connection().is_closed());
    timeout(Duration::from_secs(2), async {
        while *old_shell.exit.borrow() != capstan_ssh::ChannelExit::Failed
            || *other_shell.exit.borrow() != capstan_ssh::ChannelExit::Failed
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (host, new_shell, new_lease) = fixture
        .vault
        .open_reconnected_shell(
            next.connection_id,
            next.generation,
            Uuid::new_v4(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let (_, second_new_shell, _) = fixture
        .vault
        .open_reconnected_shell(
            next.connection_id,
            next.generation,
            Uuid::new_v4(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(host, fixture.host.id);
    assert_ne!(new_shell.connection_id(), old_shell.connection_id());
    assert_eq!(new_shell.connection_id(), second_new_shell.connection_id());
    assert_ne!(
        new_lease.connection().connection_generation(),
        old_lease.connection().connection_generation()
    );
    assert_eq!(fixture.observed.auth.load(Ordering::SeqCst), 2);
    assert_eq!(*fixture.observed.inputs.lock().unwrap(), inputs);
    assert_eq!(
        fixture
            .vault
            .disconnect_connection(next.connection_id, first.generation)
            .await
            .unwrap_err()
            .code(),
        "invalid_input"
    );
    fixture.vault.release_connection_owner(owner);
    fixture.vault.release_connection_owner(other_owner);
    assert_eq!(
        fixture
            .vault
            .connection_snapshots()
            .unwrap()
            .iter()
            .find(|snapshot| snapshot.connection_id == next.connection_id)
            .unwrap(),
        &next
    );
    let mut previous = None;
    let mut authenticating = false;
    while let Ok(event) = events.try_recv() {
        if event.state == ConnectionState::Authenticating {
            authenticating = true;
        }
        if let Some(previous) = &previous {
            validate_successor(previous, &event).unwrap();
        }
        previous = Some(event);
    }
    assert!(authenticating);
    fixture.close().await;
}

#[tokio::test]
async fn fingerprint_changes_block_before_authentication_and_do_not_retry() {
    let fixture = Fixture::new().await;
    let (_shell, _, first) = fixture.connect(Uuid::new_v4()).await;
    let trusted = fixture.vault.trusted_host_keys().unwrap()[0]
        .fingerprint
        .clone();
    *fixture.config.lock().unwrap() = configuration();
    fixture.drop_transport().await;
    let failed = fixture
        .state(first.connection_id, ConnectionState::Failed, 2)
        .await;
    assert_eq!(failed.error_code, Some(ConnectionErrorCode::HostKeyChanged));
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(fixture.observed.accepts.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.observed.auth.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.vault.trusted_host_keys().unwrap()[0].fingerprint,
        trusted
    );
    fixture.close().await;
}

#[tokio::test]
async fn authentication_failure_stops_until_explicit_reconnect() {
    let fixture = Fixture::new().await;
    let (_shell, _, first) = fixture.connect(Uuid::new_v4()).await;
    fixture.observed.reject.store(true, Ordering::SeqCst);
    fixture.drop_transport().await;
    let failed = fixture
        .state(first.connection_id, ConnectionState::Failed, 2)
        .await;
    assert_eq!(
        failed.error_code,
        Some(ConnectionErrorCode::SshAuthentication)
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(fixture.observed.accepts.load(Ordering::SeqCst), 2);
    fixture.observed.reject.store(false, Ordering::SeqCst);
    // A network or wake signal only restarts exhausted network retries.
    fixture.vault.probe_connections();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fixture.observed.accepts.load(Ordering::SeqCst), 2);
    assert_eq!(
        fixture.vault.connection_snapshots().unwrap()[0].generation,
        2
    );
    let canceled_attempt = Uuid::new_v4();
    let connecting = fixture
        .vault
        .reconnect_connection(
            failed.connection_id,
            failed.generation,
            canceled_attempt,
            fixture.broker.clone(),
        )
        .await
        .unwrap();
    assert_eq!(connecting.state, ConnectionState::Connecting);
    assert_eq!(connecting.generation, 3);
    fixture
        .vault
        .cancel_connection_attempt(canceled_attempt)
        .await;
    let canceled = fixture
        .state(first.connection_id, ConnectionState::Failed, 3)
        .await;
    assert_eq!(canceled.error_code, Some(ConnectionErrorCode::Canceled));
    fixture
        .vault
        .reconnect_connection(
            canceled.connection_id,
            canceled.generation,
            Uuid::new_v4(),
            fixture.broker.clone(),
        )
        .await
        .unwrap();
    fixture
        .state(first.connection_id, ConnectionState::Online, 4)
        .await;
    fixture.close().await;
}

#[tokio::test]
async fn a_network_signal_probes_a_blackholed_transport_and_reconnects_without_shell_input() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut fixture = Fixture::new().await;
    let upstream_port = fixture.host.port;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    fixture.host.port = listener.local_addr().unwrap().port();
    fixture
        .vault
        .save_host(fixture.host.clone(), fixture.vault.revision().unwrap())
        .await
        .unwrap();
    let blackhole = Arc::new(AtomicBool::new(false));
    let blackhole_peer = blackhole.clone();
    let stop = fixture.stop.clone();
    let tasks = fixture.tasks.clone();
    fixture.tasks.spawn(async move {
        loop {
            let downstream = tokio::select! {
                () = stop.cancelled() => break,
                socket = listener.accept() => socket.unwrap().0,
            };
            let stop = stop.clone();
            let blackhole = blackhole_peer.clone();
            tasks.spawn(async move {
                let upstream = tokio::net::TcpStream::connect(("127.0.0.1", upstream_port))
                    .await
                    .unwrap();
                let (mut upstream_read, mut upstream_write) = upstream.into_split();
                let (mut downstream_read, mut downstream_write) = downstream.into_split();
                let response = async {
                    let mut bytes = [0_u8; 32 * 1024];
                    loop {
                        let count = upstream_read.read(&mut bytes).await?;
                        if count == 0 {
                            break;
                        }
                        if !blackhole.load(Ordering::SeqCst) {
                            downstream_write.write_all(&bytes[..count]).await?;
                        }
                    }
                    Ok::<_, std::io::Error>(())
                };
                tokio::select! {
                    () = stop.cancelled() => {},
                    _ = tokio::io::copy(&mut downstream_read, &mut upstream_write) => {},
                    _ = response => {},
                }
            });
        }
    });
    let (_, lease, first) = fixture.connect(Uuid::new_v4()).await;
    blackhole.store(true, Ordering::SeqCst);
    fixture.vault.probe_connections();
    fixture
        .state(first.connection_id, ConnectionState::Reconnecting, 1)
        .await;
    assert!(lease.connection().is_closed());
    blackhole.store(false, Ordering::SeqCst);
    fixture.vault.probe_connections();
    fixture
        .state(first.connection_id, ConnectionState::Online, 2)
        .await;
    assert_eq!(fixture.observed.accepts.load(Ordering::SeqCst), 2);
    fixture.close().await;
}

#[tokio::test]
async fn a_network_signal_restarts_an_exhausted_retry_cycle_without_replaying_input() {
    let fixture = Fixture::new().await;
    let (_shell, _, first) = fixture.connect(Uuid::new_v4()).await;
    let inputs = fixture.observed.inputs.lock().unwrap().len();
    fixture
        .observed
        .drop_handshake
        .store(true, Ordering::SeqCst);
    fixture.drop_transport().await;
    // Each signal skips one backoff delay, so eight failed attempts end quickly.
    for generation in 2..=8 {
        fixture
            .state(
                first.connection_id,
                ConnectionState::Reconnecting,
                generation,
            )
            .await;
        fixture.vault.probe_connections();
    }
    let exhausted = fixture
        .state(first.connection_id, ConnectionState::Failed, 9)
        .await;
    assert_eq!(
        exhausted.error_code,
        Some(ConnectionErrorCode::ReconnectExhausted)
    );
    let accepts = fixture.observed.accepts.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(fixture.observed.accepts.load(Ordering::SeqCst), accepts);

    fixture
        .observed
        .drop_handshake
        .store(false, Ordering::SeqCst);
    fixture.vault.probe_connections();
    let online = fixture
        .state(first.connection_id, ConnectionState::Online, 10)
        .await;
    assert_eq!(online.generation, 10);
    assert_eq!(fixture.observed.inputs.lock().unwrap().len(), inputs);
    fixture.close().await;
}

#[tokio::test]
async fn network_failures_retry_but_disconnect_cancels_backoff_and_joins() {
    let fixture = Fixture::new().await;
    let (_shell, _, first) = fixture.connect(Uuid::new_v4()).await;
    fixture
        .observed
        .drop_handshake
        .store(true, Ordering::SeqCst);
    fixture.drop_transport().await;
    let retrying = fixture
        .state(first.connection_id, ConnectionState::Reconnecting, 2)
        .await;
    assert_eq!(
        retrying.error_code,
        Some(ConnectionErrorCode::SshConnection)
    );
    let disconnected = fixture
        .vault
        .disconnect_connection(first.connection_id, retrying.generation)
        .await
        .unwrap();
    assert_eq!(disconnected.state, ConnectionState::Disconnected);
    let count = fixture.observed.accepts.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(2100)).await;
    fixture.vault.probe_connections();
    assert_eq!(fixture.observed.accepts.load(Ordering::SeqCst), count);
    fixture.close().await;
}

#[tokio::test]
async fn a_network_disconnect_during_authentication_retries_instead_of_becoming_auth_failure() {
    let fixture = Fixture::new().await;
    let (_, _, first) = fixture.connect(Uuid::new_v4()).await;
    fixture.observed.abort_auth.store(true, Ordering::SeqCst);
    fixture.drop_transport().await;
    let retrying = fixture
        .state(first.connection_id, ConnectionState::Reconnecting, 2)
        .await;
    assert_eq!(
        retrying.error_code,
        Some(ConnectionErrorCode::SshConnection)
    );
    assert_eq!(fixture.observed.auth.load(Ordering::SeqCst), 2);
    fixture.observed.abort_auth.store(false, Ordering::SeqCst);
    fixture.vault.probe_connections();
    fixture
        .state(first.connection_id, ConnectionState::Online, 3)
        .await;
    fixture.close().await;
}

#[tokio::test]
async fn lock_retains_live_transport_but_blocks_reconnect_and_unlock_does_not_restart_it() {
    let fixture = Fixture::new().await;
    let (shell, lease, first) = fixture.connect(Uuid::new_v4()).await;
    fixture.vault.lock().await.unwrap();
    assert!(!lease.connection().is_closed());
    shell
        .control
        .write(b"existing-session\r".to_vec())
        .await
        .unwrap();
    fixture.vault.probe_connections();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fixture.observed.accepts.load(Ordering::SeqCst), 1);
    fixture.drop_transport().await;
    let failed = fixture
        .state(first.connection_id, ConnectionState::Failed, 1)
        .await;
    assert_eq!(failed.error_code, Some(ConnectionErrorCode::VaultLocked));
    fixture.vault.unlock(fixture.master.clone()).await.unwrap();
    fixture.vault.probe_connections();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(fixture.observed.accepts.load(Ordering::SeqCst), 1);
    fixture
        .vault
        .reconnect_connection(
            failed.connection_id,
            failed.generation,
            Uuid::new_v4(),
            fixture.broker.clone(),
        )
        .await
        .unwrap();
    fixture
        .state(first.connection_id, ConnectionState::Online, 2)
        .await;
    assert!(serde_json::to_string(&ExactU64(u64::MAX))
        .unwrap()
        .contains("18446744073709551615"));
    fixture.close().await;
}
