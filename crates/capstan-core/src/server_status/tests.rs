use super::{frame, *};
use capstan_ssh::{
    connect_password, AttemptBinding, ConnectionPool, HostPublicKey, PoolKey, SshShell,
    TrustBroker, TrustRepository,
};
use russh::{server, Channel, ChannelId, Pty};
use secrecy::SecretString;
use std::{
    collections::HashSet,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::{io::AsyncReadExt, net::TcpListener, sync::mpsc, time::timeout};
use zeroize::Zeroizing;

const STAT: &str = "cpu  192610 17 45812 1506016 5332 127 992 0 3 1\ncpu0 50723 5 17476 365403 1307 0 247 0 1 0\nintr 33125236 18 17";
const MEMORY: &str = "MemTotal:       16069584 kB\nMemFree:          933456 kB\nMemAvailable:   12002360 kB\nBuffers:          482764 kB";
const LOAD: &str = "0.17 0.06 0.01 1/246 10478";
const NETWORK_HEADER: &str = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed";
const LOOPBACK: &str = "    lo: 12345 33 0 0 0 0 0 0 12345 33 0 0 0 0 0 0";
const ETHERNET: &str = "  eth0: 92156 845 0 0 0 0 0 0 47818 618 0 0 0 0 0 0";
const DISK_HEADER: &str = "Filesystem 1024-blocks Used Available Capacity Mounted on";
const ROOT: &str = "/dev/sdb 1055762868 129540228 872518212 13% /";

fn sample_frame() -> String {
    format!("CAPSTAN-STATUS/1\n{STAT}\nCAPSTAN-MEMORY\n{MEMORY}\nCAPSTAN-LOAD\n{LOAD}\nCAPSTAN-NETWORK\n{NETWORK_HEADER}\n{LOOPBACK}\n{ETHERNET}\nCAPSTAN-DISK\n{DISK_HEADER}\n{ROOT}\nnetwork drive 300 100 200 34% /mnt/with spaces\nCAPSTAN-END\n")
}

#[test]
fn parses_linux_samples_and_computes_adjacent_exact_deltas() {
    let first = sample_frame();
    let previous = frame::parse(first.as_bytes()).unwrap();
    let initial = previous.observation(None, 17);
    assert_eq!(initial.sampled_at, ExactU64(17));
    assert_eq!(
        initial.memory.unwrap().available,
        ExactU64(12_002_360 * 1024)
    );
    assert_eq!(initial.disk.unwrap().mount, "/");
    assert_eq!(initial.details.unwrap().disks[1].mount, "/mnt/with spaces");
    assert!(initial.cpu_percent.is_none() && initial.network.is_none());
    let next = first
        .replace("192610", "192635")
        .replace("1506016", "1506091")
        .replace("92156", "92756")
        .replace("47818", "48118")
        .replace("12345", "99999");
    let current = frame::parse(next.as_bytes()).unwrap();
    let observed = current.observation(Some((&previous, Duration::from_secs(3))), 20);
    assert_eq!(observed.cpu_percent, Some(25.));
    assert_eq!(
        observed.network,
        Some(NetworkRate {
            rx_per_sec: ExactU64(200),
            tx_per_sec: ExactU64(100)
        })
    );
    let reset = next.replace("192635", "1").replace("92756", "1");
    let reset = frame::parse(reset.as_bytes())
        .unwrap()
        .observation(Some((&current, Duration::from_secs(3))), 23);
    assert!(reset.cpu_percent.is_none() && reset.network.is_none());
}

#[test]
fn parses_df_source_names_that_contain_capacity_like_columns() {
    let frame = sample_frame().replace(
        ROOT,
        "audit 10 2 8 20% prefix    16356700     0  16356700       0% /tmp/status mount",
    );
    let observation = frame::parse(frame.as_bytes())
        .unwrap()
        .observation(None, 17);
    let root = observation.details.unwrap().disks.remove(0);

    assert_eq!(root.mount, "/tmp/status mount");
    assert_eq!(root.total, ExactU64(16_356_700 * 1024));
    assert_eq!(root.used, ExactU64(0));
}

#[test]
fn rejects_truncated_frames_and_all_configured_limits() {
    let valid = sample_frame();
    for malformed in [
        valid.replace("CAPSTAN-END\n", ""),
        valid.replace(MEMORY, "MemAvailable: 1 kB"),
        valid.replace(LOAD, "NaN 1 2"),
        valid.replace(ETHERNET, "eth0: 10 0"),
        valid.replace(ROOT, "missing disk fields"),
        valid.replace(ROOT, "audit 10 2 8 20% /prefix 300 100 200 34% /"),
        valid.replace("192610", "bogus"),
    ] {
        assert_eq!(frame::parse(malformed.as_bytes()), Err(StatusError::Parse));
    }
    let cpu = (0..frame::MAX_CPU_LINES).fold(String::from("cpu 0 0 0 0\n"), |mut stat, index| {
        use std::fmt::Write;
        writeln!(stat, "cpu{index} 0 0 0 0").unwrap();
        stat
    });
    let net = (0..frame::MAX_INTERFACES)
        .fold(String::new(), |mut net, index| {
            use std::fmt::Write;
            writeln!(net, "{index}{ETHERNET}").unwrap();
            net
        })
        .replace("  eth", "eth");
    for oversized in [
        vec![b'x'; frame::MAX_FRAME_BYTES + 1],
        valid.replace(STAT, cpu.trim_end()).into_bytes(),
        valid.replace(LOOPBACK, net.trim_end()).into_bytes(),
        valid
            .replace(ROOT, &format!("{ROOT}\n").repeat(frame::MAX_MOUNTS))
            .into_bytes(),
    ] {
        assert_eq!(frame::parse(&oversized), Err(StatusError::Limit));
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
        Err(capstan_ssh::SshError::InvalidInput)
    }
}

fn attempt(address: &str, port: u16) -> AttemptBinding {
    AttemptBinding {
        attempt_id: Uuid::new_v4(),
        host_id: Uuid::new_v4(),
        host_revision: 1,
        vault_epoch: 1,
        target: address.into(),
        port,
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Linux,
    NonLinux,
    Reject,
    BadFrame,
    SlowSample,
}
#[derive(Debug, PartialEq, Eq)]
enum RemoteEvent {
    Open,
    Tick,
    StopRequested,
    Close,
}

struct Server {
    channels: HashMap<ChannelId, Channel<server::Msg>>,
    sessions: HashSet<ChannelId>,
    sftp_channels: HashSet<ChannelId>,
    sftp_input: HashMap<ChannelId, Vec<u8>>,
    max_sessions: usize,
    mode: Arc<Mutex<Mode>>,
    attempts: Arc<AtomicUsize>,
    events: mpsc::UnboundedSender<RemoteEvent>,
    slow_exec: Option<ChannelId>,
    slow_sample_exit: CancellationToken,
}

impl server::Handler for Server {
    type Error = russh::Error;
    async fn auth_password(&mut self, _: &str, _: &str) -> Result<server::Auth, Self::Error> {
        Ok(server::Auth::Accept)
    }
    async fn channel_open_session(
        &mut self,
        channel: Channel<server::Msg>,
        _: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        if !self.channels.is_empty() {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            if matches!(*self.mode.lock().unwrap(), Mode::Reject) {
                return Ok(false);
            }
        }
        if self.sessions.len() >= self.max_sessions {
            return Ok(false);
        }
        self.sessions.insert(channel.id());
        self.channels.insert(channel.id(), channel);
        Ok(true)
    }
    #[allow(clippy::too_many_arguments)]
    async fn pty_request(
        &mut self,
        id: ChannelId,
        _: &str,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
        _: &[(Pty, u32)],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(id)
    }
    async fn shell_request(
        &mut self,
        id: ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(id)
    }
    async fn channel_close(
        &mut self,
        id: ChannelId,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.channels.remove(&id);
        if self.slow_exec != Some(id) || self.slow_sample_exit.is_cancelled() {
            self.sessions.remove(&id);
        }
        self.sftp_channels.remove(&id);
        self.sftp_input.remove(&id);
        Ok(())
    }
    async fn data(
        &mut self,
        id: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if self.sftp_channels.contains(&id) {
            let initialized = {
                let input = self.sftp_input.entry(id).or_default();
                input.extend_from_slice(data);
                input.len() >= 9 && input[..4] == [0, 0, 0, 5] && input[4] == 1
            };
            if initialized {
                self.sftp_input.remove(&id);
                session.data(id, vec![0, 0, 0, 5, 2, 0, 0, 0, 3])?;
            }
        } else if self.channels.contains_key(&id) {
            session.data(id, data.to_vec())?;
        }
        Ok(())
    }
    async fn subsystem_request(
        &mut self,
        id: ChannelId,
        name: &str,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if name == "sftp" {
            self.sftp_channels.insert(id);
            session.channel_success(id)
        } else {
            session.channel_failure(id)
        }
    }
    async fn exec_request(
        &mut self,
        id: ChannelId,
        command: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        assert_eq!(command, b"sh");
        session.channel_success(id)?;
        let mut channel = self.channels.remove(&id).unwrap();
        let events = self.events.clone();
        let mode = *self.mode.lock().unwrap();
        let slow_sample_exit = self.slow_sample_exit.clone();
        if matches!(mode, Mode::SlowSample) {
            self.slow_exec = Some(id);
        }
        let handle = session.handle();
        tokio::spawn(async move {
            use russh::ChannelMsg::{Close, Data, Eof};
            let _ = events.send(RemoteEvent::Open);
            let mut input = Vec::new();
            while input.len() < frame::SCRIPT.len() {
                match channel.wait().await {
                    Some(Data { data }) => input.extend_from_slice(&data),
                    Some(Close | Eof) | None => {
                        let _ = channel.close().await;
                        return;
                    }
                    _ => {}
                }
            }
            assert_eq!(input, frame::SCRIPT.as_bytes());
            if matches!(mode, Mode::NonLinux) {
                let _ = channel.data(&b"FreeBSD\n"[..]).await;
                let _ = handle.exit_status_request(id, 0).await;
                let _ = channel.eof().await;
                let _ = channel.close().await;
                return;
            }
            if channel.data(&b"Linux\n"[..]).await.is_err() {
                return;
            }
            while let Some(message) = channel.wait().await {
                match message {
                    Data { data } => {
                        assert_eq!(&data[..], b"sample\n");
                        let _ = events.send(RemoteEvent::Tick);
                        if matches!(mode, Mode::SlowSample) {
                            continue;
                        }
                        let output = if matches!(mode, Mode::BadFrame) {
                            "bad frame\n".into()
                        } else {
                            sample_frame()
                        };
                        if channel.data(output.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                    Close | Eof => break,
                    _ => {}
                }
            }
            if matches!(mode, Mode::SlowSample) {
                let _ = events.send(RemoteEvent::StopRequested);
                slow_sample_exit.cancelled().await;
            }
            let _ = events.send(RemoteEvent::Close);
            let _ = channel.close().await;
        });
        Ok(())
    }
}

struct Fixture {
    service: StatusService,
    pool: ConnectionPool<PasswordConnection>,
    lease: ConnectionLease<PasswordConnection>,
    shell: SshShell,
    attempts: Arc<AtomicUsize>,
    mode: Arc<Mutex<Mode>>,
    events: mpsc::UnboundedReceiver<RemoteEvent>,
    slow_sample_exit: CancellationToken,
}

async fn fixture(mode: Mode) -> Fixture {
    fixture_with_max_sessions(mode, usize::MAX).await
}

async fn fixture_with_max_sessions(mode: Mode, max_sessions: usize) -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let key =
        russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::ssh_key::Algorithm::Ed25519)
            .unwrap();
    let trust = HostPublicKey::from_public_key(key.public_key()).unwrap();
    let config = server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::ZERO,
        ..server::Config::default()
    };
    let mode = Arc::new(Mutex::new(mode));
    let attempts = Arc::new(AtomicUsize::new(0));
    let (sender, events) = mpsc::unbounded_channel();
    let slow_sample_exit = CancellationToken::new();
    let handler = Server {
        channels: HashMap::new(),
        sessions: HashSet::new(),
        sftp_channels: HashSet::new(),
        sftp_input: HashMap::new(),
        max_sessions,
        mode: mode.clone(),
        attempts: attempts.clone(),
        events: sender,
        slow_exec: None,
        slow_sample_exit: slow_sample_exit.clone(),
    };
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let _ = server::run_stream(Arc::new(config), stream, handler)
            .await
            .unwrap()
            .await;
    });
    let address = "127.0.0.1";
    let policy = PoolKey {
        target: address.into(),
        port,
        username: "fixture".into(),
        credential_id: Uuid::new_v4(),
        credential_revision: 1,
        trust_fingerprint: trust.fingerprint.clone(),
        vault_epoch: 1,
        agent_identity: None,
        agent_forwarding: false,
        proxy_path: String::new(),
    };
    let pool = ConnectionPool::new(Duration::from_secs(30));
    let lease = pool
        .acquire(policy, CancellationToken::new(), move |cancel| async move {
            let (broker, _challenges) = TrustBroker::new();
            connect_password(
                address.into(),
                port,
                "fixture".into(),
                SecretString::from("test"),
                attempt(address, port),
                Arc::new(Trust(trust)),
                broker,
                cancel,
            )
            .await
        })
        .await
        .unwrap();
    let shell = lease.open_shell(80, 24).await.unwrap();
    Fixture {
        service: StatusService::default(),
        pool,
        lease,
        shell,
        attempts,
        mode,
        events,
        slow_sample_exit,
    }
}

