#![allow(clippy::pedantic)]

//! A target reached through a jump host's `direct-tcpip` channel, with both
//! servers running in process.

use async_trait::async_trait;
use capstan_ssh::{
    connect, AttemptBinding, ConnectionLease, ConnectionPool, HostPublicKey, Login,
    PasswordConnection, PoolKey, SshError, TrustBroker, TrustRepository,
};
use russh::{
    keys::{Algorithm, PrivateKey},
    server::{self, Auth, Msg, Session},
    Channel,
};
use secrecy::SecretString;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{net::TcpListener, task::JoinHandle, time::timeout};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
struct Server {
    password: &'static str,
    /// Where `direct-tcpip` requests go, whatever name they ask for.
    forward_to: Option<SocketAddr>,
    reject_forwarding: Arc<AtomicBool>,
    destinations: Arc<Mutex<Vec<(String, u32)>>>,
}

impl server::Handler for Server {
    type Error = russh::Error;

    async fn auth_password(&mut self, _: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if password == self.password {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host: &str,
        port: u32,
        _: &str,
        _: u32,
        _: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.destinations
            .lock()
            .unwrap()
            .push((host.to_owned(), port));
        let Some(target) = self.forward_to else {
            return Ok(false);
        };
        if self.reject_forwarding.load(Ordering::SeqCst) {
            return Ok(false);
        }
        let Ok(mut stream) = tokio::net::TcpStream::connect(target).await else {
            return Ok(false);
        };
        tokio::spawn(async move {
            let mut channel = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut stream, &mut channel).await;
        });
        Ok(true)
    }
}

struct TestServer {
    address: SocketAddr,
    key: HostPublicKey,
    handler: Server,
    sessions: Arc<Mutex<Vec<server::Handle>>>,
    task: JoinHandle<()>,
}
impl TestServer {
    /// Ends every session, as if the server went away.
    async fn cut(&self) {
        let sessions: Vec<_> = self.sessions.lock().unwrap().drain(..).collect();
        for session in sessions {
            let _ = session
                .disconnect(russh::Disconnect::ByApplication, "gone".into(), "en".into())
                .await;
        }
    }
}
impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn start(password: &'static str, forward_to: Option<SocketAddr>) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let key = HostPublicKey::from_public_key(host_key.public_key()).unwrap();
    let config = Arc::new(server::Config {
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![host_key],
        ..server::Config::default()
    });
    let handler = Server {
        password,
        forward_to,
        reject_forwarding: Arc::new(AtomicBool::new(false)),
        destinations: Arc::new(Mutex::new(Vec::new())),
    };
    let sessions = Arc::new(Mutex::new(Vec::new()));
    let task = {
        let handler = handler.clone();
        let sessions = sessions.clone();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let config = config.clone();
                let handler = handler.clone();
                let sessions = sessions.clone();
                tokio::spawn(async move {
                    if let Ok(session) = server::run_stream(config, socket, handler).await {
                        sessions.lock().unwrap().push(session.handle());
                        let _ = session.await;
                    }
                });
            }
        })
    };
    TestServer {
        address,
        key,
        handler,
        sessions,
        task,
    }
}

/// Knows the keys by the names the client verifies them under.
struct Known {
    keys: HashMap<(String, u16), HostPublicKey>,
    lookups: Mutex<Vec<(String, u16)>>,
}
#[async_trait]
impl TrustRepository for Known {
    async fn lookup(&self, target: &str, port: u16) -> capstan_ssh::Result<Option<HostPublicKey>> {
        self.lookups.lock().unwrap().push((target.to_owned(), port));
        Ok(self.keys.get(&(target.to_owned(), port)).cloned())
    }
    async fn is_current(&self, _: &AttemptBinding) -> bool {
        true
    }
    async fn remember(&self, _: &AttemptBinding, _: &HostPublicKey) -> capstan_ssh::Result<()> {
        Err(SshError::TrustRequired)
    }
}

fn binding(target: &str, port: u16) -> AttemptBinding {
    AttemptBinding {
        attempt_id: Uuid::new_v4(),
        host_id: Uuid::new_v4(),
        host_revision: 1,
        vault_epoch: 1,
        target: target.to_owned(),
        port,
    }
}

fn pool_key(target: &str) -> PoolKey {
    PoolKey {
        target: target.to_owned(),
        port: 22,
        username: "ops".into(),
        credential_id: Uuid::nil(),
        credential_revision: 0,
        trust_fingerprint: String::new(),
        vault_epoch: 1,
        agent_identity: None,
        agent_forwarding: false,
        proxy_path: String::new(),
    }
}

