use super::*;
use std::sync::atomic::AtomicUsize;
use tokio_util::sync::CancellationToken;

async fn request_seen(counter: &AtomicUsize) {
    timeout(Duration::from_secs(3), async {
        while counter.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

async fn tunnel_running(fixture: &Fixture, id: Uuid) {
    timeout(Duration::from_secs(5), async {
        loop {
            if fixture
                .vault
                .tunnel_runtime_snapshots()
                .unwrap()
                .iter()
                .any(|snapshot| snapshot.id == id && snapshot.state == TunnelRuntimeState::Running)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn remote_cancel_refusal_is_retryable_and_blocks_configuration_removal() {
    let fixture = Fixture::new().await;
    let destination = fixture.echo_server().await;
    let port = free_port().await;
    let sibling_port = free_port().await;
    let config = fixture.save(remote(port, destination), false).await;
    let sibling = fixture.save(local(sibling_port, destination), false).await;
    let (mut shell, _lease, connection) = fixture.connect(Uuid::new_v4()).await;
    for id in [config.id, sibling.id] {
        fixture
            .vault
            .start_tunnel(id, connection.connection_id, connection.generation)
            .await
            .unwrap();
    }
    echo(port, b"before refusal").await;
    fixture.observed.cancel_denied.store(true, Ordering::SeqCst);
    let stopped = fixture.vault.stop_tunnel(config.id).await.unwrap();
    assert_eq!(stopped.state, TunnelRuntimeState::Failed);
    assert_eq!(
        stopped.error_code,
        Some(TunnelErrorCode::RemoteForwardRejected)
    );
    assert_eq!(stopped.active_connections, 0);
    assert_eq!(stopped.bytes_sent, 14);
    assert!(TcpListener::bind(("127.0.0.1", port)).await.is_err());
    assert_eq!(
        fixture
            .vault
            .start_tunnel(config.id, connection.connection_id, connection.generation)
            .await
            .unwrap(),
        stopped
    );
    assert_eq!(
        fixture
            .observed
            .remote_requests_started
            .load(Ordering::SeqCst),
        1
    );
    let revision = fixture.vault.revision().unwrap();
    let ciphertext = std::fs::read(fixture.vault.path()).unwrap();
    assert_eq!(
        fixture
            .vault
            .save_tunnel(config.clone(), revision)
            .await
            .unwrap_err()
            .code(),
        "remote_forward_rejected"
    );
    assert_eq!(
        fixture
            .vault
            .delete_tunnel(config.id, revision)
            .await
            .unwrap_err()
            .code(),
        "remote_forward_rejected"
    );
    assert_eq!(
        fixture
            .vault
            .delete_host(fixture.host.id, revision)
            .await
            .unwrap_err()
            .code(),
        "remote_forward_rejected"
    );
    assert_eq!(fixture.vault.revision().unwrap(), revision);
    assert_eq!(std::fs::read(fixture.vault.path()).unwrap(), ciphertext);
    assert!(fixture.vault.tunnels().unwrap().contains(&config));
    echo(sibling_port, b"sibling survives").await;
    shell
        .control
        .write(b"shell survives".to_vec())
        .await
        .unwrap();
    let mut reply = [0; 14];
    timeout(Duration::from_secs(2), shell.reader.read_exact(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&reply, b"shell survives");
    fixture
        .observed
        .cancel_denied
        .store(false, Ordering::SeqCst);
    let stopped = fixture.vault.stop_tunnel(config.id).await.unwrap();
    assert_eq!(stopped.state, TunnelRuntimeState::Stopped);
    assert_eq!(stopped.error_code, None);
    let rebound = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    drop(rebound);
    fixture
        .vault
        .start_tunnel(config.id, connection.connection_id, connection.generation)
        .await
        .unwrap();
    echo(port, b"restarted").await;
    assert_eq!(fixture.observed.accepts.load(Ordering::SeqCst), 1);
    fixture.close().await;
}

#[tokio::test]
async fn timed_out_remote_cancel_retains_its_late_acknowledgement() {
    let fixture = Fixture::new().await;
    let port = free_port().await;
    let config = fixture.save(remote(port, 8000), false).await;
    let (_shell, _lease, connection) = fixture.connect(Uuid::new_v4()).await;
    fixture
        .vault
        .start_tunnel(config.id, connection.connection_id, connection.generation)
        .await
        .unwrap();
    fixture.observed.cancel_paused.store(true, Ordering::SeqCst);
    let failed = timeout(Duration::from_secs(4), fixture.vault.stop_tunnel(config.id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.state, TunnelRuntimeState::Failed);
    assert_eq!(
        failed.error_code,
        Some(TunnelErrorCode::RemoteForwardRejected)
    );
    assert!(TcpListener::bind(("127.0.0.1", port)).await.is_err());
    fixture.observed.cancel_resume.notify_one();
    let stopped = fixture.vault.stop_tunnel(config.id).await.unwrap();
    assert_eq!(stopped.state, TunnelRuntimeState::Stopped);
    assert_eq!(
        fixture.observed.cancel_requests.load(Ordering::SeqCst),
        1,
        "retry must consume the original late acknowledgement"
    );
    let _rebound = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    assert_eq!(fixture.observed.accepts.load(Ordering::SeqCst), 1);
    fixture.close().await;
}

#[tokio::test]
async fn backup_overwrite_preserves_configuration_when_remote_cancel_is_refused() {
    let fixture = Fixture::new().await;
    let config = fixture.save(remote(free_port().await, 8000), false).await;
    let backup = fixture.directory.path().join("backup.cbak");
    fixture
        .vault
        .export_backup(backup.clone(), fixture.master.clone())
        .await
        .unwrap();
    let (_shell, _lease, connection) = fixture.connect(Uuid::new_v4()).await;
    fixture
        .vault
        .start_tunnel(config.id, connection.connection_id, connection.generation)
        .await
        .unwrap();
    fixture.observed.cancel_denied.store(true, Ordering::SeqCst);
    let revision = fixture.vault.revision().unwrap();
    let before = std::fs::read(fixture.vault.path()).unwrap();
    let error = fixture
        .vault
        .import_backup(
            backup,
            fixture.master.clone(),
            BackupImportMode::Overwrite,
            revision,
            fixture.vault.epoch(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), "remote_forward_rejected");
    assert_eq!(std::fs::read(fixture.vault.path()).unwrap(), before);
    assert!(fixture
        .vault
        .tunnel_runtime_snapshots()
        .unwrap()
        .iter()
        .any(|snapshot| snapshot.id == config.id && snapshot.state == TunnelRuntimeState::Failed));
    fixture
        .observed
        .cancel_denied
        .store(false, Ordering::SeqCst);
    fixture.close().await;
}

#[tokio::test]
async fn host_delete_checks_remote_cancellation_before_stopping_sibling_tunnels() {
    let fixture = Fixture::new().await;
    let destination = fixture.echo_server().await;
    let local_ports = [free_port().await, free_port().await];
    let remote_ports = [free_port().await, free_port().await];
    let configs = [
        TunnelConfig {
            id: Uuid::from_u128(1),
            host_id: fixture.host.id,
            auto_start: false,
            forward: local(local_ports[0], destination),
        },
        TunnelConfig {
            id: Uuid::from_u128(2),
            host_id: fixture.host.id,
            auto_start: false,
            forward: local(local_ports[1], destination),
        },
        TunnelConfig {
            id: Uuid::from_u128(3),
            host_id: fixture.host.id,
            auto_start: false,
            forward: remote(remote_ports[0], destination),
        },
        TunnelConfig {
            id: Uuid::from_u128(4),
            host_id: fixture.host.id,
            auto_start: false,
            forward: remote(remote_ports[1], destination),
        },
    ];
    for config in &configs {
        fixture
            .vault
            .save_tunnel(config.clone(), fixture.vault.revision().unwrap())
            .await
            .unwrap();
    }
    let (_shell, _lease, connection) = fixture.connect(Uuid::new_v4()).await;
    for config in &configs {
        fixture
            .vault
            .start_tunnel(config.id, connection.connection_id, connection.generation)
            .await
            .unwrap();
    }
    fixture.observed.cancel_denied.store(true, Ordering::SeqCst);
    let revision = fixture.vault.revision().unwrap();
    assert_eq!(
        fixture
            .vault
            .delete_host(fixture.host.id, revision)
            .await
            .unwrap_err()
            .code(),
        "remote_forward_rejected"
    );
    assert_eq!(fixture.vault.revision().unwrap(), revision);
    assert_eq!(fixture.vault.tunnels().unwrap(), configs);
    assert_eq!(
        fixture.observed.cancel_requests.load(Ordering::SeqCst),
        1,
        "the first failed remote cancellation must abort before siblings are stopped"
    );
    let snapshots = fixture.vault.tunnel_runtime_snapshots().unwrap();
    for config in configs
        .iter()
        .filter(|config| matches!(&config.forward, TunnelForward::Local { .. }))
    {
        assert_eq!(
            snapshots
                .iter()
                .find(|snapshot| snapshot.id == config.id)
                .unwrap()
                .state,
            TunnelRuntimeState::Running,
            "local sibling runtime should remain active after the remote refusal"
        );
    }
    assert_eq!(
        snapshots
            .iter()
            .find(|snapshot| snapshot.id == Uuid::from_u128(3))
            .unwrap()
            .state,
        TunnelRuntimeState::Failed
    );
    assert_eq!(
        snapshots
            .iter()
            .find(|snapshot| snapshot.id == Uuid::from_u128(4))
            .unwrap()
            .state,
        TunnelRuntimeState::Running,
        "stable id ordering should stop before touching later remote siblings"
    );
    for port in local_ports {
        echo(port, b"sibling survived failed host deletion").await;
    }
    echo(
        remote_ports[1],
        b"later remote sibling survived failed host deletion",
    )
    .await;
    fixture
        .observed
        .cancel_denied
        .store(false, Ordering::SeqCst);
    fixture.close().await;
}

#[tokio::test]
async fn rejected_first_pty_keeps_automatic_tunnels_supervised() {
    let fixture = Fixture::new().await;
    let destination = fixture.echo_server().await;
    let port = free_port().await;
    let config = fixture.save(local(port, destination), true).await;
    fixture.observed.pty_denied.store(true, Ordering::SeqCst);
    let owner = Uuid::new_v4();
    let result = fixture
        .vault
        .connect_password_host_with_lease(
            fixture.host.id,
            owner,
            fixture.broker.clone(),
            CancellationToken::new(),
        )
        .await;
    assert_eq!(result.err().unwrap().code(), "ssh_channel_rejected");
    assert!(fixture.vault.connection_for_owner(owner).is_err());
    let connection = fixture.vault.connection_for_owner(config.id).unwrap();
    tunnel_running(&fixture, config.id).await;
    echo(port, b"no shell required").await;
    fixture.observed.pty_denied.store(false, Ordering::SeqCst);
    fixture.lose_transport().await;
    fixture
        .state(
            connection.connection_id,
            ConnectionState::Online,
            connection.generation + 1,
        )
        .await;
    tunnel_running(&fixture, config.id).await;
    echo(port, b"supervised reconnect").await;
    assert!(fixture.observed.accepts.load(Ordering::SeqCst) >= 2);
    fixture.close().await;
}

#[tokio::test]
async fn canceled_first_pty_releases_the_attempt_and_supervises_automatic_tunnels() {
    let fixture = Fixture::new().await;
    let port = free_port().await;
    let destination = fixture.echo_server().await;
    let config = fixture.save(local(port, destination), true).await;
    fixture.observed.pty_paused.store(true, Ordering::SeqCst);
    let owner = Uuid::new_v4();
    let cancel = CancellationToken::new();
    let vault = fixture.vault.clone();
    let broker = fixture.broker.clone();
    let host = fixture.host.id;
    let token = cancel.clone();
    let connect = tokio::spawn(async move {
        vault
            .connect_password_host_with_lease(host, owner, broker, token)
            .await
    });
    request_seen(&fixture.observed.pty_requests).await;
    cancel.cancel();
    timeout(
        Duration::from_secs(1),
        fixture.vault.cancel_connection_attempt(owner),
    )
    .await
    .unwrap();
    assert_eq!(
        timeout(Duration::from_secs(2), connect)
            .await
            .unwrap()
            .unwrap()
            .err()
            .unwrap()
            .code(),
        "canceled"
    );
    assert!(fixture.vault.connection_for_owner(owner).is_err());
    let connection = fixture.vault.connection_for_owner(config.id).unwrap();
    fixture.observed.pty_resume.notify_one();
    tunnel_running(&fixture, config.id).await;
    echo(port, b"cancel kept the tunnel").await;
    fixture.lose_transport().await;
    fixture
        .state(
            connection.connection_id,
            ConnectionState::Online,
            connection.generation + 1,
        )
        .await;
    tunnel_running(&fixture, config.id).await;
    echo(port, b"reconnected after cancel").await;
    fixture.close().await;
}

#[tokio::test]
async fn slow_remote_start_does_not_block_stopping_a_sibling_listener() {
    let fixture = Fixture::new().await;
    let port = free_port().await;
    let local = fixture.save(local(port, 8000), false).await;
    let remote = fixture.save(remote(free_port().await, 8000), false).await;
    let (_shell, _lease, connection) = fixture.connect(Uuid::new_v4()).await;
    fixture
        .vault
        .start_tunnel(local.id, connection.connection_id, connection.generation)
        .await
        .unwrap();
    fixture.observed.remote_paused.store(true, Ordering::SeqCst);
    let vault = fixture.vault.clone();
    let start = tokio::spawn(async move {
        vault
            .start_tunnel(remote.id, connection.connection_id, connection.generation)
            .await
    });
    request_seen(&fixture.observed.remote_requests_started).await;
    let stopped = timeout(
        Duration::from_millis(500),
        fixture.vault.stop_tunnel(local.id),
    )
    .await;
    // Release the server even on failure so fixture cleanup remains bounded.
    fixture.observed.remote_resume.notify_one();
    assert_eq!(stopped.unwrap().unwrap().state, TunnelRuntimeState::Stopped);
    let _rebound = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    assert_eq!(
        start.await.unwrap().unwrap().state,
        TunnelRuntimeState::Running
    );
    fixture.close().await;
}

#[tokio::test]
async fn stopping_a_starting_remote_tunnel_waits_for_confirmed_reclamation() {
    let fixture = Fixture::new().await;
    let port = free_port().await;
    let config = fixture.save(remote(port, 8000), false).await;
    let (_shell, _lease, connection) = fixture.connect(Uuid::new_v4()).await;
    fixture.observed.remote_paused.store(true, Ordering::SeqCst);
    let vault = fixture.vault.clone();
    let id = config.id;
    let start = tokio::spawn(async move {
        vault
            .start_tunnel(id, connection.connection_id, connection.generation)
            .await
    });
    request_seen(&fixture.observed.remote_requests_started).await;
    let vault = fixture.vault.clone();
    let stop = tokio::spawn(async move { vault.stop_tunnel(id).await });
    timeout(Duration::from_secs(1), async {
        loop {
            if fixture
                .vault
                .tunnel_runtime_snapshots()
                .unwrap()
                .iter()
                .any(|snapshot| snapshot.id == id && snapshot.state == TunnelRuntimeState::Stopping)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    fixture.observed.remote_resume.notify_one();
    assert_ne!(
        start.await.unwrap().unwrap().state,
        TunnelRuntimeState::Running
    );
    assert_eq!(
        timeout(Duration::from_secs(3), stop)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .state,
        TunnelRuntimeState::Stopped
    );
    let _rebound = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn automatic_slow_remote_start_does_not_delay_local_start_or_stop() {
    let fixture = Fixture::new().await;
    let remote = fixture.save(remote(free_port().await, 8000), true).await;
    let port = free_port().await;
    let local = fixture.save(local(port, 8000), true).await;
    fixture.observed.remote_paused.store(true, Ordering::SeqCst);
    let vault = fixture.vault.clone();
    let broker = fixture.broker.clone();
    let host = fixture.host.id;
    let owner = Uuid::new_v4();
    let connect = tokio::spawn(async move {
        vault
            .connect_password_host_with_lease(host, owner, broker, CancellationToken::new())
            .await
    });
    request_seen(&fixture.observed.remote_requests_started).await;
    tunnel_running(&fixture, local.id).await;
    let stopped = timeout(
        Duration::from_millis(500),
        fixture.vault.stop_tunnel(local.id),
    )
    .await;
    fixture.observed.remote_resume.notify_one();
    assert_eq!(stopped.unwrap().unwrap().state, TunnelRuntimeState::Stopped);
    let _rebound = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    let (_, shell, lease) = connect.await.unwrap().unwrap();
    tunnel_running(&fixture, remote.id).await;
    // A later shell on the same generation must not undo the explicit stop.
    let (_other_shell, _other_lease, _) = fixture.connect(Uuid::new_v4()).await;
    assert!(fixture
        .vault
        .tunnel_runtime_snapshots()
        .unwrap()
        .iter()
        .any(|snapshot| snapshot.id == local.id && snapshot.state == TunnelRuntimeState::Stopped));
    drop((shell, lease));
    fixture.close().await;
}
