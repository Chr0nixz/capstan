//! Explicit fixture-only measurements of the production pool/service/native path.
use super::*;
use capstan_core::transfer::{TransferService, TransferState, TransferTarget, WindowsDestination};
use sha2::{Digest, Sha256};
use std::{io::Read, path::PathBuf, time::Instant};

#[path = "performance/matrix.rs"]
mod matrix;

#[tokio::test]
async fn configured_openssh_transfer_performance() {
    if std::env::var("CAPSTAN_TEST_TRANSFER_PERFORMANCE").as_deref() != Ok("1") {
        eprintln!("SKIP: CAPSTAN_TEST_TRANSFER_PERFORMANCE=1 is not configured (production transfer performance)");
        return;
    }
    tokio::time::timeout(Duration::from_mins(15), measured())
        .await
        .unwrap();
}

async fn measured() {
    let target = std::env::var("CAPSTAN_TEST_SSHD").unwrap();
    let root = std::env::var("CAPSTAN_TEST_SFTP_ROOT").unwrap();
    let token = root
        .strip_prefix("/var/tmp/capstan-m1-ssh-")
        .unwrap()
        .strip_suffix("/home")
        .unwrap();
    assert_eq!(token.len(), 12);
    assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../target/m1-sshd-{token}"))
        .canonicalize()
        .unwrap();
    let input = PathBuf::from(std::env::var_os("CAPSTAN_TEST_SFTP_INPUT").unwrap());
    let output = PathBuf::from(std::env::var_os("CAPSTAN_TEST_SFTP_OUTPUT").unwrap());
    assert_eq!(input.parent().unwrap().canonicalize().unwrap(), workspace);
    assert_eq!(output.parent().unwrap().canonicalize().unwrap(), workspace);
    assert_eq!(input.file_name().unwrap(), "payload.bin");
    assert_eq!(output.file_name().unwrap(), "returned.bin");
    let bytes = std::fs::metadata(&input).unwrap().len();
    assert!(matches!(bytes, 67_108_864 | 1_073_741_824));
    let expected = std::env::var("CAPSTAN_TEST_SFTP_SHA256")
        .unwrap()
        .to_ascii_lowercase();
    let direction = std::env::var("CAPSTAN_TEST_SFTP_DIRECTION").unwrap();
    assert!(matches!(direction.as_str(), "upload" | "download"));
    let (username, endpoint) = target.split_once('@').unwrap();
    assert_eq!(username, format!("cm1{token}"));
    let (address, port) = endpoint.rsplit_once(':').unwrap();
    assert_eq!(address, "127.0.0.1");
    let mut password = zeroize::Zeroizing::new(String::new());
    std::fs::File::open(std::env::var_os("CAPSTAN_TEST_SSH_PASSWORD_FILE").unwrap())
        .unwrap()
        .take(8193)
        .read_to_string(&mut password)
        .unwrap();
    assert!(password.len() <= 8192);
    let password = secrecy::SecretString::from(password.trim_end_matches(['\r', '\n']).to_owned());
    let trust = Arc::new(Trust(
        HostPublicKey::from_openssh(
            &std::fs::read_to_string(std::env::var_os("CAPSTAN_TEST_SSH_HOST_KEY_FILE").unwrap())
                .unwrap(),
        )
        .unwrap(),
    ));
    let host = Uuid::new_v4();
    let binding = AttemptBinding {
        attempt_id: Uuid::new_v4(),
        host_id: host,
        host_revision: 1,
        vault_epoch: 1,
        target: address.into(),
        port: port.parse().unwrap(),
    };
    let key = PoolKey {
        target: binding.target.clone(),
        port: binding.port,
        username: username.into(),
        credential_id: Uuid::new_v4(),
        credential_revision: 1,
        trust_fingerprint: trust.0.fingerprint.clone(),
        vault_epoch: 1,
        agent_identity: None,
        agent_forwarding: false,
        proxy_path: String::new(),
    };
    let username = username.to_owned();
    let pool = ConnectionPool::<PasswordConnection>::new(Duration::from_secs(30));
    let (broker, _challenges) = TrustBroker::new();
    let setup = Instant::now();
    let lease = pool
        .acquire_checked(
            key,
            CancellationToken::new(),
            |connection| !connection.is_closed(),
            move |cancel| async move {
                capstan_ssh::connect_password(
                    binding.target.clone(),
                    binding.port,
                    username,
                    password,
                    binding,
                    trust,
                    broker,
                    cancel,
                )
                .await
            },
        )
        .await
        .unwrap();
    let first = lease.open_shell(80, 24).await.unwrap();
    let second = lease.open_shell(80, 24).await.unwrap();
    assert_eq!(first.connection_id(), second.connection_id());
    let negotiated = first.negotiated_algorithms().unwrap();
    assert_eq!(negotiated.cipher, "aes256-gcm@openssh.com");
    assert_eq!(negotiated.kex, "curve25519-sha256");
    let sftp = SftpService::default();
    sftp.register("main", "one", host, first.connection_id(), lease.clone())
        .unwrap();
    sftp.register("main", "two", host, second.connection_id(), lease.clone())
        .unwrap();
    let cancel = CancellationToken::new();
    let (a, b) = tokio::join!(
        sftp.attach("main", "one", &cancel),
        sftp.attach("main", "two", &cancel)
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a.path.as_str(), root);
    assert_eq!(a.connection_generation, b.connection_generation);
    let page = sftp
        .list("main", "one", a.session_id, a.path.clone())
        .await
        .unwrap();
    let transfers = Arc::new(TransferService::default());
    let remote_name = "product-benchmark.bin";
    let selection;
    let context;
    if direction == "upload" {
        context = sftp
            .transfer_context(
                "main",
                "one",
                a.session_id,
                TransferTarget::Upload {
                    parent: a.path.clone(),
                    name: remote_name.into(),
                },
            )
            .unwrap();
        selection = transfers
            .grant_source(
                &context,
                Arc::new(capstan_native_windows::transfer::Source::open(&input).unwrap()),
            )
            .unwrap();
    } else {
        let entry = page
            .entries
            .iter()
            .find(|entry| entry.name == remote_name)
            .unwrap();
        context = sftp
            .transfer_context(
                "main",
                "one",
                a.session_id,
                TransferTarget::Download { entry: entry.id },
            )
            .unwrap();
        selection = transfers
            .grant_destination(
                &context,
                Arc::new(WindowsDestination::new(
                    capstan_native_windows::transfer::Destination::select(&output).unwrap(),
                )),
            )
            .unwrap();
    }
    let setup_ms = setup.elapsed().as_secs_f64() * 1000.0;
    let started = Instant::now();
    let id = transfers.enqueue(context, selection.token).unwrap();
    let mut observations = Vec::new();
    let mut processed = 0;
    loop {
        let snapshot = transfers
            .snapshot("main")
            .unwrap()
            .into_iter()
            .find(|snapshot| snapshot.id == id)
            .unwrap();
        assert!(snapshot.processed.0 >= processed && snapshot.processed.0 <= bytes);
        processed = snapshot.processed.0;
        if observations
            .last()
            .is_none_or(|(state, _)| *state != snapshot.state)
        {
            observations.push((snapshot.state, started.elapsed().as_secs_f64() * 1000.0));
        }
        if matches!(
            snapshot.state,
            TransferState::Completed
                | TransferState::Failed
                | TransferState::Interrupted
                | TransferState::Canceled
                | TransferState::AwaitingConflict
        ) {
            assert_eq!(snapshot.state, TransferState::Completed, "{snapshot:?}");
            assert_eq!(snapshot.processed.0, bytes);
            assert!(!snapshot.cleanup_required);
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let task_ms = started.elapsed().as_secs_f64() * 1000.0;
    let (remaining, peak) = transfers.buffer_usage();
    assert_eq!(remaining, 0);
    assert!(peak <= capstan_core::sftp::MAX_BUFFER_BYTES);
    let at = |state| {
        observations
            .iter()
            .find(|(value, _)| *value == state)
            .unwrap()
            .1
    };
    let running = at(TransferState::Running);
    let verifying = at(TransferState::Verifying);
    let commit = observations
        .iter()
        .find(|(value, _)| *value == TransferState::Committing)
        .map(|(_, ms)| *ms);
    let transfer_ms = verifying - running;
    assert!(transfer_ms > 0.0);
    // The engine already includes full readback/local SHA verification. This
    // separate post-commit oracle is reported independently, not hidden in a
    // transfer-phase number or mislabeled as the only integrity check.
    let oracle = Instant::now();
    let mut hash = Sha256::new();
    let mut counted = 0;
    let client = lease.open_sftp(&CancellationToken::new()).await.unwrap();
    if direction == "upload" {
        let mut file = client
            .open_file(&format!("{root}/{remote_name}"), false, &cancel)
            .await
            .unwrap()
            .pipeline(&cancel)
            .await
            .unwrap();
        loop {
            let data = file
                .read(capstan_core::sftp::REQUEST_BYTES, &cancel)
                .await
                .unwrap();
            if data.is_empty() {
                break;
            }
            counted += data.len() as u64;
            hash.update(data);
        }
        file.close(&cancel).await.unwrap();
    } else {
        let mut source = std::fs::File::open(&output).unwrap();
        let mut buffer = vec![0; capstan_core::sftp::REQUEST_BYTES];
        loop {
            let count = source.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            counted += count as u64;
            hash.update(&buffer[..count]);
        }
    }
    assert_eq!(counted, bytes);
    let actual = format!("{:x}", hash.finalize());
    assert_eq!(actual, expected);
    let oracle_ms = oracle.elapsed().as_secs_f64() * 1000.0;
    let close = Instant::now();
    sftp.detach("main", "one", a.session_id).await.unwrap();
    sftp.detach("main", "two", b.session_id).await.unwrap();
    sftp.revoke_terminal("one");
    sftp.revoke_terminal("two");
    client.close().await;
    first.control.close().await.unwrap();
    second.control.close().await.unwrap();
    for connection in pool.drain().await {
        connection.disconnect().await;
    }
    let close_ms = close.elapsed().as_secs_f64() * 1000.0;
    println!(
        "TRANSFER_SAMPLE {}",
        serde_json::json!({
            "implementation": "capstan-production-pipeline", "direction": direction, "bytes": bytes.to_string(),
            "prepareMs": setup_ms + running, "transferMs": transfer_ms, "verificationMs": commit.unwrap_or(task_ms) - verifying,
            "commitMs": commit.map(|ms| task_ms - ms), "taskTotalMs": task_ms, "connectionCloseMs": close_ms,
            "totalMs": setup_ms + task_ms + close_ms, "oracleVerificationMs": oracle_ms,
            "wallMsIncludingIndependentOracle": setup_ms + task_ms + close_ms + oracle_ms,
            "sha256": actual, "hashMatched": true, "negotiated": {"cipher": negotiated.cipher, "kex": negotiated.kex},
            "phaseObservations": observations, "phaseSamplingMs": 5, "concurrentRequestsPerFile": capstan_core::sftp::REQUESTS_PER_FILE,
            "peakOwnedPayloadAdmissionBytes": peak, "finalOwnedPayloadAdmissionBytes": remaining,
            "nativeIo": true, "productionPoolAndServices": true, "sharedShells": 2, "concurrency": 1,
            "scope": "Single production job; phase sampling resolution 5 ms; no visible terminal or full M2 acceptance",
            "productionAcceptance": false
        })
    );
}
