//! Native taskbar updates and text-only background notifications.
use crate::CommandError;
use capstan_core::{
    desktop::{
        taskbar_progress, DesktopTransfer, NoticeQueue, NoticeSummary, NoticeTracker,
        ProgressState, TaskbarProgress,
    },
    settings::Settings,
    sftp::{ExactU64, LOCAL_GRANT_LIFETIME_SECONDS},
    transfer::{TransferService, TreePlanSummary},
    VaultManager,
};
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{Manager, State, WebviewWindow, WindowEvent};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const SAMPLE_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Default)]
pub struct DesktopState {
    scanning: AtomicUsize,
    tree_conflicts: Mutex<HashMap<Uuid, Instant>>,
    acknowledged: Mutex<HashMap<Uuid, u64>>,
    chinese: AtomicBool,
}

impl DesktopState {
    pub fn begin_scan(&self) -> Scanning<'_> {
        self.scanning.fetch_add(1, Ordering::Relaxed);
        Scanning(&self.scanning)
    }

    pub fn update_tree_conflict(&self, plan: &TreePlanSummary) {
        if plan.conflicts.is_empty() {
            self.clear_tree_conflict(plan.token);
        } else if let Ok(mut conflicts) = self.tree_conflicts.lock() {
            conflicts.entry(plan.token).or_insert_with(|| {
                Instant::now() + Duration::from_secs(LOCAL_GRANT_LIFETIME_SECONDS)
            });
        }
    }

    pub fn clear_tree_conflict(&self, token: Uuid) {
        if let Ok(mut conflicts) = self.tree_conflicts.lock() {
            conflicts.remove(&token);
        }
    }

    pub fn set_locale(&self, locale: &str) {
        self.chinese.store(locale == "zh-CN", Ordering::Relaxed);
    }

    fn progress(&self, tasks: &[DesktopTransfer], now: Instant) -> TaskbarProgress {
        let has_tree_conflict = self.tree_conflicts.lock().is_ok_and(|mut conflicts| {
            conflicts.retain(|_, expires| *expires > now);
            !conflicts.is_empty()
        });
        let Ok(mut acknowledged) = self.acknowledged.lock() else {
            return TaskbarProgress::default();
        };
        acknowledged.retain(|id, version| {
            tasks
                .iter()
                .any(|task| task.id == *id && task.version == *version)
        });
        taskbar_progress(
            tasks,
            self.scanning.load(Ordering::Relaxed) != 0,
            has_tree_conflict,
            &acknowledged,
        )
    }
}

pub struct Scanning<'a>(&'a AtomicUsize);
impl Drop for Scanning<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

#[tauri::command]
pub fn acknowledge_transfer_failure(
    window: WebviewWindow,
    desktop: State<'_, Arc<DesktopState>>,
    transfers: State<'_, Arc<TransferService>>,
    vault: State<'_, Arc<VaultManager>>,
    id: Uuid,
    version: ExactU64,
) -> Result<(), CommandError> {
    if window.label() != "main" {
        return Err(CommandError::new("window_source"));
    }
    if !vault
        .desktop_transfers(&transfers, "main")
        .map_err(|error| CommandError::new(error.to_string()))?
        .into_iter()
        .any(|task| {
            task.id == id && task.version == version.0 && (task.failed() || task.awaiting_reconnect)
        })
    {
        return Err(CommandError::new("scope_expired"));
    }
    desktop
        .acknowledged
        .lock()
        .map_err(|_| CommandError::new("local_io"))?
        .insert(id, version.0);
    Ok(())
}

fn send_progress(window: &WebviewWindow, progress: TaskbarProgress) -> tauri::Result<()> {
    use tauri::window::{ProgressBarState, ProgressBarStatus as Status};
    let status = match progress.state {
        ProgressState::None => Status::None,
        ProgressState::Normal => Status::Normal,
        ProgressState::Indeterminate => Status::Indeterminate,
        ProgressState::Paused => Status::Paused,
        ProgressState::Error => Status::Error,
    };
    window.set_progress_bar(ProgressBarState {
        status: Some(status),
        progress: match progress.state {
            ProgressState::Paused | ProgressState::Error => {
                Some(u64::from(progress.percent.max(1)))
            }
            ProgressState::Normal => Some(u64::from(progress.percent)),
            ProgressState::None | ProgressState::Indeterminate => None,
        },
    })
}

fn toast_body(summary: NoticeSummary, chinese: bool) -> String {
    let mut lines = Vec::new();
    if summary.completed != 0 {
        lines.push(if chinese {
            format!("传输批次完成 {} 个", summary.completed)
        } else {
            format!("Completed transfer batches: {}", summary.completed)
        });
    }
    if summary.failed != 0 {
        lines.push(if chinese {
            format!("传输批次失败 {} 个", summary.failed)
        } else {
            format!("Failed transfer batches: {}", summary.failed)
        });
    }
    if summary.disconnected != 0 {
        lines.push(if chinese {
            format!("SSH 连接意外断开 {} 条", summary.disconnected)
        } else {
            format!("Unexpected SSH disconnects: {}", summary.disconnected)
        });
    }
    lines.join(" · ")
}

