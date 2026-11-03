#![allow(clippy::pedantic)]

use capstan_core::{CredentialUpdate, VaultManager};
use capstan_ssh::{forwarding::TunnelRuntimeState, SshShell, TrustBroker};
use capstan_vault::{Host, KdfParameters, SecretText, TunnelConfig, TunnelForward};
use russh::{
    keys::{Algorithm, HashAlg, PrivateKey},
    server::{self, Auth, Session},
};
use std::{
    io::Read,
    net::SocketAddr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Clone)]
struct PasswordServer {
    password: Arc<Zeroizing<String>>,
    authentications: Arc<AtomicUsize>,
}

impl server::Server for PasswordServer {
    type Handler = Self;

    fn new_client(&mut self, _: Option<SocketAddr>) -> Self {
        self.clone()
    }
}

impl server::Handler for PasswordServer {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.authentications.fetch_add(1, Ordering::SeqCst);
        Ok(
            if user == "workflow-user" && password == self.password.as_str() {
                Auth::Accept
            } else {
                Auth::reject()
            },
        )
    }

    async fn channel_open_session(
        &mut self,
        _: russh::Channel<server::Msg>,
        _: &mut Session,
    ) -> Result<bool, Self::Error> {
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
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)
    }

    async fn shell_request(
        &mut self,
        channel: russh::ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)
    }
}

