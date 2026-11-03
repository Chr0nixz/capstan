//! M2-01 protocol probes. These are not desktop or OpenSSH acceptance tests.
#![allow(clippy::pedantic)]

use russh_sftp::{
    client::{error::Error, Config, RawSftpSession},
    protocol::{FileAttributes, FileType, OpenFlags, StatusCode},
};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream};

fn client() -> (RawSftpSession, DuplexStream) {
    let (client, server) = tokio::io::duplex(64 * 1024);
    (
        RawSftpSession::new_with_config(
            client,
            Config {
                max_packet_len: 64 * 1024,
                max_concurrent_reads: 16,
                max_concurrent_writes: 16,
                max_write_packet_len: 32 * 1024,
                request_timeout_secs: 1,
            },
        ),
        server,
    )
}

async fn receive<S: AsyncRead + Unpin>(stream: &mut S) -> Vec<u8> {
    let length = stream.read_u32().await.unwrap() as usize;
    assert!(length <= 64 * 1024);
    let mut packet = vec![0; length];
    stream.read_exact(&mut packet).await.unwrap();
    packet
}

async fn send<S: AsyncWrite + Unpin>(stream: &mut S, packet: &[u8]) {
    stream.write_u32(packet.len() as u32).await.unwrap();
    stream.write_all(packet).await.unwrap();
}

async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S) {
    assert_eq!(receive(stream).await, [1, 0, 0, 0, 3]);
    send(stream, &[2, 0, 0, 0, 3]).await;
}

fn id(packet: &[u8]) -> u32 {
    u32::from_be_bytes(packet[1..5].try_into().unwrap())
}

fn string(packet: &mut Vec<u8>, value: &[u8]) {
    packet.extend_from_slice(&(value.len() as u32).to_be_bytes());
    packet.extend_from_slice(value);
}

fn attrs(request: u32, size: u64) -> Vec<u8> {
    let mut packet = vec![105];
    packet.extend_from_slice(&request.to_be_bytes());
    packet.extend_from_slice(&1_u32.to_be_bytes());
    packet.extend_from_slice(&size.to_be_bytes());
    packet
}

fn status(request: u32, code: u32) -> Vec<u8> {
    let mut packet = vec![101];
    packet.extend_from_slice(&request.to_be_bytes());
    packet.extend_from_slice(&code.to_be_bytes());
    string(&mut packet, b"untrusted-server-message");
    string(&mut packet, b"en");
    packet
}

#[tokio::test]
async fn version_three_extensions_and_full_u64_attributes_are_preserved() {
    let (client, mut server) = client();
    let task = tokio::spawn(async move {
        receive(&mut server).await;
        let mut version = vec![2, 0, 0, 0, 3];
        string(&mut version, b"posix-rename@openssh.com");
        string(&mut version, b"1");
        send(&mut server, &version).await;
        let request = receive(&mut server).await;
        send(&mut server, &attrs(id(&request), u64::MAX)).await;
        server
    });
    let version = client.init().await.unwrap();
    assert_eq!(version.version, 3);
    assert_eq!(
        version
            .extensions
            .get("posix-rename@openssh.com")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        client.lstat("/test").await.unwrap().attrs.size,
        Some(u64::MAX)
    );
    drop(task.await.unwrap());
}

#[tokio::test]
async fn out_of_order_unknown_and_duplicate_replies_do_not_cross_deliver() {
    let (client, mut server) = client();
    let task = tokio::spawn(async move {
        handshake(&mut server).await;
        let first = id(&receive(&mut server).await);
        let second = id(&receive(&mut server).await);
        assert_ne!(first, second);
        send(&mut server, &attrs(u32::MAX, 99)).await;
        send(&mut server, &attrs(second, 2)).await;
        send(&mut server, &attrs(second, 99)).await;
        send(&mut server, &attrs(first, 1)).await;
        server
    });
    client.init().await.unwrap();
    let (first, second) = tokio::join!(client.lstat("/one"), client.lstat("/two"));
    assert_eq!(first.unwrap().attrs.size, Some(1));
    assert_eq!(second.unwrap().attrs.size, Some(2));
    drop(task.await.unwrap());
}

