//! Seed an isolated manual-review Vault; never part of application IPC.
#![allow(clippy::pedantic)]
#[cfg(windows)]
mod windows {
    use capstan_core::VaultManager;
    use capstan_native_windows::transfer::{Destination, Source};
    use capstan_ssh::HostPublicKey;
    use capstan_vault::{Host, KdfParameters, SecretText, TrustRecord};
    use std::{io, os::windows::fs::MetadataExt, path::Path};
    use uuid::Uuid;
    use zeroize::Zeroizing;

    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

    fn invalid() -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid isolated manual fixture",
        )
    }
    pub(super) fn validate(root: &Path, port: u16) -> Result<()> {
        if !root.is_absolute() || port < 1024 {
            return Err(invalid().into());
        }
        let name = root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(invalid)?;
        let suffix = name.strip_prefix("m1-sshd-").ok_or_else(invalid)?;
        if suffix.len() != 12 || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid().into());
        }
        let target = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .canonicalize()?;
        if std::fs::symlink_metadata(root)?.file_attributes() & 0x400 != 0
            || root.canonicalize()?.parent() != Some(target.as_path())
        {
            return Err(invalid().into());
        }
        Ok(())
    }
    fn secret(path: &Path, maximum: u64) -> Result<Zeroizing<String>> {
        // Source pins ancestors and rejects reparse points through the open.
        let file = Source::open(path)?;
        if file.size() == 0 || file.size() > maximum {
            return Err(invalid().into());
        }
        let bytes = Zeroizing::new(file.read(0, file.size() as usize)?);
        let text = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
        Ok(Zeroizing::new(text.to_owned()))
    }
    pub(super) async fn prepare(root: &Path, port: u16) -> Result<()> {
        validate(root, port)?;
        let name = root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(invalid)?;
        let suffix = name.strip_prefix("m1-sshd-").ok_or_else(invalid)?;
        let mut master = secret(&root.join("secrets/master.txt"), 4096)?;
        let mut password = secret(&root.join("secrets/password.txt"), 8192)?;
        let length = password.trim_end_matches(['\r', '\n']).len();
        password.truncate(length);
        let public_key =
            HostPublicKey::from_openssh(&secret(&root.join("secrets/host-key.pub"), 8192)?)?;
        let desktop = root.join("desktop");
        let data = desktop.join("data");
        let _desktop_scope = Destination::select(&desktop.join(".capstan-scope"))?;
        // The script creates desktop. Exclusive data creation prevents an
        // accidental re-seed of an existing manual Vault or user settings.
        std::fs::create_dir(&data)?;
        let vault_path = data.join("vault.capstan");
        let _ancestors = Destination::select(&vault_path)?;
        let vault = VaultManager::new(vault_path);
        vault
            .initialize(
                SecretText::new(std::mem::take(&mut *master)),
                KdfParameters::default(),
            )
            .await?;
        let credential = vault
            .add_password_credential(
                "Disposable SFTP fixture".into(),
                std::mem::take(&mut *password),
            )
            .await?;
        let hosts = [Uuid::new_v4(), Uuid::new_v4()];
        for (index, id) in hosts.iter().enumerate() {
            let host = Host {
                id: *id,
                name: format!(
                    "SFTP manual {}",
                    if index == 0 { "Graphite" } else { "Light" }
                ),
                address: "127.0.0.1".into(),
                port,
                username: format!("cm1{suffix}"),
                credential_id: credential,
                group_id: None,
                tags: vec![],
                favorite: true,
                revision: 0,
                working_directory: None,
                startup_command: None,
                encoding: "utf-8".into(),
                theme: if index == 0 { "graphite" } else { "light" }.into(),
                agent_forwarding: false,
                production: false,
            };
            vault.save_host(host, vault.revision()?).await?;
        }
        vault
            .remember_host_key(
                vault.epoch(),
                hosts[0],
                0,
                TrustRecord {
                    target: "127.0.0.1".into(),
                    port,
                    algorithm: public_key.algorithm,
                    public_key: public_key.public_key,
                    fingerprint: public_key.fingerprint,
                    first_trusted_at: 0,
                    revision: 0,
                },
            )
            .await?;
        vault.lock().await?;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        struct Root(std::path::PathBuf);
        impl Root {
            fn new() -> Self {
                let target = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("target");
                let root = target.join(format!(
                    "m1-sshd-{}",
                    &Uuid::new_v4().simple().to_string()[..12]
                ));
                std::fs::create_dir(&root).unwrap();
                std::fs::create_dir(root.join("secrets")).unwrap();
                std::fs::create_dir(root.join("desktop")).unwrap();
                Self(root)
            }
        }
        impl Drop for Root {
            fn drop(&mut self) {
                // Recheck the resolved scope before recursive fixture cleanup.
                if validate(&self.0, 22332).is_ok() {
                    std::fs::remove_dir_all(&self.0).unwrap();
                }
            }
        }
        #[test]
        fn manual_fixture_rejects_arbitrary_roots_and_privileged_ports() {
            let root = Root::new();
            assert!(validate(&root.0, 22332).is_ok());
            assert!(validate(&root.0, 22).is_err());
            assert!(validate(&root.0.join("secrets"), 22332).is_err());
            assert!(validate(Path::new("relative"), 22332).is_err());
            let outside = tempfile::tempdir().unwrap();
            assert!(validate(outside.path(), 22332).is_err());
        }
        #[tokio::test]
        async fn seeded_vault_is_locked_pinned_and_refuses_replacement() {
            let root = Root::new();
            let master = Uuid::new_v4().to_string();
            std::fs::write(root.0.join("secrets/master.txt"), &master).unwrap();
            std::fs::write(
                root.0.join("secrets/password.txt"),
                Uuid::new_v4().to_string(),
            )
            .unwrap();
            let key =
                russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)
                    .unwrap();
            std::fs::write(
                root.0.join("secrets/host-key.pub"),
                key.public_key().to_openssh().unwrap(),
            )
            .unwrap();
            prepare(&root.0, 22332).await.unwrap();
            let path = root.0.join("desktop/data/vault.capstan");
            let before = std::fs::read(&path).unwrap();
            assert!(!String::from_utf8_lossy(&before).contains(&master));
            let vault = VaultManager::new(path.clone());
            assert!(!vault.status().unlocked);
            vault.unlock(SecretText::new(master)).await.unwrap();
            assert_eq!(vault.hosts().unwrap().len(), 2);
            assert_eq!(vault.credentials().unwrap().len(), 1);
            assert_eq!(vault.trusted_host_keys().unwrap().len(), 1);
            vault.lock().await.unwrap();
            assert!(prepare(&root.0, 22332).await.is_err());
            assert_eq!(std::fs::read(path).unwrap(), before);
        }
    }
}

#[cfg(windows)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.len() != 2 {
        return Err("usage: m2_manual_fixture ABSOLUTE_FIXTURE_ROOT PORT".into());
    }
    let root = std::path::Path::new(&arguments[0]);
    let port = arguments[1].to_str().ok_or("invalid port")?.parse()?;
    windows::prepare(root, port).await?;
    println!("{{\"prepared\":true,\"hosts\":2,\"secretsPrinted\":false}}");
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("The manual fixture requires Windows.");
}