async fn isolated_vault(
    address: &str,
    port: u16,
    username: &str,
    password: &str,
) -> (tempfile::TempDir, VaultManager, SecretText, Host) {
    let directory = tempfile::tempdir().unwrap();
    let manager = VaultManager::new(directory.path().join("vault.capstan"));
    let master = SecretText::new(Uuid::new_v4().to_string());
    manager
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
    let credential_id = manager
        .add_password_credential("Workflow credential".into(), password.to_owned())
        .await
        .unwrap();
    let host = Host {
        id: Uuid::new_v4(),
        name: "Workflow host".into(),
        address: address.into(),
        port,
        username: username.into(),
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
    manager
        .save_host(host.clone(), manager.revision().unwrap())
        .await
        .unwrap();
    (directory, manager, master, host)
}

async fn first_connection(
    manager: &VaultManager,
    host: &Host,
    fingerprint: &str,
    accepted: bool,
    confirmation_delay: Duration,
) -> capstan_core::Result<(Host, SshShell)> {
    let (broker, mut challenges) = TrustBroker::new();
    let attempt = Uuid::new_v4();
    let connection =
        manager.connect_password_host(host.id, attempt, broker.clone(), CancellationToken::new());
    let decision = async {
        let challenge = timeout(Duration::from_secs(10), challenges.recv())
            .await
            .expect("trust challenge timed out")
            .expect("trust challenge stream closed");
        assert_eq!(challenge.attempt_id, attempt);
        assert_eq!(challenge.fingerprint, fingerprint);
        assert!(!challenge.key_changed);
        assert!(manager.trusted_host_keys().unwrap().is_empty());
        tokio::time::sleep(confirmation_delay).await;
        broker.respond(challenge.challenge_id, accepted).unwrap();
    };
    let (result, ()) = tokio::join!(
        timeout(confirmation_delay + Duration::from_secs(15), connection),
        decision
    );
    result.expect("SSH workflow timed out")
}

async fn known_connection(
    manager: &VaultManager,
    host: &Host,
) -> capstan_core::Result<(Host, SshShell)> {
    let (broker, mut challenges) = TrustBroker::new();
    let result = timeout(
        Duration::from_secs(15),
        manager.connect_password_host(host.id, Uuid::new_v4(), broker, CancellationToken::new()),
    )
    .await
    .expect("known-host SSH workflow timed out");
    assert!(challenges.try_recv().is_err());
    result
}

#[tokio::test]
async fn trust_save_authentication_retry_and_vault_restart_complete_the_ssh_workflow() {
    use russh::server::Server;

    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
    let password = Arc::new(Zeroizing::new(Uuid::new_v4().to_string()));
    let authentications = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = Arc::new(server::Config {
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![key],
        ..server::Config::default()
    });
    let mut server = PasswordServer {
        password: password.clone(),
        authentications: authentications.clone(),
    };
    let server_task = tokio::spawn(async move { server.run_on_socket(config, &listener).await });
    let (directory, manager, master, host) =
        isolated_vault("127.0.0.1", address.port(), "workflow-user", &password).await;

    let rejected = first_connection(&manager, &host, &fingerprint, false, Duration::ZERO).await;
    assert_eq!(rejected.err().unwrap().code(), "host_key_rejected");
    assert_eq!(authentications.load(Ordering::SeqCst), 0);
    assert!(manager.trusted_host_keys().unwrap().is_empty());

    let (_, shell) = first_connection(&manager, &host, &fingerprint, true, Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(authentications.load(Ordering::SeqCst), 1);
    assert_eq!(
        manager.trusted_host_keys().unwrap()[0].fingerprint,
        fingerprint
    );
    shell.control.close().await.unwrap();
    manager.disconnect_ssh().await;

    manager
        .save_password_credential(
            "Workflow credential".into(),
            Uuid::new_v4().to_string(),
            Some(CredentialUpdate {
                id: host.credential_id,
                expected_revision: manager.revision().unwrap(),
            }),
        )
        .await
        .unwrap();
    let failed = known_connection(&manager, &host).await;
    assert_eq!(failed.err().unwrap().code(), "ssh_authentication");
    manager
        .save_password_credential(
            "Workflow credential".into(),
            password.to_string(),
            Some(CredentialUpdate {
                id: host.credential_id,
                expected_revision: manager.revision().unwrap(),
            }),
        )
        .await
        .unwrap();
    manager.lock().await.unwrap();

    let reopened = VaultManager::new(directory.path().join("vault.capstan"));
    reopened.unlock(master).await.unwrap();
    let (_, shell) = known_connection(&reopened, &host).await.unwrap();
    shell.control.close().await.unwrap();
    reopened.disconnect_ssh().await;
    server_task.abort();
}

#[tokio::test]
async fn configured_openssh_password_completes_delayed_trust_and_vault_restart_workflow() {
    let Ok(target) = std::env::var("CAPSTAN_TEST_SSHD") else {
        eprintln!("SKIP: CAPSTAN_TEST_SSHD is not configured");
        return;
    };
    let Some(password_path) = std::env::var_os("CAPSTAN_TEST_SSH_PASSWORD_FILE") else {
        eprintln!("SKIP: CAPSTAN_TEST_SSH_PASSWORD_FILE is not configured");
        return;
    };
    let fingerprint = std::env::var("CAPSTAN_TEST_SSH_HOST_FINGERPRINT")
        .expect("verify the dedicated server fingerprint before running the Vault workflow test");
    let (username, endpoint) = target.split_once('@').expect("expected user@host:port");
    let (address, port) = endpoint.rsplit_once(':').expect("expected host:port");
    let address = address.trim_start_matches('[').trim_end_matches(']');
    let port = port.parse::<u16>().expect("expected a valid port");
    let mut password = Zeroizing::new(String::new());
    std::fs::File::open(password_path)
        .expect("open test password file")
        .take(8193)
        .read_to_string(&mut password)
        .expect("read UTF-8 test password file");
    assert!(
        password.len() <= 8192,
        "test password file exceeds size limit"
    );
    if password.ends_with('\n') {
        password.pop();
        if password.ends_with('\r') {
            password.pop();
        }
    }
    assert!(!password.is_empty());
    let (directory, manager, master, host) =
        isolated_vault(address, port, username, &password).await;
    let (_, shell) = first_connection(&manager, &host, &fingerprint, true, Duration::from_secs(20))
        .await
        .unwrap();
    shell.control.close().await.unwrap();
    manager.disconnect_ssh().await;
    manager.lock().await.unwrap();
    let reopened = VaultManager::new(directory.path().join("vault.capstan"));
    reopened.unlock(master).await.unwrap();
    let (_, shell) = known_connection(&reopened, &host).await.unwrap();
    let connections = reopened.connection_snapshots().unwrap();
    let [connection] = connections.as_slice() else {
        panic!("expected one authenticated SSH connection");
    };
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let reservation = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bind_address = reservation.local_addr().unwrap();
    let tunnel = TunnelConfig {
        id: Uuid::new_v4(),
        host_id: host.id,
        auto_start: false,
        forward: TunnelForward::Local {
            bind_host: "127.0.0.1".into(),
            bind_port: bind_address.port(),
            destination_host: "127.0.0.1".into(),
            destination_port: destination.local_addr().unwrap().port(),
        },
    };
    reopened
        .save_tunnel(tunnel.clone(), reopened.revision().unwrap())
        .await
        .unwrap();
    drop(reservation);
    assert_eq!(
        reopened
            .start_tunnel(tunnel.id, connection.connection_id, connection.generation)
            .await
            .unwrap()
            .state,
        TunnelRuntimeState::Running
    );
    timeout(Duration::from_secs(10), async {
        const PAYLOAD: &[u8] = b"Capstan OpenSSH tunnel probe";
        let mut client = TcpStream::connect(bind_address).await.unwrap();
        let (mut server, _) = destination.accept().await.unwrap();
        let mut received = [0; PAYLOAD.len()];
        client.write_all(PAYLOAD).await.unwrap();
        server.read_exact(&mut received).await.unwrap();
        assert_eq!(&received, PAYLOAD);
        server.write_all(&received).await.unwrap();
        client.read_exact(&mut received).await.unwrap();
        assert_eq!(&received, PAYLOAD);
    })
    .await
    .expect("OpenSSH local forwarding probe timed out");
    assert_eq!(
        reopened.stop_tunnel(tunnel.id).await.unwrap().state,
        TunnelRuntimeState::Stopped
    );
    shell.control.close().await.unwrap();
    reopened.disconnect_ssh().await;
}