#[tokio::test]
async fn permission_denied_and_missing_extension_remain_distinct_protocol_statuses() {
    let (client, mut server) = client();
    let task = tokio::spawn(async move {
        handshake(&mut server).await;
        for code in [3, 8] {
            let request = receive(&mut server).await;
            send(&mut server, &status(id(&request), code)).await;
        }
        server
    });
    assert!(client.init().await.unwrap().extensions.is_empty());
    for code in [StatusCode::PermissionDenied, StatusCode::OpUnsupported] {
        match client.lstat("/test").await.unwrap_err() {
            Error::Status(status) => assert_eq!(status.status_code, code),
            error => panic!("unexpected error category: {error:?}"),
        }
    }
    drop(task.await.unwrap());
}

#[tokio::test]
async fn dropped_request_late_reply_does_not_complete_the_next_request() {
    let (client, mut server) = client();
    let (first_tx, first_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        handshake(&mut server).await;
        let first = id(&receive(&mut server).await);
        first_tx.send(()).unwrap();
        let second = id(&receive(&mut server).await);
        send(&mut server, &attrs(first, 99)).await;
        send(&mut server, &attrs(second, 2)).await;
        server
    });
    client.init().await.unwrap();
    let mut first = Box::pin(client.lstat("/one"));
    tokio::select! {
        _ = first_rx => {}
        _ = &mut first => panic!("request unexpectedly completed"),
    }
    drop(first);
    assert_eq!(client.lstat("/two").await.unwrap().attrs.size, Some(2));
    drop(task.await.unwrap());
}

#[tokio::test]
async fn stalled_request_has_a_bounded_timeout() {
    let (client, mut server) = client();
    let task = tokio::spawn(async move {
        handshake(&mut server).await;
        receive(&mut server).await;
        std::future::pending::<()>().await;
    });
    client.init().await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(3), client.lstat("/stall"))
        .await
        .unwrap();
    assert!(matches!(result, Err(Error::Timeout)));
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn eof_wakes_an_in_flight_request_before_its_timeout() {
    let (client, mut server) = client();
    let task = tokio::spawn(async move {
        handshake(&mut server).await;
        receive(&mut server).await;
    });
    client.init().await.unwrap();
    let result = tokio::time::timeout(Duration::from_millis(500), client.lstat("/eof"))
        .await
        .unwrap();
    assert!(result.is_err());
    task.await.unwrap();
}

#[tokio::test]
async fn oversized_and_truncated_packets_terminate_the_sftp_stream() {
    for malicious in [u32::MAX.to_be_bytes().to_vec(), vec![0, 0, 0, 5, 105]] {
        let (client, mut server) = client();
        let task = tokio::spawn(async move {
            handshake(&mut server).await;
            receive(&mut server).await;
            server.write_all(&malicious).await.unwrap();
        });
        client.init().await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(500), client.lstat("/malformed"))
                .await
                .unwrap()
                .is_err()
        );
        task.await.unwrap();
    }
}

