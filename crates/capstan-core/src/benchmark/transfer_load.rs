//! Fixed, opt-in native transfer load for visible terminal measurements.
//! Only a private disposable configuration can supply paths; IPC supplies IDs.
use crate::{
    sftp::{ExactU64, RemotePath, SftpError, SftpService},
    transfer::{Direction, TransferService, TransferState, TransferTarget, WindowsDestination},
};
use capstan_ssh::{ConnectionLease, PasswordConnection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    input: PathBuf,
    sha256: String,
    remote_root: RemotePath,
    pub round: u8,
    pub smoke: bool,
}
impl Config {
    pub fn load(path: &Path) -> Result<Self, SftpError> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|_| SftpError::LocalIo)?
            .take(16385)
            .read_to_end(&mut bytes)
            .map_err(|_| SftpError::LocalIo)?;
        if bytes.len() > 16384 {
            return Err(SftpError::PathInvalid);
        }
        let config: Self = serde_json::from_slice(&bytes).map_err(|_| SftpError::PathInvalid)?;
        let token = config
            .remote_root
            .as_str()
            .strip_prefix("/var/tmp/capstan-m1-ssh-")
            .and_then(|value| value.strip_suffix("/home"))
            .ok_or(SftpError::PathInvalid)?;
        if token.len() != 12
            || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
            || config.round > 3
            || config.sha256.len() != 64
            || !config.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(SftpError::PathInvalid);
        }
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/m1-sshd-{token}"))
            .canonicalize()
            .map_err(|_| SftpError::PathInvalid)?;
        if path.canonicalize().map_err(|_| SftpError::PathInvalid)?
            != workspace.join("secrets/m2-load.json")
            || config
                .input
                .canonicalize()
                .map_err(|_| SftpError::PathInvalid)?
                != workspace.join("payload.bin")
        {
            return Err(SftpError::PathInvalid);
        }
        let size = std::fs::metadata(&config.input)
            .map_err(|_| SftpError::LocalIo)?
            .len();
        if size
            != if config.smoke {
                67_108_864
            } else {
                1_073_741_824
            }
        {
            return Err(SftpError::PathInvalid);
        }
        Ok(config)
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadResult {
    pub id: Uuid,
    pub round: u8,
    pub direction: Direction,
    pub connection_id: Uuid,
    pub task_ids: Vec<Uuid>,
    pub bytes_per_file: ExactU64,
    pub hashes: Vec<String>,
    pub elapsed_ms: f64,
    pub peak_owned_payload_admission_bytes: usize,
    pub final_owned_payload_admission_bytes: usize,
    pub active_jobs_after: usize,
    pub confirmed_all_running_checks: usize,
    pub smoke: bool,
}