impl Fixture {
    fn start(&self) -> Arc<Collector> {
        let id = self.lease.connection().connection_id();
        self.service.start(&self.lease);
        self.service.start(&self.lease);
        self.service.target.send_replace(Some(id));
        self.service.selection.lock().unwrap().collectors[&id].clone()
    }

    async fn event(&mut self, event: RemoteEvent) {
        assert_eq!(
            timeout(Duration::from_secs(8), self.events.recv())
                .await
                .unwrap(),
            Some(event)
        );
    }

    async fn shell_echo(&mut self) {
        self.shell
            .control
            .write(b"still open".to_vec())
            .await
            .unwrap();
        let mut reply = [0; 10];
        timeout(
            Duration::from_secs(5),
            self.shell.reader.read_exact(&mut reply),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(&reply, b"still open");
    }
}

async fn wait_state(
    observation: &mut watch::Receiver<StatusObservation>,
    state: ServerStatusState,
) -> StatusObservation {
    timeout(
        Duration::from_secs(8),
        observation.wait_for(|status| status.state == state),
    )
    .await
    .unwrap()
    .unwrap()
    .clone()
}

#[tokio::test]
async fn shell_and_sftp_open_by_yielding_the_status_session_slot() {
    let mut fixture = fixture_with_max_sessions(Mode::Linux, 2).await;
    let collector = fixture.start();
    let mut observation = collector.observation.subscribe();
    fixture.event(RemoteEvent::Open).await;
    fixture.event(RemoteEvent::Tick).await;
    wait_state(&mut observation, ServerStatusState::Running).await;

    let second_shell = timeout(Duration::from_secs(5), fixture.lease.open_shell(80, 24))
        .await
        .unwrap()
        .unwrap();
    fixture.event(RemoteEvent::Close).await;
    fixture.shell_echo().await;
    second_shell.control.close().await.unwrap();
    fixture.event(RemoteEvent::Open).await;
    fixture.event(RemoteEvent::Tick).await;
    wait_state(&mut observation, ServerStatusState::Running).await;
    fixture.service.target.send_replace(None);
    wait_state(&mut observation, ServerStatusState::Paused).await;

    let cancel = CancellationToken::new();
    let sftp = timeout(Duration::from_secs(8), fixture.lease.open_sftp(&cancel))
        .await
        .unwrap()
        .unwrap();
    fixture.event(RemoteEvent::Close).await;
    fixture.shell_echo().await;
    sftp.close().await;
    fixture.shell_echo().await;
}

#[tokio::test]
async fn sftp_waits_for_the_remote_slot_while_a_sample_is_in_flight() {
    let mut fixture = fixture_with_max_sessions(Mode::SlowSample, 2).await;
    fixture.start();
    fixture.event(RemoteEvent::Open).await;
    fixture.event(RemoteEvent::Tick).await;

    let lease = fixture.lease.clone();
    let sftp = tokio::spawn(async move { lease.open_sftp(&CancellationToken::new()).await });
    fixture.event(RemoteEvent::StopRequested).await;
    fixture.shell_echo().await;
    assert!(!sftp.is_finished());
    fixture.service.target.send_replace(None);
    fixture.slow_sample_exit.cancel();
    fixture.event(RemoteEvent::Close).await;

    let sftp = timeout(Duration::from_secs(5), sftp)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    fixture.shell_echo().await;
    sftp.close().await;
}

#[tokio::test]
async fn opening_a_shell_with_a_free_slot_keeps_the_status_channel() {
    let mut fixture = fixture_with_max_sessions(Mode::Linux, 3).await;
    fixture.start();
    fixture.event(RemoteEvent::Open).await;
    fixture.event(RemoteEvent::Tick).await;

    let second_shell = fixture.lease.open_shell(80, 24).await.unwrap();
    fixture.event(RemoteEvent::Tick).await;
    fixture.shell_echo().await;
    second_shell.control.close().await.unwrap();
}

#[tokio::test]
async fn collector_shares_the_connection_pauses_and_stops_at_last_lease() {
    let mut fixture = fixture(Mode::Linux).await;
    let collector = fixture.start();
    let mut observation = collector.observation.subscribe();
    fixture.event(RemoteEvent::Open).await;
    fixture.event(RemoteEvent::Tick).await;
    let first = wait_state(&mut observation, ServerStatusState::Running).await;
    assert!(first.cpu_percent.is_none());
    fixture.service.target.send_replace(None);
    wait_state(&mut observation, ServerStatusState::Paused).await;
    assert!(timeout(Duration::from_millis(3200), fixture.events.recv())
        .await
        .is_err());
    fixture
        .service
        .target
        .send_replace(Some(fixture.lease.connection().connection_id()));
    fixture.event(RemoteEvent::Tick).await;
    fixture.shell_echo().await;
    assert_eq!(fixture.attempts.load(Ordering::SeqCst), 1);
    fixture.shell.control.close().await.unwrap();
    assert!(!collector.activity.borrow().is_cancelled());
    let Fixture {
        lease,
        pool,
        mut events,
        ..
    } = fixture;
    drop(lease);
    assert!(collector.activity.borrow().is_cancelled());
    assert_eq!(
        timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap(),
        Some(RemoteEvent::Close)
    );
    assert_eq!(pool.entry_count().await, 1);
}

#[tokio::test]
async fn non_linux_is_terminal_for_the_collector_but_shell_keeps_working() {
    let mut fixture = fixture(Mode::NonLinux).await;
    let collector = fixture.start();
    let status = wait_state(
        &mut collector.observation.subscribe(),
        ServerStatusState::Unsupported,
    )
    .await;
    assert_eq!(status.error_code, Some(StatusError::Unsupported));
    fixture.event(RemoteEvent::Open).await;
    fixture.shell_echo().await;
    assert_eq!(fixture.attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn max_sessions_and_parse_errors_are_isolated_with_one_automatic_reopen() {
    let mut fixture = fixture(Mode::Reject).await;
    let collector = fixture.start();
    let mut observation = collector.observation.subscribe();
    let status = wait_state(&mut observation, ServerStatusState::Unavailable).await;
    assert_eq!(status.error_code, Some(StatusError::Rejected));
    fixture.shell_echo().await;
    tokio::time::sleep(Duration::from_millis(3400)).await;
    assert_eq!(fixture.attempts.load(Ordering::SeqCst), 2);
    *fixture.mode.lock().unwrap() = Mode::BadFrame;
    collector.retry.notify_one();
    fixture.event(RemoteEvent::Open).await;
    fixture.event(RemoteEvent::Tick).await;
    fixture.event(RemoteEvent::Close).await;
    let status = wait_state(&mut observation, ServerStatusState::Unavailable).await;
    assert_eq!(status.error_code, Some(StatusError::Parse));
    assert!(timeout(Duration::from_millis(3200), fixture.events.recv())
        .await
        .is_err());
    fixture.shell_echo().await;
    *fixture.mode.lock().unwrap() = Mode::Linux;
    fixture.shell.control.close().await.unwrap();
    let Fixture {
        service,
        pool,
        lease,
        attempts,
        mode,
        events,
        slow_sample_exit,
        shell: _,
    } = fixture;
    let policy = lease.policy().clone();
    let transport_id = lease.connection().connection_id();
    drop(lease);
    assert!(collector.activity.borrow().is_cancelled());
    let lease = pool
        .acquire(policy, CancellationToken::new(), |_| async {
            Err(capstan_ssh::SshError::Connection)
        })
        .await
        .unwrap();
    assert_eq!(lease.connection().connection_id(), transport_id);
    let shell = lease.open_shell(80, 24).await.unwrap();
    let mut fixture = Fixture {
        service,
        pool,
        lease,
        shell,
        attempts,
        mode,
        events,
        slow_sample_exit,
    };
    assert!(Arc::ptr_eq(&collector, &fixture.start()));
    assert_eq!(observation.borrow().error_code, Some(StatusError::Parse));
    assert!(timeout(Duration::from_millis(3200), fixture.events.recv())
        .await
        .is_err());
    collector.retry.notify_one();
    wait_state(&mut observation, ServerStatusState::Running).await;
    assert_eq!(fixture.attempts.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn visible_snapshot_and_retry_follow_the_current_transport_generation() {
    let mut old = fixture(Mode::Linux).await;
    let mut replacement = fixture(Mode::Linux).await;
    let directory = tempfile::tempdir().unwrap();
    let vault = VaultManager::new(directory.path().join("vault.capstan"));
    let id = Uuid::new_v4();
    let entry = Arc::new(crate::ManagedConnection {
        record: Mutex::new(crate::ConnectionRecord {
            snapshot: capstan_ssh::connection_state::ConnectionSnapshot {
                connection_id: id,
                generation: 1,
                state: ConnectionState::Online,
                error_code: None,
            },
            host_id: Uuid::new_v4(),
            host_revision: 1,
            policy: old.lease.policy().clone(),
            transport: old.lease.downgrade_connection(),
            transport_generation: old.lease.connection().connection_generation(),
            owners: HashMap::new(),
            attempt: None,
            operation: CancellationToken::new(),
            monitoring: false,
            broker: None,
        }),
        gate: tokio::sync::Mutex::new(()),
        stop: CancellationToken::new(),
        tasks: tokio_util::task::TaskTracker::new(),
        changed: watch::channel(0).0,
    });
    vault
        .connections
        .entries
        .lock()
        .unwrap()
        .insert(id, entry.clone());
    vault.select_server_status_connection(Some(id));
    vault.set_server_status_window_visible(true);
    assert!(vault.server_status_snapshot().is_none());
    vault.start_server_status(&old.lease);
    old.event(RemoteEvent::Open).await;
    old.event(RemoteEvent::Tick).await;
    let mut updates = vault.subscribe_server_status();
    timeout(Duration::from_secs(5), async {
        while !vault
            .server_status_snapshot()
            .is_some_and(|sample| sample.observation.state == ServerStatusState::Running)
        {
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(
        vault.server_status_snapshot().unwrap().generation,
        ExactU64(1)
    );

    {
        let mut record = entry.record.lock().unwrap();
        record.snapshot.generation = 2;
        record.transport = replacement.lease.downgrade_connection();
        record.transport_generation = replacement.lease.connection().connection_generation();
    }
    vault.refresh_server_status_target();
    assert!(vault.server_status_snapshot().is_none());
    assert!(!vault.retry_server_status(id, ExactU64(1)));
    vault.start_server_status(&replacement.lease);
    replacement.event(RemoteEvent::Open).await;
    replacement.event(RemoteEvent::Tick).await;
    assert_eq!(
        vault.server_status_snapshot().unwrap().generation,
        ExactU64(2)
    );
    let old_id = old.lease.connection().connection_id();
    let old_collector = vault.server_status.selection.lock().unwrap().collectors[&old_id].clone();
    wait_state(
        &mut old_collector.observation.subscribe(),
        ServerStatusState::Paused,
    )
    .await;
}

async fn configured_openssh_lease(target: &str) -> ConnectionLease<PasswordConnection> {
    let (username, host_port) = target.split_once('@').expect("user@host:port");
    let (host, port) = host_port.rsplit_once(':').expect("host:port");
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port = port.parse().unwrap();
    let password = Zeroizing::new(
        std::fs::read_to_string(
            std::env::var_os("CAPSTAN_TEST_SSH_PASSWORD_FILE").expect("fixture password file"),
        )
        .unwrap(),
    );
    let key = std::fs::read_to_string(
        std::env::var_os("CAPSTAN_TEST_SSH_HOST_KEY_FILE").expect("fixture public host key file"),
    )
    .unwrap();
    let key = HostPublicKey::from_openssh(key.trim()).unwrap();
    let policy = PoolKey {
        target: host.into(),
        port,
        username: username.into(),
        credential_id: Uuid::new_v4(),
        credential_revision: 1,
        trust_fingerprint: key.fingerprint.clone(),
        vault_epoch: 1,
        agent_identity: None,
        agent_forwarding: false,
        proxy_path: String::new(),
    };
    let pool = ConnectionPool::new(Duration::from_secs(30));
    let host = policy.target.clone();
    let username = policy.username.clone();
    pool.acquire(policy, CancellationToken::new(), move |cancel| async move {
        let (broker, _challenges) = TrustBroker::new();
        let binding = attempt(&host, port);
        connect_password(
            host,
            port,
            username,
            SecretString::from(password.trim_end_matches(['\r', '\n'])),
            binding,
            Arc::new(Trust(key)),
            broker,
            cancel,
        )
        .await
    })
    .await
    .unwrap()
}

async fn expect_shell_output(shell: &mut SshShell, command: &[u8], marker: &[u8]) {
    shell.control.write(command.to_vec()).await.unwrap();
    timeout(Duration::from_secs(8), async {
        let mut output = Vec::new();
        while !output.windows(marker.len()).any(|part| part == marker) {
            assert!(output.len() < 64 * 1024);
            let mut chunk = [0; 1024];
            let read = shell.reader.read(&mut chunk).await.unwrap();
            assert_ne!(read, 0);
            output.extend_from_slice(&chunk[..read]);
        }
    })
    .await
    .expect("shell did not print the fixture marker");
}

#[tokio::test]
async fn configured_openssh_status_samples_linux_proc_and_df() {
    let Ok(target) = std::env::var("CAPSTAN_TEST_SSHD") else {
        println!("SKIP: CAPSTAN_TEST_SSHD is not set; no external SSH server was contacted");
        return;
    };
    let lease = configured_openssh_lease(&target).await;
    let cancel = CancellationToken::new();
    let mut shell = lease.open_shell(80, 24).await.unwrap();
    let service = StatusService::default();
    let id = lease.connection().connection_id();
    service.start(&lease);
    service.target.send_replace(Some(id));
    let mut observation = service.selection.lock().unwrap().collectors[&id]
        .observation
        .subscribe();
    let sampled_at = timeout(
        Duration::from_secs(15),
        observation.wait_for(|status| {
            status.cpu_percent.is_some()
                && status.memory.is_some()
                && status.disk.is_some()
                && status.network.is_some()
        }),
    )
    .await
    .expect("expected a sample with CPU, memory, root disk and network rates")
    .unwrap()
    .sampled_at;
    let sftp = lease.open_sftp(&cancel).await.unwrap();
    assert!(!sftp.realpath(".", &cancel).await.unwrap().is_empty());
    sftp.close().await;
    timeout(
        Duration::from_secs(15),
        observation
            .wait_for(|sample| sample.sampled_at.0 > sampled_at.0 && sample.cpu_percent.is_some()),
    )
    .await
    .unwrap()
    .unwrap();
    service.target.send_replace(None);
    wait_state(&mut observation, ServerStatusState::Paused).await;
    let second_shell = lease.open_shell(80, 24).await.unwrap();
    expect_shell_output(
        &mut shell,
        b"printf '\\103APSTAN_STATUS_OK\\n'\n",
        b"CAPSTAN_STATUS_OK",
    )
    .await;
    second_shell.control.close().await.unwrap();
    lease.connection().disconnect().await;
}

#[tokio::test]
async fn configured_openssh_slow_df_yields_to_sftp() {
    let Ok(target) = std::env::var("CAPSTAN_TEST_SLOW_STATUS_SSHD") else {
        println!("SKIP: CAPSTAN_TEST_SLOW_STATUS_SSHD is not set");
        return;
    };
    let lease = configured_openssh_lease(&target).await;
    let mut shell = lease.open_shell(80, 24).await.unwrap();
    expect_shell_output(
        &mut shell,
        b"rm -f \"$HOME/.capstan-review-df-started\" && printf '\\123LOW_DF_READY\\n'\n",
        b"SLOW_DF_READY",
    )
    .await;
    let service = StatusService::default();
    service.start(&lease);
    service
        .target
        .send_replace(Some(lease.connection().connection_id()));
    expect_shell_output(
        &mut shell,
        b"while [ ! -e \"$HOME/.capstan-review-df-started\" ]; do sleep 0.1; done; printf '\\123LOW_DF_STARTED\\n'\n",
        b"SLOW_DF_STARTED",
    )
    .await;

    let cancel = CancellationToken::new();
    let started = tokio::time::Instant::now();
    let sftp = lease.open_sftp(&cancel).await.unwrap();
    println!(
        "SFTP opened during slow df handoff in {:?}",
        started.elapsed()
    );
    service.target.send_replace(None);
    assert!(!sftp.realpath(".", &cancel).await.unwrap().is_empty());
    expect_shell_output(
        &mut shell,
        b"printf '\\103APSTAN_STATUS_OK\\n'\n",
        b"CAPSTAN_STATUS_OK",
    )
    .await;
    sftp.close().await;
    lease.connection().disconnect().await;
}
