//! Opt-in workflows for a disposable, pinned loopback OpenSSH measurement fixture.
//! Credentials and the fixture Vault stay entirely on the Rust side.
use super::{CoreError, Host, KdfParameters, Result, SecretText, TrustRecord, VaultManager};
use serde::Deserialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zeroize::Zeroizing;

#[cfg(windows)]
pub mod transfer_load;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureConfig {
    port: u16,
    username: String,
    password_file: PathBuf,
    key_file: PathBuf,
    host_public_key_file: PathBuf,
    vault_file: PathBuf,
    remote_fixture: String,
}

pub struct SshFixture {
    vault: VaultManager,
    hosts: [Uuid; 4],
    broker: Arc<capstan_ssh::TrustBroker>,
}

fn read_bounded(path: &Path, limit: u64) -> Result<Zeroizing<String>> {
    if !path.is_absolute() {
        return Err(CoreError::InvalidInput);
    }
    let mut bytes = Zeroizing::new(Vec::new());
    std::fs::File::open(path)
        .map_err(|_| CoreError::Storage)?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CoreError::Storage)?;
    if bytes.is_empty() || bytes.len() as u64 > limit {
        return Err(CoreError::InvalidInput);
    }
    match String::from_utf8(std::mem::take(&mut *bytes)) {
        Ok(value) => Ok(Zeroizing::new(value)),
        Err(error) => {
            let _bytes = Zeroizing::new(error.into_bytes());
            Err(CoreError::InvalidInput)
        }
    }
}