struct Fixture {
    jump: TestServer,
    target: TestServer,
    repository: Arc<Known>,
    pool: ConnectionPool<PasswordConnection>,
}

async fn fixture() -> Fixture {
    let target = start("target-secret", None).await;
    let jump = start("jump-secret", Some(target.address)).await;
    let repository = Arc::new(Known {
        keys: HashMap::from([
            (("127.0.0.1".into(), jump.address.port()), jump.key.clone()),
            (("db.internal".into(), 2222), target.key.clone()),
        ]),
        lookups: Mutex::new(Vec::new()),
    });
    Fixture {
        jump,
        target,
        repository,
        pool: ConnectionPool::new(Duration::from_secs(30)),
    }
}

impl Fixture {
    async fn jump_lease(&self) -> ConnectionLease<PasswordConnection> {
        let port = self.jump.address.port();
        let repository: Arc<dyn TrustRepository> = self.repository.clone();
        self.pool
            .acquire(
                pool_key("jump"),
                CancellationToken::new(),
                move |cancel| async move {
                    let (broker, _events) = TrustBroker::new();
                    connect(
                        "127.0.0.1".into(),
                        port,
                        "ops".into(),
                        Login::password(SecretString::from("jump-secret")),
                        None,
                        binding("127.0.0.1", port),
                        repository,
                        broker,
                        cancel,
                    )
                    .await
                },
            )
            .await
            .unwrap()
    }

    async fn through(
        &self,
        via: ConnectionLease<PasswordConnection>,
        password: &str,
    ) -> capstan_ssh::Result<PasswordConnection> {
        let (broker, _events) = TrustBroker::new();
        timeout(
            Duration::from_secs(15),
            connect(
                "db.internal".into(),
                2222,
                "ops".into(),
                Login::password(SecretString::from(password.to_owned())),
                Some(via),
                binding("db.internal", 2222),
                self.repository.clone(),
                broker,
                CancellationToken::new(),
            ),
        )
        .await
        .expect("jump connection timed out")
    }
}

#[tokio::test]
async fn the_jump_host_resolves_the_target_and_its_key_is_checked_by_name() {
    let fixture = fixture().await;
    let jump = fixture.jump_lease().await;
    let target = fixture
        .through(jump.clone(), "target-secret")
        .await
        .unwrap();
    target.probe().await.unwrap();
    assert_eq!(
        fixture.jump.handler.destinations.lock().unwrap().as_slice(),
        [("db.internal".to_owned(), 2222)]
    );
    assert!(fixture
        .repository
        .lookups
        .lock()
        .unwrap()
        .contains(&("db.internal".to_owned(), 2222)));
    // The target never saw a direct connection attempt of its own.
    assert!(fixture
        .target
        .handler
        .destinations
        .lock()
        .unwrap()
        .is_empty());

    // The target holds the jump open after every other lease is gone.
    drop(jump);
    let again = fixture.jump_lease().await;
    assert!(!again.connection().is_closed());
    drop(target);
}

#[tokio::test]
async fn a_refused_forward_and_a_wrong_target_password_fail_distinctly() {
    let fixture = fixture().await;
    let jump = fixture.jump_lease().await;
    fixture
        .jump
        .handler
        .reject_forwarding
        .store(true, Ordering::SeqCst);
    assert_eq!(
        fixture
            .through(jump.clone(), "target-secret")
            .await
            .err()
            .unwrap(),
        SshError::JumpUnreachable
    );
    fixture
        .jump
        .handler
        .reject_forwarding
        .store(false, Ordering::SeqCst);
    assert_eq!(
        fixture.through(jump.clone(), "wrong").await.err().unwrap(),
        SshError::Authentication
    );
    // A rejected hop leaves the jump connection usable.
    jump.connection().probe().await.unwrap();
    fixture.through(jump, "target-secret").await.unwrap();
}

#[tokio::test]
async fn losing_the_jump_host_closes_the_target_connection() {
    let fixture = fixture().await;
    let jump = fixture.jump_lease().await;
    let target = fixture
        .through(jump.clone(), "target-secret")
        .await
        .unwrap();
    assert!(!target.is_closed());
    fixture.jump.cut().await;
    timeout(Duration::from_secs(10), target.closed())
        .await
        .expect("target stayed open after its jump host went away");
    assert!(target.is_closed());
    assert!(fixture.through(jump, "target-secret").await.is_err());
}
