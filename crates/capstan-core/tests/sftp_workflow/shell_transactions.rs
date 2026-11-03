use super::*;
use capstan_core::shell_integration::{apply_remote_plan, plan, Action, IntegrationError, Shell};

#[tokio::test]
async fn shell_install_repeat_and_uninstall_preserve_user_bytes_and_permissions() {
    for shell in [Shell::Bash, Shell::Zsh] {
        let (fixture, session, transfers) = fixture().await;
        let remote = &fixture.observations.transfer;
        let path = RemotePath::parse("/files/config").unwrap();
        let original = b"# user configuration\r\nexport KEEP=yes";
        remote.put(path.as_str(), original);
        remote
            .files
            .lock()
            .unwrap()
            .get_mut(path.as_str())
            .unwrap()
            .mode = 0o100640;
        let client = fixture
            .lease
            .open_sftp(&CancellationToken::new())
            .await
            .unwrap();
        let cancel = CancellationToken::new();
        let gate = transfers.publication_gate();
        for action in [
            Action::Install,
            Action::Install,
            Action::Uninstall,
            Action::Uninstall,
        ] {
            let source = remote.bytes(path.as_str());
            let page = fixture.list(session, "/files").await.unwrap();
            let expected = &page
                .entries
                .iter()
                .find(|entry| entry.name == "config")
                .unwrap()
                .metadata;
            let planned = plan(&source, shell, action, "host").unwrap();
            let before = remote.commits.load(Ordering::SeqCst);
            let result =
                apply_remote_plan(&client, &path, &source, &planned, expected, &gate, &cancel)
                    .await
                    .unwrap();
            assert_eq!(result.backup_created, planned.changed);
            assert_eq!(
                remote.commits.load(Ordering::SeqCst) - before,
                if planned.changed { 2 } else { 0 }
            );
            assert_eq!(remote.bytes(path.as_str()), planned.replacement);
            assert_eq!(remote.files.lock().unwrap()[path.as_str()].mode, 0o100640);
            if action == Action::Install && planned.changed {
                let mut edited = remote.bytes(path.as_str());
                edited.extend_from_slice(b"# appended by user\n");
                remote.put(path.as_str(), &edited);
                // A shell append changes bytes/mtime, not the existing mode.
                remote
                    .files
                    .lock()
                    .unwrap()
                    .get_mut(path.as_str())
                    .unwrap()
                    .mode = 0o100640;
            }
        }
        assert_eq!(
            remote.bytes(path.as_str()),
            [original.as_slice(), b"\n# appended by user\n"].concat()
        );
        let backups = remote
            .files
            .lock()
            .unwrap()
            .iter()
            .filter(|(path, _)| path.contains(".capstan-backup-"))
            .map(|(_, file)| (file.data.clone(), file.mode))
            .collect::<Vec<_>>();
        assert_eq!(backups.len(), 2);
        assert!(backups.iter().all(|(_, mode)| *mode == 0o100600));
        assert!(backups.iter().any(|(bytes, _)| bytes == original));
        assert!(remote.no_partials());
        fixture.close().await;
    }
}

#[tokio::test]
async fn shell_transactions_reject_equal_metadata_changes_and_preserve_original_on_failure() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    let path = RemotePath::parse("/files/config").unwrap();
    let original = b"# user\n";
    remote.put(path.as_str(), original);
    let page = fixture.list(session, "/files").await.unwrap();
    let expected = &page
        .entries
        .iter()
        .find(|entry| entry.name == "config")
        .unwrap()
        .metadata;
    let planned = plan(original, Shell::Bash, Action::Install, "host").unwrap();
    let client = fixture
        .lease
        .open_sftp(&CancellationToken::new())
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    let gate = transfers.publication_gate();
    remote
        .files
        .lock()
        .unwrap()
        .get_mut(path.as_str())
        .unwrap()
        .data = b"# edit\n".to_vec();
    assert_eq!(
        apply_remote_plan(&client, &path, original, &planned, expected, &gate, &cancel)
            .await
            .unwrap_err(),
        IntegrationError::Conflict
    );
    remote
        .files
        .lock()
        .unwrap()
        .get_mut(path.as_str())
        .unwrap()
        .data = original.to_vec();
    for fail_write in [true, false] {
        remote.deny_write.store(fail_write, Ordering::SeqCst);
        remote.deny_commit.store(!fail_write, Ordering::SeqCst);
        assert!(
            apply_remote_plan(&client, &path, original, &planned, expected, &gate, &cancel)
                .await
                .is_err()
        );
        assert_eq!(remote.bytes(path.as_str()), original);
        assert!(remote.no_partials());
    }
    fixture.close().await;
}
