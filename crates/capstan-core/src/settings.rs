//! Nonsecret UI preferences at the Rust-selected application data root.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use thiserror::Error;

const MAX_SETTINGS_BYTES: usize = 16 * 1024;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelDock {
    #[default]
    Right,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelLayout {
    pub dock: PanelDock,
    pub size: u8,
    pub collapsed: bool,
}
impl Default for PanelLayout {
    fn default() -> Self {
        Self {
            dock: PanelDock::Right,
            size: 36,
            collapsed: false,
        }
    }
}
impl PanelLayout {
    fn validate(self) -> Result<Self, SettingsError> {
        if !(20..=60).contains(&self.size) {
            return Err(SettingsError::Invalid);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SettingsError {
    #[error("settings_invalid")]
    Invalid,
    #[error("settings_changed")]
    Changed,
    #[error("settings_unavailable")]
    Unavailable,
}

pub struct Settings {
    path: PathBuf,
    gate: Mutex<()>,
    session: uuid::Uuid,
    sequence: AtomicU64,
    admission: Mutex<(bool, usize)>,
    idle: tokio::sync::Notify,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelPreferences {
    pub layout: PanelLayout,
    pub session: uuid::Uuid,
    pub sequence: crate::sftp::ExactU64,
}
struct PendingWrite(Arc<Settings>);
impl Drop for PendingWrite {
    fn drop(&mut self) {
        if let Ok(mut admission) = self.0.admission.lock() {
            admission.1 -= 1;
            if admission.1 == 0 {
                self.0.idle.notify_waiters();
            }
        }
    }
}
impl Settings {
    /// The caller supplies the already-selected app root, never a WebView path.
    pub fn new(directory: PathBuf) -> Self {
        Self {
            path: directory.join("settings.json"),
            gate: Mutex::new(()),
            session: uuid::Uuid::new_v4(),
            sequence: AtomicU64::new(0),
            admission: Mutex::new((false, 0)),
            idle: tokio::sync::Notify::new(),
        }
    }

    pub fn snapshot(&self) -> Result<PanelPreferences, SettingsError> {
        Ok(PanelPreferences {
            layout: self.panel()?,
            session: self.session,
            sequence: crate::sftp::ExactU64(self.sequence.load(Ordering::SeqCst)),
        })
    }

    pub async fn save(
        self: Arc<Self>,
        session: uuid::Uuid,
        sequence: crate::sftp::ExactU64,
        layout: PanelLayout,
    ) -> Result<(), SettingsError> {
        layout.validate()?;
        if session != self.session || sequence.0 == 0 {
            return Err(SettingsError::Changed);
        }
        let pending = {
            let mut admission = self
                .admission
                .lock()
                .map_err(|_| SettingsError::Unavailable)?;
            if admission.0 || admission.1 >= 32 {
                return Err(SettingsError::Unavailable);
            }
            admission.1 += 1;
            PendingWrite(self.clone())
        };
        let previous = self.sequence.fetch_max(sequence.0, Ordering::SeqCst);
        if previous >= sequence.0 {
            return Ok(());
        }
        tokio::task::spawn_blocking(move || {
            let _pending = pending;
            let _guard = self.gate.lock().map_err(|_| SettingsError::Unavailable)?;
            if self.sequence.load(Ordering::SeqCst) != sequence.0 {
                return Ok(());
            }
            self.write_panel(layout)
        })
        .await
        .map_err(|_| SettingsError::Unavailable)?
    }

    pub async fn drain(&self) -> Result<(), SettingsError> {
        loop {
            let notified = self.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut admission = self
                    .admission
                    .lock()
                    .map_err(|_| SettingsError::Unavailable)?;
                admission.0 = true;
                if admission.1 == 0 {
                    return Ok(());
                }
            }
            notified.await;
        }
    }

    pub(crate) fn resume(&self) {
        if let Ok(mut admission) = self.admission.lock() {
            admission.0 = false;
        }
    }

    pub fn panel(&self) -> Result<PanelLayout, SettingsError> {
        let _guard = self.gate.lock().map_err(|_| SettingsError::Unavailable)?;
        let (document, _) = self.read()?;
        decode_panel(&document)
    }

    pub fn save_panel(&self, layout: PanelLayout) -> Result<(), SettingsError> {
        let layout = layout.validate()?;
        let _guard = self.gate.lock().map_err(|_| SettingsError::Unavailable)?;
        self.write_panel(layout)
    }

    /// This path is selected natively and stays in Rust; the UI receives only
    /// the process-local editor configuration ID.
    pub fn editor_executable(&self) -> Result<Option<PathBuf>, SettingsError> {
        let _guard = self.gate.lock().map_err(|_| SettingsError::Unavailable)?;
        let (document, _) = self.read()?;
        decode_editor(&document)
    }

    pub async fn save_editor_executable(
        self: Arc<Self>,
        path: PathBuf,
    ) -> Result<(), SettingsError> {
        crate::remote_edit::EditorConfig::new(uuid::Uuid::new_v4(), path.clone())
            .map_err(|_| SettingsError::Invalid)?;
        let pending = {
            let mut admission = self
                .admission
                .lock()
                .map_err(|_| SettingsError::Unavailable)?;
            if admission.0 || admission.1 >= 32 {
                return Err(SettingsError::Unavailable);
            }
            admission.1 += 1;
            PendingWrite(self.clone())
        };
        tokio::task::spawn_blocking(move || {
            let _pending = pending;
            let _guard = self.gate.lock().map_err(|_| SettingsError::Unavailable)?;
            let (mut document, storage) = self.read()?;
            decode_editor(&document)?;
            document.insert("schemaVersion".into(), Value::from(1));
            document.insert(
                "remoteEditor".into(),
                serde_json::to_value(path).map_err(|_| SettingsError::Invalid)?,
            );
            commit_document(document, storage)
        })
        .await
        .map_err(|_| SettingsError::Unavailable)?
    }

    fn write_panel(&self, layout: PanelLayout) -> Result<(), SettingsError> {
        let (mut document, storage) = self.read()?;
        // Refuse a malformed existing known field instead of replacing it.
        decode_panel(&document)?;
        document.insert("schemaVersion".into(), Value::from(1));
        document.insert(
            "sftpPanel".into(),
            serde_json::to_value(layout).map_err(|_| SettingsError::Invalid)?,
        );
        commit_document(document, storage)
    }

    #[cfg(windows)]
    fn read(&self) -> Result<(Map<String, Value>, Storage), SettingsError> {
        use capstan_native_windows::transfer::{Destination, Source};
        let destination = Destination::select(&self.path).map_err(storage_error)?;
        let expected = destination.snapshot().map_err(storage_error)?;
        let document = if expected.is_some() {
            let source = Source::open(&self.path).map_err(storage_error)?;
            if source.size() > MAX_SETTINGS_BYTES as u64 {
                return Err(SettingsError::Invalid);
            }
            let bytes = source
                .read(0, MAX_SETTINGS_BYTES + 1)
                .map_err(storage_error)?;
            source.verify().map_err(storage_error)?;
            let document = decode_document(&bytes)?;
            if destination.snapshot().map_err(storage_error)? != expected {
                return Err(SettingsError::Changed);
            }
            document
        } else {
            Map::new()
        };
        Ok((
            document,
            Storage {
                destination,
                expected,
            },
        ))
    }

    #[cfg(not(windows))]
    fn read(&self) -> Result<(Map<String, Value>, Storage), SettingsError> {
        let _ = &self.path;
        Err(SettingsError::Unavailable)
    }
}

fn commit_document(document: Map<String, Value>, storage: Storage) -> Result<(), SettingsError> {
    let mut bytes = serde_json::to_vec_pretty(&document).map_err(|_| SettingsError::Invalid)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_SETTINGS_BYTES {
        return Err(SettingsError::Invalid);
    }
    storage.commit(&bytes)
}

fn decode_editor(document: &Map<String, Value>) -> Result<Option<PathBuf>, SettingsError> {
    document
        .get("remoteEditor")
        .map(|value| {
            let path: PathBuf =
                serde_json::from_value(value.clone()).map_err(|_| SettingsError::Invalid)?;
            crate::remote_edit::EditorConfig::new(uuid::Uuid::nil(), path.clone())
                .map_err(|_| SettingsError::Invalid)?;
            Ok(path)
        })
        .transpose()
}

#[cfg(any(windows, test))]
fn decode_document(bytes: &[u8]) -> Result<Map<String, Value>, SettingsError> {
    if bytes.len() > MAX_SETTINGS_BYTES {
        return Err(SettingsError::Invalid);
    }
    let document: Map<String, Value> =
        serde_json::from_slice(bytes).map_err(|_| SettingsError::Invalid)?;
    if document
        .get("schemaVersion")
        .is_some_and(|value| value.as_u64() != Some(1))
    {
        return Err(SettingsError::Invalid);
    }
    Ok(document)
}

fn decode_panel(document: &Map<String, Value>) -> Result<PanelLayout, SettingsError> {
    document
        .get("sftpPanel")
        .map_or(Ok(PanelLayout::default()), |value| {
            serde_json::from_value::<PanelLayout>(value.clone())
                .map_err(|_| SettingsError::Invalid)?
                .validate()
        })
}

#[cfg(windows)]
fn storage_error(error: std::io::Error) -> SettingsError {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        SettingsError::Changed
    } else {
        SettingsError::Unavailable
    }
}

#[cfg(windows)]
struct Storage {
    destination: capstan_native_windows::transfer::Destination,
    expected: Option<capstan_native_windows::transfer::Version>,
}
#[cfg(windows)]
impl Storage {
    fn commit(mut self, bytes: &[u8]) -> Result<(), SettingsError> {
        self.destination
            .begin(&format!(
                ".capstan-settings-{}.partial",
                uuid::Uuid::new_v4()
            ))
            .map_err(storage_error)?;
        self.destination.write(0, bytes).map_err(storage_error)?;
        self.destination
            .commit(self.expected)
            .map_err(storage_error)
    }
}
#[cfg(not(windows))]
struct Storage;
#[cfg(not(windows))]
impl Storage {
    fn commit(self, _: &[u8]) -> Result<(), SettingsError> {
        Err(SettingsError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[tokio::test]
    async fn ordered_async_writes_reject_foreign_sessions_and_drain_before_exit() {
        let root = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::new(root.path().to_owned()));
        let session = settings.snapshot().unwrap().session;
        let newest = PanelLayout {
            dock: PanelDock::Bottom,
            size: 55,
            collapsed: true,
        };
        let (new, stale) = tokio::join!(
            settings
                .clone()
                .save(session, crate::sftp::ExactU64(3), newest),
            settings
                .clone()
                .save(session, crate::sftp::ExactU64(2), PanelLayout::default()),
        );
        assert_eq!(new, Ok(()));
        assert_eq!(stale, Ok(()));
        assert_eq!(settings.panel().unwrap(), newest);
        assert_eq!(
            settings
                .clone()
                .save(
                    uuid::Uuid::new_v4(),
                    crate::sftp::ExactU64(4),
                    PanelLayout::default()
                )
                .await,
            Err(SettingsError::Changed)
        );
        settings.drain().await.unwrap();
        assert_eq!(
            settings
                .clone()
                .save(session, crate::sftp::ExactU64(5), PanelLayout::default())
                .await,
            Err(SettingsError::Unavailable)
        );
        settings.resume();
        settings
            .clone()
            .save(session, crate::sftp::ExactU64(6), PanelLayout::default())
            .await
            .unwrap();
        assert_eq!(settings.panel().unwrap(), PanelLayout::default());
    }

    #[cfg(windows)]
    #[tokio::test(start_paused = true)]
    async fn coordinated_exit_timeout_keeps_preferences_writable_after_recovery() {
        let root = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::new(root.path().to_owned()));
        settings.admission.lock().unwrap().1 += 1;
        let held = PendingWrite(settings.clone());
        let pending_settings = settings.clone();
        let exit = tokio::spawn(async move {
            crate::application::prepare_file_workflow_exit(
                &pending_settings,
                &crate::transfer::TransferService::default(),
                &crate::sftp::SftpService::default(),
                "main",
            )
            .await
        });
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_secs(31)).await;
        assert!(matches!(
            exit.await.unwrap(),
            Err(crate::application::ExitError::Settings(
                SettingsError::Unavailable
            ))
        ));
        drop(held);
        settings
            .clone()
            .save(
                settings.session,
                crate::sftp::ExactU64(1),
                PanelLayout::default(),
            )
            .await
            .unwrap();
        assert_eq!(settings.panel().unwrap(), PanelLayout::default());
    }

    #[test]
    fn known_fields_are_strict_bounded_and_versioned() {
        for bytes in [
            br#"[]"#.as_slice(),
            br#"{"schemaVersion":2}"#,
            br#"{"schemaVersion":"1"}"#,
            b"not json",
        ] {
            assert_eq!(decode_document(bytes), Err(SettingsError::Invalid));
        }
        for value in [
            serde_json::json!({"dock":"left","size":36,"collapsed":false}),
            serde_json::json!({"dock":"right","size":19,"collapsed":false}),
            serde_json::json!({"dock":"right","size":36.5,"collapsed":false}),
            serde_json::json!({"dock":"right","size":36,"collapsed":false,"path":"forbidden"}),
        ] {
            let mut document = Map::new();
            document.insert("sftpPanel".into(), value);
            assert_eq!(decode_panel(&document), Err(SettingsError::Invalid));
        }
    }

    #[cfg(windows)]
    #[test]
    fn native_atomic_save_survives_reopen_and_preserves_other_ui_preferences() {
        let root = tempfile::tempdir().unwrap();
        let settings = Settings::new(root.path().to_owned());
        assert_eq!(settings.panel().unwrap(), PanelLayout::default());
        assert!(!root.path().join("settings.json").exists());
        std::fs::write(
            root.path().join("settings.json"),
            br#"{"schemaVersion":1,"font":{"size":14}}"#,
        )
        .unwrap();
        let layout = PanelLayout {
            dock: PanelDock::Bottom,
            size: 45,
            collapsed: true,
        };
        settings.save_panel(layout).unwrap();
        assert_eq!(
            Settings::new(root.path().to_owned()).panel().unwrap(),
            layout
        );
        let document =
            decode_document(&std::fs::read(root.path().join("settings.json")).unwrap()).unwrap();
        assert_eq!(document["font"], serde_json::json!({"size":14}));
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn malformed_oversize_and_changed_files_are_preserved() {
        let root = tempfile::tempdir().unwrap();
        let settings = Settings::new(root.path().to_owned());
        let path = root.path().join("settings.json");
        for bytes in [
            b"not json".to_vec(),
            br#"{"schemaVersion":2}"#.to_vec(),
            vec![b' '; MAX_SETTINGS_BYTES + 1],
        ] {
            std::fs::write(&path, &bytes).unwrap();
            assert_eq!(
                settings.save_panel(PanelLayout::default()),
                Err(SettingsError::Invalid)
            );
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        std::fs::write(&path, b"{}").unwrap();
        let (_, storage) = settings.read().unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"{\"font\":17}").unwrap();
        assert_eq!(storage.commit(b"{}"), Err(SettingsError::Changed));
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"font\":17}");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
