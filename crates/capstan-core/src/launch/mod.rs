//! Bounded process-argument and SSH link parsing for desktop launch requests.

use serde::Serialize;
use std::path::PathBuf;
use uuid::Uuid;

pub mod instance;
pub mod recent;

pub const MAX_ARGUMENTS: usize = 8;
pub const MAX_ARGUMENT_LENGTH: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchRequest {
    Focus,
    Local {
        shell_id: Option<String>,
        working_directory: Option<PathBuf>,
    },
    Host(Uuid),
    Ssh(SshTarget),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget {
    pub username: Option<String>,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchRequestKind {
    Focus,
    Local,
    Host,
    Ssh,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchRequestView {
    pub request_id: Uuid,
    pub kind: LaunchRequestKind,
    pub display: String,
}

impl LaunchRequest {
    #[must_use]
    pub fn view(&self, request_id: Uuid) -> LaunchRequestView {
        let (kind, display) = match self {
            Self::Focus => (LaunchRequestKind::Focus, "Capstan".to_owned()),
            Self::Local { shell_id, .. } => (
                LaunchRequestKind::Local,
                shell_id.clone().unwrap_or_default(),
            ),
            Self::Host(_) => (LaunchRequestKind::Host, "Saved SSH host".to_owned()),
            Self::Ssh(target) => {
                let host = if target.host.contains(':') {
                    format!("[{}]", target.host)
                } else {
                    target.host.clone()
                };
                // Encode "@" and "%" in the user so the host part stays unambiguous.
                let authority = match &target.username {
                    Some(username) => format!(
                        "{}@{host}",
                        username.replace('%', "%25").replace('@', "%40")
                    ),
                    None => host,
                };
                (
                    LaunchRequestKind::Ssh,
                    format!("{authority}:{}", target.port),
                )
            }
        };
        LaunchRequestView {
            request_id,
            kind,
            display,
        }
    }
}

/// Parse arguments excluding argv[0]. Any malformed or unsupported request becomes Focus.
#[must_use]
pub fn parse_launch_args(args: &[String]) -> LaunchRequest {
    parse_launch_args_with(args, |path| std::path::Path::new(path).is_dir())
}

/// The predicate keeps filesystem state outside the pure grammar and makes path policy testable.
pub fn parse_launch_args_with(
    args: &[String],
    directory_exists: impl Fn(&str) -> bool,
) -> LaunchRequest {
    if args.is_empty() {
        return LaunchRequest::Focus;
    }
    if args.len() > MAX_ARGUMENTS
        || args.iter().any(|argument| {
            argument.chars().count() > MAX_ARGUMENT_LENGTH || argument.contains('\0')
        })
    {
        return LaunchRequest::Focus;
    }
    if args.len() == 1 && args[0].starts_with("ssh://") {
        return parse_ssh_url(&args[0]).map_or(LaunchRequest::Focus, LaunchRequest::Ssh);
    }
    match args[0].as_str() {
        "--local" => parse_local_args(&args[1..], directory_exists).map_or(
            LaunchRequest::Focus,
            |(shell_id, working_directory)| LaunchRequest::Local {
                shell_id,
                working_directory,
            },
        ),
        "--host" if args.len() == 2 => Uuid::parse_str(&args[1])
            .ok()
            .filter(|id| !id.is_nil())
            .map_or(LaunchRequest::Focus, LaunchRequest::Host),
        _ => LaunchRequest::Focus,
    }
}

fn parse_local_args(
    args: &[String],
    directory_exists: impl Fn(&str) -> bool,
) -> Option<(Option<String>, Option<PathBuf>)> {
    let mut shell_id = None;
    let mut working_directory = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--shell" if shell_id.is_none() => {
                let value = args.get(index + 1)?;
                if !valid_shell_id(value) {
                    return None;
                }
                shell_id = Some(value.clone());
                index += 2;
            }
            "--cwd" if working_directory.is_none() => {
                let value = args.get(index + 1)?;
                if !valid_windows_directory(value) || !directory_exists(value) {
                    return None;
                }
                working_directory = Some(PathBuf::from(value));
                index += 2;
            }
            _ => return None,
        }
    }
    Some((shell_id, working_directory))
}

