use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    ConPtyConfig, ConPtySelection, PowerShellPreference, PtyCommand, PtyError, PtySession,
    PtySessionConfig, Result, ShellDetector, ShellKind,
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
            let (id, is_wsl) = match shell.kind {
                ShellKind::PowerShell51 => ("powershell51".to_owned(), false),
                ShellKind::PowerShell7 => ("powershell7".to_owned(), false),
                ShellKind::Wsl { distribution } => (format!("wsl:{distribution}"), true),
                ShellKind::Custom => return None,
            };
            Some(LocalShellProfile {
                id,
                name: shell.display_name,
                is_wsl,
            })
        })
        .collect()
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
            .find(|shell| match &shell.kind {
                ShellKind::PowerShell51 => id == "powershell51",
                ShellKind::PowerShell7 => id == "powershell7",
                ShellKind::Wsl { distribution } => id == format!("wsl:{distribution}"),
                ShellKind::Custom => false,
            })
            .ok_or(PtyError::NoShell)?
    } else {
        detector.choose_powershell(PowerShellPreference::Prefer7)?
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
    let session = PtySession::spawn_with_runtime(
        &PtySessionConfig {
            command: PtyCommand {
                shell,
                working_directory,
                environment: Default::default(),
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
