//! Opt-in, disposable real-server workloads. No measurement entry reaches production IPC.
use super::*;
use capstan_core::{
    sftp::{AttachedSftp, RemoteName, TransferContext},
    transfer::{LocalDestination, LocalSource, LocalVersion, TransferSnapshot},
};
use serde::Deserialize;
use std::{path::Path, sync::atomic::AtomicU64};

use crate::relay;

struct Environment {
    root: RemotePath,
    workspace: PathBuf,
    input: PathBuf,
    bytes: u64,
    expected: String,
    direction: String,
    mode: String,
    port: u16,
    username: String,
    password: secrecy::SecretString,
    trust: Arc<Trust>,
    host: Uuid,
    key: PoolKey,
}
impl Environment {
    fn load() -> Self {
        let root = std::env::var("CAPSTAN_TEST_SFTP_ROOT").unwrap();
        let token = root
            .strip_prefix("/var/tmp/capstan-m1-ssh-")
            .unwrap()
            .strip_suffix("/home")
            .unwrap();
        assert_eq!(token.len(), 12);
        assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
        let expected_workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/m1-sshd-{token}"))
            .canonicalize()
            .unwrap();
        let input = PathBuf::from(std::env::var_os("CAPSTAN_TEST_SFTP_INPUT").unwrap());
        let workspace = input.parent().unwrap().to_path_buf();
        assert_eq!(workspace.canonicalize().unwrap(), expected_workspace);
        // Keep the fixture's ordinary absolute path for native capabilities.
        // canonicalize() returns a verbatim Windows prefix, which the native
        // picker boundary intentionally rejects.
        assert_eq!(input.file_name().unwrap(), "payload.bin");
        let bytes = std::fs::metadata(&input).unwrap().len();
        assert!(matches!(bytes, 67_108_864 | 1_073_741_824));
        let target = std::env::var("CAPSTAN_TEST_SSHD").unwrap();
        let (username, endpoint) = target.split_once('@').unwrap();
        assert_eq!(username, format!("cm1{token}"));
        let (address, port) = endpoint.rsplit_once(':').unwrap();
        assert_eq!(address, "127.0.0.1");
        let port: u16 = port.parse().unwrap();
        assert!(port >= 1024);
        let mut password = zeroize::Zeroizing::new(String::new());
        std::fs::File::open(std::env::var_os("CAPSTAN_TEST_SSH_PASSWORD_FILE").unwrap())
            .unwrap()
            .take(8193)
            .read_to_string(&mut password)
            .unwrap();
        assert!(password.len() <= 8192);
        let password =
            secrecy::SecretString::from(password.trim_end_matches(['\r', '\n']).to_owned());
        let trust = Arc::new(Trust(
            HostPublicKey::from_openssh(
                &std::fs::read_to_string(
                    std::env::var_os("CAPSTAN_TEST_SSH_HOST_KEY_FILE").unwrap(),
                )
                .unwrap(),
            )
            .unwrap(),
        ));
        let key = PoolKey {
            target: address.into(),
            port,
            username: username.into(),
            credential_id: Uuid::new_v4(),
            credential_revision: 1,
            trust_fingerprint: trust.0.fingerprint.clone(),
            vault_epoch: 1,
            agent_identity: None,
            agent_forwarding: false,
            proxy_path: String::new(),
        };
        let direction = std::env::var("CAPSTAN_TEST_SFTP_DIRECTION").unwrap();
        let mode = std::env::var("CAPSTAN_TEST_MATRIX_MODE").unwrap();
        assert!(matches!(direction.as_str(), "upload" | "download"));
        assert!(matches!(mode.as_str(), "files" | "tree" | "resume"));
        Self {
            root: RemotePath::parse(&root).unwrap(),
            workspace,
            input,
            bytes,
            expected: std::env::var("CAPSTAN_TEST_SFTP_SHA256")
                .unwrap()
                .to_ascii_lowercase(),
            direction,
            mode,
            port,
            username: username.into(),
            password,
            trust,
            host: Uuid::new_v4(),
            key,
        }
    }
    fn child(&self, name: &str) -> RemotePath {
        self.root
            .join(&RemoteName::from_bytes(name.as_bytes()).unwrap())
            .unwrap()
    }
}