impl SshFixture {
    pub async fn load(config_path: PathBuf) -> Result<Self> {
        let (config, password, key, public_key) = tokio::task::spawn_blocking(move || {
            let config: FixtureConfig =
                serde_json::from_str(&read_bounded(&config_path, 16 * 1024)?)
                    .map_err(|_| CoreError::InvalidInput)?;
            if config.port < 1024
                || !config.username.starts_with("cm1")
                || !config
                    .username
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric())
                || !config.vault_file.is_absolute()
                || config.vault_file.exists()
                || !config
                    .remote_fixture
                    .starts_with("/var/tmp/capstan-m1-ssh-")
                || !config.remote_fixture.ends_with("/home/benchmark.py")
                || !config
                    .remote_fixture
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"/.-".contains(&byte))
            {
                return Err(CoreError::InvalidInput);
            }
            let password = read_bounded(&config.password_file, 8192)?;
            let key = read_bounded(&config.key_file, 1024 * 1024)?;
            let public_key = capstan_ssh::HostPublicKey::from_openssh(&read_bounded(
                &config.host_public_key_file,
                8192,
            )?)?;
            Ok((config, password, key, public_key))
        })
        .await
        .map_err(|_| CoreError::TaskFailed)??;
        let vault = VaultManager::new(config.vault_file);
        // A generated, disposable master password, unrelated to the fixture account.
        vault
            .initialize(
                SecretText::new(Uuid::new_v4().to_string()),
                KdfParameters::default(),
            )
            .await?;
        let password_id = vault
            .add_password_credential("Synthetic password".to_owned(), password.trim().to_owned())
            .await?;
        let mut key = key;
        let key_id = vault
            .add_imported_key_credential(
                "Synthetic Ed25519 identity".to_owned(),
                std::mem::take(&mut *key),
                None,
            )
            .await?;
        let hosts = std::array::from_fn(|_| Uuid::new_v4());
        for (index, id) in hosts.iter().enumerate() {
            let scenario = if index % 2 == 0 { "bulk" } else { "idle" };
            let host = Host {
                id: *id,
                name: format!("Synthetic {index}"),
                address: "127.0.0.1".to_owned(),
                port: config.port,
                username: config.username.clone(),
                credential_id: if index < 2 { password_id } else { key_id },
                group_id: None,
                tags: vec![],
                favorite: false,
                revision: 0,
                working_directory: None,
                startup_command: Some(format!(
                    "stty -echo -onlcr; exec python3 {} --{scenario}",
                    config.remote_fixture
                )),
                encoding: "utf-8".to_owned(),
                theme: "graphite".to_owned(),
                agent_forwarding: false,
                production: false,
            };
            let revision = vault.status().revision.ok_or(CoreError::Locked)?;
            vault.save_host(host, revision).await?;
        }
        vault
            .remember_host_key(
                vault.epoch(),
                hosts[0],
                0,
                TrustRecord {
                    target: "127.0.0.1".to_owned(),
                    port: config.port,
                    algorithm: public_key.algorithm,
                    public_key: public_key.public_key,
                    fingerprint: public_key.fingerprint,
                    first_trusted_at: 0,
                    revision: 0,
                },
            )
            .await?;
        let (broker, _challenges) = capstan_ssh::TrustBroker::new();
        Ok(Self {
            vault,
            hosts,
            broker,
        })
    }

    pub async fn open(
        &self,
        bulk: bool,
        alternate_identity: bool,
        cancel: CancellationToken,
    ) -> Result<(Uuid, capstan_ssh::SshShell)> {
        self.open_with_lease(bulk, alternate_identity, cancel)
            .await
            .map(|(host, shell, _lease)| (host, shell))
    }

    pub async fn open_with_lease(
        &self,
        bulk: bool,
        alternate_identity: bool,
        cancel: CancellationToken,
    ) -> Result<(
        Uuid,
        capstan_ssh::SshShell,
        capstan_ssh::ConnectionLease<capstan_ssh::PasswordConnection>,
    )> {
        let index = usize::from(alternate_identity) * 2 + usize::from(!bulk);
        self.vault
            .connect_password_host_with_lease(
                self.hosts[index],
                Uuid::new_v4(),
                self.broker.clone(),
                cancel,
            )
            .await
            .map(|(host, shell, lease)| (host.id, shell, lease))
    }

    pub async fn pool_entries(&self) -> usize {
        self.vault.ssh_pool.entry_count().await
    }
    pub async fn disconnect(&self) {
        self.vault.disconnect_ssh().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn pinned_fixture_initializes_two_distinct_credential_identities_without_webview_secrets()
    {
        let root = tempfile::tempdir().unwrap();
        let private = russh::keys::PrivateKey::random(
            &mut rand::rng(),
            russh::keys::ssh_key::Algorithm::Ed25519,
        )
        .unwrap();
        let key_path = root.path().join("key");
        let host_path = root.path().join("host.pub");
        let password_path = root.path().join("password");
        std::fs::write(
            &key_path,
            private
                .to_openssh(russh::keys::ssh_key::LineEnding::LF)
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        let mut host_public = private.public_key().clone();
        host_public.set_comment("synthetic fixture comment");
        std::fs::write(&host_path, host_public.to_openssh().unwrap()).unwrap();
        std::fs::write(&password_path, Uuid::new_v4().to_string()).unwrap();
        let config = root.path().join("fixture.json");
        std::fs::write(&config, serde_json::to_vec(&serde_json::json!({
            "port":22317,"username":"cm1abcdef", "password_file":password_path,"key_file":key_path,"host_public_key_file":host_path,
            "vault_file":root.path().join("measurement.capstan"),"remote_fixture":"/var/tmp/capstan-m1-ssh-abcdef/home/benchmark.py"
        })).unwrap()).unwrap();
        let fixture = SshFixture::load(config).await.unwrap();
        let data = fixture.vault.current().unwrap().snapshot().unwrap();
        assert_eq!(data.hosts.len(), 4);
        assert_eq!(data.credentials.len(), 2);
        assert_ne!(data.hosts[0].credential_id, data.hosts[2].credential_id);
        assert_eq!(data.trust.len(), 1);
        assert_eq!(
            data.trust[0].public_key,
            private.public_key().to_openssh().unwrap()
        );
        assert_eq!(fixture.pool_entries().await, 0);
    }
    #[tokio::test]
    async fn measurement_cannot_target_an_unpinned_remote_or_existing_vault() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("fixture.json");
        std::fs::write(&config, r#"{"port":22,"username":"root","password_file":"relative","key_file":"relative","host_public_key_file":"relative","vault_file":"relative","remote_fixture":"/bin/sh"}"#).unwrap();
        assert!(matches!(
            SshFixture::load(config).await,
            Err(CoreError::InvalidInput)
        ));
        let secret = root.path().join("oversized");
        std::fs::write(&secret, b"12345").unwrap();
        assert!(read_bounded(&secret, 4).is_err());
    }
}
