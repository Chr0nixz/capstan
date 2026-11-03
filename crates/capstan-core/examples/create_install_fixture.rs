//! Creates disposable encrypted data for the guarded installer acceptance tool.

use capstan_core::VaultManager;
use capstan_vault::{Host, KdfParameters, SecretText};
use std::path::PathBuf;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("expected fixture directory")?,
    );
    std::fs::create_dir(&directory)?;
    let manager = VaultManager::new(directory.join("vault.capstan"));
    manager
        .initialize(
            SecretText::new(Uuid::new_v4().to_string()),
            KdfParameters {
                memory_kib: 19_456,
                iterations: 2,
                parallelism: 1,
            },
        )
        .await?;
    let credential_id = manager
        .add_password_credential("Installer probe".into(), Uuid::new_v4().to_string())
        .await?;
    manager
        .save_host(
            Host {
                id: Uuid::new_v4(),
                name: "Installer probe".into(),
                address: "acceptance.invalid".into(),
                port: 2222,
                username: "synthetic".into(),
                credential_id,
                group_id: None,
                tags: vec![],
                favorite: false,
                revision: 0,
                working_directory: None,
                startup_command: None,
                encoding: "utf-8".into(),
                theme: "graphite".into(),
                agent_forwarding: false,
                production: false,
            },
            manager.revision()?,
        )
        .await?;
    manager.lock().await?;
    println!("Encrypted installer fixture created; its runtime master password was discarded.");
    Ok(())
}