pub fn install(window: &WebviewWindow, executable: &Path, portable: bool) {
    let app_id = window.config().identifier.clone();
    let shortcut = std::env::var_os("APPDATA").map(|root| {
        std::path::PathBuf::from(root).join("Microsoft/Windows/Start Menu/Programs/Capstan.lnk")
    });
    let identified = !portable
        && shortcut.is_some_and(|shortcut| {
            capstan_native_windows::toast::has_installed_identity(&shortcut, executable, &app_id)
        });
    let vault = window.state::<Arc<VaultManager>>().inner().clone();
    let desktop = window.state::<Arc<DesktopState>>().inner().clone();
    let transfers = window.state::<Arc<TransferService>>().inner().clone();
    let settings = window.state::<Arc<Settings>>().inner().clone();
    let mut connections = vault.subscribe_connections();
    let stop = CancellationToken::new();
    let window_stop = stop.clone();
    let background_since = Arc::new(Mutex::new(
        window
            .is_focused()
            .is_ok_and(|focused| !focused)
            .then(Instant::now),
    ));
    let window_background = background_since.clone();
    let foreground = Arc::new(tokio::sync::Notify::new());
    let window_foreground = foreground.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Destroyed => window_stop.cancel(),
        WindowEvent::Focused(focused) => {
            if let Ok(mut since) = window_background.lock() {
                *since = (!focused).then(Instant::now);
            }
            if *focused {
                window_foreground.notify_one();
            }
        }
        _ => {}
    });
    let window = window.clone();
    tauri::async_runtime::spawn(async move {
        let _ = settings.desktop_preferences();
        let mut ticks = tokio::time::interval(SAMPLE_INTERVAL);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_progress = TaskbarProgress::default();
        let mut last_update: Option<Instant> = None;
        let mut tracker = NoticeTracker::default();
        let mut notices = NoticeQueue::default();
        let mut reported_missing_identity = false;
        let mut prune_at = Instant::now() + Duration::from_secs(10);
        let background = || background_since.lock().ok().and_then(|since| *since);
        let allowed = || settings.system_notifications_enabled() && background().is_some();
        loop {
            let deadline = notices.deadline();
            tokio::select! {
                biased;
                () = stop.cancelled() => break,
                () = foreground.notified() => { notices.take_due(Instant::now(), false); }
                () = async {
                    if let Some(deadline) = deadline {
                        tokio::time::sleep_until(deadline.into()).await;
                    } else { std::future::pending::<()>().await; }
                } => {
                    if let Some(summary) = notices.take_due(Instant::now(), allowed()) {
                        if identified {
                            let chinese = desktop.chinese.load(Ordering::Relaxed);
                            let id = app_id.clone();
                            let delivery_window = window.clone();
                            let delivery_settings = settings.clone();
                            let expected_background = background();
                            let delivery_background = background_since.clone();
                            let _ = window.run_on_main_thread(move || {
                                if delivery_settings.system_notifications_enabled()
                                    && delivery_background.lock().is_ok_and(|since| since.is_some() && *since == expected_background)
                                    && delivery_window.is_focused().is_ok_and(|focused| !focused)
                                    && capstan_native_windows::toast::show(&id, "Capstan", &toast_body(summary, chinese)).is_err()
                                {
                                    eprintln!("toast_send_failed");
                                }
                            });
                        } else if !reported_missing_identity {
                            eprintln!("toast_aumid_unavailable");
                            reported_missing_identity = true;
                        }
                    }
                }
                _ = ticks.tick() => {
                    let now = Instant::now();
                    if !allowed() { notices.take_due(now, false); }
                    if now >= prune_at {
                        prune_at = now + Duration::from_secs(10);
                        if let Ok(snapshots) = vault.connection_snapshots() {
                            tracker.retain_connections(&snapshots.into_iter().map(|snapshot| snapshot.connection_id).collect());
                        }
                    }
                    if let Ok(tasks) = vault.desktop_transfers(&transfers, "main") {
                        let progress = desktop.progress(&tasks, now);
                        if progress != last_progress
                            && last_update.is_none_or(|last| now.duration_since(last) >= SAMPLE_INTERVAL)
                        {
                            last_update = Some(now);
                            if send_progress(&window, progress).is_ok() { last_progress = progress; }
                        }
                        for notice in tracker.transfers(&tasks) {
                            let was_background = background().is_some_and(|since| since <= notice.finished);
                            notices.push(notice.kind, now, was_background && settings.system_notifications_enabled());
                        }
                    }
                }
                update = connections.recv() => match update {
                    Ok(snapshot) => {
                        if let Some(notice) = tracker.connection(snapshot) {
                            notices.push(notice, Instant::now(), allowed());
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if let Ok(snapshots) = vault.connection_snapshots() {
                            tracker.retain_connections(&snapshots.iter().map(|snapshot| snapshot.connection_id).collect());
                            for snapshot in snapshots {
                                if let Some(notice) = tracker.connection(snapshot) {
                                    notices.push(notice, Instant::now(), allowed());
                                }
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    });
}
