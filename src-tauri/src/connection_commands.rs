//! Connection commands and native signals. Recovery policy lives in core.
use crate::CommandError;
use capstan_core::{sftp::ExactU64, VaultManager};
use capstan_ssh::{
    connection_state::{ConnectionSnapshot, CONNECTION_STATE_CHANGED_EVENT},
    TrustBroker,
};
use std::sync::Arc;
use tauri::{Emitter, Manager, State};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use uuid::Uuid;

#[derive(Default)]
struct EventTasks {
    stop: CancellationToken,
    tasks: TaskTracker,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ConnectionTarget {
    pub connection_id: Uuid,
    pub expected_generation: ExactU64,
}

fn source(label: &str) -> Result<(), CommandError> {
    if label == "main" {
        Ok(())
    } else {
        Err(CommandError::new("window_source"))
    }
}

pub async fn shutdown(window: &tauri::Window) {
    #[cfg(windows)]
    {
        let cleanup_window = window.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        if window
            .run_on_main_thread(move || {
                if let Some(registration) = cleanup_window.try_state::<native::Registration>() {
                    registration.unregister();
                }
                let _ = sender.send(());
            })
            .is_ok()
        {
            let _ = receiver.await;
        }
    }
    if let Some(events) = window.try_state::<Arc<EventTasks>>() {
        events.stop.cancel();
        events.tasks.close();
        events.tasks.wait().await;
    }
    window.state::<Arc<VaultManager>>().disconnect_ssh().await;
}

#[tauri::command]
pub fn connection_snapshots(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
) -> Result<Vec<ConnectionSnapshot>, CommandError> {
    source(window.label())?;
    vault.connection_snapshots().map_err(Into::into)
}

#[tauri::command]
pub async fn reconnect_connection(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
    broker: State<'_, Arc<TrustBroker>>,
    connection_id: Uuid,
    expected_generation: ExactU64,
    attempt_id: Uuid,
) -> Result<ConnectionSnapshot, CommandError> {
    source(window.label())?;
    vault
        .reconnect_connection(
            connection_id,
            expected_generation.0,
            attempt_id,
            broker.inner().clone(),
        )
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn disconnect_connection(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
    connection_id: Uuid,
    expected_generation: ExactU64,
) -> Result<ConnectionSnapshot, CommandError> {
    source(window.label())?;
    vault
        .disconnect_connection(connection_id, expected_generation.0)
        .await
        .map_err(Into::into)
}

pub fn install(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let vault = app.state::<Arc<VaultManager>>().inner().clone();
    let transfers = app
        .state::<Arc<capstan_core::transfer::TransferService>>()
        .inner()
        .clone();
    vault.configure_connection_recovery(&transfers)?;
    let mut events = vault.subscribe_connections();
    let lifecycle = Arc::new(EventTasks::default());
    app.manage(lifecycle.clone());
    if let Some(window) = app.get_webview_window("main") {
        let event_window = window.clone();
        let stop_events = lifecycle.stop.clone();
        lifecycle.tasks.spawn(async move {
            loop {
                let event = tokio::select! {
                    biased;
                    () = stop_events.cancelled() => break,
                    event = events.recv() => event,
                };
                match event {
                    Ok(snapshot) => {
                        if event_window
                            .emit(CONNECTION_STATE_CHANGED_EVENT, snapshot)
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if let Ok(snapshots) = vault.connection_snapshots() {
                            for snapshot in snapshots {
                                let _ = event_window.emit(CONNECTION_STATE_CHANGED_EVENT, snapshot);
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        #[cfg(windows)]
        {
            let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
            let registration = native::Registration::install(window.hwnd()?.0 as isize, sender)?;
            app.manage(registration);
            let signal_vault = app.state::<Arc<VaultManager>>().inner().clone();
            let stop_signals = lifecycle.stop.clone();
            lifecycle.tasks.spawn(async move {
                loop {
                    tokio::select! {
                        biased;
                        () = stop_signals.cancelled() => break,
                        signal = receiver.recv() => { if signal.is_none() { break; } },
                    }
                    // Native notifications often arrive in bursts after resume.
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    while receiver.try_recv().is_ok() {}
                    signal_vault.probe_connections();
                }
            });
        }
    }
    Ok(())
}

#[cfg(windows)]
#[allow(unsafe_code)] // Narrow Win32 registration boundary; no domain work in callbacks.
pub mod native {
    use std::{ffi::c_void, io, sync::Mutex};
    use tokio::sync::mpsc::Sender;
    const SUBCLASS_ID: usize = 0x43415053;
    type Subclass = unsafe extern "system" fn(isize, u32, usize, isize, usize, usize) -> isize;
    #[link(name = "comctl32")]
    unsafe extern "system" {
        fn SetWindowSubclass(
            hwnd: isize,
            callback: Option<Subclass>,
            id: usize,
            data: usize,
        ) -> i32;
        fn RemoveWindowSubclass(hwnd: isize, callback: Option<Subclass>, id: usize) -> i32;
        fn DefSubclassProc(hwnd: isize, message: u32, wparam: usize, lparam: isize) -> isize;
    }
    #[link(name = "iphlpapi")]
    unsafe extern "system" {
        fn NotifyIpInterfaceChange(
            family: u16,
            callback: Option<unsafe extern "system" fn(*mut c_void, *mut c_void, i32)>,
            context: *mut c_void,
            initial: u8,
            handle: *mut isize,
        ) -> u32;
        fn CancelMibChangeNotify2(handle: isize) -> u32;
    }
    pub struct Registration(Mutex<Option<OwnedRegistration>>);
    struct OwnedRegistration {
        hwnd: isize,
        network: isize,
        // Callbacks borrow the stable Box until both registrations are removed.
        context: Box<Sender<()>>,
        thread: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThreadId() -> u32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn IsWindow(hwnd: isize) -> i32;
    }
    unsafe extern "system" fn window_signal(
        hwnd: isize,
        message: u32,
        wparam: usize,
        lparam: isize,
        _: usize,
        data: usize,
    ) -> isize {
        if message == 0x218 && matches!(wparam, 7 | 18) {
            let sender = unsafe { &*(data as *const Sender<()>) };
            let _ = sender.try_send(());
        }
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }
    unsafe extern "system" fn network_signal(context: *mut c_void, _: *mut c_void, _: i32) {
        let sender = unsafe { &*context.cast::<Sender<()>>() };
        let _ = sender.try_send(());
    }
    impl Registration {
        /// Called on the window thread. The owner must unregister on that thread.
        pub fn install(hwnd: isize, sender: Sender<()>) -> io::Result<Self> {
            let context = Box::new(sender);
            let data = (&*context as *const Sender<()>) as usize;
            if unsafe { SetWindowSubclass(hwnd, Some(window_signal), SUBCLASS_ID, data) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut network = 0;
            let status = unsafe {
                NotifyIpInterfaceChange(
                    0,
                    Some(network_signal),
                    data as *mut c_void,
                    0,
                    &mut network,
                )
            };
            if status != 0 {
                unsafe {
                    RemoveWindowSubclass(hwnd, Some(window_signal), SUBCLASS_ID);
                }
                return Err(io::Error::from_raw_os_error(status as i32));
            }
            Ok(Self(Mutex::new(Some(OwnedRegistration {
                hwnd,
                network,
                context,
                thread: unsafe { GetCurrentThreadId() },
            }))))
        }
        /// CancelMibChangeNotify2 waits for callbacks; never call inside a callback.
        pub fn unregister(&self) {
            if let Ok(mut owned) = self.0.lock() {
                if owned.as_ref().is_some_and(|registration| {
                    registration.thread != unsafe { GetCurrentThreadId() }
                }) {
                    return;
                }
                if let Some(registration) = owned.take() {
                    let removed = unsafe {
                        RemoveWindowSubclass(registration.hwnd, Some(window_signal), SUBCLASS_ID)
                    };
                    let status = unsafe { CancelMibChangeNotify2(registration.network) };
                    if (removed == 0 && unsafe { IsWindow(registration.hwnd) } != 0) || status != 0
                    {
                        // A failed OS unregister must never free borrowed callback data.
                        let _ = Box::leak(registration.context);
                    } else {
                        drop(registration.context);
                    }
                }
            }
        }
    }
    impl Drop for Registration {
        fn drop(&mut self) {
            self.unregister();
            // App teardown may run away from the HWND thread. Preserve any
            // still-registered callback data rather than freeing a borrowed Box.
            if let Ok(owned) = self.0.get_mut() {
                if let Some(registration) = owned.take() {
                    let _ = Box::leak(registration.context);
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[link(name = "user32")]
        unsafe extern "system" {
            fn CreateWindowExW(
                style: u32,
                class: *const u16,
                title: *const u16,
                window_style: u32,
                x: i32,
                y: i32,
                width: i32,
                height: i32,
                parent: isize,
                menu: isize,
                instance: isize,
                parameter: *const c_void,
            ) -> isize;
            fn SendMessageW(hwnd: isize, message: u32, wparam: usize, lparam: isize) -> isize;
            fn DestroyWindow(hwnd: isize) -> i32;
        }
        #[test]
        fn native_notifications_coalesce_and_unregister_before_context_release() {
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    class.as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    std::ptr::null(),
                )
            };
            assert_ne!(hwnd, 0);
            let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
            let registration = Registration::install(hwnd, sender).unwrap();
            unsafe {
                SendMessageW(hwnd, 0x218, 4, 0); // Suspend is not resume.
            }
            assert!(receiver.try_recv().is_err());
            unsafe {
                SendMessageW(hwnd, 0x218, 7, 0);
                SendMessageW(hwnd, 0x218, 18, 0);
                let owned = registration.0.lock().unwrap();
                let context =
                    (&*owned.as_ref().unwrap().context as *const Sender<()>) as *mut c_void;
                network_signal(context, std::ptr::null_mut(), 0);
            }
            assert!(receiver.try_recv().is_ok());
            assert!(receiver.try_recv().is_err());
            registration.unregister();
            registration.unregister();
            unsafe {
                SendMessageW(hwnd, 0x218, 7, 0);
            }
            assert!(receiver.try_recv().is_err());
            assert!(receiver.is_closed());
            unsafe {
                DestroyWindow(hwnd);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_main_window_can_control_connections() {
        assert!(super::source("main").is_ok());
        assert_eq!(
            super::source("secondary").unwrap_err().code,
            "window_source"
        );
    }
}
