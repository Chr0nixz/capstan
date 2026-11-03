//! Exercise all three forwarding types on the already authenticated OpenSSH transport.
use super::{exercise_shell, print_marker, TestTarget};
use capstan_ssh::{
    forwarding::TunnelRuntimeState,
    forwarding_runtime::{ForwardSpec, TunnelRuntime},
    ConnectionPool, PasswordConnection, PoolKey,
};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::broadcast,
    time::timeout,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use uuid::Uuid;

pub(super) async fn exercise(
    target: &TestTarget,
    connection: PasswordConnection,
) -> Result<(), String> {
    let pool = ConnectionPool::new(Duration::from_secs(30));
    // This test-local pool only wraps the connection whose ephemeral trust was
    // verified by the caller. Its factory cannot establish another transport.
    let lease = pool
        .acquire(
            PoolKey {
                target: target.address.clone(),
                port: target.port,
                username: target.username.clone(),
                credential_id: Uuid::new_v4(),
                credential_revision: 1,
                trust_fingerprint: "verified-ephemeral-test-trust".into(),
                vault_epoch: 1,
                agent_identity: None,
                agent_forwarding: false,
                proxy_path: String::new(),
            },
            CancellationToken::new(),
            move |_| async move { Ok(connection) },
        )
        .await
        .map_err(|error| format!("could not retain the test connection ({})", error.code()))?;
    let stop = CancellationToken::new();
    let tasks = TaskTracker::new();
    let mut tunnels = Vec::new();
    let result = timeout(Duration::from_secs(45), async {
        let echo = TcpListener::bind("127.0.0.1:0").await.map_err(io_error)?;
        let destination = echo.local_addr().map_err(io_error)?.port();
        // Reserve distinct candidate ports even when the server is on this
        // machine. A remote collision must fail rather than reuse a listener.
        let remote = TcpListener::bind("127.0.0.1:0").await.map_err(io_error)?;
        let local = TcpListener::bind("127.0.0.1:0").await.map_err(io_error)?;
        let socks = TcpListener::bind("127.0.0.1:0").await.map_err(io_error)?;
        let remote_port = remote.local_addr().map_err(io_error)?.port();
        let local_port = local.local_addr().map_err(io_error)?.port();
        let socks_port = socks.local_addr().map_err(io_error)?.port();
        drop((remote, local, socks));
        start_echo(echo, stop.clone(), tasks.clone());
        let (events, _) = broadcast::channel(32);
        for spec in [
            ForwardSpec::Remote {
                remote_bind_host: "127.0.0.1".into(),
                remote_bind_port: remote_port,
                local_destination_host: "127.0.0.1".into(),
                local_destination_port: destination,
            },
            ForwardSpec::Local {
                bind_host: "127.0.0.1".into(),
                bind_port: local_port,
                destination_host: "127.0.0.1".into(),
                destination_port: remote_port,
            },
            ForwardSpec::Socks5 {
                bind_host: "127.0.0.1".into(),
                bind_port: socks_port,
            },
        ] {
            tunnels.push(TunnelRuntime::spawn(
                Uuid::new_v4(),
                spec,
                lease.clone(),
                events.clone(),
            ));
            let snapshot = tunnels.last().unwrap().started().await;
            if snapshot.state != TunnelRuntimeState::Running {
                return Err(format!(
                    "OpenSSH forwarding setup failed: {:?}",
                    snapshot.error_code
                ));
            }
        }
        // Both paths traverse OpenSSH's remote listener and return to our echo
        // server through forwarded-tcpip. No remote process or file is needed.
        let payload = [0, 0x7f, 0x80, 0xff, b'\r', b'\n'].repeat(8192);
        let stream = TcpStream::connect(("127.0.0.1", local_port))
            .await
            .map_err(io_error)?;
        round_trip(stream, &payload).await?;
        let stopped = tunnels[1].stop().await;
        if stopped.state != TunnelRuntimeState::Stopped
            || stopped.active_connections != 0
            || stopped.bytes_sent != payload.len() as u64
            || stopped.bytes_received != payload.len() as u64
        {
            return Err("local forwarding did not drain and stop with exact counters".into());
        }
        let _released = TcpListener::bind(("127.0.0.1", local_port))
            .await
            .map_err(io_error)?;
        let mut stream = TcpStream::connect(("127.0.0.1", socks_port))
            .await
            .map_err(io_error)?;
        stream.write_all(&[5, 1, 0]).await.map_err(io_error)?;
        let mut auth = [0; 2];
        stream.read_exact(&mut auth).await.map_err(io_error)?;
        if auth != [5, 0] {
            return Err("SOCKS5 did not accept the no-authentication method".into());
        }
        let mut request = b"\x05\x01\x00\x03\x09localhost".to_vec();
        request.extend_from_slice(&remote_port.to_be_bytes());
        stream.write_all(&request).await.map_err(io_error)?;
        let mut reply = [0; 10];
        stream.read_exact(&mut reply).await.map_err(io_error)?;
        if reply[..4] != [5, 0, 0, 1] {
            return Err("SOCKS5 domain CONNECT was rejected".into());
        }
        round_trip(stream, &payload).await?;
        // Stopping -L must leave -R, -D and the shell on the same transport alive.
        let mut shell = lease
            .connection()
            .open_shell(80, 24)
            .await
            .map_err(|error| format!("shell after tunnel stop failed ({})", error.code()))?;
        if shell.connection_id() != lease.connection().connection_id() {
            return Err("forwarding isolation shell used a different transport".into());
        }
        let marker = format!("CAPSTAN_M3_FORWARDING_{}", Uuid::new_v4().simple());
        let output = exercise_shell(
            &mut shell,
            &format!("unset HISTFILE; {}; exit 0", print_marker(&marker)),
        )
        .await?;
        if !super::contains_output_line(&output, &marker) {
            return Err("stopping a tunnel interrupted the shared SSH shell".into());
        }
        Ok(())
    })
    .await
    .unwrap_or_else(|_| Err("OpenSSH forwarding probe timed out".into()));

    let mut cleanup: Result<(), String> = Ok(());
    for tunnel in tunnels.iter().rev() {
        let snapshot = tunnel.stop().await;
        if snapshot.state != TunnelRuntimeState::Stopped || snapshot.active_connections != 0 {
            cleanup = Err("OpenSSH forwarding cleanup was not acknowledged".into());
        }
    }
    lease.connection().disconnect().await;
    stop.cancel();
    tasks.close();
    tasks.wait().await;
    result?;
    cleanup?;
    println!("OPENSSH_M3_FORWARDING local=ok remote=ok socks5_domain=ok half_close=ok stop_isolation=ok cleanup=ok");
    Ok(())
}

fn io_error(_: std::io::Error) -> String {
    "OpenSSH forwarding probe TCP I/O failed".into()
}

async fn round_trip(mut stream: TcpStream, payload: &[u8]) -> Result<(), String> {
    stream.write_all(payload).await.map_err(io_error)?;
    stream.shutdown().await.map_err(io_error)?;
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply).await.map_err(io_error)?;
    if reply != payload {
        return Err("OpenSSH forwarding changed or lost bytes across half-close".into());
    }
    Ok(())
}

fn start_echo(listener: TcpListener, stop: CancellationToken, tasks: TaskTracker) {
    let clients = tasks.clone();
    tasks.spawn(async move {
        loop {
            let accepted = tokio::select! {
                biased;
                () = stop.cancelled() => break,
                result = listener.accept() => result,
            };
            let Ok((mut stream, _)) = accepted else {
                break;
            };
            let stop = stop.clone();
            clients.spawn(async move {
                let (mut read, mut write) = stream.split();
                tokio::select! {
                    biased;
                    () = stop.cancelled() => {},
                    _ = async {
                        let _ = tokio::io::copy(&mut read, &mut write).await;
                        let _ = write.shutdown().await;
                    } => {},
                }
            });
        }
    });
}
