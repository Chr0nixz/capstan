use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use std::{io::Read, process::Child};

use serde::{Deserialize, Serialize};

use crate::{PtyError, Result};

const MAX_WSL_PROBE_OUTPUT: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WslSource {
    Registry,
    WslExeFallback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WslDistribution {
    pub name: String,
    pub base_path: Option<String>,
    pub version: Option<u32>,
    pub installed: bool,
    pub is_default: bool,
    pub source: WslSource,
}

#[derive(Debug, Clone)]
pub struct WslProbeConfig {
    pub allow_fallback: bool,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslProbeResult {
    pub distributions: Vec<WslDistribution>,
    pub source: WslSource,
    pub fallback_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WslProbe {
    config: WslProbeConfig,
}

impl Default for WslProbe {
    fn default() -> Self {
        Self::new(WslProbeConfig::default())
    }
}

impl Default for WslProbeConfig {
    fn default() -> Self {
        Self {
            allow_fallback: true,
            timeout_ms: 1_500,
        }
    }
}

impl WslProbe {
    pub fn new(config: WslProbeConfig) -> Self {
        Self { config }
    }

    pub fn probe(&self) -> Result<WslProbeResult> {
        #[cfg(windows)]
        {
            match probe_registry() {
                Ok(distributions) if !distributions.is_empty() => {
                    return Ok(WslProbeResult {
                        distributions,
                        source: WslSource::Registry,
                        fallback_reason: None,
                    });
                }
                Ok(_) => {
                    if !self.config.allow_fallback {
                        return Ok(WslProbeResult {
                            distributions: Vec::new(),
                            source: WslSource::Registry,
                            fallback_reason: Some(
                                "registry returned no installed distributions".to_owned(),
                            ),
                        });
                    }
                }
                Err(error) => {
                    if !self.config.allow_fallback {
                        return Err(error);
                    }
                    return self.fallback_with_reason(error.to_string());
                }
            }
            self.fallback_with_reason("registry data was empty".to_owned())
        }

        #[cfg(not(windows))]
        {
            Ok(WslProbeResult {
                distributions: Vec::new(),
                source: WslSource::Registry,
                fallback_reason: Some("registry probing is only available on Windows".to_owned()),
            })
        }
    }

    pub fn from_fixtures(
        registry: Result<Vec<WslDistribution>>,
        fallback: Result<Vec<WslDistribution>>,
    ) -> Result<WslProbeResult> {
        match registry {
            Ok(distributions) if !distributions.is_empty() => Ok(WslProbeResult {
                distributions,
                source: WslSource::Registry,
                fallback_reason: None,
            }),
            Ok(_) => fallback.map(|distributions| WslProbeResult {
                distributions,
                source: WslSource::WslExeFallback,
                fallback_reason: Some("registry data was empty".to_owned()),
            }),
            Err(error) => fallback.map(|distributions| WslProbeResult {
                distributions,
                source: WslSource::WslExeFallback,
                fallback_reason: Some(error.to_string()),
            }),
        }
    }

    fn fallback_with_reason(&self, reason: String) -> Result<WslProbeResult> {
        let distributions = probe_wsl_exe(self.config.timeout_ms)?;
        Ok(WslProbeResult {
            distributions,
            source: WslSource::WslExeFallback,
            fallback_reason: Some(reason),
        })
    }
}

fn probe_wsl_exe(timeout_ms: u64) -> Result<Vec<WslDistribution>> {
    let executable = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .map(|root| root.join(r"System32\wsl.exe"))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| std::path::PathBuf::from("wsl.exe"));
    let mut child = Command::new(executable)
        .args(["--list", "--verbose"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| PtyError::WslProbe(format!("failed to start wsl.exe: {error}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| PtyError::WslProbe("wsl.exe stdout pipe was not created".to_owned()))?;
    let stdout_reader = thread::spawn(move || {
        let mut output = Vec::new();
        stdout
            .take(MAX_WSL_PROBE_OUTPUT)
            .read_to_end(&mut output)
            .map(|_| output)
    });
    let timeout = Duration::from_millis(timeout_ms.max(1));
    let status = match wait_with_timeout(&mut child, timeout) {
        Ok(status) => status,
        Err(error) => {
            let _ = stdout_reader.join();
            return Err(error);
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| PtyError::WslProbe("wsl.exe output reader failed".to_owned()))?
        .map_err(|error| PtyError::WslProbe(format!("cannot read wsl.exe output: {error}")))?;
    if !status.success() {
        return Err(PtyError::WslProbe(format!(
            "wsl.exe exited with status {status}"
        )));
    }
    Ok(parse_wsl_verbose_output(&stdout))
}

pub(crate) fn validate_directory(shell: &crate::ShellInfo, path: &str) -> Result<()> {
    if !path.starts_with('/') || path.len() > 4096 || path.contains('\0') {
        return Err(PtyError::InvalidWorkingDirectory);
    }
    let mut command = Command::new(&shell.executable);
    command
        .args(&shell.args)
        .args([
            "--exec",
            "/bin/sh",
            "-c",
            "test -d \"$1\"",
            "capstan-directory-probe",
            path,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command
        .spawn()
        .map_err(|_| PtyError::WslProbe("directory_probe_failed".into()))?;
    if !wait_with_timeout(&mut child, Duration::from_secs(15))?.success() {
        return Err(PtyError::InvalidWorkingDirectory);
    }
    Ok(())
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Result<std::process::ExitStatus> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(PtyError::WslProbe(format!("cannot poll wsl.exe: {error}")));
            }
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(PtyError::WslProbe(format!(
                "wsl.exe probe timed out after {} ms",
                timeout.as_millis()
            )));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn parse_wsl_verbose_output(bytes: &[u8]) -> Vec<WslDistribution> {
    let text = decode_wsl_output(bytes);
    let mut distributions = Vec::new();
    for line in text.lines().skip(1) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let is_default = line.starts_with('*');
        let fields: Vec<&str> = line.trim_start_matches('*').split_whitespace().collect();
        if fields.len() < 2 {
            continue;
        }
        let name = fields[0].to_owned();
        let version = fields.get(2).and_then(|value| value.parse::<u32>().ok());
        distributions.push(WslDistribution {
            name,
            base_path: None,
            version,
            installed: true,
            is_default,
            source: WslSource::WslExeFallback,
        });
    }
    distributions
}

fn decode_wsl_output(bytes: &[u8]) -> String {
    let (encoding, body) = match bytes {
        [0xff, 0xfe, rest @ ..] => (Some(false), rest),
        [0xfe, 0xff, rest @ ..] => (Some(true), rest),
        _ if looks_like_utf16_le(bytes) => (Some(false), bytes),
        _ if looks_like_utf16_be(bytes) => (Some(true), bytes),
        _ => (None, bytes),
    };

    if let Some(big_endian) = encoding {
        let units = body
            .chunks_exact(2)
            .map(|pair| {
                if big_endian {
                    u16::from_be_bytes([pair[0], pair[1]])
                } else {
                    u16::from_le_bytes([pair[0], pair[1]])
                }
            })
            .collect::<Vec<_>>();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn looks_like_utf16_le(bytes: &[u8]) -> bool {
    if bytes.len() < 8 || !bytes.len().is_multiple_of(2) {
        return false;
    }
    let pairs = bytes.chunks_exact(2).collect::<Vec<_>>();
    let odd_nulls = pairs.iter().filter(|pair| pair[1] == 0).count();
    let even_nulls = pairs.iter().filter(|pair| pair[0] == 0).count();
    odd_nulls * 5 >= pairs.len() * 3 && even_nulls * 4 < pairs.len()
}

fn looks_like_utf16_be(bytes: &[u8]) -> bool {
    if bytes.len() < 8 || !bytes.len().is_multiple_of(2) {
        return false;
    }
    let pairs = bytes.chunks_exact(2).collect::<Vec<_>>();
    let even_nulls = pairs.iter().filter(|pair| pair[0] == 0).count();
    let odd_nulls = pairs.iter().filter(|pair| pair[1] == 0).count();
    even_nulls * 5 >= pairs.len() * 3 && odd_nulls * 4 < pairs.len()
}

#[cfg(windows)]
fn probe_registry() -> Result<Vec<WslDistribution>> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let lxss = current_user
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Lxss")
        .map_err(|error| PtyError::WslProbe(format!("cannot open WSL registry key: {error}")))?;
    let default_name: Option<String> = lxss.get_value("DefaultDistribution").ok();
    let mut distributions = Vec::new();
    for entry in lxss.enum_keys() {
        let Ok(name) = entry else {
            continue;
        };
        let Ok(key) = lxss.open_subkey(&name) else {
            continue;
        };
        let Ok(distribution_name) = key.get_value::<String, _>("DistributionName") else {
            continue;
        };
        let base_path: Option<String> = key.get_value("BasePath").ok();
        let version: Option<u32> = key.get_value("Version").ok();
        let state: Option<u32> = key.get_value("State").ok();
        distributions.push(WslDistribution {
            is_default: default_name
                .as_deref()
                .is_some_and(|default| default == distribution_name || default == name),
            name: distribution_name,
            base_path,
            version,
            installed: state != Some(0),
            source: WslSource::Registry,
        });
    }
    Ok(distributions)
}

#[cfg(test)]
mod tests {
    use super::{parse_wsl_verbose_output, WslDistribution, WslProbe, WslSource};

    fn distribution(name: &str) -> WslDistribution {
        WslDistribution {
            name: name.to_owned(),
            base_path: None,
            version: Some(2),
            installed: true,
            is_default: name == "Ubuntu",
            source: WslSource::Registry,
        }
    }

    #[test]
    fn uses_registry_when_data_is_complete() {
        let result = WslProbe::from_fixtures(Ok(vec![distribution("Ubuntu")]), Ok(Vec::new()))
            .expect("fixture probe");
        assert_eq!(result.source, WslSource::Registry);
        assert_eq!(result.distributions[0].name, "Ubuntu");
    }

    #[test]
    fn falls_back_when_registry_is_empty() {
        let result = WslProbe::from_fixtures(
            Ok(Vec::new()),
            Ok(vec![WslDistribution {
                source: WslSource::WslExeFallback,
                ..distribution("Debian")
            }]),
        )
        .expect("fixture probe");
        assert_eq!(result.source, WslSource::WslExeFallback);
        assert_eq!(
            result.fallback_reason.as_deref(),
            Some("registry data was empty")
        );
    }

    #[test]
    fn registry_error_is_reported_if_fallback_fails() {
        let result = WslProbe::from_fixtures(
            Err(crate::PtyError::WslProbe("registry unavailable".to_owned())),
            Err(crate::PtyError::WslProbe("wsl unavailable".to_owned())),
        );
        assert!(result.is_err());
    }

    #[test]
    fn parses_utf8_wsl_verbose_output_without_exposing_command_text() {
        let output = b"  NAME                   STATE           VERSION\n* Ubuntu                 Running         2\n  Debian                 Stopped         1\n";
        let distributions = parse_wsl_verbose_output(output);
        assert_eq!(distributions.len(), 2);
        assert_eq!(distributions[0].name, "Ubuntu");
        assert!(distributions[0].is_default);
        assert_eq!(distributions[1].version, Some(1));
    }

    #[test]
    fn parses_utf16_wsl_verbose_output() {
        let text = "  NAME                   STATE           VERSION\n* Ubuntu                 Running         2\n";
        let mut output = vec![0xff, 0xfe];
        for unit in text.encode_utf16() {
            output.extend_from_slice(&unit.to_le_bytes());
        }
        let distributions = parse_wsl_verbose_output(&output);
        assert_eq!(distributions[0].name, "Ubuntu");
        assert_eq!(distributions[0].version, Some(2));
    }

    #[test]
    fn parses_utf16le_wsl_output_without_a_bom() {
        let text = "  NAME                   STATE           VERSION\r\n* Ubuntu                 Running         2\r\n";
        let output = text
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let distributions = parse_wsl_verbose_output(&output);
        assert_eq!(distributions.len(), 1);
        assert_eq!(distributions[0].name, "Ubuntu");
        assert!(distributions[0].is_default);
        assert_eq!(distributions[0].version, Some(2));
    }
}
