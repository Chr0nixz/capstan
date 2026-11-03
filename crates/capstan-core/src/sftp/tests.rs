use super::*;
use std::time::{Duration, Instant};

#[test]
fn runtime_budgets_match_the_frozen_pre_candidate_policy() {
    let policy: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../docs/agent/baselines/M2/performance-policy.json"
    ))
    .unwrap();
    assert_eq!(policy["status"], "frozen_before_product_candidate");
    assert_eq!(policy["largeFileMinimumWinScpThroughputRatio"], 0.8);
    assert_eq!(policy["smallTreeMaximumWinScpDurationRatio"], 1.5);
    let limits = &policy["transferResourceLimits"];
    for (key, actual) in [
        ("requestPayloadBytes", REQUEST_BYTES as u64),
        ("requestsPerFile", REQUESTS_PER_FILE as u64),
        ("globalTransferBufferBytes", MAX_BUFFER_BYTES as u64),
        ("globalWorkers", TRANSFER_WORKERS as u64),
        ("queuedTasks", MAX_QUEUED_TRANSFERS as u64),
        ("retainedCompletedRows", MAX_FINAL_TRANSFERS as u64),
        ("localUnusedGrants", MAX_LOCAL_GRANTS as u64),
        ("localGrantLifetimeSeconds", LOCAL_GRANT_LIFETIME_SECONDS),
        ("exitDrainTimeoutSeconds", EXIT_DRAIN_TIMEOUT_SECONDS),
        ("oscHintBytes", MAX_OSC_BYTES as u64),
        ("oscExplicitAliases", MAX_CWD_ALIASES as u64),
        ("recursiveDepth", MAX_DIRECTORY_DEPTH as u64),
        ("directoryPageEntries", DIRECTORY_PAGE_ENTRIES as u64),
        ("requestTimeoutSeconds", REQUEST_TIMEOUT_SECONDS),
        ("cancellationTargetSeconds", CANCEL_TIMEOUT_SECONDS),
        ("progressIntervalMs", PROGRESS_INTERVAL_MS),
    ] {
        assert_eq!(limits[key].as_u64(), Some(actual), "{key}");
    }
    let required_payload = limits["globalWorkers"].as_u64().unwrap()
        * limits["requestsPerFile"].as_u64().unwrap()
        * limits["requestPayloadBytes"].as_u64().unwrap()
        * 2;
    assert!(MAX_BUFFER_BYTES as u64 >= required_payload);
    assert_eq!(policy["mixedTerminalLoadLimits"]["m1LimitsUnchanged"], true);
    for network in ["loopback", "controlled_high_rtt"] {
        let absolute = &policy["absoluteLimits"]["byNetwork"][network];
        for key in [
            "minimumUploadBytesPerSecond",
            "minimumDownloadBytesPerSecond",
        ] {
            assert!(absolute[key].as_f64().unwrap() > 0.0);
        }
        assert!(
            policy["mixedTerminalLoadLimits"]["byNetwork"][network]["maximumInputP95Ms"]
                .as_f64()
                .unwrap()
                > 0.0
        );
    }
}

fn metadata() -> Metadata {
    Metadata {
        kind: FileKind::File,
        size: Some(ExactU64(9_007_199_254_740_993)),
        modified_seconds: Some(ExactU64(1_792_192_000)),
        permissions: Some(0o100_644),
    }
}

fn binding() -> Binding {
    Binding {
        window: "main".into(),
        terminal: Uuid::new_v4(),
        connection: Uuid::new_v4(),
        generation: 1,
        host: Uuid::new_v4(),
    }
}

#[test]
fn decimal_wire_encoding_preserves_u64_and_rejects_ambiguous_inputs() {
    for value in [0, 9_007_199_254_740_993, u64::MAX] {
        let json = serde_json::to_string(&ExactU64(value)).unwrap();
        assert_eq!(json, format!("\"{value}\""));
        assert_eq!(serde_json::from_str::<ExactU64>(&json).unwrap().0, value);
    }
    for json in [
        "0",
        "9007199254740993",
        "null",
        "\"\"",
        "\"01\"",
        "\"+1\"",
        "\"-1\"",
        "\"1.0\"",
        "\"1e3\"",
        "\" 1\"",
        "\"18446744073709551616\"",
    ] {
        assert!(serde_json::from_str::<ExactU64>(json).is_err(), "{json}");
    }
}

