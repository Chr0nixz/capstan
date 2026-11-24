#![allow(clippy::pedantic)]

//! A saved host reached through a saved jump host, end to end through the
//! Vault: trust for both hops, the shared connection pool and deletion.

use capstan_core::VaultManager;
use capstan_ssh::TrustBroker;
use capstan_vault::{Host, KdfParameters, SecretText};
use russh::{
    keys::{Algorithm, HashAlg, PrivateKey},
    server::{self, Auth, Msg, Session},
    Channel, ChannelId,
};
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, time::timeout};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
struct Server {
    password: &'static str,
    forward_to: Option<SocketAddr>,
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
    async fn channel_open_session(
        &mut self,
        _: Channel<Msg>,
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

struct Running {
    address: SocketAddr,
    fingerprint: String,
    destinations: Arc<Mutex<Vec<(String, u32)>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Running {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn start(password: &'static str, forward_to: Option<SocketAddr>) -> Running {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
    let config = Arc::new(server::Config {
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![key],
        ..server::Config::default()
    });
    let handler = Server {
        password,
        forward_to,
        destinations: Arc::new(Mutex::new(Vec::new())),
    };
    let destinations = handler.destinations.clone();
    let task = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let config = config.clone();
            let handler = handler.clone();
            tokio::spawn(async move {
                if let Ok(session) = server::run_stream(config, socket, handler).await {
                    let _ = session.await;
                }
            });
        }
    });
    Running {
        address,
        fingerprint,
        destinations,
        task,
    }
}

fn host(name: &str, address: &str, port: u16, credential_id: Uuid, jump: Option<Uuid>) -> Host {
    Host {
        id: Uuid::new_v4(),
        name: name.into(),
        address: address.into(),
        port,
        username: "ops".into(),
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
        jump_host_id: jump,
    }
}

#[tokio::test]
async fn a_host_behind_a_jump_host_confirms_both_keys_and_shares_the_jump() {
    let target = start("target-secret", None).await;
    let jump = start("jump-secret", Some(target.address)).await;
    let directory = tempfile::tempdir().unwrap();
    let manager = VaultManager::new(directory.path().join("vault.capstan"));
    manager
        .initialize(
            SecretText::new(Uuid::new_v4().to_string()),
            KdfParameters {
                memory_kib: 19_456,
                iterations: 2,
                parallelism: 1,
            },
        )
        .await
        .unwrap();
    let jump_credential = manager
        .add_password_credential("jump".into(), "jump-secret".into())
        .await
        .unwrap();
    let target_credential = manager
        .add_password_credential("target".into(), "target-secret".into())
        .await
        .unwrap();
    let bastion = host(
        "bastion",
        "127.0.0.1",
        jump.address.port(),
        jump_credential,
        None,
    );
    manager
        .save_host(bastion.clone(), manager.revision().unwrap())
        .await
        .unwrap();
    // Only the jump host can resolve this name.
    let db = host(
        "db",
        "db.internal",
        2222,
        target_credential,
        Some(bastion.id),
    );
    manager
        .save_host(db.clone(), manager.revision().unwrap())
        .await
        .unwrap();

    let (broker, mut challenges) = TrustBroker::new();
    let attempt = Uuid::new_v4();
    let connection =
        manager.connect_password_host(db.id, attempt, broker.clone(), CancellationToken::new());
    let confirm = async {
        let mut seen = Vec::new();
        for _ in 0..2 {
            let challenge = timeout(Duration::from_secs(10), challenges.recv())
                .await
                .expect("trust challenge timed out")
                .unwrap();
            assert_eq!(challenge.attempt_id, attempt);
            seen.push((
                challenge.host_id,
                challenge.target.clone(),
                challenge.fingerprint.clone(),
            ));
            broker.respond(challenge.challenge_id, true).unwrap();
        }
        seen
    };
    let (result, seen) = tokio::join!(timeout(Duration::from_secs(20), connection), confirm);
    let (_, shell) = result.expect("connection timed out").unwrap();
    assert_eq!(
        seen,
        vec![
            (bastion.id, "127.0.0.1".to_owned(), jump.fingerprint.clone()),
            (db.id, "db.internal".to_owned(), target.fingerprint.clone()),
        ]
    );
    assert_eq!(
        jump.destinations.lock().unwrap().as_slice(),
        [("db.internal".to_owned(), 2222)]
    );

    // A terminal on the jump host itself reuses the connection the target rides on.
    let (_, bastion_shell) = timeout(
        Duration::from_secs(10),
        manager.connect_password_host(
            bastion.id,
            Uuid::new_v4(),
            broker.clone(),
            CancellationToken::new(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(challenges.try_recv().is_err());

    let revision = manager.revision().unwrap();
    assert_eq!(
        manager
            .delete_host(bastion.id, revision)
            .await
            .unwrap_err()
            .code(),
        "host_in_use"
    );
    // Jump chains cannot loop back through the host itself.
    let saved = manager
        .hosts()
        .unwrap()
        .into_iter()
        .find(|host| host.id == bastion.id)
        .unwrap();
    let mut looping = bastion.clone();
    looping.revision = saved.revision;
    looping.jump_host_id = Some(db.id);
    assert_eq!(
        manager
            .save_host(looping, manager.revision().unwrap())
            .await
            .unwrap_err()
            .code(),
        "invalid_input"
    );

    bastion_shell.control.close().await.unwrap();
    shell.control.close().await.unwrap();
    manager.disconnect_ssh().await;
}
