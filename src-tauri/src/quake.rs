//! Quake mode reuses the main webview and never owns terminal sessions.

use crate::CommandError;
use capstan_core::{
    hotkey::validate_hotkey,
    settings::{DesktopPreferences, QuakePreferences, Settings},
    VaultManager,
};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

#[cfg(windows)]
use capstan_native_windows::WindowPlacement as SavedWindow;

#[cfg(not(windows))]
#[derive(Clone, Copy)]
struct SavedWindow {
    position: PhysicalPosition<i32>,
    inner_size: PhysicalSize<u32>,
    maximized: bool,
}

#[cfg(windows)]
fn save_window(window: &WebviewWindow) -> Result<SavedWindow, CommandError> {
    let handle = window
        .hwnd()
        .map_err(|_| CommandError::new("window_operation"))?;
    capstan_native_windows::capture_window_placement(handle.0 as isize)
        .map_err(|_| CommandError::new("window_operation"))
}

#[cfg(not(windows))]
fn save_window(window: &WebviewWindow) -> Result<SavedWindow, CommandError> {
    let operation_error = |_| CommandError::new("window_operation");
    let maximized = window.is_maximized().map_err(operation_error)?;
    window.unminimize().map_err(operation_error)?;
    window.unmaximize().map_err(operation_error)?;
    Ok(SavedWindow {
        position: window.outer_position().map_err(operation_error)?,
        inner_size: window.inner_size().map_err(operation_error)?,
        maximized,
    })
}

#[derive(Default)]
struct QuakeConfiguration {
    enabled: bool,
    hotkey: Option<String>,
    shortcut_id: Option<u32>,
    height_percent: u8,
    active: bool,
    original: Option<SavedWindow>,
    error_code: Option<&'static str>,
}

#[derive(Default)]
pub struct QuakeController {
    configuration: Mutex<QuakeConfiguration>,
    updates: tokio::sync::Mutex<()>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuakeStatus {
    pub enabled: bool,
    pub error_code: Option<&'static str>,
}

impl QuakeController {
    fn configuration(&self) -> Result<QuakeConfigurationSnapshot, CommandError> {
        let configuration = self
            .configuration
            .lock()
            .map_err(|_| CommandError::new("quake_unavailable"))?;
        Ok(QuakeConfigurationSnapshot {
            enabled: configuration.enabled,
            hotkey: configuration.hotkey.clone(),
            shortcut_id: configuration.shortcut_id,
            height_percent: configuration.height_percent,
            active: configuration.active,
            error_code: configuration.error_code,
        })
    }

    fn replace(&self, configuration: QuakeConfiguration) -> Result<(), CommandError> {
        *self
            .configuration
            .lock()
            .map_err(|_| CommandError::new("quake_unavailable"))? = configuration;
        Ok(())
    }
}

#[derive(Clone)]
struct QuakeConfigurationSnapshot {
    enabled: bool,
    hotkey: Option<String>,
    shortcut_id: Option<u32>,
    height_percent: u8,
    active: bool,
    error_code: Option<&'static str>,
}

pub fn install(app: &AppHandle, settings: Arc<Settings>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let controller = app.state::<QuakeController>();
        let _update = controller.updates.lock().await;
        let read_settings = settings.clone();
        let preferences =
            tauri::async_runtime::spawn_blocking(move || read_settings.desktop_preferences()).await;
        let Ok(Ok(preferences)) = preferences else {
            return;
        };
        if !preferences.quake_enabled {
            return;
        }
        let Some(hotkey) = preferences.quake_hotkey.as_deref() else {
            return;
        };
        let Ok(shortcut) = validate_hotkey(hotkey) else {
            return;
        };
        let registration_app = app.clone();
        let registration_key = shortcut.clone();
        let registration = tauri::async_runtime::spawn_blocking(move || {
            registration_app
                .global_shortcut()
                .register(registration_key.as_str())
        })
        .await;
        let registered = matches!(registration, Ok(Ok(())));
        if registered {
            let shortcut_id = shortcut.parse::<Shortcut>().ok().map(|value| value.id());
            if let Some(controller) = app.try_state::<QuakeController>() {
                let _ = controller.replace(QuakeConfiguration {
                    enabled: true,
                    hotkey: Some(shortcut),
                    shortcut_id,
                    height_percent: preferences.quake_height_percent,
                    active: false,
                    original: None,
                    error_code: None,
                });
            }
            return;
        }

        if let Some(controller) = app.try_state::<QuakeController>() {
            if let Ok(mut current) = controller.configuration.lock() {
                current.enabled = false;
                current.shortcut_id = None;
                current.error_code = Some("quake_hotkey_busy");
            }
        }
        let _ = settings
            .save_quake_preferences(QuakePreferences {
                enabled: false,
                hotkey: preferences.quake_hotkey,
                height_percent: preferences.quake_height_percent,
            })
            .await;
    });
}