fn valid_shell_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.starts_with('-')
        && value
            .chars()
            .all(|character| !character.is_control() && character != '\\' && character != '/')
}

/// Quote one Windows argv token; never concatenate an unquoted profile ID.
#[must_use]
pub fn shell_task_arguments(id: &str) -> Option<String> {
    if !valid_shell_id(id) {
        return None;
    }
    // Profile IDs cannot contain backslashes, so only embedded quotes need escaping.
    Some(format!("--local --shell \"{}\"", id.replace('"', "\\\"")))
}

fn valid_windows_directory(value: &str) -> bool {
    if value.is_empty()
        || value.chars().count() > MAX_ARGUMENT_LENGTH
        || value.chars().any(char::is_control)
    {
        return false;
    }
    let normalized = value.replace('/', "\\");
    let value = normalized.as_str();
    if value.starts_with("\\\\.\\")
        || value.starts_with("\\\\?\\")
        || value.split('\\').any(|part| part == "." || part == "..")
    {
        return false;
    }
    let bytes = value.as_bytes();
    let drive_path =
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    let unc_path = value.starts_with("\\\\")
        && value
            .trim_start_matches('\\')
            .split('\\')
            .filter(|part| !part.is_empty())
            .count()
            >= 2;
    if !drive_path && !unc_path {
        return false;
    }
    if drive_path && value[3..].contains(':') || unc_path && value.contains(':') {
        return false;
    }
    true
}

/// Parse only the connection authority; path, query, fragment and fingerprint parameters are ignored.
pub fn parse_ssh_url(value: &str) -> Option<SshTarget> {
    if value.chars().count() > MAX_ARGUMENT_LENGTH || !value.starts_with("ssh://") {
        return None;
    }
    if percent_decode(value)?.chars().any(char::is_control) {
        return None;
    }
    let remainder = &value[6..];
    let authority_end = remainder
        .find(['/', '?', '#', ';'])
        .unwrap_or(remainder.len());
    let authority = &remainder[..authority_end];
    if authority.is_empty() {
        return None;
    }
    let (raw_user, raw_address) = match authority.rsplit_once('@') {
        Some((username, address)) if !username.contains('@') => (Some(username), address),
        Some(_) => return None,
        None => (None, authority),
    };
    let username = match raw_user {
        Some(username) => Some(percent_decode(username)?),
        None => None,
    };
    if username.as_ref().is_some_and(|user| {
        user.is_empty()
            || user.chars().count() > 256
            || user.chars().any(char::is_control)
            || user.contains(':')
    }) {
        return None;
    }

    let (raw_host, port) = if let Some(bracketed) = raw_address.strip_prefix('[') {
        let close = bracketed.find(']')?;
        let host = &bracketed[..close];
        let rest = &bracketed[close + 1..];
        let port = if rest.is_empty() {
            22
        } else {
            parse_port(rest.strip_prefix(':')?)?
        };
        if host.parse::<std::net::Ipv6Addr>().is_err() {
            return None;
        }
        (host, port)
    } else {
        if raw_address.matches(':').count() > 1 {
            return None;
        }
        match raw_address.rsplit_once(':') {
            Some((host, port)) => (host, parse_port(port)?),
            None => (raw_address, 22),
        }
    };
    if port == 0 {
        return None;
    }
    let host = percent_decode(raw_host)?;
    let is_ipv6 = host.parse::<std::net::Ipv6Addr>().is_ok();
    if host.is_empty()
        || host.chars().count() > 253
        || host.chars().any(char::is_control)
        || host.chars().any(|character| {
            character.is_whitespace()
                || matches!(
                    character,
                    '/' | '\\' | '?' | '#' | '@' | '[' | ']' | ';' | '%' | '"' | '<' | '>' | '|'
                )
                || character == ':' && !is_ipv6
        })
    {
        return None;
    }
    Some(SshTarget {
        username,
        host,
        port,
    })
}

/// A link selects saved credentials only on the same normalized address, port and user.
#[must_use]
pub fn ssh_target_matches(target: &SshTarget, address: &str, port: u16, username: &str) -> bool {
    target.port == port
        && target.username.as_deref() == Some(username)
        && capstan_vault::normalize_target(&target.host).is_ok_and(|host| {
            capstan_vault::normalize_target(address).is_ok_and(|saved| saved == host)
        })
}