#[test]
fn paths_use_remote_semantics_and_never_decode_manual_input() {
    for path in [
        "/",
        "/etc/../var",
        "/link/../data",
        "/中文/a b",
        "/literal%2f%00",
        "/literal\\name",
        "//namespace",
    ] {
        assert_eq!(RemotePath::parse(path).unwrap().as_str(), path);
    }
    for path in [
        "",
        "etc",
        "C:\\etc",
        "file://host/etc",
        "/a\0b",
        "/a\nb",
        "/a\u{7f}b",
    ] {
        assert_eq!(RemotePath::parse(path), Err(SftpError::PathInvalid));
    }
    assert!(RemotePath::parse(&format!("/{}", "a".repeat(MAX_REMOTE_PATH_BYTES))).is_err());
    assert!(serde_json::from_str::<RemotePath>("\"relative\"").is_err());
}

#[test]
fn entry_names_cannot_escape_a_directory_or_reuse_lossy_bytes() {
    for name in [
        b"".as_slice(),
        b".",
        b"..",
        b"/etc",
        b"a/b",
        b"a\0b",
        b"a\nb",
    ] {
        assert!(RemoteName::from_bytes(name).is_err());
    }
    assert_eq!(
        RemoteName::from_bytes(&[0xff]),
        Err(SftpError::UnsupportedFileType)
    );
    assert_eq!(
        RemoteName::from_library_string("bad\u{fffd}"),
        Err(SftpError::UnsupportedFileType)
    );
    let name = RemoteName::from_bytes("中文 %2f\\name".as_bytes()).unwrap();
    assert_eq!(
        RemotePath::parse("/")
            .unwrap()
            .join(&name)
            .unwrap()
            .as_str(),
        "/中文 %2f\\name"
    );
    assert!(RemoteName::from_bytes(&vec![b'a'; MAX_REMOTE_NAME_BYTES + 1]).is_err());
}

#[test]
fn file_types_use_the_exact_posix_type_mask() {
    for (mode, kind) in [
        (0o100_644, FileKind::File),
        (0o040_755, FileKind::Directory),
        (0o120_777, FileKind::Symlink),
        (0o060_600, FileKind::Other),
        (0o020_600, FileKind::Other),
        (0o010_600, FileKind::Other),
        (0o140_600, FileKind::Other),
        (0o644, FileKind::Unknown),
    ] {
        assert_eq!(FileKind::from_permissions(Some(mode)), kind);
    }
    assert_eq!(FileKind::from_permissions(None), FileKind::Unknown);
}

#[test]
fn target_confirmation_requires_complete_unchanged_attributes() {
    let baseline = metadata();
    assert!(baseline.matches_baseline(&baseline));
    for changed in [
        Metadata {
            size: None,
            ..baseline.clone()
        },
        Metadata {
            modified_seconds: None,
            ..baseline.clone()
        },
        Metadata {
            kind: FileKind::Symlink,
            ..baseline.clone()
        },
        Metadata {
            size: Some(ExactU64(1)),
            ..baseline.clone()
        },
        Metadata {
            modified_seconds: Some(ExactU64(1)),
            ..baseline.clone()
        },
    ] {
        assert!(!changed.matches_baseline(&baseline));
        if changed.size.is_none() || changed.modified_seconds.is_none() {
            assert!(!changed.matches_baseline(&changed));
        }
    }
}