pub fn handle_shortcut(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state != ShortcutState::Pressed {
        return;
    }
    let is_current = app
        .try_state::<QuakeController>()
        .and_then(|controller| controller.configuration().ok())
        .is_some_and(|configuration| {
            configuration.enabled && configuration.shortcut_id == Some(shortcut.id())
        });
    if is_current {
        toggle_window(app);
    }
}

fn window_for(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

fn cursor_monitor(window: &WebviewWindow) -> Option<tauri::Monitor> {
    let cursor = window.cursor_position().ok()?;
    window
        .available_monitors()
        .ok()?
        .into_iter()
        .find(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            cursor.x >= f64::from(position.x)
                && cursor.y >= f64::from(position.y)
                && cursor.x < f64::from(position.x) + f64::from(size.width)
                && cursor.y < f64::from(position.y) + f64::from(size.height)
        })
        .or_else(|| window.current_monitor().ok().flatten())
}

fn quake_size(available_height: u32, height_percent: u8) -> u32 {
    let percent = u32::from(height_percent.clamp(20, 90));
    available_height.saturating_mul(percent).saturating_add(50) / 100
}

fn position_quake(window: &WebviewWindow, height_percent: u8) -> Result<(), CommandError> {
    let monitor = cursor_monitor(window).ok_or_else(|| CommandError::new("window_monitor"))?;
    let area = monitor.work_area();
    window
        .unminimize()
        .map_err(|_| CommandError::new("window_operation"))?;
    window
        .unmaximize()
        .map_err(|_| CommandError::new("window_operation"))?;
    window
        .set_always_on_top(true)
        .map_err(|_| CommandError::new("window_operation"))?;
    window
        .set_position(tauri::Position::Physical(PhysicalPosition::new(
            area.position.x,
            area.position.y,
        )))
        .map_err(|_| CommandError::new("window_operation"))?;
    window
        .set_size(tauri::Size::Physical(PhysicalSize::new(
            area.size.width,
            quake_size(area.size.height, height_percent),
        )))
        .map_err(|_| CommandError::new("window_operation"))?;
    Ok(())
}

fn sync_status_visibility(window: &WebviewWindow, visible: bool) {
    if let Some(vault) = window.try_state::<Arc<VaultManager>>() {
        vault.set_server_status_window_visible(visible && !window.is_minimized().unwrap_or(true));
    }
}

fn toggle_window(app: &AppHandle) {
    let toggle_app = app.clone();
    // Keep capture, native restoration and Tauri setters ordered on the window's owning thread.
    let _ = app.run_on_main_thread(move || toggle_window_on_main_thread(&toggle_app));
}

fn toggle_window_on_main_thread(app: &AppHandle) {
    let Some(window) = window_for(app) else {
        return;
    };
    let Some(controller) = app.try_state::<QuakeController>() else {
        return;
    };
    let Ok(mut configuration) = controller.configuration.lock() else {
        return;
    };
    if !configuration.enabled {
        return;
    }
    if !configuration.active {
        let Ok(saved) = save_window(&window) else {
            return;
        };
        configuration.original = Some(saved);
        configuration.active = true;
    }
    let height = configuration.height_percent;
    drop(configuration);
    if window.is_focused().unwrap_or(false) && window.is_visible().unwrap_or(false) {
        if window.hide().is_ok() {
            sync_status_visibility(&window, false);
            let _ = window.emit("quake-visibility-changed", false);
        }
        return;
    }
    if position_quake(&window, height).is_ok() {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        sync_status_visibility(&window, true);
        let _ = window.emit("quake-visibility-changed", true);
    }
}

fn restore_window(app: &AppHandle, saved: Option<SavedWindow>) {
    let Some(window) = window_for(app) else {
        return;
    };
    let Some(saved) = saved else {
        return;
    };
    let restore_window = window.clone();
    let _ =
        window.run_on_main_thread(move || restore_window_on_main_thread(&restore_window, saved));
}

