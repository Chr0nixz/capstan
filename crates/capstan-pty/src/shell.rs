use std::env;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{PtyError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShellKind {
    PowerShell51,
    PowerShell7,
    Wsl { distribution: String },
    Posix { profile: String },
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerShellPreference {
    Prefer7,
    Prefer51,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShellSource {
    SystemPath,
    WellKnownPath,
    WslProbe,
    UserProvided,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellInfo {
    pub kind: ShellKind,
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub display_name: String,
    pub source: ShellSource,
}

impl ShellInfo {
    pub fn powershell_51(path: PathBuf, source: ShellSource) -> Self {
        Self {
            kind: ShellKind::PowerShell51,
            executable: path,
            args: vec![
                "-NoLogo".to_owned(),
                "-NoProfile".to_owned(),
                "-NoExit".to_owned(),
                "-Command".to_owned(),
                "$OutputEncoding = [Console]::OutputEncoding = [Console]::InputEncoding = [System.Text.UTF8Encoding]::new($false)".to_owned(),
            ],
            display_name: "Windows PowerShell 5.1".to_owned(),
            source,
        }
    }

    pub fn powershell_7(path: PathBuf, source: ShellSource) -> Self {
        Self {
            kind: ShellKind::PowerShell7,
            executable: path,
            args: vec!["-NoLogo".to_owned(), "-NoProfile".to_owned()],
            display_name: "PowerShell 7".to_owned(),
            source,
        }
    }

    pub fn wsl(distribution: impl Into<String>) -> Self {
        Self::wsl_with_executable(distribution, PathBuf::from("wsl.exe"))
    }

    pub fn wsl_with_executable(distribution: impl Into<String>, executable: PathBuf) -> Self {
        let distribution = distribution.into();
        Self {
            kind: ShellKind::Wsl {
                distribution: distribution.clone(),
            },
            executable,
            args: vec!["--distribution".to_owned(), distribution.clone()],
            display_name: format!("WSL: {distribution}"),
            source: ShellSource::WslProbe,
        }
    }
}

/// Discovers local shells without exposing arbitrary command execution.
#[derive(Debug, Clone)]
pub struct ShellDetector {
    powershell_51: Vec<PathBuf>,
    powershell_7: Vec<PathBuf>,
    wsl: Vec<ShellInfo>,
}

impl Default for ShellDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellDetector {
    pub fn new() -> Self {
        let system_root = env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let program_files = env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
        let local_app_data = env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Users\Default\AppData\Local"));

        let powershell_51 =
            vec![system_root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe")];
        let powershell_7 = vec![
            program_files.join(r"PowerShell\7\pwsh.exe"),
            local_app_data.join(r"Microsoft\PowerShell\7\pwsh.exe"),
            PathBuf::from("pwsh.exe"),
        ];
        let wsl_executable = system_root.join(r"System32\wsl.exe");
        let wsl_executable = resolve_executable(&wsl_executable)
            .or_else(|| resolve_executable(Path::new("wsl.exe")));
        let wsl = wsl_executable
            .map(|executable| {
                crate::WslProbe::default()
                    .probe()
                    .ok()
                    .into_iter()
                    .flat_map(|result| result.distributions)
                    .filter(|distribution| distribution.installed)
                    .map(|distribution| {
                        ShellInfo::wsl_with_executable(distribution.name, executable.clone())
                    })
                    .collect()
            })
            .unwrap_or_default();

        Self {
            powershell_51,
            powershell_7,
            wsl,
        }
    }

    pub fn with_candidates(
        powershell_51: Vec<PathBuf>,
        powershell_7: Vec<PathBuf>,
        wsl: Vec<ShellInfo>,
    ) -> Self {
        Self {
            powershell_51,
            powershell_7,
            wsl,
        }
    }

    pub fn detect_powershells(&self) -> Vec<ShellInfo> {
        let mut found = Vec::new();
        if let Some(path) = first_existing(&self.powershell_51) {
            found.push(ShellInfo::powershell_51(path, ShellSource::WellKnownPath));
        }
        if let Some(path) = first_existing(&self.powershell_7) {
            let source = if path.components().count() == 1 {
                ShellSource::SystemPath
            } else {
                ShellSource::WellKnownPath
            };
            found.push(ShellInfo::powershell_7(path, source));
        }
        found
    }

    pub fn detect_all(&self) -> Vec<ShellInfo> {
        let mut found = self.detect_powershells();
        found.extend(self.wsl.clone());
        #[cfg(unix)]
        found.extend(posix_profiles(env::var_os("SHELL").map(PathBuf::from)));
        found
    }

    pub(crate) fn choose_default(&self) -> Result<ShellInfo> {
        #[cfg(unix)]
        {
            posix_login(env::var_os("SHELL").map(PathBuf::from)).ok_or(PtyError::NoShell)
        }
        #[cfg(not(unix))]
        self.choose_powershell(PowerShellPreference::Prefer7)
    }

    pub fn choose_powershell(&self, preference: PowerShellPreference) -> Result<ShellInfo> {
        let found = self.detect_powershells();
        let preferred = match preference {
            PowerShellPreference::Prefer7 => ShellKind::PowerShell7,
            PowerShellPreference::Prefer51 => ShellKind::PowerShell51,
        };
        found
            .iter()
            .find(|shell| shell.kind == preferred)
            .cloned()
            .or_else(|| found.first().cloned())
            .ok_or(PtyError::NoShell)
    }

    pub fn add_wsl(&mut self, distribution: impl Into<String>) {
        self.wsl.push(ShellInfo::wsl(distribution));
    }
}

#[cfg(unix)]
fn posix_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.is_absolute()
        && path
            .metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(unix)]
fn posix_profile(id: &str, path: PathBuf, source: ShellSource) -> ShellInfo {
    ShellInfo {
        display_name: path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("sh")
            .to_owned(),
        kind: ShellKind::Posix {
            profile: id.to_owned(),
        },
        executable: path,
        args: vec!["-l".to_owned(), "-i".to_owned()],
        source,
    }
}

#[cfg(unix)]
fn posix_login(login_shell: Option<PathBuf>) -> Option<ShellInfo> {
    login_shell
        .filter(|path| posix_executable(path))
        .map(|path| posix_profile("login", path, ShellSource::SystemPath))
        .or_else(|| {
            ["/bin/zsh", "/bin/bash", "/bin/sh"]
                .into_iter()
                .find(|path| posix_executable(Path::new(path)))
                .map(|path| posix_profile("login", PathBuf::from(path), ShellSource::WellKnownPath))
        })
}

#[cfg(unix)]
fn posix_profiles(login_shell: Option<PathBuf>) -> Vec<ShellInfo> {
    let mut profiles = posix_login(login_shell).into_iter().collect::<Vec<_>>();
    for (id, paths) in [
        (
            "zsh",
            [
                "/bin/zsh",
                "/opt/homebrew/bin/zsh",
                "/usr/local/bin/zsh",
                "/usr/bin/zsh",
            ],
        ),
        (
            "bash",
            [
                "/bin/bash",
                "/opt/homebrew/bin/bash",
                "/usr/local/bin/bash",
                "/usr/bin/bash",
            ],
        ),
        (
            "fish",
            [
                "/opt/homebrew/bin/fish",
                "/usr/local/bin/fish",
                "/usr/bin/fish",
                "/bin/fish",
            ],
        ),
    ] {
        if let Some(path) = paths
            .into_iter()
            .find(|path| posix_executable(Path::new(path)))
        {
            let path = PathBuf::from(path);
            if !profiles.iter().any(|shell| shell.executable == path) {
                profiles.push(posix_profile(id, path, ShellSource::WellKnownPath));
            }
        }
    }
    profiles
}

fn first_existing(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates
        .iter()
        .find_map(|candidate| resolve_executable(candidate))
}

fn resolve_executable(candidate: &Path) -> Option<PathBuf> {
    if candidate.is_file() {
        // Legacy PowerShell's .NET initialization rejects verbatim Windows paths.
        return dunce::canonicalize(candidate)
            .ok()
            .or_else(|| candidate.is_absolute().then(|| candidate.to_path_buf()));
    }
    if candidate.components().count() != 1 {
        return None;
    }
    let path_var = env::var_os("PATH")?;
    for directory in env::split_paths(&path_var) {
        let path_candidate = directory.join(candidate);
        let candidates = if cfg!(windows) {
            vec![
                path_candidate.clone(),
                path_candidate.with_extension("exe"),
                path_candidate.with_extension("cmd"),
                path_candidate.with_extension("bat"),
            ]
        } else {
            vec![path_candidate]
        };
        if let Some(found) = candidates.into_iter().find(|path| path.is_file()) {
            return dunce::canonicalize(&found).ok().or(Some(found));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{PowerShellPreference, ShellDetector, ShellInfo, ShellKind};

    #[test]
    fn detects_existing_candidates_and_preserves_order() {
        let directory = tempdir().expect("temporary directory");
        let ps51 = directory.path().join("powershell.exe");
        let ps7 = directory.path().join("pwsh.exe");
        fs::write(&ps51, b"test").expect("write powershell fixture");
        fs::write(&ps7, b"test").expect("write pwsh fixture");

        let detector = ShellDetector::with_candidates(vec![ps51], vec![ps7], Vec::new());
        let shells = detector.detect_powershells();
        assert_eq!(shells.len(), 2);
        assert_eq!(shells[0].kind, ShellKind::PowerShell51);
        assert_eq!(shells[1].kind, ShellKind::PowerShell7);
    }

    #[test]
    fn falls_back_to_available_powershell() {
        let detector =
            ShellDetector::with_candidates(Vec::new(), Vec::new(), vec![ShellInfo::wsl("Ubuntu")]);
        assert!(detector
            .choose_powershell(PowerShellPreference::Prefer7)
            .is_err());
    }

    #[test]
    fn wsl_shell_is_represented_as_a_fixed_command() {
        let shell = ShellInfo::wsl("Ubuntu");
        assert_eq!(shell.executable, std::path::PathBuf::from("wsl.exe"));
        assert_eq!(shell.args, vec!["--distribution", "Ubuntu"]);
    }

    #[test]
    fn powershell_51_configures_utf8_for_interactive_io() {
        let shell =
            ShellInfo::powershell_51("powershell.exe".into(), super::ShellSource::UserProvided);
        assert!(shell.args.contains(&"-NoExit".to_owned()));
        assert!(shell.args.iter().any(|arg| arg.contains("UTF8Encoding")));
    }

    #[test]
    fn resolves_path_candidates_to_absolute_executables() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let executable = directory.path().join("pwsh.exe");
        fs::write(&executable, b"fixture").expect("write executable fixture");
        let resolved = super::resolve_executable(&executable).expect("resolve executable");
        assert!(resolved.is_absolute());
        assert_eq!(resolved.file_name(), Some(std::ffi::OsStr::new("pwsh.exe")));
    }

    #[cfg(unix)]
    mod posix {
        use std::os::unix::fs::PermissionsExt;
        use std::path::PathBuf;

        use super::super::{posix_login, posix_profiles, ShellKind, ShellSource};

        fn script(directory: &std::path::Path, name: &str, mode: u32) -> PathBuf {
            let path = directory.join(name);
            std::fs::write(&path, b"#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            path
        }

        #[test]
        fn login_shell_comes_from_the_environment_and_starts_as_a_login_shell() {
            let directory = tempfile::tempdir().unwrap();
            let shell = script(directory.path(), "nu", 0o755);
            let login = posix_login(Some(shell.clone())).unwrap();
            assert_eq!(login.executable, shell);
            assert_eq!(login.args, ["-l", "-i"]);
            assert_eq!(login.display_name, "nu");
            assert_eq!(login.source, ShellSource::SystemPath);
            assert_eq!(
                login.kind,
                ShellKind::Posix {
                    profile: "login".to_owned()
                }
            );
        }

        #[test]
        fn unusable_login_shells_fall_back_to_a_system_shell() {
            let directory = tempfile::tempdir().unwrap();
            for shell in [
                None,
                Some(PathBuf::from("zsh")),
                Some(directory.path().join("missing")),
                Some(directory.path().to_owned()),
                Some(script(directory.path(), "not-executable", 0o644)),
            ] {
                let login = posix_login(shell).unwrap();
                assert_eq!(login.source, ShellSource::WellKnownPath);
                assert!(login.executable.starts_with("/bin"));
            }
        }

        #[test]
        fn the_login_shell_is_not_listed_twice() {
            let login = posix_login(None).unwrap();
            let profiles = posix_profiles(Some(login.executable.clone()));
            assert_eq!(profiles[0].kind, login.kind);
            let listed = profiles
                .iter()
                .filter(|shell| shell.executable == login.executable)
                .count();
            assert_eq!(listed, 1);
        }
    }

    #[cfg(windows)]
    #[test]
    fn detected_windows_powershell_can_initialize_legacy_dotnet() {
        let system_root = std::env::var_os("SystemRoot").expect("Windows system root");
        let executable = std::path::PathBuf::from(system_root)
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        let detector = ShellDetector::with_candidates(vec![executable], Vec::new(), Vec::new());
        let shell = detector
            .choose_powershell(PowerShellPreference::Prefer51)
            .expect("detect Windows PowerShell");
        assert!(
            !shell
                .executable
                .as_os_str()
                .to_string_lossy()
                .starts_with(r"\\?\"),
            "legacy PowerShell requires a conventional absolute executable path"
        );
        let output = std::process::Command::new(&shell.executable)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::Write('capstan-shell-started')",
            ])
            .output()
            .expect("launch detected Windows PowerShell");
        assert!(output.status.success(), "PowerShell failed to initialize");
        assert_eq!(output.stdout, b"capstan-shell-started");
    }
}
