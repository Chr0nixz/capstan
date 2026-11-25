use super::*;
use std::collections::BTreeMap;

#[derive(Default)]
struct MemoryFiles(BTreeMap<PathBuf, String>);
impl MemoryFiles {
    fn with(mut self, path: &str, text: &str) -> Self {
        self.0.insert(PathBuf::from(path), text.to_owned());
        self
    }
}
impl ConfigFiles for MemoryFiles {
    fn read(&self, path: &Path) -> Option<String> {
        self.0.get(path).cloned()
    }
    fn matching(&self, directory: &Path, pattern: &str) -> Vec<PathBuf> {
        self.0
            .keys()
            .filter(|path| {
                path.parent() == Some(directory)
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| glob(pattern, name, false))
            })
            .cloned()
            .collect()
    }
    fn is_file(&self, path: &Path) -> bool {
        self.0.contains_key(path)
    }
}

fn parse_with(files: &MemoryFiles) -> Parsed {
    parse(&Environment {
        home: Path::new("/home/me"),
        local_user: Some("me"),
        files,
    })
    .unwrap()
}

#[test]
fn first_obtained_values_win_across_matching_blocks() {
    let files = MemoryFiles::default()
        .with(
            "/home/me/.ssh/config",
            "# global defaults\n\
             ServerAliveInterval 30\n\
             Host web-01 web-02\n\
             \x20 HostName %h.example.test\n\
             \x20 User deploy\n\
             Host web-02\n\
             \x20 Port 2222\n\
             \x20 User ignored\n\
             Host = db \"db primary\"\n\
             \x20 HostName 10.0.0.5 # trailing comment\n\
             \x20 IdentityFile ~/.ssh/db_key\n\
             Host *\n\
             \x20 User fallback\n\
             \x20 Port 2200\n",
        )
        .with("/home/me/.ssh/db_key", "key");
    let parsed = parse_with(&files);
    let summary: Vec<_> = parsed
        .candidates
        .iter()
        .map(|candidate| {
            (
                candidate.alias.as_str(),
                candidate.address.as_str(),
                candidate.port,
                candidate.username.as_str(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            ("web-01", "web-01.example.test", 2200, "deploy"),
            ("web-02", "web-02.example.test", 2222, "deploy"),
            ("db", "10.0.0.5", 2200, "fallback"),
            ("db primary", "10.0.0.5", 2200, "fallback"),
        ]
    );
    assert_eq!(
        parsed.candidates[2].identity_file.as_deref(),
        Some(Path::new("/home/me/.ssh/db_key"))
    );
    assert!(parsed.skipped.is_empty());
}

#[test]
fn wildcards_negations_and_match_blocks_are_not_hosts() {
    let files = MemoryFiles::default().with(
        "/home/me/.ssh/config",
        "Host *.internal !bastion.internal gateway\n\
         \x20 User ops\n\
         Match host gateway\n\
         \x20 User matched\n\
         \x20 Port 1\n\
         Host bastion.internal\n\
         \x20 User admin\n",
    );
    let parsed = parse_with(&files);
    let aliases: Vec<_> = parsed
        .candidates
        .iter()
        .map(|candidate| {
            (
                candidate.alias.as_str(),
                candidate.username.as_str(),
                candidate.port,
            )
        })
        .collect();
    assert_eq!(
        aliases,
        vec![("gateway", "ops", 22), ("bastion.internal", "admin", 22)]
    );
    assert_eq!(parsed.skipped, vec![Skip::MatchBlock]);
    assert!(matches_host(&["*.internal".into()], "DB.Internal"));
    assert!(!matches_host(
        &["*.internal".into(), "!db.internal".into()],
        "db.internal"
    ));
}

#[test]
fn identities_fall_back_to_default_keys_or_asking() {
    let files = MemoryFiles::default()
        .with(
            "/home/me/.ssh/config",
            "Host plain\n\
             Host missing\n\
             \x20 IdentityFile ~/.ssh/gone\n\
             Host token\n\
             \x20 IdentityFile ~/.ssh/%l_key\n\
             Host remote-user\n\
             \x20 User git\n\
             \x20 IdentityFile %d/.ssh/%r_%h\n\
             Host none\n\
             \x20 IdentityFile none\n",
        )
        .with("/home/me/.ssh/id_ecdsa", "key")
        .with("/home/me/.ssh/id_rsa", "key")
        .with("/home/me/.ssh/git_remote-user", "key");
    let parsed = parse_with(&files);
    let identities: Vec<_> = parsed
        .candidates
        .iter()
        .map(|candidate| {
            (
                candidate.alias.as_str(),
                candidate.identity_file.clone(),
                candidate.notes.clone(),
            )
        })
        .collect();
    assert_eq!(
        identities,
        vec![
            (
                "plain",
                Some(PathBuf::from("/home/me/.ssh/id_ecdsa")),
                vec![Note::LocalUser, Note::DefaultIdentity]
            ),
            (
                "missing",
                None,
                vec![Note::LocalUser, Note::IdentityUnavailable]
            ),
            (
                "token",
                None,
                vec![Note::LocalUser, Note::IdentityUnavailable]
            ),
            (
                "remote-user",
                Some(PathBuf::from("/home/me/.ssh/git_remote-user")),
                vec![]
            ),
            ("none", None, vec![Note::LocalUser]),
        ]
    );
}

#[test]
fn proxy_commands_are_noted_and_invalid_hosts_skipped() {
    let files = MemoryFiles::default().with(
        "/home/me/.ssh/config",
        "Host cmd\n\
         \x20 User me\n\
         \x20 ProxyCommand ssh -W %h:%p gw\n\
         Host direct\n\
         \x20 User me\n\
         \x20 ProxyCommand none\n\
         Host first\n\
         \x20 User me\n\
         \x20 ProxyCommand nc %h %p\n\
         \x20 ProxyJump ignored\n\
         Host multi\n\
         \x20 User me\n\
         \x20 ProxyJump a,b\n\
         Host bad-port\n\
         \x20 Port 70000\n\
         Host bad-address\n\
         \x20 HostName \"two words\"\n\
         Host bad-token\n\
         \x20 HostName %n.example\n",
    );
    let parsed = parse_with(&files);
    let summary: Vec<_> = parsed
        .candidates
        .iter()
        .map(|candidate| {
            (
                candidate.alias.as_str(),
                candidate.jump,
                candidate.notes.clone(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            ("cmd", None, vec![Note::ProxyUnsupported]),
            ("direct", None, vec![]),
            ("first", None, vec![Note::ProxyUnsupported]),
            ("multi", None, vec![Note::ProxyUnsupported]),
        ]
    );
    assert_eq!(parsed.skipped, vec![Skip::InvalidHost; 3]);
}

#[test]
fn proxy_jumps_link_to_jump_hosts_resolved_through_the_config() {
    let files = MemoryFiles::default().with(
        "/home/me/.ssh/config",
        "Host db\n HostName 10.0.0.5\n User app\n ProxyJump bastion\n\
         Host bastion\n HostName gw.example.test\n User ops\n Port 2200\n\
         Host cache\n HostName 10.0.0.6\n User app\n ProxyJump admin@bastion:2222\n\
         Host queue\n HostName 10.0.0.7\n User app\n ProxyJump ssh://admin@bastion:2222\n\
         Host web\n User app\n ProxyJump edge\n\
         Host inner\n User app\n ProxyJump db\n\
         Host loop-a\n User x\n ProxyJump loop-b\n\
         Host loop-b\n User x\n ProxyJump loop-a\n\
         Host self\n User x\n ProxyJump self\n\
         Host behind-cmd\n User x\n ProxyJump cmd-gw\n\
         Host cmd-gw\n User x\n ProxyCommand nc %h %p\n",
    );
    let parsed = parse_with(&files);
    let summary: Vec<_> = parsed
        .candidates
        .iter()
        .map(|candidate| {
            (
                candidate.alias.as_str(),
                candidate.address.as_str(),
                candidate.port,
                candidate.username.as_str(),
                candidate.jump,
                candidate.notes.clone(),
            )
        })
        .collect();
    let unsupported = vec![Note::ProxyUnsupported];
    assert_eq!(
        summary,
        vec![
            ("db", "10.0.0.5", 22, "app", Some(1), vec![]),
            ("bastion", "gw.example.test", 2200, "ops", None, vec![]),
            ("cache", "10.0.0.6", 22, "app", Some(11), vec![]),
            ("queue", "10.0.0.7", 22, "app", Some(11), vec![]),
            ("web", "web", 22, "app", Some(12), vec![]),
            ("inner", "inner", 22, "app", Some(0), vec![]),
            ("loop-a", "loop-a", 22, "x", None, unsupported.clone()),
            ("loop-b", "loop-b", 22, "x", Some(6), unsupported.clone()),
            ("self", "self", 22, "x", None, unsupported.clone()),
            (
                "behind-cmd",
                "behind-cmd",
                22,
                "x",
                Some(10),
                unsupported.clone()
            ),
            ("cmd-gw", "cmd-gw", 22, "x", None, unsupported),
            // Jump hosts without a `Host` entry of their own, with the
            // spelling's user and port over the config's.
            (
                "admin@bastion:2222",
                "gw.example.test",
                2222,
                "admin",
                None,
                vec![]
            ),
            ("edge", "edge", 22, "me", None, vec![Note::LocalUser]),
        ]
    );
}

#[test]
fn a_jump_hop_reuses_only_a_candidate_taking_the_same_route() {
    let files = MemoryFiles::default().with(
        "/home/me/.ssh/config",
        "Host a\n HostName 10.0.0.1\n User ops\n ProxyJump gw\n\
         Host gw\n HostName gw.example.test\n User ops\n\
         Host b\n HostName 10.0.0.9\n User app\n ProxyJump ops@10.0.0.1\n\
         Host c\n HostName 10.0.0.8\n User app\n ProxyJump ops@gw.example.test\n",
    );
    let parsed = parse_with(&files);
    let links: Vec<_> = parsed
        .candidates
        .iter()
        .map(|candidate| (candidate.alias.as_str(), candidate.jump))
        .collect();
    assert_eq!(
        links,
        vec![
            ("a", Some(1)),
            ("gw", None),
            // OpenSSH reaches 10.0.0.1 directly for b, so b does not go
            // through a and its jump host.
            ("b", Some(4)),
            // gw.example.test is reached directly either way, so c reuses gw.
            ("c", Some(1)),
            ("ops@10.0.0.1", None),
        ]
    );
}

#[test]
fn jump_chains_longer_than_the_limit_are_cut() {
    let config: String = (0..6)
        .map(|hop| format!("Host h{hop}\n User x\n ProxyJump h{}\n", hop + 1))
        .chain(["Host h6\n User x\n".to_owned()])
        .collect();
    let files = MemoryFiles::default().with("/home/me/.ssh/config", &config);
    let jumps: Vec<_> = parse_with(&files)
        .candidates
        .iter()
        .map(|candidate| (candidate.jump, candidate.notes.clone()))
        .collect();
    // h0 and h1 would pass through five or more hosts; h2 needs four.
    assert_eq!(jumps[0], (None, vec![Note::ProxyUnsupported]));
    assert_eq!(jumps[1], (None, vec![Note::ProxyUnsupported]));
    assert_eq!(jumps[2], (Some(3), vec![]));
    assert_eq!(jumps[6], (None, vec![]));
}

#[test]
fn jump_spellings_follow_openssh() {
    assert_eq!(
        jump_target("ops@gw:2200"),
        Some((Some("ops".into()), "gw".into(), Some(2200)))
    );
    assert_eq!(
        jump_target("ssh://[2001:db8::1]:2022/"),
        Some((None, "2001:db8::1".into(), Some(2022)))
    );
    assert_eq!(
        jump_target("2001:db8::1"),
        Some((None, "2001:db8::1".into(), None))
    );
    for invalid in ["@gw", "gw:0", "gw:port", "[2001:db8::1", "ops@"] {
        assert_eq!(jump_target(invalid), None, "{invalid}");
    }
}

#[test]
fn a_missing_local_user_skips_hosts_without_user() {
    let files = MemoryFiles::default().with("/home/me/.ssh/config", "Host a\nHost b\n User x\n");
    let parsed = parse(&Environment {
        home: Path::new("/home/me"),
        local_user: None,
        files: &files,
    })
    .unwrap();
    assert_eq!(parsed.candidates.len(), 1);
    assert_eq!(parsed.candidates[0].alias, "b");
    assert_eq!(parsed.skipped, vec![Skip::InvalidHost]);
}

#[test]
fn includes_are_globbed_bounded_and_inherit_the_current_block() {
    let files = MemoryFiles::default()
        .with(
            "/home/me/.ssh/config",
            "Include config.d/*.conf ~/missing.conf\n\
             Host after\n\
             \x20 Include extra\n",
        )
        .with("/home/me/.ssh/config.d/b.conf", "Host b\n User bee\n")
        .with("/home/me/.ssh/config.d/a.conf", "Host a\n User ay\n")
        .with("/home/me/.ssh/config.d/skip.txt", "Host txt\n")
        .with("/home/me/.ssh/extra", "User inherited\n")
        .with("/home/me/.ssh/loop", "Include loop\nHost looped\n User l\n");
    let parsed = parse_with(&files);
    let aliases: Vec<_> = parsed
        .candidates
        .iter()
        .map(|candidate| (candidate.alias.as_str(), candidate.username.as_str()))
        .collect();
    assert_eq!(
        aliases,
        vec![("a", "ay"), ("b", "bee"), ("after", "inherited")]
    );
    assert_eq!(parsed.skipped, vec![Skip::Include]);

    let looping = MemoryFiles::default()
        .with("/home/me/.ssh/config", "Include loop\n")
        .with("/home/me/.ssh/loop", "Include loop\nHost looped\n User l\n");
    let parsed = parse_with(&looping);
    assert_eq!(parsed.candidates.len(), 1);
    assert!(parsed.skipped.contains(&Skip::Include));
}

#[test]
fn lines_after_an_include_return_to_the_including_block() {
    let files = MemoryFiles::default()
        .with(
            "/home/me/.ssh/config",
            "Include common\n\
             Port 2200\n\
             Host app\n\
             \x20 Include work\n\
             \x20 User alice\n\
             Host plain\n",
        )
        .with("/home/me/.ssh/common", "Host shared\n User sam\n")
        .with("/home/me/.ssh/work", "Host build\n User builder\n");
    let parsed = parse_with(&files);
    let hosts: Vec<_> = parsed
        .candidates
        .iter()
        .map(|candidate| {
            (
                candidate.alias.as_str(),
                candidate.username.as_str(),
                candidate.port,
            )
        })
        .collect();
    // `User alice` belongs to `app`, not to `build`, and the global `Port`
    // after the first Include applies to every host, not just `shared`.
    assert_eq!(
        hosts,
        vec![
            ("shared", "sam", 2200),
            ("app", "alice", 2200),
            ("build", "builder", 2200),
            ("plain", "me", 2200),
        ]
    );
}

#[test]
fn a_byte_order_mark_does_not_hide_the_first_line() {
    let files = MemoryFiles::default()
        .with(
            "/home/me/.ssh/config",
            "\u{feff}Host first\n User one\nInclude more\n",
        )
        .with("/home/me/.ssh/more", "\u{feff}Host second\n User two\n");
    let parsed = parse_with(&files);
    let hosts: Vec<_> = parsed
        .candidates
        .iter()
        .map(|candidate| (candidate.alias.as_str(), candidate.username.as_str()))
        .collect();
    assert_eq!(hosts, vec![("first", "one"), ("second", "two")]);
}

#[test]
fn every_identity_file_is_kept_in_order_like_openssh() {
    let files = MemoryFiles::default()
        .with(
            "/home/me/.ssh/config",
            "Host *\n\
             \x20 IdentityFile ~/.ssh/hardware_key\n\
             Host several\n\
             \x20 User me\n\
             \x20 IdentityFile ~/.ssh/work\n\
             \x20 IdentityFile ~/.ssh/home\n\
             \x20 IdentityFile ~/.ssh/work\n\
             Host one\n\
             \x20 User me\n\
             \x20 IdentityFile ~/.ssh/home\n\
             Host nothing\n\
             \x20 User me\n",
        )
        .with("/home/me/.ssh/work", "key")
        .with("/home/me/.ssh/home", "key");
    let parsed = parse_with(&files);
    let identities: Vec<_> = parsed
        .candidates
        .iter()
        .map(|candidate| {
            (
                candidate.alias.as_str(),
                candidate.identity_file.clone(),
                candidate.other_identities.clone(),
                candidate.notes.clone(),
            )
        })
        .collect();
    // The missing key from `Host *` no longer hides the ones that exist.
    assert_eq!(
        identities,
        vec![
            (
                "several",
                Some(PathBuf::from("/home/me/.ssh/work")),
                vec![PathBuf::from("/home/me/.ssh/home")],
                vec![Note::SeveralIdentities]
            ),
            (
                "one",
                Some(PathBuf::from("/home/me/.ssh/home")),
                vec![],
                vec![]
            ),
            ("nothing", None, vec![], vec![Note::IdentityUnavailable]),
        ]
    );
}

#[test]
fn lines_split_like_openssh() {
    assert_eq!(
        split_line("  HostName=example.test"),
        Some(("hostname".into(), vec!["example.test".into()]))
    );
    assert_eq!(
        split_line("IdentityFile \"C:\\Users\\me\\key file\" #c"),
        Some((
            "identityfile".into(),
            vec!["C:\\Users\\me\\key file".into()]
        ))
    );
    assert_eq!(
        split_line("Host a#b"),
        Some(("host".into(), vec!["a#b".into()]))
    );
    assert_eq!(split_line("Host \"unterminated"), None);
    assert_eq!(split_line("   # comment"), None);
    assert!(glob("*.conf", "a.conf", false));
    assert!(!glob("*.conf", "a.conf.bak", false));
    assert!(glob("web-?", "web-1", true));
}

#[test]
fn a_missing_config_yields_nothing() {
    let files = MemoryFiles::default();
    assert_eq!(
        parse(&Environment {
            home: Path::new("/home/me"),
            local_user: Some("me"),
            files: &files,
        }),
        None
    );
    let empty = MemoryFiles::default().with("/home/me/.ssh/config", "# nothing\n");
    assert_eq!(parse_with(&empty), Parsed::default());
}

#[test]
fn home_paths_use_the_platform_separator() {
    let home = Path::new("/home/me");
    // A key chosen in the file picker has exactly this form, separators included.
    let key = home.join(".ssh").join("key");
    assert_eq!(expand_home("~/.ssh/key", home).as_os_str(), key.as_os_str());
    #[cfg(windows)]
    assert_eq!(
        expand_home("~\\.ssh\\key", home).as_os_str(),
        key.as_os_str()
    );
    assert_eq!(expand_home("~", home), home);
}