#[tokio::test]
async fn closing_the_client_shuts_down_its_owned_stream() {
    let (client, mut server) = client();
    let task = tokio::spawn(async move {
        handshake(&mut server).await;
        let mut byte = [0];
        assert_eq!(server.read(&mut byte).await.unwrap(), 0);
    });
    client.init().await.unwrap();
    client.close_session().unwrap();
    drop(client);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn probe_non_utf8_names_are_lossy_in_the_selected_library() {
    let (client, mut server) = client();
    let task = tokio::spawn(async move {
        handshake(&mut server).await;
        let request = receive(&mut server).await;
        let mut packet = vec![104];
        packet.extend_from_slice(&id(&request).to_be_bytes());
        packet.extend_from_slice(&2_u32.to_be_bytes());
        for name in [b"bad\xff".as_slice(), b"bad\xfe".as_slice()] {
            string(&mut packet, name);
            string(&mut packet, b"");
            packet.extend_from_slice(&0_u32.to_be_bytes());
        }
        send(&mut server, &packet).await;
        server
    });
    client.init().await.unwrap();
    let response = client.readdir("probe").await.unwrap();
    // Characterization of a release blocker: distinct remote names collapse.
    assert_eq!(response.files[0].filename, "bad\u{fffd}");
    assert_eq!(response.files[0].filename, response.files[1].filename);
    drop(task.await.unwrap());
}

#[tokio::test]
async fn probe_opaque_binary_handles_are_lossy_in_the_selected_library() {
    let (client, mut server) = client();
    let task = tokio::spawn(async move {
        handshake(&mut server).await;
        let request = receive(&mut server).await;
        let mut packet = vec![102];
        packet.extend_from_slice(&id(&request).to_be_bytes());
        string(&mut packet, &[0, 0xff, 0x80, 1]);
        send(&mut server, &packet).await;
        server
    });
    client.init().await.unwrap();
    let response = client.opendir("/").await.unwrap();
    assert_ne!(response.handle.as_bytes(), &[0, 0xff, 0x80, 1]);
    assert!(response.handle.contains('\u{fffd}'));
    drop(task.await.unwrap());
}

#[tokio::test]
async fn bounded_adapter_round_trips_every_opaque_handle_byte_and_preserves_file_data() {
    let (stream, mut server) = tokio::io::duplex(64 * 1024);
    let (stream, wire) = capstan_ssh::sftp::SftpWire::bridge(stream);
    let client = RawSftpSession::new(stream);
    let task = tokio::spawn(async move {
        handshake(&mut server).await;
        let handle: Vec<u8> = (0..=255).collect();
        let request = receive(&mut server).await;
        let mut packet = vec![102];
        packet.extend_from_slice(&id(&request).to_be_bytes());
        string(&mut packet, &handle);
        send(&mut server, &packet).await;
        for kind in [5, 6, 4] {
            let request = receive(&mut server).await;
            assert_eq!(request[0], kind);
            let length = u32::from_be_bytes(request[5..9].try_into().unwrap()) as usize;
            assert_eq!(length, 256);
            assert_eq!(request[9..9 + length], handle);
            if kind == 5 {
                let mut data = vec![103];
                data.extend_from_slice(&id(&request).to_be_bytes());
                string(&mut data, &[255, 128, 0, 1]);
                send(&mut server, &data).await;
            } else {
                if kind == 6 {
                    assert_eq!(&request[request.len() - 4..], &[255, 128, 0, 1]);
                }
                send(&mut server, &status(id(&request), 0)).await;
            }
        }
        server
    });
    client.init().await.unwrap();
    let handle = client
        .open(
            "/test",
            OpenFlags::READ | OpenFlags::WRITE,
            FileAttributes::default(),
        )
        .await
        .unwrap()
        .handle;
    assert_eq!(handle.len(), 512);
    assert_eq!(
        client.read(handle.clone(), 0, 4).await.unwrap().data,
        [255, 128, 0, 1]
    );
    client
        .write(handle.clone(), 0, vec![255, 128, 0, 1])
        .await
        .unwrap();
    client.close(handle).await.unwrap();
    let server = task.await.unwrap();
    drop(client);
    wire.close().await;
    assert!(wire.is_closed());
    drop(server);
}

#[tokio::test]
async fn bounded_adapter_removes_untrusted_error_text_and_joins_both_tasks() {
    let (stream, mut server) = tokio::io::duplex(64 * 1024);
    let (stream, wire) = capstan_ssh::sftp::SftpWire::bridge(stream);
    let client = RawSftpSession::new(stream);
    let task = tokio::spawn(async move {
        handshake(&mut server).await;
        let request = receive(&mut server).await;
        send(&mut server, &status(id(&request), 3)).await;
        server
    });
    client.init().await.unwrap();
    let error = client.lstat("/test").await.unwrap_err();
    match error {
        Error::Status(status) => {
            assert_eq!(status.status_code, StatusCode::PermissionDenied);
            assert!(status.error_message.is_empty());
            assert!(status.language_tag.is_empty());
        }
        other => panic!("unexpected error: {other:?}"),
    }
    drop(task.await.unwrap());
    wire.close().await;
    assert!(client.lstat("/closed").await.is_err());
}

#[tokio::test]
async fn bounded_adapter_rejects_oversized_prefix_without_waiting_for_a_body() {
    let (stream, mut server) = tokio::io::duplex(64);
    let (mut stream, wire) = capstan_ssh::sftp::SftpWire::bridge(stream);
    server.write_u32(u32::MAX).await.unwrap();
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), stream.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    wire.close().await;
}