fn parse_port(value: &str) -> Option<u16> {
    if value.is_empty() || !value.bytes().all(|value| value.is_ascii_digit()) {
        return None;
    }
    value.parse::<u16>().ok().filter(|port| *port != 0)
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_digit(*bytes.get(index + 1)?)?;
            let low = hex_digit(*bytes.get(index + 2)?)?;
            decoded.push(high * 16 + low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[cfg(windows)]
    #[test]
    fn explorer_commands_preserve_directory_argv_in_a_real_child() {
        use std::{os::windows::process::CommandExt, process::Command};

        const EXPECTED_PATH: &str = "CAPSTAN_TEST_EXPLORER_PATH";
        const EXISTING_PATH: &str = "CAPSTAN_TEST_EXPLORER_EXISTS";
        // Re-enter only this test so the real Rust startup code parses the raw command line.
        if let Ok(expected) = std::env::var(EXPECTED_PATH) {
            let received: Vec<String> = std::env::args()
                .skip_while(|value| value != "--")
                .skip(1)
                .collect();
            assert_eq!(
                received,
                args(&["--local", "--cwd", &format!("{expected}/")])
            );
            let request = LaunchRequest::Local {
                shell_id: None,
                working_directory: Some(PathBuf::from(&received[2])),
            };
            assert_eq!(parse_launch_args_with(&received, |_| true), request);
            assert_eq!(PathBuf::from(&received[2]), PathBuf::from(expected));
            if std::env::var_os(EXISTING_PATH).is_some() {
                assert_eq!(parse_launch_args(&received), request);
            }
            println!("explorer_argv_verified");
            return;
        }

        let directory = tempfile::tempdir().unwrap();
        let unicode_directory = directory.path().join("Capstan 目录 & spaces");
        std::fs::create_dir(&unicode_directory).unwrap();
        let root = format!("{}\\", std::env::var("SystemDrive").unwrap());
        let paths = [
            (root, true),
            (unicode_directory.to_str().unwrap().to_owned(), true),
            (format!("{}\\", unicode_directory.display()), true),
            (r"C:\folder with spaces".into(), false),
            (r"C:\folder with spaces\".into(), false),
            (r"\\server\share".into(), false),
            (r"\\server\shared 目录\".into(), false),
            (r"\\server\share\nested folder\".into(), false),
        ];
        let hooks = include_str!("../../../../src-tauri/windows/hooks.nsh");
        let templates: Vec<_> = hooks
            .lines()
            .filter_map(|line| line.split_once(" --local --cwd "))
            .map(|(_, suffix)| suffix.strip_suffix('\'').unwrap())
            .collect();
        assert_eq!(templates.len(), 2, "check both Explorer menu entries");
        for template in templates {
            for (path, exists) in &paths {
                let mut child = Command::new(std::env::current_exe().unwrap());
                child
                    .args([
                        "--exact",
                        "launch::tests::explorer_commands_preserve_directory_argv_in_a_real_child",
                        "--nocapture",
                        "--",
                    ])
                    .raw_arg(format!("--local --cwd {}", template.replace("%V", path)))
                    .env(EXPECTED_PATH, path)
                    .env_remove(EXISTING_PATH);
                if *exists {
                    child.env(EXISTING_PATH, "1");
                }
                let output = child.output().unwrap();
                assert!(
                    output.status.success()
                        && String::from_utf8_lossy(&output.stdout)
                            .contains("explorer_argv_verified"),
                    "argv child failed: {}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }

    #[test]
    fn parses_local_and_host_requests_with_exact_argument_grammar() {
        assert_eq!(
            parse_launch_args_with(&args(&["--local", "--shell", "wsl:Ubuntu"]), |_| false),
            LaunchRequest::Local {
                shell_id: Some("wsl:Ubuntu".into()),
                working_directory: None,
            }
        );
        assert_eq!(
            parse_launch_args_with(&args(&["--local", "--cwd", "C:\\work"]), |_| true),
            LaunchRequest::Local {
                shell_id: None,
                working_directory: Some(PathBuf::from("C:\\work")),
            }
        );
        let id = Uuid::new_v4();
        assert_eq!(
            parse_launch_args(&args(&["--host", &id.to_string()])),
            LaunchRequest::Host(id)
        );
        for invalid in [
            args(&["--local", "--shell", "x", "--shell", "y"]),
            args(&["--local", "--cwd", "relative"]),
            args(&["--local", "--cwd", "C:\\work:stream"]),
            args(&["--local", "--cwd", "\\\\.\\PhysicalDrive0"]),
            args(&["--host", "not-a-uuid", "extra"]),
            args(&["--unknown"]),
        ] {
            assert_eq!(
                parse_launch_args_with(&invalid, |_| true),
                LaunchRequest::Focus
            );
        }
    }

    #[test]
    fn bounds_parameter_count_length_and_windows_path_forms() {
        let too_many = vec!["--local".to_owned(); MAX_ARGUMENTS + 1];
        let too_long = vec!["x".repeat(MAX_ARGUMENT_LENGTH + 1)];
        assert_eq!(parse_launch_args(&too_many), LaunchRequest::Focus);
        assert_eq!(parse_launch_args(&too_long), LaunchRequest::Focus);
        for value in [
            "C:\\valid\\folder",
            "\\\\server\\share\\folder",
            "C:/valid/folder",
            "//server/share/folder",
        ] {
            assert!(valid_windows_directory(value), "{value}");
        }
        for value in [
            "relative\\folder",
            "C:relative",
            "C:\\folder\\..\\other",
            "C:\\folder\\file:stream",
            "\\\\?\\C:\\device",
            "\\\\server\\share\\folder:stream",
            "//?/C:/device",
            "//./pipe/data",
            "C:/folder/../other",
        ] {
            assert!(!valid_windows_directory(value), "{value}");
        }
    }

    #[test]
    fn ssh_url_decodes_fields_supports_ipv6_and_ignores_connection_parameters() {
        assert_eq!(
            parse_ssh_url("ssh://alice%20smith@[2001:db8::1]:2222/?fingerprint=ssh-ed25519%20abc"),
            Some(SshTarget {
                username: Some("alice smith".into()),
                host: "2001:db8::1".into(),
                port: 2222,
            })
        );
        assert_eq!(
            parse_ssh_url("ssh://build.example.test"),
            Some(SshTarget {
                username: None,
                host: "build.example.test".into(),
                port: 22,
            })
        );
        for invalid in [
            "ssh://",
            "ssh://user:password@host",
            "ssh://host:0",
            "ssh://host:65536",
            "ssh://[not-ipv6]:22",
            "ssh://host%0Aname",
            "ssh://%ZZ@example.test",
            "ssh://user@@host",
        ] {
            assert_eq!(parse_ssh_url(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn saved_hosts_match_links_on_normalized_address_port_and_user() {
        let target = |url| parse_ssh_url(url).unwrap();
        for (url, address) in [
            ("ssh://alice@Build.Example.Test.", "build.example.test"),
            ("ssh://alice@build.example.test", "BUILD.example.test."),
            ("ssh://alice@[2001:0db8:0:0::1]", "2001:db8::1"),
            ("ssh://alice@[::1]", "[0:0:0:0:0:0:0:1]"),
        ] {
            assert!(
                ssh_target_matches(&target(url), address, 22, "alice"),
                "{url}"
            );
        }
        let link = target("ssh://alice@build.example.test:2222");
        assert!(ssh_target_matches(
            &link,
            "build.example.test",
            2222,
            "alice"
        ));
        assert!(!ssh_target_matches(
            &link,
            "build.example.test",
            22,
            "alice"
        ));
        assert!(!ssh_target_matches(
            &link,
            "build.example.test",
            2222,
            "Alice"
        ));
        assert!(!ssh_target_matches(
            &link,
            "other.example.test",
            2222,
            "alice"
        ));
        assert!(!ssh_target_matches(&link, "not a host", 2222, "alice"));
        let anonymous = target("ssh://build.example.test");
        assert!(!ssh_target_matches(
            &anonymous,
            "build.example.test",
            22,
            ""
        ));
    }

    #[test]
    fn link_display_keeps_the_host_unambiguous_for_users_containing_at() {
        let target = parse_ssh_url("ssh://alice%40evil.example@good.example:2222").unwrap();
        assert_eq!(target.username.as_deref(), Some("alice@evil.example"));
        assert_eq!(target.host, "good.example");
        let view = LaunchRequest::Ssh(target).view(Uuid::nil());
        assert_eq!(view.display, "alice%40evil.example@good.example:2222");
        let view =
            LaunchRequest::Ssh(parse_ssh_url("ssh://100%25@[::1]").unwrap()).view(Uuid::nil());
        assert_eq!(view.display, "100%25@[::1]:22");
    }

    #[test]
    fn launch_view_never_contains_the_local_directory() {
        let request = LaunchRequest::Local {
            shell_id: Some("wsl:Ubuntu".into()),
            working_directory: Some(PathBuf::from("C:\\private\\directory")),
        };
        let view = request.view(Uuid::nil());
        assert_eq!(view.kind, LaunchRequestKind::Local);
        assert!(!view.display.contains("private"));
        assert_eq!(
            serde_json::to_value(view).unwrap(),
            serde_json::json!({"requestId":Uuid::nil(),"kind":"local","display":"wsl:Ubuntu"})
        );
    }

    #[test]
    fn malformed_arguments_and_nonexistent_directories_fall_back_to_focus() {
        for invalid in [
            args(&["--local", "--cwd", "C:\\missing"]),
            args(&["--local", "--cwd"]),
            args(&["--local", "--shell"]),
            args(&["--local", "--shell", "--host"]),
            args(&["--local", "--shell", "a\nb"]),
            args(&["--local", "--cwd", "C:\\x", "--cwd", "C:\\y"]),
            args(&["--host", "00000000-0000-0000-0000-000000000000"]),
            args(&["ssh://host", "--local"]),
        ] {
            assert_eq!(
                parse_launch_args_with(&invalid, |_| false),
                LaunchRequest::Focus
            );
        }
        assert_eq!(
            shell_task_arguments("wsl:Ubuntu Dev").unwrap(),
            "--local --shell \"wsl:Ubuntu Dev\""
        );
        assert_eq!(
            shell_task_arguments("wsl:a\" --host b").unwrap(),
            "--local --shell \"wsl:a\\\" --host b\""
        );
    }

    #[test]
    fn url_boundaries_and_ignored_fingerprints_cannot_enlarge_trust() {
        for invalid in [
            "ssh://host:+22",
            "ssh://host:-1",
            "ssh://host:",
            "ssh://user%00@host",
            "ssh://host/path%0A",
            "ssh://host?ignored=%7f",
            "ssh://host%2fother",
            "ssh://host%5Cother",
            "ssh://user:%70ass@host",
            "ssh://[::1]suffix",
            "ssh://host:22:33",
            "ssh://::1",
        ] {
            assert!(parse_ssh_url(invalid).is_none(), "{invalid}");
        }
        let target = parse_ssh_url("ssh://user@host:65535").unwrap();
        for suffix in [
            ";fingerprint=ignore",
            ";fingerprint=ignore@other-host:2222",
            "/ignored;fingerprint=ignore",
            "?fingerprint=ignore|other",
            "#fingerprint=ignore",
        ] {
            assert_eq!(
                parse_ssh_url(&format!("ssh://user@host:65535{suffix}")),
                Some(target.clone())
            );
        }
        for (user_length, host_length, valid) in
            [(256, 253, true), (257, 253, false), (256, 254, false)]
        {
            assert_eq!(
                parse_ssh_url(&format!(
                    "ssh://{}@{}:1",
                    "u".repeat(user_length),
                    "h".repeat(host_length)
                ))
                .is_some(),
                valid
            );
        }
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../docs/agent/fixtures/M4-contract.json"
        ))
        .unwrap();
        assert_eq!(
            serde_json::to_value(
                LaunchRequest::Ssh(SshTarget {
                    username: Some("user".into()),
                    host: "example.test".into(),
                    port: 22
                })
                .view(Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap())
            )
            .unwrap(),
            fixture["launchRequest"]
        );
    }
}
