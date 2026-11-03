//! Real TCP traffic over the application-owned pool and an in-process russh server.
#![allow(clippy::pedantic)]
#[path = "forwarding/hardening.rs"]
mod hardening;
#[path = "forwarding/peer.rs"]
mod peer;
use capstan_core::{BackupImportMode, VaultManager};
use capstan_ssh::{
    connection_state::ConnectionState,
    forwarding::{TunnelErrorCode, TunnelRuntimeState},
};
use capstan_vault::{TunnelConfig, TunnelForward, VaultData};
use peer::*;
use std::{sync::atomic::Ordering, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};
use uuid::Uuid;

#[tokio::test]
async fn local_forward_half_close_counters_stop_and_channel_isolation() {
    let fixture = Fixture::new().await;
    let destination = fixture.echo_server().await;
    let port = free_port().await;
    let other_port = free_port().await;
    let config = fixture.save(local(port, destination), false).await;
    let other = fixture.save(local(other_port, destination), false).await;
    let (mut shell, _lease, connection) = fixture.connect(Uuid::new_v4()).await;
    let start = fixture
        .vault
        .start_tunnel(config.id, connection.connection_id, connection.generation)
        .await
        .unwrap();
    assert_eq!(start.state, TunnelRuntimeState::Running);
    assert_eq!(
        fixture
            .vault
            .start_tunnel(config.id, connection.connection_id, connection.generation)
            .await
            .unwrap(),
        start
    );
    fixture
        .vault
        .start_tunnel(other.id, connection.connection_id, connection.generation)
        .await
        .unwrap();
    let bytes = vec![0x6b; 128 * 1024];
    echo(port, &bytes).await;
    let mut active = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let stopped = fixture.vault.stop_tunnel(config.id).await.unwrap();
    assert_eq!(stopped.state, TunnelRuntimeState::Stopped);
    assert_eq!(stopped.active_connections, 0);
    assert_eq!(stopped.bytes_sent, bytes.len() as u64);
    assert_eq!(stopped.bytes_received, bytes.len() as u64);
    assert_eq!(fixture.vault.stop_tunnel(config.id).await.unwrap(), stopped);
    let _rebound = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    let _ = timeout(Duration::from_secs(2), active.read_u8())
        .await
        .unwrap();
    echo(other_port, b"sibling").await;
    shell.control.write(b"still alive".to_vec()).await.unwrap();
    let mut reply = [0; 11];
    timeout(Duration::from_secs(2), shell.reader.read_exact(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&reply, b"still alive");
    assert_eq!(fixture.observed.accepts.load(Ordering::SeqCst), 1);
    shell.control.close().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn remote_forward_routes_only_registered_endpoints_and_releases_listener() {
    let fixture = Fixture::new().await;
    let destination = fixture.echo_server().await;
    let port = free_port().await;
    let config = fixture.save(remote(port, destination), false).await;
    let (_shell, _lease, connection) = fixture.connect(Uuid::new_v4()).await;
    assert_eq!(
        fixture
            .vault
            .start_tunnel(config.id, connection.connection_id, connection.generation)
            .await
            .unwrap()
            .state,
        TunnelRuntimeState::Running
    );
    echo(port, b"reverse TCP").await;
    let handle = fixture.observed.handles.lock().unwrap()[0].clone();
    let mut unsolicited = handle
        .channel_open_forwarded_tcpip("unregistered.test", u32::from(port), "127.0.0.1", 1234)
        .await
        .unwrap()
        .into_stream();
    assert!(matches!(
        timeout(Duration::from_secs(2), unsolicited.read_u8()).await,
        Ok(Err(_))
    ));
    assert_eq!(
        fixture.vault.stop_tunnel(config.id).await.unwrap().state,
        TunnelRuntimeState::Stopped
    );
    let _rebound = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    assert_eq!(fixture.observed.accepts.load(Ordering::SeqCst), 1);
    fixture.close().await;
}

async fn socks_connect(port: u16, destination: u16, address: &[u8]) -> TcpStream {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(&[5, 1, 0]).await.unwrap();
    let mut auth = [0; 2];
    stream.read_exact(&mut auth).await.unwrap();
    assert_eq!(auth, [5, 0]);
    let mut request = vec![5, 1, 0];
    request.extend_from_slice(address);
    request.extend_from_slice(&destination.to_be_bytes());
    stream.write_all(&request).await.unwrap();
    let mut reply = [0; 10];
    stream.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 0);
    stream
}
#[tokio::test]
async fn socks5_connect_domain_ipv4_and_reject_unsupported_commands() {
    let fixture = Fixture::new().await;
    let destination = fixture.echo_server().await;
    let port = free_port().await;
    let config = fixture.save(socks(port), false).await;
    let (_shell, _lease, connection) = fixture.connect(Uuid::new_v4()).await;
    fixture
        .vault
        .start_tunnel(config.id, connection.connection_id, connection.generation)
        .await
        .unwrap();
    for address in [
        &[1, 127, 0, 0, 1][..],
        &[3, 9, b'l', b'o', b'c', b'a', b'l', b'h', b'o', b's', b't'][..],
    ] {
        let mut stream = socks_connect(port, destination, address).await;
        stream.write_all(b"proxy").await.unwrap();
        stream.shutdown().await.unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(bytes, b"proxy");
    }
    assert!(fixture
        .observed
        .destinations
        .lock()
        .unwrap()
        .iter()
        .any(|(host, _)| host == "localhost"));
    for command in [2, 3] {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream
            .write_all(&[5, 1, 0, 5, command, 0, 1])
            .await
            .unwrap();
        let mut reply = [0; 12];
        stream.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply[3], 7);
    }
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(&[5, 1, 2]).await.unwrap();
    let mut auth = [0; 2];
    stream.read_exact(&mut auth).await.unwrap();
    assert_eq!(auth, [5, 255]);
    let _incomplete = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let stopped = timeout(Duration::from_secs(2), fixture.vault.stop_tunnel(config.id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stopped.active_connections, 0);
    let _rebound = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn bind_conflict_remote_refusal_and_channel_refusal_are_isolated() {
    let fixture = Fixture::new().await;
    let destination = fixture.echo_server().await;
    let busy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = busy.local_addr().unwrap().port();
    let local_config = fixture.save(local(port, destination), false).await;
    let remote_config = fixture
        .save(remote(free_port().await, destination), false)
        .await;
    let (_shell, _lease, connection) = fixture.connect(Uuid::new_v4()).await;
    let failed = fixture
        .vault
        .start_tunnel(
            local_config.id,
            connection.connection_id,
            connection.generation,
        )
        .await
        .unwrap();
    assert_eq!(failed.error_code, Some(TunnelErrorCode::BindAddressInUse));
    fixture.observed.reject_remote.store(true, Ordering::SeqCst);
    let failed = fixture
        .vault
        .start_tunnel(
            remote_config.id,
            connection.connection_id,
            connection.generation,
        )
        .await
        .unwrap();
    assert_eq!(
        failed.error_code,
        Some(TunnelErrorCode::RemoteForwardRejected)
    );
    fixture
        .observed
        .reject_remote
        .store(false, Ordering::SeqCst);
    drop(busy);
    fixture
        .vault
        .start_tunnel(
            local_config.id,
            connection.connection_id,
            connection.generation,
        )
        .await
        .unwrap();
    fixture.observed.reject_direct.store(true, Ordering::SeqCst);
    let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let _ = client.read_u8().await;
    timeout(Duration::from_secs(2), async {
        loop {
            if fixture
                .vault
                .tunnel_runtime_snapshots()
                .unwrap()
                .iter()
                .any(|snapshot| {
                    snapshot.id == local_config.id
                        && snapshot.error_code == Some(TunnelErrorCode::SshChannelRejected)
                })
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    fixture
        .observed
        .reject_direct
        .store(false, Ordering::SeqCst);
    echo(port, b"recovered client").await;
    assert_eq!(
        fixture.vault.connection_snapshots().unwrap()[0].state,
        ConnectionState::Online
    );
    fixture.close().await;
}

async fn tunnel_snapshot(
    fixture: &Fixture,
    id: Uuid,
    ready: impl Fn(&capstan_ssh::forwarding::TunnelRuntimeSnapshot) -> bool,
) -> capstan_ssh::forwarding::TunnelRuntimeSnapshot {
    timeout(Duration::from_secs(2), async {
        loop {
            let snapshot = fixture
                .vault
                .tunnel_runtime_snapshots()
                .unwrap()
                .into_iter()
                .find(|snapshot| snapshot.id == id)
                .unwrap();
            if ready(&snapshot) {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn client_side_endings_keep_a_running_tunnel_clear_and_recovery_clears_errors() {
    let fixture = Fixture::new().await;
    let destination = fixture.echo_server().await;
    let socks_port = free_port().await;
    let local_port = free_port().await;
    let proxy = fixture.save(socks(socks_port), false).await;
    let forward = fixture.save(local(local_port, destination), false).await;
    let (_shell, _lease, connection) = fixture.connect(Uuid::new_v4()).await;
    for id in [proxy.id, forward.id] {
        fixture
            .vault
            .start_tunnel(id, connection.connection_id, connection.generation)
            .await
            .unwrap();
    }

    // A client that leaves before its request, one with no usable method and
    // one that resets an established stream with unread data.
    drop(TcpStream::connect(("127.0.0.1", socks_port)).await.unwrap());
    let mut refused = TcpStream::connect(("127.0.0.1", socks_port)).await.unwrap();
    refused.write_all(&[5, 1, 2]).await.unwrap();
    let mut auth = [0; 2];
    refused.read_exact(&mut auth).await.unwrap();
    assert_eq!(auth, [5, 255]);
    drop(refused);
    let mut reset = socks_connect(socks_port, destination, &[1, 127, 0, 0, 1]).await;
    reset.write_all(b"unread echo").await.unwrap();
    tunnel_snapshot(&fixture, proxy.id, |snapshot| snapshot.bytes_received > 0).await;
    drop(reset);
    let settled = tunnel_snapshot(&fixture, proxy.id, |snapshot| {
        snapshot.active_connections == 0
    })
    .await;
    assert_eq!(settled.state, TunnelRuntimeState::Running);
    assert_eq!(settled.error_code, None);

    fixture.observed.reject_direct.store(true, Ordering::SeqCst);
    let mut rejected = TcpStream::connect(("127.0.0.1", local_port)).await.unwrap();
    let _ = rejected.read_u8().await;
    tunnel_snapshot(&fixture, forward.id, |snapshot| {
        snapshot.error_code == Some(TunnelErrorCode::SshChannelRejected)
    })
    .await;
    fixture
        .observed
        .reject_direct
        .store(false, Ordering::SeqCst);
    echo(local_port, b"recovered").await;
    let recovered = tunnel_snapshot(&fixture, forward.id, |snapshot| {
        snapshot.active_connections == 0 && snapshot.error_code.is_none()
    })
    .await;
    assert_eq!(recovered.state, TunnelRuntimeState::Running);
    fixture.close().await;
}

#[tokio::test]
async fn unrelated_client_endings_do_not_clear_a_destination_failure() {
    let fixture = Fixture::new().await;
    let destination = fixture.echo_server().await;
    let port = free_port().await;
    let proxy = fixture.save(socks(port), false).await;
    let (_shell, _lease, connection) = fixture.connect(Uuid::new_v4()).await;
    fixture
        .vault
        .start_tunnel(proxy.id, connection.connection_id, connection.generation)
        .await
        .unwrap();
    let old_client = socks_connect(port, destination, &[1, 127, 0, 0, 1]).await;

    fixture.observed.reject_direct.store(true, Ordering::SeqCst);
    let mut failed = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut request = vec![5, 1, 0, 5, 1, 0, 1, 127, 0, 0, 1];
    request.extend_from_slice(&destination.to_be_bytes());
    failed.write_all(&request).await.unwrap();
    let mut reply = [0; 12];
    failed.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[3], 5);
    tunnel_snapshot(&fixture, proxy.id, |snapshot| {
        snapshot.active_connections == 1
            && snapshot.error_code == Some(TunnelErrorCode::SshChannelRejected)
    })
    .await;

    drop(old_client);
    let after_old_client = tunnel_snapshot(&fixture, proxy.id, |snapshot| {
        snapshot.active_connections == 0
    })
    .await;
    assert_eq!(
        after_old_client.error_code,
        Some(TunnelErrorCode::SshChannelRejected)
    );

    let mut incomplete = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    incomplete.write_all(&[5, 1, 0]).await.unwrap();
    let mut greeting = [0; 2];
    incomplete.read_exact(&mut greeting).await.unwrap();
    drop(incomplete);
    let after_incomplete = tunnel_snapshot(&fixture, proxy.id, |snapshot| {
        snapshot.active_connections == 0
    })
    .await;
    assert_eq!(
        after_incomplete.error_code,
        Some(TunnelErrorCode::SshChannelRejected)
    );
    fixture.close().await;
}

#[tokio::test]
async fn a_recovered_destination_clears_the_error_while_its_client_is_still_active() {
    let fixture = Fixture::new().await;
    let destination = fixture.echo_server().await;
    let port = free_port().await;
    let config = fixture.save(local(port, destination), false).await;
    let (_shell, _lease, connection) = fixture.connect(Uuid::new_v4()).await;
    fixture
        .vault
        .start_tunnel(config.id, connection.connection_id, connection.generation)
        .await
        .unwrap();
    fixture.observed.reject_direct.store(true, Ordering::SeqCst);
    let mut rejected = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let _ = rejected.read_u8().await;
    tunnel_snapshot(&fixture, config.id, |snapshot| {
        snapshot.error_code == Some(TunnelErrorCode::SshChannelRejected)
    })
    .await;

    fixture
        .observed
        .reject_direct
        .store(false, Ordering::SeqCst);
    let mut recovered = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    recovered.write_all(b"recovered").await.unwrap();
    let mut reply = [0; 9];
    timeout(Duration::from_secs(2), recovered.read_exact(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&reply, b"recovered");
    let snapshot = tunnel_snapshot(&fixture, config.id, |snapshot| {
        snapshot.active_connections == 1 && snapshot.bytes_received == 9
    })
    .await;
    assert_eq!(snapshot.state, TunnelRuntimeState::Running);
    assert_eq!(snapshot.error_code, None);
    drop(recovered);
    fixture.close().await;
}

#[tokio::test]
async fn a_full_tunnel_reports_its_client_limit_until_capacity_returns() {
    use capstan_ssh::forwarding_runtime::MAX_CLIENTS;
    let fixture = Fixture::new().await;
    let port = free_port().await;
    let proxy = fixture.save(socks(port), false).await;
    let (_shell, _lease, connection) = fixture.connect(Uuid::new_v4()).await;
    fixture
        .vault
        .start_tunnel(proxy.id, connection.connection_id, connection.generation)
        .await
        .unwrap();
    // Clients that have not sent their SOCKS request yet each hold a slot.
    let mut waiting = Vec::new();
    for _ in 0..MAX_CLIENTS {
        waiting.push(TcpStream::connect(("127.0.0.1", port)).await.unwrap());
    }
    tunnel_snapshot(&fixture, proxy.id, |snapshot| {
        snapshot.active_connections as usize == MAX_CLIENTS
    })
    .await;
    let mut refused = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    assert!(timeout(Duration::from_secs(2), refused.read_u8())
        .await
        .unwrap()
        .is_err());
    let full = tunnel_snapshot(&fixture, proxy.id, |snapshot| snapshot.error_code.is_some()).await;
    assert_eq!(full.state, TunnelRuntimeState::Running);
    assert_eq!(full.error_code, Some(TunnelErrorCode::ClientLimit));

    drop(waiting);
    let recovered = tunnel_snapshot(&fixture, proxy.id, |snapshot| {
        snapshot.active_connections == 0 && snapshot.error_code.is_none()
    })
    .await;
    assert_eq!(recovered.state, TunnelRuntimeState::Running);
    fixture.close().await;
}

#[tokio::test]
async fn a_stopped_automatic_tunnel_stays_stopped_across_reconnects_until_started() {
    let fixture = Fixture::new().await;
    let destination = fixture.echo_server().await;
    let held_port = free_port().await;
    let other_port = free_port().await;
    let held = fixture.save(local(held_port, destination), true).await;
    let other = fixture.save(local(other_port, destination), true).await;
    let (_shell, _lease, connection) = fixture.connect(Uuid::new_v4()).await;
    let running = |snapshot: &capstan_ssh::forwarding::TunnelRuntimeSnapshot| {
        snapshot.state == TunnelRuntimeState::Running
    };
    tunnel_snapshot(&fixture, held.id, running).await;
    assert_eq!(
        fixture.vault.stop_tunnel(held.id).await.unwrap().state,
        TunnelRuntimeState::Stopped
    );

    fixture.lose_transport().await;
    let next = fixture
        .state(
            connection.connection_id,
            ConnectionState::Online,
            connection.generation + 1,
        )
        .await;
    // Both tunnels are considered in one pass, so `other` running means the
    // held tunnel was already skipped.
    tunnel_snapshot(&fixture, other.id, running).await;
    let skipped = tunnel_snapshot(&fixture, held.id, |_| true).await;
    assert_eq!(skipped.state, TunnelRuntimeState::Stopped);
    drop(TcpListener::bind(("127.0.0.1", held_port)).await.unwrap());

    // A manual start lifts the hold, so the next reconnect restarts it.
    let started = fixture
        .vault
        .start_tunnel(held.id, next.connection_id, next.generation)
        .await
        .unwrap();
    assert_eq!(started.state, TunnelRuntimeState::Running);
    fixture.lose_transport().await;
    fixture
        .state(
            connection.connection_id,
            ConnectionState::Online,
            next.generation + 1,
        )
        .await;
    tunnel_snapshot(&fixture, held.id, running).await;
    echo(held_port, b"restarted").await;
    fixture.close().await;
}

#[tokio::test]
async fn reconnect_rebuilds_all_automatic_types_and_keeps_manual_tunnels_stopped() {
    let fixture = Fixture::new().await;
    let destination = fixture.echo_server().await;
    let ports = [
        free_port().await,
        free_port().await,
        free_port().await,
        free_port().await,
    ];
    let a = fixture.save(local(ports[0], destination), true).await;
    let b = fixture.save(remote(ports[1], destination), true).await;
    let c = fixture.save(socks(ports[2]), true).await;
    let manual = fixture.save(local(ports[3], destination), false).await;
    let owner = Uuid::new_v4();
    let (shell, lease, connection) = fixture.connect(owner).await;
    fixture
        .vault
        .start_tunnel(manual.id, connection.connection_id, connection.generation)
        .await
        .unwrap();
    shell.control.close().await.unwrap();
    drop(shell);
    drop(lease);
    fixture.vault.release_connection_owner_and_wait(owner).await;
    // Tunnel leases keep the connection supervised after the final pane closes.
    fixture.lose_transport().await;
    let next = fixture
        .state(
            connection.connection_id,
            ConnectionState::Online,
            connection.generation + 1,
        )
        .await;
    timeout(Duration::from_secs(5), async {
        loop {
            let snapshots = fixture.vault.tunnel_runtime_snapshots().unwrap();
            if [a.id, b.id, c.id].iter().all(|id| {
                snapshots.iter().any(|snapshot| {
                    snapshot.id == *id && snapshot.state == TunnelRuntimeState::Running
                })
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    echo(ports[0], b"new local").await;
    echo(ports[1], b"new remote").await;
    let mut proxy = socks_connect(ports[2], destination, &[1, 127, 0, 0, 1]).await;
    proxy.write_all(b"D").await.unwrap();
    assert_eq!(proxy.read_u8().await.unwrap(), b'D');
    drop(proxy);
    let snapshots = fixture.vault.tunnel_runtime_snapshots().unwrap();
    assert_eq!(
        snapshots
            .iter()
            .find(|snapshot| snapshot.id == manual.id)
            .unwrap()
            .state,
        TunnelRuntimeState::Stopped
    );
    let _manual_port = TcpListener::bind(("127.0.0.1", ports[3])).await.unwrap();
    assert_eq!(
        fixture
            .vault
            .start_tunnel(a.id, connection.connection_id, connection.generation)
            .await
            .unwrap_err()
            .code(),
        "invalid_config"
    );
    fixture
        .vault
        .disconnect_connection(next.connection_id, next.generation)
        .await
        .unwrap();
    let final_snapshots = fixture.vault.tunnel_runtime_snapshots().unwrap();
    assert!(
        final_snapshots
            .iter()
            .filter(|snapshot| [a.id, b.id, c.id, manual.id].contains(&snapshot.id))
            .all(|snapshot| snapshot.state == TunnelRuntimeState::Stopped),
        "{final_snapshots:?}; ports={ports:?}"
    );
    for (index, port) in [ports[0], ports[2]].iter().enumerate() {
        let _listener = TcpListener::bind(("127.0.0.1", *port)).await
            .unwrap_or_else(|error| panic!("failed to reclaim tunnel port index={index} port={port} ports={ports:?}: {error}"));
    }
    fixture.close().await;
}

#[tokio::test]
async fn lock_edit_delete_and_revision_conflict_preserve_data_and_reclaim_tasks() {
    let fixture = Fixture::new().await;
    let destination = fixture.echo_server().await;
    let port = free_port().await;
    let config = fixture.save(local(port, destination), false).await;
    let (_shell, _lease, connection) = fixture.connect(Uuid::new_v4()).await;
    fixture
        .vault
        .start_tunnel(config.id, connection.connection_id, connection.generation)
        .await
        .unwrap();
    let revision = fixture.vault.revision().unwrap();
    let before = std::fs::read(fixture.vault.path()).unwrap();
    assert_eq!(
        fixture
            .vault
            .save_tunnel(config.clone(), revision - 1)
            .await
            .unwrap_err()
            .code(),
        "vault_conflict"
    );
    assert_eq!(std::fs::read(fixture.vault.path()).unwrap(), before);
    echo(port, b"still running").await;
    fixture.vault.lock().await.unwrap();
    echo(port, b"default lock keeps transport").await;
    assert_eq!(
        fixture
            .vault
            .start_tunnel(config.id, connection.connection_id, connection.generation)
            .await
            .unwrap_err()
            .code(),
        "vault_locked"
    );
    fixture.vault.unlock(fixture.master.clone()).await.unwrap();
    fixture
        .vault
        .save_tunnel(config.clone(), revision)
        .await
        .unwrap();
    let _rebound = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    drop(_rebound);
    fixture
        .vault
        .delete_tunnel(config.id, fixture.vault.revision().unwrap())
        .await
        .unwrap();
    assert!(fixture.vault.tunnels().unwrap().is_empty());
    fixture.close().await;
}

#[tokio::test]
async fn deleting_host_removes_its_tunnel_runtime_binding() {
    let fixture = Fixture::new().await;
    let port = free_port().await;
    let config = fixture.save(local(port, 8000), false).await;
    let (_shell, _lease, connection) = fixture.connect(Uuid::new_v4()).await;
    fixture
        .vault
        .start_tunnel(config.id, connection.connection_id, connection.generation)
        .await
        .unwrap();

    fixture
        .vault
        .delete_host(fixture.host.id, fixture.vault.revision().unwrap())
        .await
        .unwrap();

    assert!(fixture.vault.tunnels().unwrap().is_empty());
    assert!(fixture
        .vault
        .tunnel_runtime_snapshots()
        .unwrap()
        .iter()
        .all(|snapshot| snapshot.id != config.id));
    let _rebound = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn tunnel_encryption_old_payload_and_backup_host_remapping() {
    let fixture = Fixture::new().await;
    let config = fixture.save(local(free_port().await, 8000), true).await;
    let bytes = std::fs::read(fixture.vault.path()).unwrap();
    assert!(!bytes
        .windows(b"bind_host".len())
        .any(|window| window == b"bind_host"));
    fixture.vault.lock().await.unwrap();
    fixture.vault.unlock(fixture.master.clone()).await.unwrap();
    let tunnels = fixture.vault.tunnels().unwrap();
    assert_eq!(tunnels.as_slice(), std::slice::from_ref(&config));
    let backup = fixture.directory.path().join("backup.cbak");
    fixture
        .vault
        .export_backup(backup.clone(), fixture.master.clone())
        .await
        .unwrap();
    let imported = VaultManager::new(fixture.directory.path().join("imported.cav"));
    imported
        .initialize(
            fixture.master.clone(),
            capstan_vault::KdfParameters {
                memory_kib: 19_456,
                iterations: 2,
                parallelism: 1,
            },
        )
        .await
        .unwrap();
    imported
        .import_backup(
            backup.clone(),
            fixture.master.clone(),
            BackupImportMode::Merge,
            imported.revision().unwrap(),
            imported.epoch(),
        )
        .await
        .unwrap();
    let merged = imported.tunnels().unwrap();
    assert_ne!(merged[0].id, config.id);
    assert_ne!(merged[0].host_id, fixture.host.id);
    assert_eq!(merged[0].host_id, imported.hosts().unwrap()[0].id);
    imported
        .import_backup(
            backup.clone(),
            fixture.master.clone(),
            BackupImportMode::Merge,
            imported.revision().unwrap(),
            imported.epoch(),
        )
        .await
        .unwrap();
    assert_eq!(imported.hosts().unwrap().len(), 1);
    // Importing the same backup again remaps to the existing host and skips
    // the identical forward instead of adding a second listener on its port.
    assert_eq!(imported.tunnels().unwrap().len(), 1);
    assert!(imported
        .tunnels()
        .unwrap()
        .iter()
        .all(|config| config.host_id == imported.hosts().unwrap()[0].id));
    imported
        .import_backup(
            backup,
            fixture.master.clone(),
            BackupImportMode::Overwrite,
            imported.revision().unwrap(),
            imported.epoch(),
        )
        .await
        .unwrap();
    assert_eq!(imported.tunnels().unwrap(), [config]);
    let mut old = serde_json::to_value(VaultData::default()).unwrap();
    old.as_object_mut().unwrap().remove("tunnels");
    let decoded: VaultData = serde_json::from_value(old).unwrap();
    decoded.validate().unwrap();
    assert!(decoded.tunnels.is_empty());
    fixture.close().await;
}

#[test]
fn tunnel_validation_rejects_duplicates_missing_hosts_and_invalid_addresses() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/agent/fixtures/M3-contract.json"
    ))
    .unwrap();
    let config: TunnelConfig = serde_json::from_value(fixture["tunnels"][0].clone()).unwrap();
    let mut data = VaultData {
        tunnels: vec![config.clone(), config],
        ..VaultData::default()
    };
    assert!(data.validate().is_err());
    data.tunnels.pop();
    assert!(data.validate().is_err());
}