#[tokio::test]
async fn bounded_adapter_cancel_reclaims_tasks_while_a_slow_consumer_blocks_output() {
    let (stream, mut server) = tokio::io::duplex(64 * 1024);
    let (stream, wire) = capstan_ssh::sftp::SftpWire::bridge(stream);
    let writer = tokio::spawn(async move {
        let mut data = vec![103, 0, 0, 0, 1];
        string(&mut data, &vec![0; 128 * 1024]);
        // More data than either pipe can hold, while the client consumes none.
        send(&mut server, &data).await;
        server
    });
    tokio::task::yield_now().await;
    tokio::time::timeout(Duration::from_secs(1), wire.close())
        .await
        .unwrap();
    drop(stream);
    writer.abort();
    let _ = writer.await;
}

#[test]
fn probe_device_type_helpers_cannot_be_used_for_recursive_traversal() {
    let block_device = FileAttributes {
        permissions: Some(0o060_600),
        ..FileAttributes::default()
    };
    assert!(
        block_device.is_dir(),
        "upstream behavior changed: reassess the adapter"
    );
    assert_eq!(block_device.file_type(), FileType::Other);
}

#[derive(Clone)]
struct ProbeServer {
    password: std::sync::Arc<zeroize::Zeroizing<String>>,
    authentications: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    subsystems: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    channels: std::collections::HashMap<
        russh::ChannelId,
        std::sync::Arc<tokio::sync::Mutex<Option<russh::Channel<russh::server::Msg>>>>,
    >,
    tasks: tokio_util::task::TaskTracker,
    cancel: tokio_util::sync::CancellationToken,
}

impl russh::server::Server for ProbeServer {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        let mut handler = self.clone();
        handler.channels.clear();
        handler
    }
}

impl russh::server::Handler for ProbeServer {
    type Error = russh::Error;

    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<russh::server::Auth, Self::Error> {
        self.authentications
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(if user == "probe" && password == self.password.as_str() {
            russh::server::Auth::Accept
        } else {
            russh::server::Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        channel: russh::Channel<russh::server::Msg>,
        _: &mut russh::server::Session,
    ) -> Result<bool, Self::Error> {
        self.channels.insert(
            channel.id(),
            std::sync::Arc::new(tokio::sync::Mutex::new(Some(channel))),
        );
        Ok(true)
    }

    async fn shell_request(
        &mut self,
        channel: russh::ChannelId,
        session: &mut russh::server::Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)
    }

    async fn data(
        &mut self,
        channel: russh::ChannelId,
        data: &[u8],
        session: &mut russh::server::Session,
    ) -> Result<(), Self::Error> {
        // SFTP's stream owns its data; shell data is echoed through the handler.
        if self.channels.contains_key(&channel) {
            session.data(channel, data.to_vec())?;
        }
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel: russh::ChannelId,
        name: &str,
        session: &mut russh::server::Session,
    ) -> Result<(), Self::Error> {
        if name != "sftp" {
            return session.channel_failure(channel);
        }
        self.subsystems
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let owned = self
            .channels
            .remove(&channel)
            .ok_or(russh::Error::Inconsistent)?;
        let owned = owned
            .lock()
            .await
            .take()
            .ok_or(russh::Error::Inconsistent)?;
        session.channel_success(channel)?;
        let cancel = self.cancel.clone();
        self.tasks.spawn(async move {
            let mut stream = owned.into_stream();
            tokio::select! {
                () = cancel.cancelled() => {}
                () = async {
                    handshake(&mut stream).await;
                    // Deliberately stall metadata requests while adjacent shells run.
                    receive(&mut stream).await;
                    std::future::pending::<()>().await;
                } => {}
            }
        });
        Ok(())
    }
}

struct PinnedHostKey(russh::keys::PublicKey);
impl russh::client::Handler for PinnedHostKey {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(key.key_data() == self.0.key_data())
    }
}

