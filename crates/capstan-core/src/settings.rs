//! Nonsecret UI preferences at the Rust-selected application data root.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
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
    system_notifications: AtomicBool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DesktopPreferences {
    pub copy_on_select: bool,
    pub system_notifications: bool,
    pub jump_list_recent_hosts: bool,
    pub quake_enabled: bool,
    pub quake_hotkey: Option<String>,
    pub quake_height_percent: u8,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DesktopPreference {
    CopyOnSelect,
    SystemNotifications,
    JumpListRecentHosts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuakePreferences {
    pub enabled: bool,
    pub hotkey: Option<String>,
    pub height_percent: u8,
}

impl Default for QuakePreferences {
    fn default() -> Self {
        Self {
            enabled: false,
            hotkey: None,
            height_percent: 50,
        }
    }
}

impl QuakePreferences {
    fn validate(mut self) -> Result<Self, SettingsError> {
        if !(20..=90).contains(&self.height_percent) {
            return Err(SettingsError::Invalid);
        }
        self.hotkey = self
            .hotkey
            .as_deref()
            .map(crate::hotkey::validate_hotkey)
            .transpose()
            .map_err(|_| SettingsError::Invalid)?;
        if self.enabled && self.hotkey.is_none() {
            return Err(SettingsError::Invalid);
        }
        Ok(self)
    }
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
            system_notifications: AtomicBool::new(false),
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
        let pending = self.admit()?;
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

    fn admit(self: &Arc<Self>) -> Result<PendingWrite, SettingsError> {
        let mut admission = self
            .admission
            .lock()
            .map_err(|_| SettingsError::Unavailable)?;
        if admission.0 || admission.1 >= 32 {
            return Err(SettingsError::Unavailable);
        }
        admission.1 += 1;
        Ok(PendingWrite(self.clone()))
    }

    pub fn desktop_preferences(&self) -> Result<DesktopPreferences, SettingsError> {
        let _guard = self.gate.lock().map_err(|_| SettingsError::Unavailable)?;
        let (document, _) = self.read()?;
        let preferences = decode_desktop(&document)?;
        self.system_notifications
            .store(preferences.system_notifications, Ordering::Relaxed);
        Ok(preferences)
    }

    pub fn check_updates_on_startup(&self) -> Result<bool, SettingsError> {
        let _guard = self.gate.lock().map_err(|_| SettingsError::Unavailable)?;
        let (document, _) = self.read()?;
        document
            .get("checkUpdatesOnStartup")
            .map_or(Ok(false), |value| {
                value.as_bool().ok_or(SettingsError::Invalid)
            })
    }

    pub async fn save_update_preference(
        self: Arc<Self>,
        enabled: bool,
    ) -> Result<(), SettingsError> {
        let pending = self.admit()?;
        tokio::task::spawn_blocking(move || {
            let _pending = pending;
            let _guard = self.gate.lock().map_err(|_| SettingsError::Unavailable)?;
            let (mut document, storage) = self.read()?;
            document.insert("schemaVersion".into(), Value::from(1));
            document.insert("checkUpdatesOnStartup".into(), Value::from(enabled));
            commit_document(document, storage)
        })
        .await
        .map_err(|_| SettingsError::Unavailable)?
    }

    pub fn system_notifications_enabled(&self) -> bool {
        self.system_notifications.load(Ordering::Relaxed)
    }

    pub async fn save_desktop_preference(
        self: Arc<Self>,
        preference: DesktopPreference,
        value: bool,
    ) -> Result<DesktopPreferences, SettingsError> {
        let pending = self.admit()?;
        tokio::task::spawn_blocking(move || {
            let _pending = pending;
            let _guard = self.gate.lock().map_err(|_| SettingsError::Unavailable)?;
            let (mut document, storage) = self.read()?;
            decode_desktop(&document)?;
            document.insert("schemaVersion".into(), Value::from(1));
            document.insert(
                match preference {
                    DesktopPreference::CopyOnSelect => "copyOnSelect",
                    DesktopPreference::SystemNotifications => "systemNotifications",
                    DesktopPreference::JumpListRecentHosts => "jumpListRecentHosts",
                }
                .into(),
                Value::from(value),
            );
            if matches!(preference, DesktopPreference::JumpListRecentHosts) && !value {
                document.remove("recentHostIds");
            }
            let preferences = decode_desktop(&document)?;
            commit_document(document, storage)?;
            self.system_notifications
                .store(preferences.system_notifications, Ordering::Relaxed);
            Ok(preferences)
        })
        .await
        .map_err(|_| SettingsError::Unavailable)?
    }

    pub async fn save_quake_preferences(
        self: Arc<Self>,
        preferences: QuakePreferences,
    ) -> Result<DesktopPreferences, SettingsError> {
        let preferences = preferences.validate()?;
        let pending = self.admit()?;
        tokio::task::spawn_blocking(move || {
            let _pending = pending;
            let _guard = self.gate.lock().map_err(|_| SettingsError::Unavailable)?;
            let (mut document, storage) = self.read()?;
            decode_desktop(&document)?;
            document.insert("schemaVersion".into(), Value::from(1));
            document.insert("quakeEnabled".into(), Value::from(preferences.enabled));
            document.insert(
                "quakeHotkey".into(),
                serde_json::to_value(preferences.hotkey).map_err(|_| SettingsError::Invalid)?,
            );
            document.insert(
                "quakeHeightPercent".into(),
                Value::from(preferences.height_percent),
            );
            let desktop = decode_desktop(&document)?;
            commit_document(document, storage)?;
            Ok(desktop)
        })
        .await
        .map_err(|_| SettingsError::Unavailable)?
    }

    pub fn recent_hosts(&self) -> Result<Vec<uuid::Uuid>, SettingsError> {
        let _guard = self.gate.lock().map_err(|_| SettingsError::Unavailable)?;
        let (document, _) = self.read()?;
        if !decode_desktop(&document)?.jump_list_recent_hosts {
            return Ok(Vec::new());
        }
        decode_recent_hosts(&document)
    }

    pub async fn update_recent_hosts(
        self: Arc<Self>,
        change: crate::launch::recent::HistoryChange,
    ) -> Result<(), SettingsError> {
        let pending = self.admit()?;
        tokio::task::spawn_blocking(move || {
            use crate::launch::recent::{record, HistoryChange};
            let _pending = pending;
            let _guard = self.gate.lock().map_err(|_| SettingsError::Unavailable)?;
            let (mut document, storage) = self.read()?;
            let enabled = decode_desktop(&document)?.jump_list_recent_hosts;
            let mut history = if matches!(change, HistoryChange::Clear) {
                Vec::new()
            } else {
                decode_recent_hosts(&document)?
            };
            let previous = document.get("recentHostIds").cloned();
            match change {
                HistoryChange::Connected(id) if enabled => record(&mut history, id),
                HistoryChange::Connected(_) => return Ok(()),
                HistoryChange::Remove(id) => history.retain(|existing| *existing != id),
                HistoryChange::Clear => history.clear(),
            }
            let next = if history.is_empty() {
                None
            } else {
                Some(serde_json::to_value(history).map_err(|_| SettingsError::Invalid)?)
            };
            if previous == next {
                return Ok(());
            }
            document.insert("schemaVersion".into(), Value::from(1));
            if let Some(next) = next {
                document.insert("recentHostIds".into(), next);
            } else {
                document.remove("recentHostIds");
            }
            commit_document(document, storage)
        })
        .await
        .map_err(|_| SettingsError::Unavailable)?
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
        let pending = self.admit()?;
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

fn decode_desktop(document: &Map<String, Value>) -> Result<DesktopPreferences, SettingsError> {
    let setting = |key, default| {
        document.get(key).map_or(Ok(default), |value| {
            value.as_bool().ok_or(SettingsError::Invalid)
        })
    };
    let quake = decode_quake(document)?;
    Ok(DesktopPreferences {
        copy_on_select: setting("copyOnSelect", false)?,
        system_notifications: setting("systemNotifications", true)?,
        jump_list_recent_hosts: setting("jumpListRecentHosts", false)?,
        quake_enabled: quake.enabled,
        quake_hotkey: quake.hotkey,
        quake_height_percent: quake.height_percent,
    })
}

fn decode_quake(document: &Map<String, Value>) -> Result<QuakePreferences, SettingsError> {
    let preferences = QuakePreferences {
        enabled: document.get("quakeEnabled").map_or(Ok(false), |value| {
            value.as_bool().ok_or(SettingsError::Invalid)
        })?,
        hotkey: match document.get("quakeHotkey") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) => Some(value.clone()),
            Some(_) => return Err(SettingsError::Invalid),
        },
        height_percent: document.get("quakeHeightPercent").map_or(Ok(50), |value| {
            value
                .as_u64()
                .and_then(|value| u8::try_from(value).ok())
                .ok_or(SettingsError::Invalid)
        })?,
    };
    preferences.validate()
}

fn decode_recent_hosts(document: &Map<String, Value>) -> Result<Vec<uuid::Uuid>, SettingsError> {
    let Some(value) = document.get("recentHostIds") else {
        return Ok(Vec::new());
    };
    let history: Vec<uuid::Uuid> =
        serde_json::from_value(value.clone()).map_err(|_| SettingsError::Invalid)?;
    if history.len() > crate::launch::recent::MAX_RECENT_HOSTS
        || history.iter().any(uuid::Uuid::is_nil)
        || history
            .iter()
            .enumerate()
            .any(|(index, id)| history[..index].contains(id))
    {
        return Err(SettingsError::Invalid);
    }
    Ok(history)
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

    #[test]
    fn desktop_defaults_match_shared_fixture_and_reject_non_booleans() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../docs/agent/fixtures/M4-contract.json"
        ))
        .unwrap();
        let defaults = serde_json::to_value(decode_desktop(&Map::new()).unwrap()).unwrap();
        for (key, value) in defaults.as_object().unwrap() {
            assert_eq!(value, &fixture["settingsDefaults"][key]);
            if key == "quakeHotkey" && value.is_null() {
                continue;
            }
            assert_eq!(
                decode_desktop(&Map::from_iter([(key.clone(), Value::Null)])),
                Err(SettingsError::Invalid)
            );
        }
    }

    #[test]
    fn quake_preferences_enforce_hotkey_and_height_policy() {
        assert_eq!(decode_quake(&Map::new()), Ok(QuakePreferences::default()));
        assert_eq!(
            QuakePreferences {
                enabled: true,
                hotkey: None,
                height_percent: 50,
            }
            .validate(),
            Err(SettingsError::Invalid)
        );
        for invalid in ["A", "Shift+A", "Ctrl+A", "Ctrl+Ctrl+F1"] {
            assert_eq!(
                QuakePreferences {
                    enabled: false,
                    hotkey: Some(invalid.into()),
                    height_percent: 50,
                }
                .validate(),
                Err(SettingsError::Invalid)
            );
        }
        for height_percent in [19, 91] {
            assert_eq!(
                QuakePreferences {
                    enabled: false,
                    hotkey: None,
                    height_percent,
                }
                .validate(),
                Err(SettingsError::Invalid)
            );
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn desktop_writes_preserve_existing_and_unknown_settings() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        std::fs::write(
            &path,
            br#"{"schemaVersion":1,"quakeEnabled":true,"quakeHotkey":"Ctrl+Alt+F12"}"#,
        )
        .unwrap();
        let settings = Arc::new(Settings::new(root.path().to_owned()));
        assert!(settings.desktop_preferences().unwrap().system_notifications);
        assert!(settings.system_notifications_enabled());
        settings
            .clone()
            .save_desktop_preference(DesktopPreference::CopyOnSelect, true)
            .await
            .unwrap();
        settings
            .clone()
            .save_desktop_preference(DesktopPreference::SystemNotifications, false)
            .await
            .unwrap();
        assert!(!settings.system_notifications_enabled());
        let saved = decode_document(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(saved["quakeEnabled"], true);
        assert_eq!(saved["schemaVersion"], 1);
        assert_eq!(saved["copyOnSelect"], true);
        assert_eq!(saved["systemNotifications"], false);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn recent_history_is_opt_in_bounded_and_cleared_without_changing_other_preferences() {
        use crate::launch::recent::HistoryChange;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        let original =
            br#"{"schemaVersion":1,"unknownSetting":{"keep":true},"quakeHeightPercent":70}"#;
        std::fs::write(&path, original).unwrap();
        let settings = Arc::new(Settings::new(root.path().to_owned()));
        settings
            .clone()
            .update_recent_hosts(HistoryChange::Connected(uuid::Uuid::new_v4()))
            .await
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original);
        settings
            .clone()
            .save_desktop_preference(DesktopPreference::JumpListRecentHosts, true)
            .await
            .unwrap();
        for id in 1..=12 {
            settings
                .clone()
                .update_recent_hosts(HistoryChange::Connected(uuid::Uuid::from_u128(id)))
                .await
                .unwrap();
        }
        settings
            .clone()
            .update_recent_hosts(HistoryChange::Connected(uuid::Uuid::from_u128(7)))
            .await
            .unwrap();
        let history = settings.recent_hosts().unwrap();
        assert_eq!(history.len(), 10);
        assert_eq!(history[0], uuid::Uuid::from_u128(7));
        let reopened = Settings::new(root.path().to_owned());
        assert_eq!(reopened.recent_hosts().unwrap(), history);
        settings
            .clone()
            .update_recent_hosts(HistoryChange::Remove(uuid::Uuid::from_u128(7)))
            .await
            .unwrap();
        assert!(!settings
            .recent_hosts()
            .unwrap()
            .contains(&uuid::Uuid::from_u128(7)));
        settings
            .clone()
            .update_recent_hosts(HistoryChange::Clear)
            .await
            .unwrap();
        assert!(settings.recent_hosts().unwrap().is_empty());
        settings
            .clone()
            .update_recent_hosts(HistoryChange::Connected(uuid::Uuid::from_u128(6)))
            .await
            .unwrap();
        settings
            .clone()
            .save_desktop_preference(DesktopPreference::JumpListRecentHosts, false)
            .await
            .unwrap();
        let saved = decode_document(&std::fs::read(&path).unwrap()).unwrap();
        assert!(!saved.contains_key("recentHostIds"));
        assert_eq!(saved["quakeHeightPercent"], 70);
        assert_eq!(saved["unknownSetting"]["keep"], true);
        settings
            .clone()
            .save_desktop_preference(DesktopPreference::JumpListRecentHosts, true)
            .await
            .unwrap();
        assert!(settings.recent_hosts().unwrap().is_empty());
    }

    #[test]
    fn persisted_recent_history_rejects_duplicate_nil_and_excessive_ids() {
        let id = uuid::Uuid::from_u128(1);
        for invalid in [
            serde_json::json!([id, id]),
            serde_json::json!([uuid::Uuid::nil()]),
            serde_json::json!(["not-id"]),
            serde_json::json!((1..=11).map(uuid::Uuid::from_u128).collect::<Vec<_>>()),
        ] {
            assert_eq!(
                decode_recent_hosts(&Map::from_iter([("recentHostIds".into(), invalid)])),
                Err(SettingsError::Invalid)
            );
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn quake_settings_are_atomic_and_invalid_edits_leave_the_previous_shortcut_intact() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        let settings = Arc::new(Settings::new(root.path().to_owned()));
        let saved = settings
            .clone()
            .save_quake_preferences(QuakePreferences {
                enabled: true,
                hotkey: Some("ctrl+alt+space".into()),
                height_percent: 20,
            })
            .await
            .unwrap();
        assert!(saved.quake_enabled);
        assert_eq!(saved.quake_hotkey.as_deref(), Some("CTRL+ALT+SPACE"));
        let original = std::fs::read(&path).unwrap();
        for (key, height) in [
            (Some("Ctrl+C".into()), 50),
            (None, 50),
            (Some("Ctrl+Alt+F12".into()), 91),
        ] {
            assert_eq!(
                settings
                    .clone()
                    .save_quake_preferences(QuakePreferences {
                        enabled: true,
                        hotkey: key,
                        height_percent: height
                    })
                    .await,
                Err(SettingsError::Invalid)
            );
            assert_eq!(std::fs::read(&path).unwrap(), original);
        }
        assert_eq!(
            Settings::new(root.path().to_owned())
                .desktop_preferences()
                .unwrap(),
            saved
        );
    }

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
