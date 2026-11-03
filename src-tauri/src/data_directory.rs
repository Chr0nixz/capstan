use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const PORTABLE_MARKER_NAME: &str = "capstan.portable";
pub const PORTABLE_MARKER_CONTENT: &[u8] = b"CAPSTAN_PORTABLE_V1";
const PORTABLE_DATA_DIRECTORY: &str = "data";
static PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Select the portable data directory only when the executable's sibling marker is valid.
/// The chosen directory is created and checked for write access before it is returned.
pub fn resolve_data_directory(executable: &Path, roaming_data: &Path) -> io::Result<PathBuf> {
    let executable_directory = executable.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "executable path has no parent directory",
        )
    })?;
    let marker = executable_directory.join(PORTABLE_MARKER_NAME);
    let directory = match fs::read(marker) {
        Ok(content) if content == PORTABLE_MARKER_CONTENT => {
            executable_directory.join(PORTABLE_DATA_DIRECTORY)
        }
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "portable marker is invalid",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => roaming_data.to_path_buf(),
        Err(error) => return Err(error),
    };

    ensure_writable(&directory)?;
    Ok(directory)
}

fn ensure_writable(directory: &Path) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let sequence = PROBE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let probe = directory.join(format!(
        ".capstan-write-check-{}-{sequence}.tmp",
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)?;
    let write_result = file
        .write_all(b"Capstan data directory check")
        .and_then(|()| file.sync_all());
    drop(file);
    let remove_result = fs::remove_file(probe);
    write_result?;
    remove_result
}

#[cfg(test)]
mod tests {
    use super::{resolve_data_directory, PORTABLE_MARKER_CONTENT, PORTABLE_MARKER_NAME};
    use std::fs;
    use std::io::ErrorKind;

    #[test]
    fn absent_marker_uses_the_roaming_directory_even_if_data_exists_next_to_executable() {
        let root = tempfile::tempdir().expect("temporary directory");
        let install = root.path().join("installed");
        fs::create_dir_all(install.join("data")).expect("unmarked data directory");
        let roaming = root.path().join("AppData").join("Capstan");
        let executable = install.join("capstan.exe");

        let selected = resolve_data_directory(&executable, &roaming).expect("roaming path");

        assert_eq!(selected, roaming);
        assert!(selected.is_dir());
        assert!(!install.join(PORTABLE_MARKER_NAME).exists());
    }

    #[test]
    fn valid_sibling_marker_selects_and_checks_the_portable_data_directory() {
        let root = tempfile::tempdir().expect("temporary directory");
        let install = root.path().join("portable");
        fs::create_dir_all(&install).expect("portable directory");
        fs::write(install.join(PORTABLE_MARKER_NAME), PORTABLE_MARKER_CONTENT)
            .expect("portable marker");
        let executable = install.join("capstan.exe");
        let roaming = root.path().join("AppData").join("Capstan");

        let selected = resolve_data_directory(&executable, &roaming).expect("portable path");

        assert_eq!(selected, install.join("data"));
        assert!(selected.is_dir());
        assert!(!roaming.exists());
        assert_eq!(
            fs::read_dir(&selected)
                .expect("read data directory")
                .count(),
            0
        );
    }

    #[test]
    fn invalid_marker_fails_closed_instead_of_falling_back_to_roaming_data() {
        let root = tempfile::tempdir().expect("temporary directory");
        let install = root.path().join("portable");
        fs::create_dir_all(&install).expect("portable directory");
        fs::write(install.join(PORTABLE_MARKER_NAME), b"CAPSTAN_PORTABLE_V0")
            .expect("invalid marker");
        let roaming = root.path().join("AppData").join("Capstan");

        let error = resolve_data_directory(&install.join("capstan.exe"), &roaming)
            .expect_err("invalid marker must fail");

        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(!roaming.exists());
    }

    #[test]
    fn unwritable_portable_data_path_returns_an_error() {
        let root = tempfile::tempdir().expect("temporary directory");
        let install = root.path().join("portable");
        fs::create_dir_all(&install).expect("portable directory");
        fs::write(install.join(PORTABLE_MARKER_NAME), PORTABLE_MARKER_CONTENT)
            .expect("portable marker");
        fs::write(install.join("data"), b"not a directory").expect("blocking file");

        let error = resolve_data_directory(
            &install.join("capstan.exe"),
            &root.path().join("AppData").join("Capstan"),
        )
        .expect_err("portable path must be writable");

        assert!(matches!(
            error.kind(),
            ErrorKind::AlreadyExists | ErrorKind::NotADirectory
        ));
    }
}