#[tokio::test]
async fn vendored_russh_two_shells_and_stalled_sftp_share_one_authenticated_transport() {
    use russh::server::Server as _;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    let key =
        russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519).unwrap();
    let public_key = key.public_key().clone();
    let password = Arc::new(zeroize::Zeroizing::new(uuid::Uuid::new_v4().to_string()));
    let authentications = Arc::new(AtomicUsize::new(0));
    let subsystems = Arc::new(AtomicUsize::new(0));
    let tasks = tokio_util::task::TaskTracker::new();
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut server = ProbeServer {
        password: password.clone(),
        authentications: authentications.clone(),
        subsystems: subsystems.clone(),
        channels: std::collections::HashMap::new(),
        tasks: tasks.clone(),
        cancel: cancel.clone(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = Arc::new(russh::server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let server_task = tokio::spawn(async move { server.run_on_socket(config, &listener).await });
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let mut ssh = russh::client::connect(
            Arc::new(russh::client::Config::default()),
            address,
            PinnedHostKey(public_key),
        )
        .await
        .unwrap();
        assert!(ssh
            .authenticate_password("probe", password.as_str())
            .await
            .unwrap()
            .success());
        let mut first = ssh.channel_open_session().await.unwrap();
        let mut second = ssh.channel_open_session().await.unwrap();
        first.request_shell(true).await.unwrap();
        second.request_shell(true).await.unwrap();
        assert!(matches!(
            first.wait().await,
            Some(russh::ChannelMsg::Success)
        ));
        assert!(matches!(
            second.wait().await,
            Some(russh::ChannelMsg::Success)
        ));
        let mut channel = ssh.channel_open_session().await.unwrap();
        channel.request_subsystem(true, "sftp").await.unwrap();
        assert!(matches!(
            channel.wait().await,
            Some(russh::ChannelMsg::Success)
        ));
        let sftp = RawSftpSession::new_with_config(
            channel.into_stream(),
            Config {
                request_timeout_secs: 1,
                ..Default::default()
            },
        );
        sftp.init().await.unwrap();
        let (metadata, echo) = tokio::join!(sftp.lstat("/stalled"), async {
            second.data(&b"shell-still-responsive"[..]).await.unwrap();
            loop {
                if let Some(russh::ChannelMsg::Data { data }) = second.wait().await {
                    assert_eq!(&data[..], b"shell-still-responsive");
                    break;
                }
            }
        });
        assert!(matches!(metadata, Err(Error::Timeout)));
        assert_eq!(echo, ());
        assert_eq!(authentications.load(Ordering::SeqCst), 1);
        assert_eq!(subsystems.load(Ordering::SeqCst), 1);
        first.close().await.unwrap();
        sftp.close_session().unwrap();
        second.data(&b"after-sftp-close"[..]).await.unwrap();
        loop {
            if let Some(russh::ChannelMsg::Data { data }) = second.wait().await {
                assert_eq!(&data[..], b"after-sftp-close");
                break;
            }
        }
        ssh.disconnect(russh::Disconnect::ByApplication, "probe complete", "en")
            .await
            .unwrap();
    })
    .await;
    cancel.cancel();
    tasks.close();
    tasks.wait().await;
    server_task.abort();
    let _ = server_task.await;
    assert!(result.is_ok(), "in-process probe timed out");
}

