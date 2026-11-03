use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{PtyError, Result};

/// Where the ConPTY implementation is obtained from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConPtySource {
    Bundled,
    System,
}

/// A selected ConPTY runtime and the reason it was selected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConPtySelection {
    pub source: ConPtySource,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    pub architecture: String,
    pub reason: String,
}

/// Inputs used when selecting a runtime.  A bundle is only considered when its
/// path is absolute and the file exists; this prevents an unverified relative
/// DLL from silently becoming a release dependency.
#[derive(Debug, Clone, Default)]
pub struct ConPtyConfig {
    pub bundled_path: Option<PathBuf>,
    pub bundled_version: Option<String>,
    pub expected_architecture: Option<String>,
}

impl ConPtyConfig {
    /// Application-controlled runtime directory, never the working directory.
    pub fn for_executable(executable: &Path) -> Self {
        let directory = executable
            .parent()
            .map(|parent| parent.join("runtime").join(std::env::consts::ARCH));
        let bundled_version = directory.as_deref().and_then(runtime_package_version);
        Self {
            bundled_path: directory.map(|directory| directory.join("conpty.dll")),
            bundled_version,
            expected_architecture: Some(std::env::consts::ARCH.to_owned()),
        }
    }
}

fn runtime_package_version(directory: &Path) -> Option<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(directory.join("manifest.json"))
        .ok()?
        .take(16 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 16 * 1024 {
        return None;
    }
    let metadata: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    if metadata.get("package")?.as_str()? != "Microsoft.Windows.Console.ConPTY"
        || metadata.get("architecture")?.as_str()? != std::env::consts::ARCH
    {
        return None;
    }
    let version = metadata.get("package_version")?.as_str()?;
    (version.len() <= 64
        && !version.is_empty()
        && version.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
    .then(|| version.to_owned())
}

/// Result status used by diagnostics and the M0 compatibility report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConPtyStatus {
    Selected(ConPtySelection),
    RejectedBundle { path: PathBuf, reason: String },
}

impl ConPtySelection {
    pub fn system() -> Self {
        Self {
            source: ConPtySource::System,
            path: None,
            version: None,
            architecture: std::env::consts::ARCH.to_owned(),
            reason: "using the operating system ConPTY implementation".to_owned(),
        }
    }
}

/// Select the bundled runtime only after basic path and architecture checks.
/// The actual ConPTY loading remains owned by `portable-pty`/Windows; this
/// function records the decision and provides a safe system fallback.
pub fn select_conpty(config: &ConPtyConfig) -> Result<ConPtyStatus> {
    let Some(path) = config.bundled_path.as_ref() else {
        return Ok(ConPtyStatus::Selected(ConPtySelection::system()));
    };

    if !path.is_absolute() {
        return Ok(ConPtyStatus::RejectedBundle {
            path: path.clone(),
            reason: "bundled ConPTY path must be absolute".to_owned(),
        });
    }
    if !path.is_file() {
        return Ok(ConPtyStatus::RejectedBundle {
            path: path.clone(),
            reason: "bundled ConPTY file does not exist".to_owned(),
        });
    }

    if let Some(expected) = config.expected_architecture.as_deref() {
        let current = std::env::consts::ARCH;
        if expected != current {
            return Ok(ConPtyStatus::RejectedBundle {
                path: path.clone(),
                reason: format!(
                    "bundle architecture {expected} does not match process architecture {current}"
                ),
            });
        }
    }

    if let Err(error) = validate_pe_architecture(path) {
        return Ok(ConPtyStatus::RejectedBundle {
            path: path.clone(),
            reason: error.to_string(),
        });
    }

    Ok(ConPtyStatus::Selected(ConPtySelection {
        source: ConPtySource::Bundled,
        path: Some(path.clone()),
        version: config.bundled_version.clone(),
        architecture: config
            .expected_architecture
            .clone()
            .unwrap_or_else(|| std::env::consts::ARCH.to_owned()),
        reason: "selected an existing absolute bundled ConPTY path".to_owned(),
    }))
}

