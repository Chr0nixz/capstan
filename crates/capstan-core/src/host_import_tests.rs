use super::*;
use crate::ssh_config::{self, Environment, SystemFiles};
use capstan_vault::KdfParameters;

async fn unlocked(directory: &Path) -> VaultManager {
    let manager = VaultManager::new(directory.join("vault.capstan"));
    manager
        .initialize(
            SecretText::new("master password for tests".into()),
            KdfParameters {
                memory_kib: 19_456,
                iterations: 2,
                parallelism: 1,
            },
        )
        .await
        .unwrap();
    manager
}

fn key_text(passphrase: Option<&str>) -> String {
    let key =
        russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519).unwrap();
    let key = match passphrase {
        Some(passphrase) => key.encrypt(&mut rand::rng(), passphrase).unwrap(),
        None => key,
    };
    key.to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .unwrap()
        .to_string()
}

#[test]
fn quick_targets_accept_common_ssh_spellings_only() {
    let target = |text| {
        parse_quick_target(text).map(|target| (target.username.unwrap(), target.host, target.port))
    };
    assert_eq!(
        target("deploy@web-01"),
        Some(("deploy".into(), "web-01".into(), 22))
    );
    assert_eq!(
        target("  ssh deploy@web-01 -p 2222 "),
        Some(("deploy".into(), "web-01".into(), 2222))
    );
    assert_eq!(
        target("deploy@10.0.0.5:2200"),
        Some(("deploy".into(), "10.0.0.5".into(), 2200))
    );
    assert_eq!(
        target("ssh://root@[2001:db8::1]:2022"),
        Some(("root".into(), "2001:db8::1".into(), 2022))
    );
    assert_eq!(
        target("-p 22 ops@db"),
        Some(("ops".into(), "db".into(), 22))
    );
    for invalid in [
        "web-01",
        "deploy@",
        "deploy@web-01 extra@host",
        "deploy@web-01 -i key",
        "deploy@web-01 -p",
        "deploy@web-01 -p 0",
        "deploy@web-01:2200 -p 2222",
        "deploy@web-01/path",
        "ssh",
        "deploy@web\u{7}01",
        "deploy@two words",
    ] {
        assert_eq!(parse_quick_target(invalid), None, "{invalid}");
    }
    assert_eq!(
        display_name("root", "2001:db8::1", 2022),
        "root@[2001:db8::1]:2022"
    );
    assert_eq!(display_name("deploy", "web-01", 22), "deploy@web-01");
}

#[tokio::test]
async fn quick_connect_reuses_saved_hosts_and_one_asking_credential() {
    let directory = tempfile::tempdir().unwrap();
    let manager = unlocked(directory.path()).await;
    let first = manager
        .quick_connect_host("deploy@WEB-01.example.test", "Ask at connect".into())
        .await
        .unwrap();
    assert!(first.created);
    let again = manager
        .quick_connect_host("ssh deploy@web-01.example.test -p 22", "unused".into())
        .await
        .unwrap();
    assert_eq!(
        again,
        QuickConnect {
            host_id: first.host_id,
            created: false
        }
    );
    let other = manager
        .quick_connect_host("ops@web-01.example.test:2222", "unused".into())
        .await
        .unwrap();
    assert!(other.created);

    let hosts = manager.hosts().unwrap();
    assert_eq!(hosts.len(), 2);
    assert_eq!(hosts[0].name, "deploy@WEB-01.example.test");
    assert_eq!(hosts[0].address, "web-01.example.test");
    assert_eq!(hosts[1].port, 2222);
    let credentials = manager.credentials().unwrap();
    assert_eq!(credentials.len(), 1);
    assert_eq!(credentials[0].kind, "interactive");
    assert_eq!(credentials[0].name, "Ask at connect");
    assert!(hosts
        .iter()
        .all(|host| host.credential_id == credentials[0].id));

    for invalid in ["web-01", "deploy@bad host", "deploy@-bad"] {
        assert!(manager
            .quick_connect_host(invalid, "Ask".into())
            .await
            .is_err());
    }
    assert!(manager
        .quick_connect_host("deploy@new-host", " ".into())
        .await
        .is_err());
    assert_eq!(manager.hosts().unwrap().len(), 2);
}

struct Home {
    directory: tempfile::TempDir,
}
impl Home {
    fn new(config: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let ssh = directory.path().join(".ssh");
        std::fs::create_dir(&ssh).unwrap();
        std::fs::write(ssh.join("config"), config).unwrap();
        Self { directory }
    }
    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.directory.path().join(".ssh").join(name), text).unwrap();
    }
    fn key(&self, name: &str) -> PathBuf {
        self.directory.path().join(".ssh").join(name)
    }
    fn parse(&self) -> Vec<Candidate> {
        ssh_config::parse(&Environment {
            home: self.directory.path(),
            local_user: Some("me"),
            files: &SystemFiles,
        })
        .unwrap()
        .candidates
    }
}