#[test]
fn capabilities_require_the_whole_rust_binding_and_expire() {
    let mut registry = ScopeRegistry::default();
    let now = Instant::now();
    let owner = binding();
    let id = registry.attach(owner.clone(), now).unwrap();
    let root = RemotePath::parse("/").unwrap();
    for invalid in [
        Binding {
            window: "other".into(),
            ..owner.clone()
        },
        Binding {
            terminal: Uuid::new_v4(),
            ..owner.clone()
        },
        Binding {
            connection: Uuid::new_v4(),
            ..owner.clone()
        },
        Binding {
            generation: 2,
            ..owner.clone()
        },
        Binding {
            host: Uuid::new_v4(),
            ..owner.clone()
        },
    ] {
        assert_eq!(
            registry.begin_directory(id, &invalid, root.clone(), now),
            Err(SftpError::ScopeExpired)
        );
        assert_eq!(
            registry.detach(id, &invalid, now),
            Err(SftpError::ScopeExpired)
        );
    }
    registry
        .begin_directory(id, &owner, root.clone(), now)
        .unwrap();
    assert_eq!(
        registry.begin_directory(id, &owner, root, now + Duration::from_secs(1800)),
        Err(SftpError::ScopeExpired)
    );
}

#[test]
fn late_pages_and_old_entry_ids_cannot_move_or_authorize_another_view() {
    let mut registry = ScopeRegistry::default();
    let now = Instant::now();
    let owner = binding();
    let id = registry.attach(owner.clone(), now).unwrap();
    let revision = registry
        .begin_directory(id, &owner, RemotePath::parse("/first").unwrap(), now)
        .unwrap();
    let other = registry.attach(owner.clone(), now).unwrap();
    registry
        .begin_directory(other, &owner, RemotePath::parse("/second").unwrap(), now)
        .unwrap();
    let page = registry
        .accept_page(
            id,
            &owner,
            PageInput {
                omitted_entries: 0,
                request_revision: revision,
                page_sequence: 0,
                entries: vec![(RemoteName::from_bytes(b"file").unwrap(), metadata())],
                has_more: false,
            },
            now,
        )
        .unwrap();
    let entry = page.entries[0].id;
    assert_eq!(
        registry
            .resolve_entry(id, &owner, entry, now)
            .unwrap()
            .0
            .as_str(),
        "/first/file"
    );
    assert_eq!(
        registry.resolve_entry(other, &owner, entry, now),
        Err(SftpError::ScopeExpired)
    );
    registry
        .begin_directory(id, &owner, RemotePath::parse("/new").unwrap(), now)
        .unwrap();
    assert_eq!(
        registry.accept_page(
            id,
            &owner,
            PageInput {
                omitted_entries: 0,
                request_revision: revision,
                page_sequence: 0,
                entries: vec![],
                has_more: false
            },
            now
        ),
        Err(SftpError::ScopeExpired)
    );
    assert_eq!(
        registry.resolve_entry(id, &owner, entry, now),
        Err(SftpError::ScopeExpired)
    );
}

#[test]
fn revocation_is_specific_to_terminal_and_connection_generation() {
    let mut registry = ScopeRegistry::default();
    let now = Instant::now();
    let owner = binding();
    let other = Binding {
        terminal: Uuid::new_v4(),
        ..owner.clone()
    };
    let first = registry.attach(owner.clone(), now).unwrap();
    let second = registry.attach(other.clone(), now).unwrap();
    registry.revoke_terminal(owner.terminal);
    let root = RemotePath::parse("/").unwrap();
    assert!(registry
        .begin_directory(first, &owner, root.clone(), now)
        .is_err());
    registry.revoke_connection(owner.connection, 2);
    assert!(registry
        .begin_directory(second, &other, root.clone(), now)
        .is_ok());
    registry.revoke_connection(owner.connection, 1);
    assert!(registry.begin_directory(second, &other, root, now).is_err());
}

#[test]
fn scope_and_page_budgets_are_enforced_before_registering_entries() {
    let mut registry = ScopeRegistry::default();
    let now = Instant::now();
    let owner = binding();
    for _ in 0..MAX_BROWSE_SCOPES {
        registry.attach(owner.clone(), now).unwrap();
    }
    assert_eq!(
        registry.attach(owner.clone(), now),
        Err(SftpError::ResourceLimit)
    );
    let now = now + Duration::from_secs(1801);
    let id = registry.attach(owner.clone(), now).unwrap();
    let revision = registry
        .begin_directory(id, &owner, RemotePath::parse("/").unwrap(), now)
        .unwrap();
    let entries = (0..=DIRECTORY_PAGE_ENTRIES)
        .map(|n| {
            (
                RemoteName::from_bytes(n.to_string().as_bytes()).unwrap(),
                metadata(),
            )
        })
        .collect();
    assert_eq!(
        registry.accept_page(
            id,
            &owner,
            PageInput {
                omitted_entries: 0,
                request_revision: revision,
                page_sequence: 0,
                entries,
                has_more: true
            },
            now
        ),
        Err(SftpError::ResourceLimit)
    );
    assert!(registry
        .accept_page(
            id,
            &owner,
            PageInput {
                omitted_entries: 0,
                request_revision: revision,
                page_sequence: 0,
                entries: vec![],
                has_more: false
            },
            now
        )
        .is_ok());
}