fn restore_window_on_main_thread(window: &WebviewWindow, saved: SavedWindow) {
    let _ = window.set_always_on_top(false);
    #[cfg(windows)]
    {
        let Ok(handle) = window.hwnd() else {
            return;
        };
        if capstan_native_windows::restore_window_placement(handle.0 as isize, saved).is_err() {
            return;
        }
    }
    #[cfg(not(windows))]
    {
        let _ = window.unminimize();
        let _ = window.unmaximize();
        let _ = window.set_position(tauri::Position::Physical(saved.position));
        let _ = window.set_size(tauri::Size::Physical(saved.inner_size));
        if saved.maximized {
            let _ = window.maximize();
        }
    }
    let _ = window.show();
    let _ = window.set_focus();
    sync_status_visibility(window, true);
    let _ = window.emit("quake-visibility-changed", true);
}

pub(crate) fn show_for_launch(window: &WebviewWindow) {
    let show_window = window.clone();
    let _ = window.run_on_main_thread(move || show_for_launch_on_main_thread(&show_window));
}

fn show_for_launch_on_main_thread(window: &WebviewWindow) {
    if let Some(configuration) = window
        .try_state::<QuakeController>()
        .and_then(|controller| controller.configuration().ok())
    {
        if configuration.enabled && configuration.active {
            let _ = position_quake(window, configuration.height_percent);
        }
    }
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    sync_status_visibility(window, true);
    let _ = window.emit("quake-visibility-changed", true);
}

fn source(window: &WebviewWindow) -> Result<(), CommandError> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err(CommandError::new("window_source"))
    }
}

#[tauri::command]
pub fn quake_status(
    window: WebviewWindow,
    controller: State<'_, QuakeController>,
) -> Result<QuakeStatus, CommandError> {
    source(&window)?;
    let configuration = controller.configuration()?;
    Ok(QuakeStatus {
        enabled: configuration.enabled,
        error_code: configuration.error_code,
    })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri injects each managed service separately.
pub async fn save_quake_preferences(
    window: WebviewWindow,
    app: AppHandle,
    settings: State<'_, Arc<Settings>>,
    controller: State<'_, QuakeController>,
    enabled: bool,
    hotkey: Option<String>,
    height_percent: u8,
) -> Result<DesktopPreferences, CommandError> {
    source(&window)?;
    let _update = controller.updates.lock().await;
    if !(20..=90).contains(&height_percent) || (enabled && hotkey.is_none()) {
        return Err(CommandError::new("settings_invalid"));
    }
    let normalized = hotkey
        .as_deref()
        .map(validate_hotkey)
        .transpose()
        .map_err(|_| CommandError::new("settings_invalid"))?;
    let current = controller.configuration()?;
    let old_shortcut = current.hotkey.clone().filter(|_| current.enabled);
    let new_shortcut = if enabled { normalized.clone() } else { None };
    let new_shortcut_id = new_shortcut
        .as_deref()
        .map(|key| key.parse::<Shortcut>())
        .transpose()
        .map_err(|_| CommandError::new("settings_invalid"))?
        .map(|key| key.id());
    let persistence = async {
        settings
            .inner()
            .clone()
            .save_quake_preferences(QuakePreferences {
                enabled,
                hotkey: normalized.clone(),
                height_percent,
            })
            .await
            .map_err(|error| CommandError::new(error.to_string()))
    };
    let register_app = app.clone();
    let unregister_app = app.clone();
    let preferences = apply_shortcut_change(
        old_shortcut,
        new_shortcut,
        move |key| {
            let app = register_app.clone();
            async move {
                let result = tauri::async_runtime::spawn_blocking(move || {
                    app.global_shortcut().register(key.as_str())
                })
                .await;
                if matches!(result, Ok(Ok(()))) {
                    Ok(())
                } else {
                    Err(CommandError::new("quake_hotkey_busy"))
                }
            }
        },
        move |key| {
            let app = unregister_app.clone();
            async move {
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    app.global_shortcut().unregister(key.as_str())
                })
                .await;
            }
        },
        persistence,
    )
    .await;
    let preferences = match preferences {
        Ok(preferences) => preferences,
        Err(error) => {
            if error.code == "quake_hotkey_busy" {
                if let Ok(mut current) = controller.configuration.lock() {
                    current.error_code = Some("quake_hotkey_busy");
                }
            }
            return Err(error);
        }
    };

    // A hotkey can be pressed while registration/persistence is in flight.
    let (active, original) = {
        let mut latest = controller
            .configuration
            .lock()
            .map_err(|_| CommandError::new("quake_unavailable"))?;
        let active = latest.active;
        let original = latest.original;
        *latest = QuakeConfiguration {
            enabled,
            hotkey: normalized.clone(),
            shortcut_id: if enabled { new_shortcut_id } else { None },
            height_percent,
            active: enabled && active,
            original: if enabled { original } else { None },
            error_code: None,
        };
        (active, original)
    };

    if enabled && active {
        let position_window = window.clone();
        let _ = window.run_on_main_thread(move || {
            let _ = position_quake(&position_window, height_percent);
        });
    } else if !enabled {
        restore_window(&app, original);
    }
    Ok(preferences)
}