fn names() -> ImportNames {
    ImportNames {
        interactive_credential: "Ask at connect".into(),
        group: "SSH config".into(),
    }
}

#[tokio::test]
async fn ssh_config_import_checks_keys_and_skips_saved_hosts() {
    let home = Home::new(
        "Host plain\n HostName 10.0.0.1\n User a\n IdentityFile ~/.ssh/plain_key\n\
         Host locked\n HostName 10.0.0.2\n User b\n IdentityFile ~/.ssh/locked_key\n\
         Host broken\n HostName 10.0.0.3\n User c\n IdentityFile ~/.ssh/broken_key\n\
         Host saved\n HostName 10.0.0.4\n User d\n IdentityFile ~/.ssh/plain_key\n\
         Host twin\n HostName 10.0.0.1\n User a\n",
    );
    home.write("plain_key", &key_text(None));
    home.write("locked_key", &key_text(Some("open sesame")));
    home.write("broken_key", "not a key");
    let directory = tempfile::tempdir().unwrap();
    let manager = unlocked(directory.path()).await;
    manager
        .quick_connect_host("d@10.0.0.4", "Ask at connect".into())
        .await
        .unwrap();

    let (plan, previews) = manager.plan_ssh_config_import(home.parse()).unwrap();
    let summary: Vec<_> = previews
        .iter()
        .map(|preview| {
            (
                preview.alias.as_str(),
                preview.identity_file.is_some(),
                preview.encrypted,
                preview.saved,
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            ("plain", true, false, false),
            ("locked", true, true, false),
            ("broken", false, false, false),
            ("saved", true, false, true),
            ("twin", false, false, false),
        ]
    );
    assert_eq!(previews[2].notes, vec![Note::IdentityUnavailable]);
    assert_eq!(
        plan.encrypted_keys(&[0, 1, 3]),
        vec![home.key("locked_key")]
    );
    assert!(plan.encrypted_keys(&[0, 2]).is_empty());

    let revision = manager.current().unwrap().snapshot().unwrap().revision;
    let wrong = manager
        .import_ssh_config_hosts(
            &plan,
            &[0, 1, 2, 3, 4],
            vec![(home.key("locked_key"), "wrong".into())],
            names(),
        )
        .await;
    assert!(wrong.is_err());
    assert_eq!(
        manager.current().unwrap().snapshot().unwrap().revision,
        revision
    );

    let summary = manager
        .import_ssh_config_hosts(
            &plan,
            &[0, 1, 2, 3, 4],
            vec![(home.key("locked_key"), "open sesame".into())],
            names(),
        )
        .await
        .unwrap();
    // `twin` matches `plain`'s address and user, and `saved` already exists.
    assert_eq!(
        summary,
        ImportSummary {
            created: 3,
            existing: 2
        }
    );
    let data = manager.current().unwrap().snapshot().unwrap();
    assert_eq!(data.groups.len(), 1);
    assert_eq!(data.groups[0].name, "SSH config");
    let kinds: Vec<_> = data
        .credentials
        .iter()
        .map(|credential| credential.material.kind())
        .collect();
    assert_eq!(kinds, vec!["interactive", "key_file", "key_file"]);
    let imported: Vec<_> = data
        .hosts
        .iter()
        .filter(|host| host.group_id == Some(data.groups[0].id))
        .map(|host| {
            let credential = data
                .credentials
                .iter()
                .find(|credential| credential.id == host.credential_id)
                .unwrap();
            (host.name.as_str(), credential.material.kind())
        })
        .collect();
    assert_eq!(
        imported,
        vec![
            ("plain", "key_file"),
            ("locked", "key_file"),
            ("broken", "interactive")
        ]
    );
    assert!(data.credentials.iter().any(|credential| matches!(
        &credential.material,
        CredentialMaterial::KeyFile { passphrase: Some(passphrase), .. } if passphrase.expose() == "open sesame"
    )));

    // A second import adds nothing and reuses the group and credentials.
    let (plan, _) = manager.plan_ssh_config_import(home.parse()).unwrap();
    let summary = manager
        .import_ssh_config_hosts(&plan, &[0, 1, 2, 3, 4], Vec::new(), names())
        .await
        .unwrap();
    assert_eq!(
        summary,
        ImportSummary {
            created: 0,
            existing: 5
        }
    );
    let after = manager.current().unwrap().snapshot().unwrap();
    assert_eq!(after.revision, data.revision);
}

#[tokio::test]
async fn ssh_config_import_reuses_key_credentials_and_rejects_bad_names() {
    let home = Home::new("Host one\n HostName 10.0.1.1\n User a\n IdentityFile ~/.ssh/key\n");
    home.write("key", &key_text(None));
    let directory = tempfile::tempdir().unwrap();
    let manager = unlocked(directory.path()).await;
    manager
        .add_key_file_credential("mine".into(), home.key("key"), None)
        .await
        .unwrap();
    let (plan, _) = manager.plan_ssh_config_import(home.parse()).unwrap();
    assert!(manager
        .import_ssh_config_hosts(
            &plan,
            &[0],
            Vec::new(),
            ImportNames {
                interactive_credential: "Ask".into(),
                group: "\u{7}".into(),
            },
        )
        .await
        .is_err());
    let summary = manager
        .import_ssh_config_hosts(&plan, &[0], Vec::new(), names())
        .await
        .unwrap();
    assert_eq!(summary.created, 1);
    let credentials = manager.credentials().unwrap();
    assert_eq!(credentials.len(), 1);
    assert_eq!(credentials[0].name, "mine");
    // Indices outside the plan select nothing.
    let summary = manager
        .import_ssh_config_hosts(&plan, &[7], Vec::new(), names())
        .await
        .unwrap();
    assert_eq!(
        summary,
        ImportSummary {
            created: 0,
            existing: 0
        }
    );
}

#[tokio::test]
async fn ssh_config_import_brings_jump_hosts_along_and_matches_by_route() {
    let home = Home::new(
        "Host db\n HostName 10.0.0.5\n User app\n ProxyJump bastion\n\
         Host bastion\n HostName 10.0.0.1\n User ops\n IdentityFile ~/.ssh/locked_key\n\
         Host db-direct\n HostName 10.0.0.5\n User app\n",
    );
    home.write("locked_key", &key_text(Some("open sesame")));
    let directory = tempfile::tempdir().unwrap();
    let manager = unlocked(directory.path()).await;

    let (plan, previews) = manager.plan_ssh_config_import(home.parse()).unwrap();
    let jumps: Vec<_> = previews
        .iter()
        .map(|preview| (preview.alias.as_str(), preview.jump, preview.saved))
        .collect();
    assert_eq!(
        jumps,
        vec![
            ("db", Some(1), false),
            ("bastion", None, false),
            ("db-direct", None, false)
        ]
    );
    // Importing db alone needs the jump host's passphrase too.
    assert_eq!(plan.encrypted_keys(&[0]), vec![home.key("locked_key")]);

    let summary = manager
        .import_ssh_config_hosts(
            &plan,
            &[0],
            vec![(home.key("locked_key"), "open sesame".into())],
            names(),
        )
        .await
        .unwrap();
    assert_eq!(
        summary,
        ImportSummary {
            created: 2,
            existing: 0
        }
    );
    let hosts = manager.current().unwrap().snapshot().unwrap().hosts;
    let bastion = hosts.iter().find(|host| host.name == "bastion").unwrap();
    let db = hosts.iter().find(|host| host.name == "db").unwrap();
    assert_eq!(db.jump_host_id, Some(bastion.id));
    assert_eq!(bastion.jump_host_id, None);

    // The same address reached directly is a different host.
    let (plan, previews) = manager.plan_ssh_config_import(home.parse()).unwrap();
    assert_eq!(
        previews
            .iter()
            .map(|preview| preview.saved)
            .collect::<Vec<_>>(),
        vec![true, true, false]
    );
    let summary = manager
        .import_ssh_config_hosts(&plan, &[0, 1, 2], Vec::new(), names())
        .await
        .unwrap();
    assert_eq!(
        summary,
        ImportSummary {
            created: 1,
            existing: 2
        }
    );
    let hosts = manager.hosts().unwrap();
    assert_eq!(hosts.len(), 3);
    let direct = hosts.iter().find(|host| host.name == "db-direct").unwrap();
    assert_eq!(direct.jump_host_id, None);
    // Quick connect matches only hosts reached directly.
    let quick = manager
        .quick_connect_host("app@10.0.0.5", "Ask".into())
        .await
        .unwrap();
    assert_eq!(quick.host_id, direct.id);
}

#[tokio::test]
async fn ssh_config_import_reaches_new_hosts_through_a_saved_jump_host() {
    let home = Home::new("Host db\n HostName 10.0.0.5\n User app\n ProxyJump ops@10.0.0.1\n");
    let directory = tempfile::tempdir().unwrap();
    let manager = unlocked(directory.path()).await;
    let saved = manager
        .quick_connect_host("ops@10.0.0.1", "Ask at connect".into())
        .await
        .unwrap();
    let (plan, previews) = manager.plan_ssh_config_import(home.parse()).unwrap();
    assert_eq!(previews.len(), 2);
    assert_eq!(previews[1].alias, "ops@10.0.0.1");
    assert!(previews[1].saved);
    let summary = manager
        .import_ssh_config_hosts(&plan, &[0], Vec::new(), names())
        .await
        .unwrap();
    assert_eq!(summary.created, 1);
    let hosts = manager.current().unwrap().snapshot().unwrap().hosts;
    let db = hosts.iter().find(|host| host.name == "db").unwrap();
    assert_eq!(db.jump_host_id, Some(saved.host_id));
}