#[test]
fn directory_pages_are_sequential_and_terminal_pages_cannot_be_replayed() {
    let mut registry = ScopeRegistry::default();
    let now = Instant::now();
    let owner = binding();
    let id = registry.attach(owner.clone(), now).unwrap();
    let revision = registry
        .begin_directory(id, &owner, RemotePath::parse("/").unwrap(), now)
        .unwrap();
    for sequence in [1, 2] {
        assert_eq!(
            registry.accept_page(
                id,
                &owner,
                PageInput {
                    omitted_entries: 0,
                    request_revision: revision,
                    page_sequence: sequence,
                    entries: vec![],
                    has_more: true
                },
                now
            ),
            Err(SftpError::ScopeExpired)
        );
    }
    registry
        .accept_page(
            id,
            &owner,
            PageInput {
                omitted_entries: 0,
                request_revision: revision,
                page_sequence: 0,
                entries: vec![],
                has_more: false,
            },
            now,
        )
        .unwrap();
    for sequence in [0, 1] {
        assert_eq!(
            registry.accept_page(
                id,
                &owner,
                PageInput {
                    omitted_entries: 0,
                    request_revision: revision,
                    page_sequence: sequence,
                    entries: vec![],
                    has_more: false
                },
                now
            ),
            Err(SftpError::ScopeExpired)
        );
    }
}

#[test]
fn duplicate_names_are_rejected_atomically_and_large_directories_release_pages() {
    let mut registry = ScopeRegistry::default();
    let now = Instant::now();
    let owner = binding();
    let id = registry.attach(owner.clone(), now).unwrap();
    let revision = registry
        .begin_directory(id, &owner, RemotePath::parse("/").unwrap(), now)
        .unwrap();
    let duplicate = (RemoteName::from_bytes(b"same").unwrap(), metadata());
    assert_eq!(
        registry.accept_page(
            id,
            &owner,
            PageInput {
                omitted_entries: 0,
                request_revision: revision,
                page_sequence: 0,
                entries: vec![duplicate.clone(), duplicate],
                has_more: true
            },
            now
        ),
        Err(SftpError::TargetConflict)
    );
    for sequence in 0..32 {
        let entries = (0..DIRECTORY_PAGE_ENTRIES)
            .map(|n| {
                (
                    RemoteName::from_bytes(format!("{sequence}-{n}").as_bytes()).unwrap(),
                    metadata(),
                )
            })
            .collect();
        let page = registry
            .accept_page(
                id,
                &owner,
                PageInput {
                    omitted_entries: 0,
                    request_revision: revision,
                    page_sequence: sequence,
                    entries,
                    has_more: sequence != 31,
                },
                now,
            )
            .unwrap();
        let entry = page.entries[0].id;
        let ids: Vec<_> = page.entries.iter().map(|entry| entry.id).collect();
        registry.release_entries(id, &owner, &ids, now).unwrap();
        assert_eq!(
            registry.resolve_entry(id, &owner, entry, now),
            Err(SftpError::ScopeExpired)
        );
    }
}

#[test]
fn rust_types_consume_the_shared_typescript_contract_fixture() {
    let fixture = include_str!("../../../../docs/agent/fixtures/M2-sftp.json");
    let page: DirectoryPage = serde_json::from_str(fixture).unwrap();
    assert_eq!(page.entries[0].metadata.size, Some(ExactU64(u64::MAX)));
    assert_eq!(page.request_revision, ExactU64(9_007_199_254_740_993));
    assert_eq!(
        serde_json::to_value(page).unwrap(),
        serde_json::from_str::<serde_json::Value>(fixture).unwrap()
    );
}
