use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    ConPtyConfig, ConPtySelection, PtyCommand, PtyError, PtySession, PtySessionConfig, Result,
    ShellDetector, ShellKind,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalTerminalOptions {
    pub shell_id: Option<String>,
    pub working_directory: Option<String>,
}

/// Public profile metadata; executable paths and command arguments stay local.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalShellProfile {
    pub id: String,
    pub name: String,
    pub is_wsl: bool,
}

pub fn local_shell_profiles() -> Vec<LocalShellProfile> {
    profiles_from_shells(ShellDetector::new().detect_all())
}

fn profiles_from_shells(shells: Vec<crate::ShellInfo>) -> Vec<LocalShellProfile> {
    shells
        .into_iter()
        .filter_map(|shell| {
            let id = profile_id(&shell.kind)?;
            Some(LocalShellProfile {
                id,
                name: shell.display_name,
                is_wsl: matches!(shell.kind, ShellKind::Wsl { .. }),
            })
        })
        .collect()
}

fn profile_id(kind: &ShellKind) -> Option<String> {
    match kind {
        ShellKind::PowerShell51 => Some("powershell51".into()),
        ShellKind::PowerShell7 => Some("powershell7".into()),
        ShellKind::Wsl { distribution } => Some(format!("wsl:{distribution}")),
        ShellKind::Posix { profile } => Some(profile.clone()),
        ShellKind::Custom => None,
    }
}

/// Apps started from Finder inherit no locale and no terminal identity. A value
/// inherited from another terminal (`pnpm tauri dev`) must not leak in either:
/// macOS `/etc/zshrc` sources per-terminal setup named after `TERM_PROGRAM`.
fn posix_environment(lang: Option<&OsStr>) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::from([
        ("TERM".to_owned(), "xterm-256color".to_owned()),
        ("COLORTERM".to_owned(), "truecolor".to_owned()),
        ("TERM_PROGRAM".to_owned(), "Capstan".to_owned()),
        (
            "TERM_PROGRAM_VERSION".to_owned(),
            env!("CARGO_PKG_VERSION").to_owned(),
        ),
    ]);
    if cfg!(target_os = "macos") && lang.is_none_or(OsStr::is_empty) {
        environment.insert("LANG".to_owned(), "en_US.UTF-8".to_owned());
    }
    environment
}

pub fn open_local_terminal(
    options: LocalTerminalOptions,
    runtime: ConPtyConfig,
) -> Result<(capstan_terminal::BackendParts, ConPtySelection)> {
    let detector = ShellDetector::new();
    let shell = if let Some(id) = options.shell_id {
        detector
            .detect_all()
            .into_iter()
            .find(|shell| profile_id(&shell.kind).as_deref() == Some(id.as_str()))
            .ok_or(PtyError::NoShell)?
    } else {
        detector.choose_default()?
    };
    let working_directory = options.working_directory.map(PathBuf::from);
    if let Some(path) = &working_directory {
        if path.as_os_str().is_empty()
            || path.to_string_lossy().contains('\0')
            || (!matches!(shell.kind, ShellKind::Wsl { .. }) && !path.is_dir())
        {
            return Err(PtyError::InvalidWorkingDirectory);
        }
        if matches!(shell.kind, ShellKind::Wsl { .. }) {
            crate::wsl::validate_directory(&shell, &path.to_string_lossy())?;
        }
    }
    let environment = if matches!(shell.kind, ShellKind::Posix { .. }) {
        posix_environment(std::env::var_os("LANG").as_deref())
    } else {
        Default::default()
    };
    let session = PtySession::spawn_with_runtime(
        &PtySessionConfig {
            command: PtyCommand {
                shell,
                working_directory,
                environment,
            },
            columns: 80,
            rows: 24,
        },
        &runtime,
    )?;
    let selected = session.runtime().clone();
    Ok((session.into_backend()?, selected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ShellInfo, ShellSource};

    #[test]
    fn posix_shells_identify_the_terminal_and_keep_an_existing_locale() {
        let inherited = posix_environment(Some(OsStr::new("zh_CN.UTF-8")));
        assert_eq!(inherited["TERM"], "xterm-256color");
        assert_eq!(inherited["COLORTERM"], "truecolor");
        assert_eq!(inherited["TERM_PROGRAM"], "Capstan");
        assert!(!inherited.contains_key("LANG"));
        for missing in [None, Some(OsStr::new(""))] {
            assert_eq!(
                posix_environment(missing).get("LANG").map(String::as_str),
                cfg!(target_os = "macos").then_some("en_US.UTF-8")
            );
        }
    }

    #[test]
    fn profiles_use_stable_ids_without_exposing_paths_or_arguments() {
        let profiles = profiles_from_shells(vec![
            ShellInfo::powershell_51(
                "private-path/powershell.exe".into(),
                ShellSource::WellKnownPath,
            ),
            ShellInfo::powershell_7("private-path/pwsh.exe".into(), ShellSource::WellKnownPath),
            ShellInfo::wsl("Ubuntu Dev"),
        ]);
        assert_eq!(
            profiles
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            ["powershell51", "powershell7", "wsl:Ubuntu Dev"]
        );
        assert!(profiles[2].is_wsl);
        let serialized = serde_json::to_string(&profiles).unwrap();
        assert!(!serialized.contains("private-path"));
        assert!(!serialized.contains("args"));
    }
}