fn validate_pe_architecture(path: &Path) -> Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let mut dos = [0_u8; 64];
    file.read_exact(&mut dos)?;
    if &dos[..2] != b"MZ" {
        return Err(PtyError::ConPty("invalid PE header".to_owned()));
    }
    let offset = u32::from_le_bytes([dos[60], dos[61], dos[62], dos[63]]);
    file.seek(SeekFrom::Start(u64::from(offset)))?;
    let mut pe = [0_u8; 6];
    file.read_exact(&mut pe)?;
    let expected = match std::env::consts::ARCH {
        "x86_64" => 0x8664,
        "aarch64" => 0xaa64,
        "x86" => 0x014c,
        _ => 0,
    };
    if &pe[..4] != b"PE\0\0" || u16::from_le_bytes([pe[4], pe[5]]) != expected {
        return Err(PtyError::ConPty(
            "ConPTY PE architecture does not match the process".to_owned(),
        ));
    }
    Ok(())
}

pub fn validate_bundle_path(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(PtyError::ConPty("bundle path must be absolute".to_owned()));
    }
    if !path.is_file() {
        return Err(PtyError::ConPty(format!(
            "bundle path does not exist: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{select_conpty, ConPtyConfig, ConPtySource, ConPtyStatus};

    #[test]
    fn packaged_runtime_metadata_is_bounded_and_architecture_specific() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = directory
            .path()
            .join("runtime")
            .join(std::env::consts::ARCH);
        std::fs::create_dir_all(&runtime).unwrap();
        let manifest = runtime.join("manifest.json");
        std::fs::write(
            &manifest,
            serde_json::to_vec(&serde_json::json!({
                "package": "Microsoft.Windows.Console.ConPTY",
                "package_version": "1.25.260930003",
                "architecture": std::env::consts::ARCH,
            }))
            .unwrap(),
        )
        .unwrap();
        let executable = directory.path().join("Capstan.exe");
        assert_eq!(
            ConPtyConfig::for_executable(&executable)
                .bundled_version
                .as_deref(),
            Some("1.25.260930003")
        );
        std::fs::write(&manifest, vec![b' '; 16 * 1024 + 1]).unwrap();
        assert!(ConPtyConfig::for_executable(&executable)
            .bundled_version
            .is_none());
        std::fs::write(
            &manifest,
            br#"{"package":"another-runtime","package_version":"1.0","architecture":"x86_64"}"#,
        )
        .unwrap();
        assert!(ConPtyConfig::for_executable(&executable)
            .bundled_version
            .is_none());
    }

    #[test]
    fn missing_bundle_falls_back_to_system() {
        let status = select_conpty(&ConPtyConfig::default()).expect("selection succeeds");
        let ConPtyStatus::Selected(selection) = status else {
            panic!("missing bundle must select system runtime");
        };
        assert_eq!(selection.source, ConPtySource::System);
    }

    #[test]
    fn relative_bundle_is_rejected() {
        let status = select_conpty(&ConPtyConfig {
            bundled_path: Some("runtime/conpty.dll".into()),
            ..ConPtyConfig::default()
        })
        .expect("selection succeeds");
        assert!(matches!(status, ConPtyStatus::RejectedBundle { .. }));
    }

    #[test]
    fn rejects_a_pe_with_a_different_machine_type_before_loading() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("conpty.dll");
        let mut header = [0_u8; 128];
        header[..2].copy_from_slice(b"MZ");
        header[60..64].copy_from_slice(&64_u32.to_le_bytes());
        header[64..68].copy_from_slice(b"PE\0\0");
        let other_machine: u16 = if cfg!(target_arch = "aarch64") {
            0x8664
        } else {
            0xaa64
        };
        header[68..70].copy_from_slice(&other_machine.to_le_bytes());
        std::fs::write(&path, header).expect("fixture");
        let selected = select_conpty(&ConPtyConfig {
            bundled_path: Some(path),
            ..ConPtyConfig::default()
        })
        .expect("selection");
        assert!(matches!(selected, ConPtyStatus::RejectedBundle { .. }));
    }
}
