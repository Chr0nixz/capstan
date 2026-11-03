//! Explicit opt-in library probe; never compiled into a production application.
use russh::{client, keys::PublicKey};
use russh_sftp::{
    client::SftpSession,
    protocol::{FileAttributes, OpenFlags},
};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct PinnedHost {
    key: PublicKey,
    negotiated: Arc<Mutex<Option<serde_json::Value>>>,
}
impl client::Handler for PinnedHost {
    type Error = russh::Error;
    async fn check_server_key(&mut self, key: &PublicKey) -> Result<bool, Self::Error> {
        Ok(key.key_data() == self.key.key_data())
    }
    async fn kex_done(
        &mut self,
        _: Option<&[u8]>,
        names: &russh::Names,
        _: &mut client::Session,
    ) -> Result<(), Self::Error> {
        *self.negotiated.lock().unwrap() = Some(serde_json::json!({
            "kex": names.kex.as_ref(), "cipher": names.cipher.as_ref(),
            "clientMac": names.client_mac.as_ref(), "serverMac": names.server_mac.as_ref(),
        }));
        Ok(())
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One probe keeps setup, transfer, verification, and evidence emission together.
async fn configured_openssh_sftp_performance_probe() {
    if std::env::var("CAPSTAN_TEST_SFTP_PERFORMANCE").as_deref() != Ok("1") {
        eprintln!(
            "SKIP: CAPSTAN_TEST_SFTP_PERFORMANCE=1 is not configured (library performance probe)"
        );
        return;
    }
    let target = std::env::var("CAPSTAN_TEST_SSHD").unwrap();
    let root = std::env::var("CAPSTAN_TEST_SFTP_ROOT").unwrap();
    assert!(
        root.starts_with("/var/tmp/capstan-m1-ssh-")
            && root.ends_with("/home")
            && !root.contains("..")
    );
    let input = PathBuf::from(std::env::var_os("CAPSTAN_TEST_SFTP_INPUT").unwrap());
    let local = PathBuf::from(std::env::var_os("CAPSTAN_TEST_SFTP_OUTPUT").unwrap());
    // Both local paths come only from the explicit isolated probe runner.
    let bytes = std::fs::metadata(&input).unwrap().len();
    assert!(matches!(bytes, 67_108_864 | 1_073_741_824));
    // 64 MiB is an A1 high-RTT feasibility probe; product large-file
    // acceptance still requires exactly 1 GiB in every frozen scenario.
    let expected_hash = std::env::var("CAPSTAN_TEST_SFTP_SHA256")
        .unwrap()
        .to_ascii_lowercase();
    let direction = std::env::var("CAPSTAN_TEST_SFTP_DIRECTION").unwrap();
    assert!(direction == "upload" || direction == "download");
    let requests: usize =
        std::env::var("CAPSTAN_TEST_SFTP_REQUESTS").map_or(16, |value| value.parse().unwrap());
    assert!(matches!(requests, 16 | 32 | 64));
    let remote = format!("{root}/library-benchmark.bin");
    let mut password = zeroize::Zeroizing::new(String::new());
    std::fs::File::open(std::env::var_os("CAPSTAN_TEST_SSH_PASSWORD_FILE").unwrap())
        .unwrap()
        .take(8193)
        .read_to_string(&mut password)
        .unwrap();
    assert!(password.len() <= 8192);
    let key = PublicKey::from_openssh(
        &std::fs::read_to_string(std::env::var_os("CAPSTAN_TEST_SSH_HOST_KEY_FILE").unwrap())
            .unwrap(),
    )
    .unwrap();
    let (username, endpoint) = target.split_once('@').unwrap();
    let (address, port) = endpoint.rsplit_once(':').unwrap();
    let negotiated = Arc::new(Mutex::new(None));
    let total = Instant::now();
    tokio::time::timeout(Duration::from_mins(10), async {
        // Match the reference cipher/KEX and disable compression on both sides.
        let preferred = russh::Preferred {
            cipher: std::borrow::Cow::Borrowed(&[russh::cipher::AES_256_GCM]),
            kex: std::borrow::Cow::Borrowed(&[russh::kex::CURVE25519]),
            ..Default::default()
        };
        let mut ssh = client::connect(Arc::new(client::Config { nodelay: true, preferred, ..Default::default() }),
            (address, port.parse::<u16>().unwrap()), PinnedHost { key, negotiated: negotiated.clone() }).await.unwrap();
        assert!(ssh.authenticate_password(username, password.trim_end_matches(['\r', '\n'])).await.unwrap().success());
        let mut channel = ssh.channel_open_session().await.unwrap();
        channel.request_subsystem(true, "sftp").await.unwrap();
        loop {
            match channel.wait().await {
                Some(russh::ChannelMsg::Success) => break,
                Some(russh::ChannelMsg::WindowAdjusted { .. }) => {},
                _ => panic!("isolated server rejected SFTP"),
            }
        }
        let (stream, wire) = capstan_ssh::sftp::SftpWire::bridge(channel.into_stream());
        let session = SftpSession::new_with_config(stream, russh_sftp::client::Config {
            // The library, rather than a second pipeline, owns all in-flight I/O.
            max_concurrent_reads: requests, max_concurrent_writes: requests,
            max_packet_len: 32 * 1024 + 13, max_write_packet_len: 32 * 1024,
            request_timeout_secs: 15,
        }).await.unwrap();
        let prepared_ms = total.elapsed().as_secs_f64() * 1000.0;
        let transfer = Instant::now();
        let mut buffer = vec![0; 128 * 1024];
        let mut transferred = 0_u64;
        if direction == "upload" {
            let mut source = std::fs::File::open(&input).unwrap();
            let mut destination = session.open_with_flags_and_attributes(&remote,
                OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE,
                FileAttributes { permissions: Some(0o600), ..Default::default() }).await.unwrap();
            loop {
                let count = source.read(&mut buffer).unwrap();
                if count == 0 { break; }
                destination.write_all(&buffer[..count]).await.unwrap();
                transferred += count as u64;
            }
            destination.flush().await.unwrap();
            destination.close().await.unwrap();
        } else {
            let mut source = session.open(&remote).await.unwrap();
            let mut destination = std::fs::OpenOptions::new().write(true).create_new(true).open(&local).unwrap();
            loop {
                let count = source.read(&mut buffer).await.unwrap();
                if count == 0 { break; }
                destination.write_all(&buffer[..count]).unwrap();
                transferred += count as u64;
            }
            destination.flush().unwrap();
            source.close().await.unwrap();
        }
        let transfer_ms = transfer.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(transferred, bytes);
        assert_eq!(session.metadata(&remote).await.unwrap().len(), bytes);
        let verification = Instant::now();
        // Read back uploaded data using the selected library, without retaining a file-sized buffer.
        let mut hash = Sha256::new();
        if direction == "upload" {
            let mut source = session.open(&remote).await.unwrap();
            loop {
                let count = source.read(&mut buffer).await.unwrap();
                if count == 0 { break; }
                hash.update(&buffer[..count]);
            }
            source.close().await.unwrap();
        } else {
            let mut source = std::fs::File::open(&local).unwrap();
            loop {
                let count = source.read(&mut buffer).unwrap();
                if count == 0 { break; }
                hash.update(&buffer[..count]);
            }
        }
        let actual_hash = format!("{:x}", hash.finalize());
        assert_eq!(actual_hash, expected_hash);
        let verification_ms = verification.elapsed().as_secs_f64() * 1000.0;
        session.close().await.unwrap();
        drop(session);
        wire.close().await;
        ssh.disconnect(russh::Disconnect::ByApplication, "probe complete", "en").await.unwrap();
        eprintln!("SFTP_SAMPLE {}", serde_json::json!({
            "implementation": "russh-sftp-3.0.1-library-pipeline", "direction": direction,
            "bytes": bytes.to_string(), "prepareMs": prepared_ms, "transferMs": transfer_ms,
            "verificationMs": verification_ms, "totalMs": total.elapsed().as_secs_f64()*1000.0,
            "sha256": actual_hash, "hashMatched": true, "negotiated": negotiated.lock().unwrap().clone(),
            "concurrentRequestsPerFile": requests, "payloadLimitBytes": 32768,
            "productionAcceptance": false,
        }));
    }).await.expect("library performance probe exceeded its deadline");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn configured_openssh_shell_rtt_reference() {
    if std::env::var("CAPSTAN_TEST_SFTP_PERFORMANCE").as_deref() != Ok("1") {
        eprintln!("SKIP: CAPSTAN_TEST_SFTP_PERFORMANCE=1 is not configured (SSH RTT reference)");
        return;
    }
    let target = std::env::var("CAPSTAN_TEST_SSHD").unwrap();
    let (username, endpoint) = target.split_once('@').unwrap();
    let (address, port) = endpoint.rsplit_once(':').unwrap();
    assert_eq!(address, "127.0.0.1");
    let key = PublicKey::from_openssh(
        &std::fs::read_to_string(std::env::var_os("CAPSTAN_TEST_SSH_HOST_KEY_FILE").unwrap())
            .unwrap(),
    )
    .unwrap();
    let mut password = zeroize::Zeroizing::new(String::new());
    std::fs::File::open(std::env::var_os("CAPSTAN_TEST_SSH_PASSWORD_FILE").unwrap())
        .unwrap()
        .take(8193)
        .read_to_string(&mut password)
        .unwrap();
    assert!(password.len() <= 8192);
    let negotiated = Arc::new(Mutex::new(None));
    let config = client::Config {
        nodelay: true,
        preferred: russh::Preferred {
            cipher: std::borrow::Cow::Borrowed(&[russh::cipher::AES_256_GCM]),
            kex: std::borrow::Cow::Borrowed(&[russh::kex::CURVE25519]),
            ..Default::default()
        },
        ..Default::default()
    };
    tokio::time::timeout(Duration::from_mins(1), async {
        let mut ssh = client::connect(
            Arc::new(config),
            (address, port.parse::<u16>().unwrap()),
            PinnedHost { key, negotiated },
        )
        .await
        .unwrap();
        assert!(ssh
            .authenticate_password(username, password.trim_end_matches(['\r', '\n']))
            .await
            .unwrap()
            .success());
        let mut channel = ssh.channel_open_session().await.unwrap();
        // A test-only exec over the authenticated transport; no application
        // command exposes arbitrary execution. Only known nonce equality and
        // timings are recorded, never shell/terminal contents.
        channel.exec(true, "cat").await.unwrap();
        loop {
            match channel.wait().await {
                Some(russh::ChannelMsg::Success) => break,
                Some(russh::ChannelMsg::WindowAdjusted { .. }) => {}
                _ => panic!("isolated server rejected the RTT reference"),
            }
        }
        let mut stream = channel.into_stream();
        let mut samples = Vec::new();
        for sequence in 0_u32..22 {
            let message = format!("capstan-m2-rtt-{sequence:08}\n").into_bytes();
            let started = Instant::now();
            stream.write_all(&message).await.unwrap();
            let mut received = vec![0; message.len()];
            stream.read_exact(&mut received).await.unwrap();
            assert_eq!(received, message);
            if sequence >= 2 {
                samples.push(started.elapsed().as_secs_f64() * 1000.0);
            }
        }
        stream.shutdown().await.unwrap();
        drop(stream);
        ssh.disconnect(russh::Disconnect::ByApplication, "reference complete", "en")
            .await
            .unwrap();
        let mut ordered = samples.clone();
        ordered.sort_by(f64::total_cmp);
        eprintln!(
            "SSH_RTT_SAMPLE {}",
            serde_json::json!({
                "method": "authenticated SSH exec-cat nonce echo, two warmups then 20 samples",
                "samplesMs": samples, "medianMs": f64::midpoint(ordered[9], ordered[10]),
                "p95Ms": ordered[18], "maximumMs": ordered[19],
                "terminalRenderingMeasured": false, "productionAcceptance": false,
            })
        );
    })
    .await
    .expect("SSH RTT reference exceeded its deadline");
}
