use super::*;
use capstan_core::sftp::{DeleteOutcome, DeleteTreeSummary};

fn seed(state: &RemoteState) {
    let mut files = state.files.lock().unwrap();
    for (path, mode) in [
        ("/files", 0o40700),
        ("/files/tree", 0o40700),
        ("/files/tree/sub", 0o40700),
        ("/files/tree/a", 0o100600),
        ("/files/tree/sub/b", 0o100600),
        ("/files/tree/link", 0o120777),
        ("/files/outside", 0o100600),
    ] {
        files.insert(
            path.into(),
            Record {
                data: b"keep outside target".to_vec(),
                mode,
                mtime: 100,
            },
        );
    }
}
async fn prepare(
    fixture: &Fixture,
    session: SftpSessionId,
) -> Result<DeleteTreeSummary, SftpError> {
    let page = fixture.list(session, "/files").await?;
    let entry = page
        .entries
        .iter()
        .find(|entry| entry.name == "tree")
        .unwrap();
    fixture
        .service
        .prepare_tree_delete("main", "one", session, entry.id)
        .await
}
#[tokio::test]
async fn recursive_delete_is_bound_single_use_and_unlinks_symlinks_without_traversing() {
    let (fixture, session, _) = fixture().await;
    seed(&fixture.observations.transfer);
    let plan = prepare(&fixture, session).await.unwrap();
    assert_eq!((plan.files, plan.directories, plan.symlinks), (2, 2, 1));
    let second = fixture.attach("two").await;
    assert!(matches!(
        fixture
            .service
            .apply_tree_delete("main", "two", second, plan.token)
            .await,
        Err(SftpError::ScopeExpired)
    ));
    let report = fixture
        .service
        .apply_tree_delete("main", "one", session, plan.token)
        .await
        .unwrap();
    assert!(report
        .entries
        .iter()
        .all(|entry| entry.outcome == DeleteOutcome::Removed));
    assert_eq!(report.entries.len(), 5);
    assert!(matches!(
        fixture
            .service
            .apply_tree_delete("main", "one", session, plan.token)
            .await,
        Err(SftpError::ScopeExpired)
    ));
    assert!(!fixture
        .observations
        .transfer
        .files
        .lock()
        .unwrap()
        .keys()
        .any(|path| path.starts_with("/files/tree")));
    assert_eq!(
        fixture.observations.transfer.bytes("/files/outside"),
        b"keep outside target"
    );
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 1);
    fixture.close().await;
}
#[tokio::test]
async fn recursive_delete_reports_partial_failure_and_preserves_failed_ancestors() {
    let (fixture, session, _) = fixture().await;
    seed(&fixture.observations.transfer);
    let plan = prepare(&fixture, session).await.unwrap();
    fixture
        .observations
        .transfer
        .denied_removals
        .lock()
        .unwrap()
        .insert("/files/tree/sub/b".into());
    let report = fixture
        .service
        .apply_tree_delete("main", "one", session, plan.token)
        .await
        .unwrap();
    let failed = report
        .entries
        .iter()
        .find(|entry| entry.path.as_str().ends_with("/sub/b"))
        .unwrap();
    assert_eq!(failed.error, Some(SftpError::PermissionDenied));
    assert_eq!(failed.outcome, DeleteOutcome::Failed);
    assert_eq!(
        report
            .entries
            .iter()
            .filter(|entry| entry.outcome == DeleteOutcome::Retained)
            .count(),
        2
    );
    assert!(!fixture
        .observations
        .transfer
        .files
        .lock()
        .unwrap()
        .contains_key("/files/tree/a"));
    assert!(fixture
        .observations
        .transfer
        .files
        .lock()
        .unwrap()
        .contains_key("/files/tree/sub/b"));
    fixture.close().await;
}
#[tokio::test]
async fn recursive_delete_rejects_changed_trees_and_unsupported_names_before_mutation() {
    let (fixture, session, _) = fixture().await;
    seed(&fixture.observations.transfer);
    let plan = prepare(&fixture, session).await.unwrap();
    fixture
        .observations
        .transfer
        .put("/files/tree/sub/new", b"new user file");
    assert!(matches!(
        fixture
            .service
            .apply_tree_delete("main", "one", session, plan.token)
            .await,
        Err(SftpError::TargetConflict)
    ));
    assert!(fixture
        .observations
        .transfer
        .files
        .lock()
        .unwrap()
        .contains_key("/files/tree/a"));
    fixture
        .observations
        .transfer
        .put("/files/tree/bad\u{fffd}", b"unsupported name");
    assert!(matches!(
        prepare(&fixture, session).await,
        Err(SftpError::UnsupportedFileType | SftpError::PathInvalid)
    ));
    assert_eq!(fixture.observations.handles.load(Ordering::SeqCst), 0);
    fixture.close().await;
}
#[tokio::test]
async fn recursive_delete_cancellation_waits_for_the_admitted_ack_and_retains_the_rest() {
    let (fixture, session, _) = fixture().await;
    seed(&fixture.observations.transfer);
    let plan = prepare(&fixture, session).await.unwrap();
    let state = &fixture.observations.transfer;
    let settings_root = tempfile::tempdir().unwrap();
    let settings = Arc::new(capstan_core::settings::Settings::new(
        settings_root.path().to_owned(),
    ));
    #[cfg(windows)]
    let settings_session = settings.snapshot().unwrap().session;
    let transfers = TransferService::default();
    state.stall_remove.store(true, Ordering::SeqCst);
    let apply = fixture
        .service
        .apply_tree_delete("main", "one", session, plan.token);
    let cancel = async {
        tokio::time::timeout(Duration::from_secs(3), state.started.notified())
            .await
            .unwrap();
        fixture
            .service
            .cancel_tree_delete("main", "one", session, plan.token)
            .unwrap();
        let exit = capstan_core::application::prepare_file_workflow_exit(
            &settings,
            &transfers,
            &fixture.service,
            "main",
        );
        tokio::pin!(exit);
        assert!(tokio::time::timeout(Duration::from_millis(30), &mut exit)
            .await
            .is_err());
        #[cfg(windows)]
        assert_eq!(
            settings
                .clone()
                .save(
                    settings_session,
                    capstan_core::sftp::ExactU64(1),
                    capstan_core::settings::PanelLayout::default(),
                )
                .await,
            Err(capstan_core::settings::SettingsError::Unavailable)
        );
        state.stall_remove.store(false, Ordering::SeqCst);
        state.release.notify_one();
        exit.await.unwrap();
    };
    let (report, ()) = tokio::join!(apply, cancel);
    let report = report.unwrap();
    assert_eq!(
        report
            .entries
            .iter()
            .filter(|entry| entry.outcome == DeleteOutcome::Removed)
            .count(),
        1
    );
    assert_eq!(
        report
            .entries
            .iter()
            .filter(|entry| entry.outcome == DeleteOutcome::Retained)
            .count(),
        4
    );
    assert!(report
        .entries
        .iter()
        .filter(|entry| entry.outcome == DeleteOutcome::Retained)
        .all(|entry| entry.error == Some(SftpError::OperationCanceled)));
    assert!(!fixture.service.has_active_tree_deletes("main").unwrap());
    assert!(matches!(
        prepare(&fixture, session).await,
        Err(SftpError::ScopeExpired)
    ));
    fixture.service.abort_tree_delete_exit("main");
    fixture.close().await;
}