struct Connected {
    lease: ConnectionLease<PasswordConnection>,
    view: AttachedSftp,
    shells: [capstan_ssh::SshShell; 2],
}
async fn connect(
    env: &Environment,
    pool: &ConnectionPool<PasswordConnection>,
    service: &SftpService,
    port: u16,
) -> Connected {
    let mut key = env.key.clone();
    key.port = port;
    let binding = AttemptBinding {
        attempt_id: Uuid::new_v4(),
        host_id: env.host,
        host_revision: 1,
        vault_epoch: 1,
        target: "127.0.0.1".into(),
        port,
    };
    let username = env.username.clone();
    let password = env.password.clone();
    let trust = env.trust.clone();
    let (broker, _challenges) = TrustBroker::new();
    let lease = pool
        .acquire_checked(
            key,
            CancellationToken::new(),
            |connection| !connection.is_closed(),
            move |cancel| async move {
                capstan_ssh::connect_password(
                    binding.target.clone(),
                    port,
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
    let algorithms = first.negotiated_algorithms().unwrap();
    assert_eq!(algorithms.cipher, "aes256-gcm@openssh.com");
    assert_eq!(algorithms.kex, "curve25519-sha256");
    for terminal in ["matrix-one", "matrix-two"] {
        service
            .register(
                "main",
                terminal,
                env.host,
                first.connection_id(),
                lease.clone(),
            )
            .unwrap();
    }
    let cancel = CancellationToken::new();
    let (a, b) = tokio::join!(
        service.attach("main", "matrix-one", &cancel),
        service.attach("main", "matrix-two", &cancel)
    );
    let (view, b) = (a.unwrap(), b.unwrap());
    assert_eq!(view.path, env.root);
    assert_eq!(view.connection_generation, b.connection_generation);
    service
        .list("main", "matrix-one", view.session_id, env.root.clone())
        .await
        .unwrap();
    Connected {
        lease,
        view,
        shells: [first, second],
    }
}
async fn close(connection: Connected, service: &SftpService) {
    for terminal in ["matrix-one", "matrix-two"] {
        service.revoke_terminal(terminal);
    }
    for shell in connection.shells {
        shell.control.close().await.unwrap();
    }
}
async fn entry(
    service: &SftpService,
    connection: &Connected,
    root: &RemotePath,
    name: &str,
) -> capstan_core::sftp::RemoteEntryId {
    let page = service
        .list(
            "main",
            "matrix-one",
            connection.view.session_id,
            root.clone(),
        )
        .await
        .unwrap();
    page.entries
        .iter()
        .find(|entry| entry.name == name)
        .unwrap()
        .id
}
async fn context(
    env: &Environment,
    service: &SftpService,
    connection: &Connected,
    name: &str,
) -> TransferContext {
    let target = if env.direction == "upload" {
        TransferTarget::Upload {
            parent: env.root.clone(),
            name: name.into(),
        }
    } else {
        TransferTarget::Download {
            entry: entry(service, connection, &env.root, name).await,
        }
    };
    service
        .transfer_context("main", "matrix-one", connection.view.session_id, target)
        .unwrap()
}
fn ms(time: Instant) -> f64 {
    time.elapsed().as_secs_f64() * 1000.0
}
fn local_hash(path: &Path) -> (u64, String) {
    let mut file = std::fs::File::open(path).unwrap();
    let mut hasher = Sha256::new();
    let mut bytes = 0;
    let mut buffer = vec![0; capstan_core::sftp::REQUEST_BYTES];
    loop {
        let n = file.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        bytes += n as u64;
        hasher.update(&buffer[..n]);
    }
    (bytes, format!("{:x}", hasher.finalize()))
}
async fn remote_hash(connection: &Connected, path: &str) -> (u64, String) {
    let cancel = CancellationToken::new();
    let client = connection.lease.open_sftp(&cancel).await.unwrap();
    let mut file = client
        .open_file(path, false, &cancel)
        .await
        .unwrap()
        .pipeline(&cancel)
        .await
        .unwrap();
    let mut hasher = Sha256::new();
    let mut bytes = 0;
    loop {
        let data = file
            .read(capstan_core::sftp::REQUEST_BYTES, &cancel)
            .await
            .unwrap();
        if data.is_empty() {
            break;
        }
        bytes += data.len() as u64;
        hasher.update(data);
    }
    file.close(&cancel).await.unwrap();
    (bytes, format!("{:x}", hasher.finalize()))
}
fn terminal(state: TransferState) -> bool {
    matches!(
        state,
        TransferState::Completed
            | TransferState::Failed
            | TransferState::Interrupted
            | TransferState::Canceled
            | TransferState::AwaitingConflict
    )
}
fn snapshot(transfers: &TransferService, id: Uuid) -> TransferSnapshot {
    transfers
        .snapshot("main")
        .unwrap()
        .into_iter()
        .find(|row| row.id == id)
        .unwrap()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TreeManifest {
    directories: Vec<String>,
    files: Vec<TreeFile>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TreeFile {
    path: String,
    bytes: u64,
    sha256: String,
}
fn manifest(env: &Environment) -> TreeManifest {
    let raw = std::fs::read(env.workspace.join("matrix-tree-manifest.json")).unwrap();
    assert!(raw.len() <= 128 * 1024);
    let result: TreeManifest = serde_json::from_slice(&raw).unwrap();
    assert!(
        !result.files.is_empty() && result.files.len() <= 256 && result.directories.len() <= 64
    );
    for path in result
        .directories
        .iter()
        .map(String::as_str)
        .chain(result.files.iter().map(|file| file.path.as_str()))
    {
        assert!(path == "matrix-tree" || path.starts_with("matrix-tree/"));
        for component in path.split('/') {
            RemoteName::from_bytes(component.as_bytes()).unwrap();
        }
    }
    result
}

#[tokio::test]
async fn configured_openssh_transfer_matrix() {
    if std::env::var("CAPSTAN_TEST_TRANSFER_MATRIX").as_deref() != Ok("1") {
        eprintln!(
            "SKIP: CAPSTAN_TEST_TRANSFER_MATRIX=1 is not configured (production workflow matrix)"
        );
        return;
    }
    tokio::time::timeout(Duration::from_mins(40), async {
        let env = Environment::load();
        if env.mode == "resume" {
            resume(env).await;
        } else {
            batch(env).await;
        }
    })
    .await
    .unwrap();
}

async fn batch(env: Environment) {
    let timer = Instant::now();
    let pool = ConnectionPool::<PasswordConnection>::new(Duration::from_secs(30));
    let service = SftpService::default();
    let connection = connect(&env, &pool, &service, env.port).await;
    let transfers = Arc::new(TransferService::default());
    let tree = (env.mode == "tree").then(|| manifest(&env));
    let count = if let Some(tree) = &tree {
        tree.files.len()
    } else {
        3
    };
    let payload = tree.as_ref().map_or(env.bytes * 3, |tree| {
        tree.files.iter().map(|file| file.bytes).sum()
    });
    let mut jobs = Vec::new();
    let prepared;
    let running;
    if let Some(tree) = &tree {
        let plan = if env.direction == "upload" {
            transfers
                .prepare_upload_tree(
                    &service,
                    "main",
                    "matrix-one",
                    connection.view.session_id,
                    env.root.clone(),
                    env.workspace.join("matrix-tree"),
                )
                .await
                .unwrap()
        } else {
            let entry = entry(&service, &connection, &env.root, "matrix-tree").await;
            transfers
                .prepare_download_tree(
                    &service,
                    "main",
                    "matrix-one",
                    connection.view.session_id,
                    entry,
                    env.workspace.join("tree-output"),
                )
                .await
                .unwrap()
        };
        assert!(plan.conflicts.is_empty(), "unexpected fixture conflict");
        prepared = ms(timer);
        running = Instant::now();
        let result = transfers
            .resolve_tree_plan(
                &service,
                "main",
                "matrix-one",
                connection.view.session_id,
                plan.token,
                plan.revision,
                vec![],
            )
            .await
            .unwrap();
        assert!(result.plan.is_none());
        assert_eq!(result.queued as usize, tree.files.len());
        jobs.extend(
            transfers
                .snapshot("main")
                .unwrap()
                .into_iter()
                .map(|row| row.id),
        );
    } else {
        let mut ready = Vec::new();
        for index in 0..3 {
            let name = format!("matrix-file-{index}.bin");
            let context = context(&env, &service, &connection, &name).await;
            let selection = if env.direction == "upload" {
                transfers
                    .grant_source(
                        &context,
                        Arc::new(
                            capstan_native_windows::transfer::Source::open(&env.input).unwrap(),
                        ),
                    )
                    .unwrap()
            } else {
                transfers
                    .grant_destination(
                        &context,
                        Arc::new(WindowsDestination::new(
                            capstan_native_windows::transfer::Destination::select(
                                &env.workspace.join(format!("matrix-returned-{index}.bin")),
                            )
                            .unwrap(),
                        )),
                    )
                    .unwrap()
            };
            ready.push((context, selection.token));
        }
        // Resolve every grant before starting any task. Otherwise the first
        // download can run while later directory lookups remain unmeasured.
        prepared = ms(timer);
        running = Instant::now();
        for (context, token) in ready {
            jobs.push(transfers.enqueue(context, token).unwrap());
        }
    }
    assert_eq!(jobs.len(), count);
    let mut observations = HashMap::<Uuid, Vec<(TransferState, f64)>>::new();
    let mut progress = HashMap::<Uuid, u64>::new();
    let mut first_progress = HashMap::<Uuid, f64>::new();
    let mut max_running = 0;
    let mut min_fraction_when_first_completes = None;
    loop {
        let rows = transfers.snapshot("main").unwrap();
        max_running = max_running.max(
            rows.iter()
                .filter(|row| row.state == TransferState::Running)
                .count(),
        );
        let mut complete = 0;
        for row in &rows {
            assert!(
                row.processed.0 >= *progress.get(&row.id).unwrap_or(&0)
                    && row.processed.0 <= row.total.0
            );
            progress.insert(row.id, row.processed.0);
            if row.processed.0 > 0 {
                first_progress.entry(row.id).or_insert_with(|| ms(running));
            }
            let phases = observations.entry(row.id).or_default();
            if phases.last().is_none_or(|(state, _)| *state != row.state) {
                phases.push((row.state, ms(running)));
            }
            if terminal(row.state) {
                assert_eq!(row.state, TransferState::Completed, "{row:?}");
                assert!(!row.cleanup_required);
                complete += 1;
            }
        }
        if complete > 0 && min_fraction_when_first_completes.is_none() {
            min_fraction_when_first_completes = Some(
                rows.iter()
                    .filter(|row| row.total.0 > 0)
                    .map(|row| row.processed.0 as f64 / row.total.0 as f64)
                    .fold(1.0, f64::min),
            );
        }
        if complete == count {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let task_ms = ms(running);
    let data_start = observations
        .values()
        .flat_map(|phases| phases.iter())
        .filter(|(state, _)| *state == TransferState::Running)
        .map(|(_, ms)| *ms)
        .reduce(f64::min)
        .unwrap_or(0.0);
    let data_end = observations
        .values()
        .flat_map(|phases| phases.iter())
        .filter(|(state, _)| *state == TransferState::Verifying)
        .map(|(_, ms)| *ms)
        .reduce(f64::max)
        .unwrap_or(task_ms);
    let (remaining, peak) = transfers.buffer_usage();
    assert_eq!(remaining, 0);
    assert!(peak <= capstan_core::sftp::MAX_BUFFER_BYTES);
    if tree.is_none() {
        assert_eq!(max_running, 3);
        assert_eq!(first_progress.len(), 3);
        assert!(min_fraction_when_first_completes.unwrap() > 0.0);
    }
    let oracle = Instant::now();
    let mut hashes = Vec::new();
    if let Some(tree) = &tree {
        for file in &tree.files {
            let (bytes, hash) = if env.direction == "upload" {
                remote_hash(&connection, &format!("{}/{}", env.root.as_str(), file.path)).await
            } else {
                local_hash(&env.workspace.join("tree-output").join(&file.path))
            };
            assert_eq!(bytes, file.bytes);
            assert_eq!(hash, file.sha256.to_ascii_lowercase());
            hashes.push(hash);
        }
        let client = connection
            .lease
            .open_sftp(&CancellationToken::new())
            .await
            .unwrap();
        for directory in &tree.directories {
            if env.direction == "upload" {
                let attr = client
                    .lstat(
                        &format!("{}/{directory}", env.root.as_str()),
                        &CancellationToken::new(),
                    )
                    .await
                    .unwrap();
                assert_eq!(attr.permissions.unwrap() & 0o170000, 0o40000);
            } else {
                assert!(env.workspace.join("tree-output").join(directory).is_dir());
            }
        }
    } else {
        for index in 0..3 {
            let (bytes, hash) = if env.direction == "upload" {
                remote_hash(
                    &connection,
                    env.child(&format!("matrix-file-{index}.bin")).as_str(),
                )
                .await
            } else {
                local_hash(&env.workspace.join(format!("matrix-returned-{index}.bin")))
            };
            assert_eq!(bytes, env.bytes);
            assert_eq!(hash, env.expected);
            hashes.push(hash);
        }
    }
    let oracle_ms = ms(oracle);
    let closing = Instant::now();
    transfers.prepare_exit("main").await.unwrap();
    close(connection, &service).await;
    for connection in pool.drain().await {
        connection.disconnect().await;
    }
    assert_eq!(pool.entry_count().await, 0);
    assert!(!transfers.has_active("main").unwrap());
    let close_ms = ms(closing);
    println!(
        "MATRIX_SAMPLE {}",
        serde_json::json!({
            "implementation":"capstan-production-matrix","workload":env.mode,"direction":env.direction,"concurrency":3,
            "files":count,"bytes":payload.to_string(),"perFileBytes":env.bytes.to_string(),"prepareMs":prepared,
            "transferMs":(data_end-data_start).max(0.001),"taskTotalMs":task_ms,"connectionCloseMs":close_ms,
            "totalMs":prepared+task_ms+close_ms,"oracleVerificationMs":oracle_ms,"wallMsIncludingIndependentOracle":prepared+task_ms+close_ms+oracle_ms,
            "hashMatched":true,"hashes":hashes,"emptyDirectoriesChecked":tree.as_ref().map(|tree|tree.directories.len()),
            "maxSimultaneouslyRunningJobs":max_running,"firstProgressMs":jobs.iter().map(|id|first_progress.get(id).copied()).collect::<Vec<_>>(),
            "minimumCompletedFractionAtFirstCompletion":min_fraction_when_first_completes,
            "phaseObservations":jobs.iter().map(|id|observations.get(id)).collect::<Vec<_>>(),"phaseSamplingMs":5,
            "peakOwnedPayloadAdmissionBytes":peak,"finalOwnedPayloadAdmissionBytes":remaining,"activeJobsAfter":0,"poolEntriesAfter":0,
            "sshConnections":1,"siblingShells":2,"nativeIo":true,"productionPoolAndServices":true,
            "negotiated":{"cipher":"aes256-gcm@openssh.com","kex":"curve25519-sha256"},"performanceAcceptance":false,"desktopAcceptance":false
        })
    );
}

struct ReadSource {
    inner: Arc<dyn LocalSource>,
    read_bytes: AtomicU64,
}
impl LocalSource for ReadSource {
    fn name(&self) -> Result<String, SftpError> {
        self.inner.name()
    }
    fn size(&self) -> u64 {
        self.inner.size()
    }
    fn verify(&self) -> Result<(), SftpError> {
        self.inner.verify()
    }
    fn read(&self, offset: u64, length: usize) -> Result<Vec<u8>, SftpError> {
        let bytes = self.inner.read(offset, length)?;
        self.read_bytes
            .fetch_add(bytes.len() as u64, Ordering::SeqCst);
        Ok(bytes)
    }
}
struct WrittenDestination {
    inner: WindowsDestination,
    written: AtomicU64,
    first_offset: AtomicU64,
}
impl LocalDestination for WrittenDestination {
    fn name(&self) -> Result<String, SftpError> {
        self.inner.name()
    }
    fn target_path(&self) -> Result<Option<PathBuf>, SftpError> {
        self.inner.target_path()
    }
    fn snapshot(&self) -> Result<Option<LocalVersion>, SftpError> {
        self.inner.snapshot()
    }
    fn retarget(&self, name: &str) -> Result<(), SftpError> {
        self.inner.retarget(name)
    }
    fn begin(&self, name: &str) -> Result<(), SftpError> {
        self.inner.begin(name)
    }
    fn partial_size(&self) -> Result<Option<u64>, SftpError> {
        self.inner.partial_size()
    }
    fn partial_path(&self) -> Result<Option<PathBuf>, SftpError> {
        self.inner.partial_path()
    }
    fn partial_identity(&self) -> Result<Option<u128>, SftpError> {
        self.inner.partial_identity()
    }
    fn preserve_partial(&self) -> Result<(), SftpError> {
        self.inner.preserve_partial()
    }
    fn recover_partial(
        &self,
        path: &Path,
        identity: u128,
        minimum: u64,
        maximum: u64,
    ) -> Result<(), SftpError> {
        self.inner.recover_partial(path, identity, minimum, maximum)
    }
    fn write(&self, offset: u64, data: &[u8]) -> Result<(), SftpError> {
        self.inner.write(offset, data)?;
        self.first_offset.fetch_min(offset, Ordering::SeqCst);
        self.written.fetch_add(data.len() as u64, Ordering::SeqCst);
        Ok(())
    }
    fn read(&self, offset: u64, length: usize) -> Result<Vec<u8>, SftpError> {
        self.inner.read(offset, length)
    }
    fn sync(&self) -> Result<(), SftpError> {
        self.inner.sync()
    }
    fn commit(&self, expected: Option<LocalVersion>) -> Result<(), SftpError> {
        self.inner.commit(expected)
    }
    fn discard(&self) -> Result<(), SftpError> {
        self.inner.discard()
    }
}
async fn resume(env: Environment) {
    let cut: u64 = std::env::var("CAPSTAN_TEST_MATRIX_CUT_PERCENT")
        .unwrap()
        .parse()
        .unwrap();
    assert!(matches!(cut, 20 | 50 | 85));
    let proxy = relay::Relay::start(env.port).await;
    let pool = ConnectionPool::<PasswordConnection>::new(Duration::from_secs(30));
    let service = SftpService::default();
    let connection = connect(&env, &pool, &service, proxy.port).await;
    let old_generation = connection.view.connection_generation;
    let transfers = Arc::new(TransferService::default());
    let remote_name = if env.direction == "upload" {
        "resume-target.bin"
    } else {
        "resume-source.bin"
    };
    let local_path = env.workspace.join("resume-returned.bin");
    let destination = Arc::new(WrittenDestination {
        inner: WindowsDestination::new(
            capstan_native_windows::transfer::Destination::select(&local_path).unwrap(),
        ),
        written: AtomicU64::new(0),
        first_offset: AtomicU64::new(u64::MAX),
    });
    let context = context(&env, &service, &connection, remote_name).await;
    let selection = if env.direction == "upload" {
        transfers
            .grant_source(
                &context,
                Arc::new(capstan_native_windows::transfer::Source::open(&env.input).unwrap()),
            )
            .unwrap()
    } else {
        transfers
            .grant_destination(&context, destination.clone())
            .unwrap()
    };
    let started = Instant::now();
    let id = transfers.enqueue(context, selection.token).unwrap();
    let cutoff = env.bytes * cut / 100;
    let at_cut = loop {
        let row = snapshot(&transfers, id);
        assert!(!terminal(row.state), "{row:?}");
        if row.state == TransferState::Running && row.processed.0 >= cutoff {
            break row.processed.0;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    proxy.break_connections();
    let interrupted = loop {
        let row = snapshot(&transfers, id);
        if terminal(row.state) {
            break row;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert_eq!(
        interrupted.state,
        TransferState::Interrupted,
        "{interrupted:?}"
    );
    assert!(interrupted.cleanup_required);
    assert!(!local_path.exists());
    let before_ms = ms(started);
    for terminal in ["matrix-one", "matrix-two"] {
        service.revoke_terminal(terminal);
    }
    connection.lease.connection().disconnect().await;
    drop(connection);
    let reconnect = Instant::now();
    let current = connect(&env, &pool, &service, proxy.port).await;
    assert_ne!(old_generation, current.view.connection_generation);
    let partial = format!("{}/.capstan-{id}.partial", env.root.as_str());
    let retained = if env.direction == "upload" {
        let client = current
            .lease
            .open_sftp(&CancellationToken::new())
            .await
            .unwrap();
        client
            .lstat(&partial, &CancellationToken::new())
            .await
            .unwrap()
            .size
            .unwrap()
    } else {
        destination.partial_size().unwrap().unwrap()
    };
    assert!(retained > 0 && retained < env.bytes);
    let source = Arc::new(ReadSource {
        inner: Arc::new(capstan_native_windows::transfer::Source::open(&env.input).unwrap()),
        read_bytes: AtomicU64::new(0),
    });
    if env.direction == "upload" {
        let context = service
            .transfer_context_for_recovery(
                "main",
                "matrix-one",
                current.view.session_id,
                transfers.target("main", id).unwrap(),
            )
            .unwrap();
        let selection = transfers.grant_source(&context, source.clone()).unwrap();
        transfers
            .retry_with_selection("main", id, context, selection.token)
            .unwrap();
    } else {
        let context = service
            .transfer_context_for_recovered_download(
                "main",
                "matrix-one",
                current.view.session_id,
                transfers.download_retry("main", id).unwrap(),
            )
            .await
            .unwrap();
        destination.written.store(0, Ordering::SeqCst);
        destination.first_offset.store(u64::MAX, Ordering::SeqCst);
        transfers.retry_with_context("main", id, context).unwrap();
    }
    let reconnect_ms = ms(reconnect);
    let resumed = Instant::now();
    let mut prefix_preparation_ms = None;
    let mut verifying_at_ms = None;
    let mut committing_at_ms = None;
    let mut resume_phases = Vec::new();
    loop {
        let row = snapshot(&transfers, id);
        let elapsed = ms(resumed);
        if row.processed.0 >= retained {
            prefix_preparation_ms.get_or_insert(elapsed);
        }
        if row.state == TransferState::Verifying {
            verifying_at_ms.get_or_insert(elapsed);
        }
        if row.state == TransferState::Committing {
            committing_at_ms.get_or_insert(elapsed);
        }
        if resume_phases
            .last()
            .is_none_or(|(state, _)| *state != row.state)
        {
            resume_phases.push((row.state, elapsed));
        }
        if terminal(row.state) {
            assert_eq!(row.state, TransferState::Completed, "{row:?}");
            assert_eq!(row.processed.0, env.bytes);
            assert!(!row.cleanup_required);
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let resumed_ms = ms(resumed);
    let prefix_preparation_ms = prefix_preparation_ms.unwrap();
    if env.direction == "download" {
        assert_eq!(destination.first_offset.load(Ordering::SeqCst), retained);
        assert_eq!(
            destination.written.load(Ordering::SeqCst),
            env.bytes - retained
        );
    } else {
        assert_eq!(source.read_bytes.load(Ordering::SeqCst), env.bytes);
    }
    let actual = if env.direction == "upload" {
        remote_hash(&current, env.child(remote_name).as_str()).await
    } else {
        local_hash(&local_path)
    };
    assert_eq!(actual.0, env.bytes);
    assert_eq!(actual.1, env.expected);
    let client = current
        .lease
        .open_sftp(&CancellationToken::new())
        .await
        .unwrap();
    if env.direction == "upload" {
        assert!(matches!(
            client.lstat(&partial, &CancellationToken::new()).await,
            Err(capstan_ssh::sftp::ClientError::NotFound)
        ));
    } else {
        assert!(!env
            .workspace
            .join(format!(".capstan-{id}.partial"))
            .exists());
    }
    transfers.prepare_exit("main").await.unwrap();
    close(current, &service).await;
    for connection in pool.drain().await {
        connection.disconnect().await;
    }
    let connections = proxy.opened();
    proxy.close().await;
    assert_eq!(connections, 2);
    let (remaining, peak) = transfers.buffer_usage();
    assert_eq!(remaining, 0);
    assert!(peak <= capstan_core::sftp::MAX_BUFFER_BYTES);
    println!(
        "MATRIX_SAMPLE {}",
        serde_json::json!({"implementation":"capstan-production-resume","direction":env.direction,"bytes":env.bytes.to_string(),"requestedCutPercent":cut,"confirmedBytesAtSocketClose":at_cut.to_string(),"confirmedBytesAfterInterruption":interrupted.processed.0.to_string(),"retainedPrefixBytes":retained.to_string(),"expectedAppendedBytes":(env.bytes-retained).to_string(),"downloadActualAppendedBytes":destination.written.load(Ordering::SeqCst).to_string(),"downloadFirstAppendOffset":destination.first_offset.load(Ordering::SeqCst).to_string(),"uploadSourceBytesIncludingPrefixVerification":source.read_bytes.load(Ordering::SeqCst).to_string(),"beforeInterruptionMs":before_ms,"explicitReconnectMs":reconnect_ms,"resumeMs":resumed_ms,"resumePrefixValidationAndPreparationMs":prefix_preparation_ms,"resumeDataMs":verifying_at_ms.map(|value|(value-prefix_preparation_ms).max(0.0)),"resumeVerificationMs":verifying_at_ms.map(|start|(committing_at_ms.unwrap_or(resumed_ms)-start).max(0.0)),"resumeCommitMs":committing_at_ms.map(|start|(resumed_ms-start).max(0.0)),"resumePhaseObservations":resume_phases,"phaseSamplingMs":5,"sha256":actual.1,"hashMatched":true,"oldGeneration":old_generation,"newGenerationDifferent":true,"tcpConnections":connections,"fault":"owned TCP sockets dropped without an SSH disconnect message","finalOwnedPayloadAdmissionBytes":remaining,"peakOwnedPayloadAdmissionBytes":peak,"partialGoneAfterCommit":true,"activeJobsAfter":0,"poolEntriesAfter":pool.entry_count().await,"performanceAcceptance":false,"desktopAcceptance":false})
    );
}
