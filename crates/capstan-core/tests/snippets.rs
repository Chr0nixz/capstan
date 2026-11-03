use capstan_core::{BackupImportMode, VaultManager};
use capstan_vault::{KdfParameters, SecretText, Snippet, SnippetGroup, VaultData};
use uuid::Uuid;

fn password() -> SecretText {
    SecretText::new("snippet test vault password".into())
}

fn parameters() -> KdfParameters {
    KdfParameters {
        memory_kib: 19_456,
        iterations: 2,
        parallelism: 1,
    }
}

#[tokio::test]
async fn nonempty_snippets_survive_restart_and_errors_preserve_the_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snippets.cav");
    let manager = VaultManager::new(path.clone());
    manager.initialize(password(), parameters()).await.unwrap();
    let group = SnippetGroup {
        id: Uuid::new_v4(),
        name: "Snippet group".into(),
    };
    manager.save_snippet_group(group.clone(), 0).await.unwrap();
    let snippet = Snippet {
        id: Uuid::new_v4(),
        group_id: Some(group.id),
        name: "Change directory".into(),
        template: "cd {{目录:/tmp}}; echo SNIPPET_PLAINTEXT_SENTINEL".into(),
        auto_enter: true,
    };
    manager.save_snippet(snippet.clone(), 1).await.unwrap();
    let baseline = std::fs::read(&path).unwrap();
    assert!(!baseline
        .windows(b"SNIPPET_PLAINTEXT_SENTINEL".len())
        .any(|window| window == b"SNIPPET_PLAINTEXT_SENTINEL"));
    assert_eq!(
        manager
            .save_snippet(snippet.clone(), 1)
            .await
            .unwrap_err()
            .code(),
        "vault_conflict"
    );
    assert_eq!(
        manager
            .delete_snippet(Uuid::new_v4(), 2)
            .await
            .unwrap_err()
            .code(),
        "not_found"
    );
    assert_eq!(
        manager
            .delete_snippet_group(group.id, 2)
            .await
            .unwrap_err()
            .code(),
        "group_in_use"
    );
    let mut invalid = snippet.clone();
    invalid.group_id = Some(Uuid::new_v4());
    assert_eq!(
        manager.save_snippet(invalid, 2).await.unwrap_err().code(),
        "invalid_input"
    );
    let mut oversized = snippet.clone();
    oversized.template = "x".repeat(8193);
    assert_eq!(
        manager.save_snippet(oversized, 2).await.unwrap_err().code(),
        "invalid_input"
    );
    assert_eq!(manager.revision().unwrap(), 2);
    assert_eq!(std::fs::read(&path).unwrap(), baseline);
    manager.lock().await.unwrap();
    assert_eq!(manager.snippets().unwrap_err().code(), "vault_locked");
    assert_eq!(
        manager
            .save_snippet(snippet.clone(), 2)
            .await
            .unwrap_err()
            .code(),
        "vault_locked"
    );
    assert_eq!(std::fs::read(&path).unwrap(), baseline);
    let reopened = VaultManager::new(path);
    reopened.unlock(password()).await.unwrap();
    let saved = reopened.snippets().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].template, snippet.template);
    assert_eq!(saved[0].group_id, Some(group.id));
    assert!(saved[0].auto_enter);
    assert_eq!(reopened.snippet_groups().unwrap()[0].id, group.id);
}

#[tokio::test]
async fn snippet_backup_roundtrip_preserves_content_and_remaps_group_references() {
    let directory = tempfile::tempdir().unwrap();
    let source = VaultManager::new(directory.path().join("source.cav"));
    source.initialize(password(), parameters()).await.unwrap();
    let group = SnippetGroup {
        id: Uuid::new_v4(),
        name: "Snippet group".into(),
    };
    source.save_snippet_group(group.clone(), 0).await.unwrap();
    let snippet = Snippet {
        id: Uuid::new_v4(),
        group_id: Some(group.id),
        name: "Change directory".into(),
        template: "cd {{目录:/tmp}}".into(),
        auto_enter: false,
    };
    source.save_snippet(snippet.clone(), 1).await.unwrap();
    let backup = directory.path().join("snippets.cbak");
    source
        .export_backup(backup.clone(), password())
        .await
        .unwrap();
    let destination = VaultManager::new(directory.path().join("destination.cav"));
    destination
        .initialize(password(), parameters())
        .await
        .unwrap();
    let preview = destination
        .preview_backup(backup.clone(), password())
        .await
        .unwrap();
    assert_eq!(preview.snippet_count, 1);
    assert_eq!(preview.snippet_group_count, 1);
    destination
        .import_backup(
            backup,
            password(),
            BackupImportMode::Merge,
            0,
            destination.epoch(),
        )
        .await
        .unwrap();
    let merged = destination.snippets().unwrap();
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].template, snippet.template);
    assert!(!merged[0].auto_enter);
    assert_ne!(merged[0].id, snippet.id);
    assert_ne!(merged[0].group_id, snippet.group_id);
    assert_eq!(
        merged[0].group_id,
        Some(destination.snippet_groups().unwrap()[0].id)
    );
    destination.lock().await.unwrap();
    destination.unlock(password()).await.unwrap();
    assert_eq!(
        destination.snippets().unwrap()[0].template,
        snippet.template
    );
}

#[test]
fn schema_one_payload_without_snippet_fields_still_decodes() {
    let mut old = serde_json::to_value(VaultData::default()).unwrap();
    old.as_object_mut().unwrap().remove("snippets");
    old.as_object_mut().unwrap().remove("snippet_groups");
    let decoded: VaultData = serde_json::from_value(old).unwrap();
    decoded.validate().unwrap();
    assert!(decoded.snippets.is_empty());
    assert!(decoded.snippet_groups.is_empty());
}