async fn apply_shortcut_change<T, Register, Unregister>(
    old: Option<String>,
    new: Option<String>,
    register: impl Fn(String) -> Register,
    unregister: impl Fn(String) -> Unregister,
    persist: impl std::future::Future<Output = Result<T, CommandError>>,
) -> Result<T, CommandError>
where
    Register: std::future::Future<Output = Result<(), CommandError>>,
    Unregister: std::future::Future<Output = ()>,
{
    let changed = new != old;
    if changed {
        if let Some(key) = &new {
            register(key.clone()).await?;
        }
    }
    match persist.await {
        Ok(saved) => {
            if changed {
                if let Some(key) = old {
                    unregister(key).await;
                }
            }
            Ok(saved)
        }
        Err(error) => {
            if changed {
                if let Some(key) = new {
                    unregister(key).await;
                }
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::quake_size;

    #[test]
    fn quake_height_is_bounded_to_the_work_area() {
        assert_eq!(quake_size(1000, 20), 200);
        assert_eq!(quake_size(1000, 50), 500);
        assert_eq!(quake_size(1000, 90), 900);
        assert_eq!(quake_size(1000, 100), 900);
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../docs/agent/fixtures/M4-contract.json"))
                .unwrap();
        assert_eq!(
            serde_json::to_value(super::QuakeStatus {
                enabled: false,
                error_code: None
            })
            .unwrap(),
            fixture["quakeStatus"]
        );
        assert_eq!(
            serde_json::to_value(super::QuakePreferences::default()).unwrap(),
            fixture["quakePreferences"]
        );
    }

    #[tokio::test]
    async fn shortcut_swap_registers_before_saving_and_rolls_back_on_failure() {
        use super::{apply_shortcut_change, CommandError};
        use std::{cell::RefCell, rc::Rc};
        for (registration_ok, persistence_ok, expected) in [
            (false, true, vec!["register:new"]),
            (
                true,
                false,
                vec!["register:new", "persist", "unregister:new"],
            ),
            (
                true,
                true,
                vec!["register:new", "persist", "unregister:old"],
            ),
        ] {
            let events = Rc::new(RefCell::new(Vec::<String>::new()));
            let registrations = events.clone();
            let unregistrations = events.clone();
            let writes = events.clone();
            let result = apply_shortcut_change(
                Some("old".into()),
                Some("new".into()),
                move |key| {
                    registrations.borrow_mut().push(format!("register:{key}"));
                    std::future::ready(if registration_ok {
                        Ok(())
                    } else {
                        Err(CommandError::new("quake_hotkey_busy"))
                    })
                },
                move |key| {
                    unregistrations
                        .borrow_mut()
                        .push(format!("unregister:{key}"));
                    std::future::ready(())
                },
                async move {
                    writes.borrow_mut().push("persist".into());
                    if persistence_ok {
                        Ok(())
                    } else {
                        Err(CommandError::new("settings_unavailable"))
                    }
                },
            )
            .await;
            assert_eq!(*events.borrow(), expected);
            assert_eq!(result.is_ok(), registration_ok && persistence_ok);
        }
    }

    #[test]
    fn normalized_policy_keys_are_accepted_by_the_actual_shortcut_parser() {
        for value in [
            "Win+J",
            "Ctrl+Alt+Space",
            "Alt+Backquote",
            "Ctrl+Shift+C",
            "Ctrl+F24",
            "Alt+ArrowUp",
        ] {
            assert!(
                super::validate_hotkey(value)
                    .unwrap()
                    .parse::<super::Shortcut>()
                    .is_ok(),
                "{value}"
            );
        }
    }
}