#[tokio::test]
async fn configured_openssh_sftp_probe() {
    use std::{io::Read, sync::Arc};
    let Ok(target) = std::env::var("CAPSTAN_TEST_SSHD") else {
        eprintln!("SKIP: CAPSTAN_TEST_SSHD is not configured (real SFTP probe)");
        return;
    };
    // All write probes require a dedicated fixture, never an ordinary SSH home.
    let root =
        std::env::var("CAPSTAN_TEST_SFTP_ROOT").expect("configure an isolated SFTP fixture root");
    assert!(
        root.starts_with("/var/tmp/capstan-m1-ssh-")
            && root.ends_with("/home")
            && !root.contains("..")
    );
    let password_file = std::env::var_os("CAPSTAN_TEST_SSH_PASSWORD_FILE")
        .expect("configure the fixture password file");
    let key_file = std::env::var_os("CAPSTAN_TEST_SSH_HOST_KEY_FILE")
        .expect("configure the pinned fixture host public key");
    let mut password = zeroize::Zeroizing::new(String::new());
    std::fs::File::open(password_file)
        .unwrap()
        .take(8193)
        .read_to_string(&mut password)
        .unwrap();
    assert!(password.len() <= 8192);
    let password = password.trim_end_matches(['\r', '\n']);
    let key =
        russh::keys::PublicKey::from_openssh(&std::fs::read_to_string(key_file).unwrap()).unwrap();
    let (username, endpoint) = target.split_once('@').expect("expected user@host:port");
    let (address, port) = endpoint.rsplit_once(':').expect("expected host:port");
    let address = address.trim_start_matches('[').trim_end_matches(']');
    let port = port.parse::<u16>().unwrap();
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut ssh = russh::client::connect(Arc::new(russh::client::Config { nodelay: true, ..Default::default() }), (address, port), PinnedHostKey(key)).await.unwrap();
        assert!(ssh.authenticate_password(username, password).await.unwrap().success());
        let mut channel = ssh.channel_open_session().await.unwrap();
        channel.request_subsystem(true, "sftp").await.unwrap();
        loop {
            match channel.wait().await {
                Some(russh::ChannelMsg::Success) => break,
                Some(russh::ChannelMsg::WindowAdjusted { .. }) => {}
                _ => panic!("the isolated server rejected the SFTP subsystem"),
            }
        }
        let (stream, wire) = capstan_ssh::sftp::SftpWire::bridge(channel.into_stream());
        let client = RawSftpSession::new_with_config(stream, Config { max_packet_len: 256 * 1024, request_timeout_secs: 15, ..Default::default() });
        let version = client.init().await.unwrap();
        assert_eq!(version.version, 3);
        let canonical = client.realpath(".").await.unwrap();
        assert_eq!(canonical.files.len(), 1);
        assert_eq!(canonical.files[0].filename, root);
        let directory = client.opendir(root.clone()).await.unwrap().handle;
        let page = client.readdir(directory.clone()).await.unwrap();
        assert!(page.files.iter().any(|entry| entry.filename == "."));
        client.close(directory).await.unwrap();
        let path = format!("{root}/capstan-probe-{}", uuid::Uuid::new_v4());
        let handle = client.open(path.clone(), OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE, FileAttributes { permissions: Some(0o600), ..Default::default() }).await.unwrap().handle;
        let mut chunk = vec![0; 32 * 1024];
        for (index, byte) in chunk.iter_mut().enumerate() { *byte = (index.wrapping_mul(131).wrapping_add(index / 256) % 256) as u8; }
        for offset in (0..1024 * 1024).step_by(chunk.len()) {
            client.write(handle.clone(), offset as u64, chunk.clone()).await.unwrap();
        }
        client.close(handle).await.unwrap();
        assert_eq!(client.lstat(path.clone()).await.unwrap().attrs.size, Some(1024 * 1024));
        let handle = client.open(path.clone(), OpenFlags::READ, FileAttributes::default()).await.unwrap().handle;
        for offset in (0..1024 * 1024).step_by(chunk.len()) {
            // Short reads are valid; consume the remaining range before advancing.
            let mut read = 0;
            while read < chunk.len() {
                let result = client.read(handle.clone(), (offset + read) as u64, (chunk.len() - read) as u32).await.unwrap();
                assert!(!result.data.is_empty() && result.data.len() <= chunk.len() - read);
                assert_eq!(result.data, chunk[read..read + result.data.len()]);
                read += result.data.len();
            }
        }
        client.close(handle).await.unwrap();
        client.remove(path).await.unwrap();
        eprintln!("SFTP_PROBE {}", serde_json::json!({
            "version": version.version,
            "posixRenameAdvertised": version.extensions.get("posix-rename@openssh.com").is_some_and(|value| value == "1"),
            "fsyncAdvertised": version.extensions.get("fsync@openssh.com").is_some_and(|value| value == "1"),
            "limitsAdvertised": version.extensions.get("limits@openssh.com").is_some_and(|value| value == "1"),
            "roundTripBytes": "1048576", "comparison": "complete_byte_equality", "performanceAcceptance": false
        }));
        client.close_session().unwrap();
        drop(client);
        wire.close().await;
        ssh.disconnect(russh::Disconnect::ByApplication, "probe complete", "en").await.unwrap();
    }).await.expect("real SFTP probe timed out");
}
