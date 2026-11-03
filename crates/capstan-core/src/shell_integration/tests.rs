use super::*;

#[test]
fn uninstall_preserves_the_line_boundary_after_an_unterminated_original() {
    for shell in [Shell::Bash, Shell::Zsh] {
        let original = b"export KEEP=yes";
        let installed = plan(original, shell, Action::Install, "host").unwrap();
        for suffix in [b"# appended\n".as_slice(), b"\n# appended\n"] {
            let edited = [installed.replacement.as_slice(), suffix].concat();
            let removed = plan(&edited, shell, Action::Uninstall, "host").unwrap();
            assert_eq!(removed.replacement, b"export KEEP=yes\n# appended\n");
        }
    }
}

#[test]
fn install_twice_and_uninstall_preserve_original_bytes() {
    for shell in [Shell::Bash, Shell::Zsh] {
        for input in [
            b"".as_slice(),
            b"# existing\n",
            b"export KEEP='yes'",
            b"# windows\r\n",
            "\u{feff}# 中文\n".as_bytes(),
        ] {
            let first = plan(input, shell, Action::Install, "EXAMPLE.com.").unwrap();
            assert!(first.changed && first.added_lines > 20);
            assert!(first.replacement.starts_with(input));
            let repeated = plan(&first.replacement, shell, Action::Install, "example.com").unwrap();
            assert!(!repeated.changed);
            assert_eq!(repeated.replacement, first.replacement);
            let removed =
                plan(&first.replacement, shell, Action::Uninstall, "example.com").unwrap();
            assert_eq!(removed.replacement, input);
            assert!(removed.changed && removed.removed_lines == first.added_lines);
            assert!(
                !plan(input, shell, Action::Uninstall, "example.com")
                    .unwrap()
                    .changed
            );
        }
    }
}

#[test]
fn unrelated_edits_survive_and_ambiguous_separator_edits_are_refused() {
    let first = plan(b"# before\n", Shell::Bash, Action::Install, "host").unwrap();
    let edited = format!(
        "# inserted\n{}# after\n",
        std::str::from_utf8(&first.replacement).unwrap()
    );
    let removed = plan(edited.as_bytes(), Shell::Bash, Action::Uninstall, "host").unwrap();
    assert_eq!(removed.replacement, b"# inserted\n# before\n# after\n");
    let first = plan(b"no-newline", Shell::Zsh, Action::Install, "host").unwrap();
    let edited = [b"# new\n".as_slice(), &first.replacement].concat();
    assert!(matches!(
        plan(&edited, Shell::Zsh, Action::Uninstall, "host"),
        Err(IntegrationError::Modified)
    ));
}

#[test]
fn edited_unknown_duplicate_and_colliding_blocks_fail_closed() {
    let first = plan(b"# user\n", Shell::Bash, Action::Install, "host").unwrap();
    let text = std::str::from_utf8(&first.replacement).unwrap();
    for bad in [
        text.replace("8192", "8191"),
        text.replace("capstan-osc7:v1", "capstan-osc7:v2"),
        text.replace(END, ""),
        format!("{text}{text}"),
        format!("{text}__capstan_osc7_v1_emit\n"),
    ] {
        assert!(matches!(
            plan(bad.as_bytes(), Shell::Bash, Action::Uninstall, "host"),
            Err(IntegrationError::Modified)
        ));
    }
    assert!(matches!(
        plan(
            b"__capstan_osc7_v1_emit() { :; }\n",
            Shell::Bash,
            Action::Install,
            "host"
        ),
        Err(IntegrationError::Conflict)
    ));
    assert!(matches!(
        plan(&first.replacement, Shell::Zsh, Action::Install, "host"),
        Err(IntegrationError::Conflict)
    ));
    assert!(matches!(
        plan(
            &first.replacement,
            Shell::Bash,
            Action::Install,
            "another-host"
        ),
        Err(IntegrationError::Conflict)
    ));
}