#[tokio::test]
async fn recursive_delete_rejects_entry_depth_and_path_limits_without_mutation() {
    let (fixture, session, _) = fixture().await;
    let state = &fixture.observations.transfer;
    for scenario in ["entries", "depth", "path_bytes"] {
        state.files.lock().unwrap().clear();
        seed(state);
        match scenario {
            "entries" => {
                for directory in 0..32 {
                    let parent = format!("/files/tree/dir{directory}");
                    state.files.lock().unwrap().insert(
                        parent.clone(),
                        Record {
                            data: vec![],
                            mode: 0o40700,
                            mtime: 100,
                        },
                    );
                    for index in 0..128 {
                        state.put(&format!("{parent}/f{index}"), b"keep");
                    }
                }
            }
            "depth" => {
                let mut path = "/files/tree".to_owned();
                let mut files = state.files.lock().unwrap();
                for _ in 0..64 {
                    path.push_str("/d");
                    files.insert(
                        path.clone(),
                        Record {
                            data: vec![],
                            mode: 0o40700,
                            mtime: 100,
                        },
                    );
                }
            }
            "path_bytes" => {
                for directory in 0..8 {
                    let parent = format!("/files/tree/dir{directory}");
                    state.files.lock().unwrap().insert(
                        parent.clone(),
                        Record {
                            data: vec![],
                            mode: 0o40700,
                            mtime: 100,
                        },
                    );
                    for index in 0..150 {
                        state.put(&format!("{parent}/{index:04}{}", "x".repeat(250)), b"keep");
                    }
                }
            }
            _ => unreachable!(),
        }
        let count = state.files.lock().unwrap().len();
        let result = prepare(&fixture, session).await;
        assert!(
            matches!(result, Err(SftpError::ResourceLimit)),
            "{scenario} limit must reject the preview: {result:?}"
        );
        assert_eq!(state.files.lock().unwrap().len(), count);
        assert_eq!(state.bytes("/files/tree/a"), b"keep outside target");
        assert_eq!(fixture.observations.handles.load(Ordering::SeqCst), 0);
    }
    fixture.close().await;
}

#[tokio::test]
async fn recursive_delete_canceled_previews_and_directory_type_changes_never_delete() {
    let (fixture, session, _) = fixture().await;
    seed(&fixture.observations.transfer);
    let plan = prepare(&fixture, session).await.unwrap();
    fixture
        .service
        .cancel_tree_delete("main", "one", session, plan.token)
        .unwrap();
    assert!(matches!(
        fixture
            .service
            .apply_tree_delete("main", "one", session, plan.token)
            .await,
        Err(SftpError::ScopeExpired)
    ));
    let plan = prepare(&fixture, session).await.unwrap();
    fixture
        .observations
        .transfer
        .files
        .lock()
        .unwrap()
        .get_mut("/files/tree/sub")
        .unwrap()
        .mode = 0o120777;
    assert!(matches!(
        fixture
            .service
            .apply_tree_delete("main", "one", session, plan.token)
            .await,
        Err(SftpError::TargetConflict)
    ));
    assert_eq!(
        fixture.observations.transfer.bytes("/files/tree/sub/b"),
        b"keep outside target"
    );
    fixture.close().await;
}