pub struct TransferLoad {
    config: Config,
    result: LoadResult,
    transfers: Arc<TransferService>,
    service: SftpService,
    client: Arc<capstan_ssh::sftp::SftpClient>,
    remote_paths: Vec<RemotePath>,
    local_paths: Vec<PathBuf>,
    started: Instant,
    active_checks: AtomicUsize,
}
impl TransferLoad {
    pub async fn start(
        config: Config,
        host: Uuid,
        connection: Uuid,
        lease: ConnectionLease<PasswordConnection>,
        direction: Direction,
    ) -> Result<Self, SftpError> {
        // Verify the deterministic local fixture before granting any source.
        let input = config.input.clone();
        let expected = config.sha256.clone();
        let bytes = tokio::task::spawn_blocking(move || {
            let (bytes, hash) = hash_file(&input)?;
            if !hash.eq_ignore_ascii_case(&expected) {
                return Err(SftpError::ResumeMismatch);
            }
            Ok(bytes)
        })
        .await
        .map_err(|_| SftpError::LocalIo)??;
        let service = SftpService::default();
        service.register("main", "m2-visible-load", host, connection, lease.clone())?;
        let view = service
            .attach("main", "m2-visible-load", &CancellationToken::new())
            .await?;
        if view.path != config.remote_root {
            return Err(SftpError::ScopeExpired);
        }
        let page = service
            .list(
                "main",
                "m2-visible-load",
                view.session_id,
                config.remote_root.clone(),
            )
            .await?;
        let source = page
            .entries
            .iter()
            .find(|entry| entry.name == "visible-source.bin")
            .map(|entry| entry.id);
        let transfers = Arc::new(TransferService::default());
        let id = Uuid::new_v4();
        let mut grants = Vec::new();
        let mut remote_paths = Vec::new();
        let mut local_paths = Vec::new();
        for index in 0..3 {
            let target = match direction {
                Direction::Upload => TransferTarget::Upload {
                    parent: config.remote_root.clone(),
                    name: format!("visible-{id}-{index}.bin"),
                },
                Direction::Download => TransferTarget::Download {
                    entry: source.ok_or(SftpError::RemoteIo)?,
                },
            };
            let context =
                service.transfer_context("main", "m2-visible-load", view.session_id, target)?;
            remote_paths.push(context.path.clone());
            let selection = match direction {
                Direction::Upload => transfers.grant_source(
                    &context,
                    Arc::new(
                        capstan_native_windows::transfer::Source::open(&config.input)
                            .map_err(|_| SftpError::LocalIo)?,
                    ),
                )?,
                Direction::Download => {
                    let output = config
                        .input
                        .with_file_name(format!("visible-{id}-{index}.bin"));
                    let destination =
                        capstan_native_windows::transfer::Destination::select(&output)
                            .map_err(|_| SftpError::LocalIo)?;
                    local_paths.push(output);
                    transfers.grant_destination(
                        &context,
                        Arc::new(WindowsDestination::new(destination)),
                    )?
                }
            };
            grants.push((context, selection.token));
        }
        let started = Instant::now();
        let mut task_ids = Vec::new();
        for (context, grant) in grants {
            task_ids.push(transfers.enqueue(context, grant)?);
        }
        let client = lease
            .open_sftp(&CancellationToken::new())
            .await
            .map_err(|_| SftpError::ConnectionLost)?;
        Ok(Self {
            config: config.clone(),
            result: LoadResult {
                id,
                round: config.round,
                direction,
                connection_id: connection,
                task_ids,
                bytes_per_file: ExactU64(bytes),
                hashes: Vec::new(),
                elapsed_ms: 0.0,
                peak_owned_payload_admission_bytes: 0,
                final_owned_payload_admission_bytes: 0,
                active_jobs_after: 0,
                confirmed_all_running_checks: 0,
                smoke: config.smoke,
            },
            transfers,
            service,
            client,
            remote_paths,
            local_paths,
            started,
            active_checks: AtomicUsize::new(0),
        })
    }
    pub fn id(&self) -> Uuid {
        self.result.id
    }
    pub fn all_running(&self) -> Result<bool, SftpError> {
        let rows = self.transfers.snapshot("main")?;
        let running = rows.len() == 3 && rows.iter().all(|row| row.state == TransferState::Running);
        if running {
            self.active_checks.fetch_add(1, Ordering::Relaxed);
        }
        Ok(running)
    }
    /// Bounded benchmark diagnostics deliberately omit names and paths.
    pub fn diagnostic_progress(&self) -> Result<Vec<(TransferState, u64)>, SftpError> {
        Ok(self
            .transfers
            .snapshot("main")?
            .into_iter()
            .map(|row| (row.state, row.processed.0))
            .collect())
    }
    pub async fn cancel(&self) -> Result<(), SftpError> {
        for id in &self.result.task_ids {
            let _ = self.transfers.cancel("main", *id);
        }
        self.transfers.prepare_exit("main").await?;
        self.service.revoke_terminal("m2-visible-load");
        Ok(())
    }
    pub async fn finish(&self) -> Result<LoadResult, SftpError> {
        tokio::time::timeout(Duration::from_secs(3600), async {
            loop {
                let rows = self.transfers.snapshot("main")?;
                if rows.len() != 3 {
                    return Err(SftpError::LocalIo);
                }
                if rows.iter().any(|row| {
                    matches!(
                        row.state,
                        TransferState::Failed
                            | TransferState::Interrupted
                            | TransferState::Canceled
                            | TransferState::AwaitingConflict
                    )
                }) {
                    return Err(SftpError::RemoteIo);
                }
                if rows.iter().all(|row| row.state == TransferState::Completed) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let mut result = self.result.clone();
            for index in 0..3 {
                let (bytes, hash) = match result.direction {
                    Direction::Upload => {
                        let cancel = CancellationToken::new();
                        let mut file = self
                            .client
                            .open_file(self.remote_paths[index].as_str(), false, &cancel)
                            .await
                            .map_err(|_| SftpError::RemoteIo)?
                            .pipeline(&cancel)
                            .await
                            .map_err(|_| SftpError::RemoteIo)?;
                        let mut hasher = Sha256::new();
                        let mut total = 0;
                        loop {
                            let data = file
                                .read(crate::sftp::REQUEST_BYTES, &cancel)
                                .await
                                .map_err(|_| SftpError::RemoteIo)?;
                            if data.is_empty() {
                                break;
                            }
                            total += data.len() as u64;
                            hasher.update(data);
                        }
                        file.close(&cancel).await.map_err(|_| SftpError::RemoteIo)?;
                        (total, format!("{:x}", hasher.finalize()))
                    }
                    Direction::Download => {
                        let path = self.local_paths[index].clone();
                        tokio::task::spawn_blocking(move || hash_file(&path))
                            .await
                            .map_err(|_| SftpError::LocalIo)??
                    }
                };
                if bytes != result.bytes_per_file.0
                    || !hash.eq_ignore_ascii_case(&self.config.sha256)
                {
                    return Err(SftpError::ResumeMismatch);
                }
                result.hashes.push(hash);
            }
            self.transfers.prepare_exit("main").await?;
            self.service.revoke_terminal("m2-visible-load");
            if self.transfers.has_active("main")? {
                return Err(SftpError::LocalIo);
            }
            let (current, peak) = self.transfers.buffer_usage();
            if current != 0 || peak > crate::sftp::MAX_BUFFER_BYTES {
                return Err(SftpError::LocalIo);
            }
            result.final_owned_payload_admission_bytes = current;
            result.peak_owned_payload_admission_bytes = peak;
            result.elapsed_ms = self.started.elapsed().as_secs_f64() * 1000.0;
            result.confirmed_all_running_checks = self.active_checks.load(Ordering::Relaxed);
            Ok(result)
        })
        .await
        .map_err(|_| SftpError::ConnectionLost)?
    }
}
fn hash_file(path: &Path) -> Result<(u64, String), SftpError> {
    let mut file = std::fs::File::open(path).map_err(|_| SftpError::LocalIo)?;
    let mut hasher = Sha256::new();
    let mut total = 0;
    let mut buffer = vec![0; crate::sftp::REQUEST_BYTES];
    loop {
        let count = file.read(&mut buffer).map_err(|_| SftpError::LocalIo)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        hasher.update(&buffer[..count]);
    }
    Ok((total, format!("{:x}", hasher.finalize())))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visible_load_accepts_its_owned_private_configuration() {
        let token = Uuid::new_v4().simple().to_string()[..12].to_owned();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("target")
            .join(format!("m1-sshd-{token}"));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("secrets")).unwrap();
        let input = root.join("payload.bin");
        std::fs::File::create(&input)
            .unwrap()
            .set_len(67_108_864)
            .unwrap();
        let path = root.join("secrets").join("m2-load.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "input": input, "sha256": "a".repeat(64),
                "remoteRoot": format!("/var/tmp/capstan-m1-ssh-{token}/home"),
                "round": 0, "smoke": true,
            }))
            .unwrap(),
        )
        .unwrap();
        let result = Config::load(&path);
        // This exact test-created root contains only nonsecret fixture files.
        std::fs::remove_dir_all(&root).unwrap();
        assert!(result.is_ok(), "owned native configuration was rejected");
    }
    #[test]
    fn visible_load_rejects_non_fixture_paths_and_unbounded_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("m2-load.json");
        std::fs::write(&path,serde_json::to_vec(&serde_json::json!({"input":directory.path().join("payload.bin"),"sha256":"a".repeat(64),"remoteRoot":"/home/user","round":1,"smoke":false})).unwrap()).unwrap();
        assert!(matches!(Config::load(&path), Err(SftpError::PathInvalid)));
        std::fs::write(&path, vec![b' '; 16385]).unwrap();
        assert!(matches!(Config::load(&path), Err(SftpError::PathInvalid)));
    }
}