#[test]
fn malformed_encoding_controls_size_and_authority_are_rejected() {
    for invalid in [b"\xff".as_slice(), b"nul\0", b"escape\x1b", b"delete\x7f"] {
        assert!(matches!(
            plan(invalid, Shell::Bash, Action::Install, "host"),
            Err(IntegrationError::Invalid)
        ));
    }
    assert!(matches!(
        plan(
            &vec![b'x'; MAX_CONFIG_BYTES + 1],
            Shell::Bash,
            Action::Install,
            "host"
        ),
        Err(IntegrationError::ResourceLimit)
    ));
    assert!(matches!(
        plan(
            &vec![b'x'; MAX_CONFIG_BYTES],
            Shell::Bash,
            Action::Install,
            "host"
        ),
        Err(IntegrationError::ResourceLimit)
    ));
    for host in [
        "host'$(touch /tmp/no)",
        "user@host",
        "host:22",
        "local host",
        "",
        "☃",
    ] {
        assert!(plan(b"", Shell::Zsh, Action::Install, host).is_err());
    }
    let ipv6 = plan(b"", Shell::Bash, Action::Install, "2001:db8::1").unwrap();
    assert!(std::str::from_utf8(&ipv6.replacement)
        .unwrap()
        .contains("file://[2001:db8::1]"));
}

#[test]
fn export_configured_shell_probe_fixtures_or_explicit_skip() {
    let Ok(directory) = std::env::var("CAPSTAN_TEST_SHELL_INTEGRATION") else {
        eprintln!("SKIP Bash/Zsh fixture export: CAPSTAN_TEST_SHELL_INTEGRATION is not set; this case alone does not execute real shells");
        return;
    };
    // The external harness owns this directory and runs the exact generated
    // bytes in real shells. It receives no application IPC or credentials.
    let directory = std::path::PathBuf::from(directory);
    assert!(directory.is_dir());
    for shell in [Shell::Bash, Shell::Zsh] {
        let generated = plan(
            b"# preserved user configuration\n",
            shell,
            Action::Install,
            "127.0.0.1",
        )
        .unwrap();
        let path = directory.join(format!("{}.sh", shell.name()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        use std::io::Write;
        file.write_all(&generated.replacement).unwrap();
        file.sync_all().unwrap();
    }
}

#[test]
fn preview_tokens_are_bound_single_use_and_rechecked_against_current_bytes() {
    let registry = PreviewRegistry::default();
    let binding = PreviewBinding {
        window: "main".into(),
        terminal: "terminal".into(),
        generation: 4,
        path: "/home/.bashrc".into(),
    };
    let summary = registry
        .create(
            binding.clone(),
            b"# user\n",
            Shell::Bash,
            Action::Install,
            "host",
        )
        .unwrap();
    assert_eq!(
        summary.source_hash,
        format!("{:X}", Sha256::digest(b"# user\n"))
    );
    assert!(matches!(
        registry.consume(&binding, summary.token, b"changed"),
        Err(IntegrationError::Modified)
    ));
    let summary = registry
        .create(
            binding.clone(),
            b"# user\n",
            Shell::Bash,
            Action::Install,
            "host",
        )
        .unwrap();
    let mut other_path = binding.clone();
    other_path.path = "/home/.zshrc".into();
    assert!(matches!(
        registry.consume(&other_path, summary.token, b"# user\n"),
        Err(IntegrationError::Modified)
    ));
    let summary = registry
        .create(
            binding.clone(),
            b"# user\n",
            Shell::Bash,
            Action::Install,
            "host",
        )
        .unwrap();
    let (_, _, _, plan) = registry
        .consume(&binding, summary.token, b"# user\n")
        .unwrap();
    assert!(plan.changed);
    assert!(matches!(
        registry.consume(&binding, summary.token, b"# user\n"),
        Err(IntegrationError::Invalid)
    ));
}

#[test]
fn remote_snapshot_matching_requires_size_mtime_kind_and_permissions() {
    let expected = crate::sftp::Metadata {
        kind: crate::sftp::FileKind::File,
        size: Some(crate::sftp::ExactU64(12)),
        modified_seconds: Some(crate::sftp::ExactU64(7)),
        permissions: Some(0o100_600),
    };
    let attributes = capstan_ssh::sftp::Attributes {
        size: Some(12),
        modified_seconds: Some(7),
        permissions: Some(0o100_600),
    };
    assert!(attributes_match(attributes.clone(), &expected));
    for changed in [
        capstan_ssh::sftp::Attributes {
            size: Some(13),
            ..attributes.clone()
        },
        capstan_ssh::sftp::Attributes {
            modified_seconds: Some(8),
            ..attributes.clone()
        },
        capstan_ssh::sftp::Attributes {
            permissions: Some(0o100_644),
            ..attributes.clone()
        },
        capstan_ssh::sftp::Attributes {
            permissions: Some(0o040_755),
            ..attributes.clone()
        },
    ] {
        assert!(!attributes_match(changed, &expected));
    }
}
