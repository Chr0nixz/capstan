//! Bounded launch requests. Paths and raw process arguments stay in Rust.

use crate::CommandError;
use capstan_core::launch::{LaunchRequest, LaunchRequestView, SshTarget};
use serde::Serialize;
use std::{collections::VecDeque, sync::Mutex};
use tauri::{Emitter, Manager, State};
use uuid::Uuid;

use capstan_core::{launch::recent::HistoryChange, settings::Settings, VaultManager};
use std::sync::Arc;

const MAX_PENDING_REQUESTS: usize = 8;
const MAX_LOCAL_RECIPES: usize = 256;

pub struct JumpListState {
    gate: tokio::sync::Mutex<()>,
    supported: bool,
    executable: std::path::PathBuf,
    app_id: String,
    shells: tokio::sync::OnceCell<Vec<capstan_pty::LocalShellProfile>>,
}

pub(crate) fn install_jump_list(
    app: &tauri::AppHandle,
    executable: &std::path::Path,
    portable: bool,
) {
    let app_id = app.config().identifier.clone();
    let supported = cfg!(windows) && !portable;
    app.manage(Arc::new(JumpListState {
        gate: tokio::sync::Mutex::new(()),
        supported,
        executable: executable.to_owned(),
        app_id,
        shells: tokio::sync::OnceCell::new(),
    }));
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        refresh_jump_list(&app).await;
    });
}

pub(crate) async fn refresh_jump_list(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<Arc<JumpListState>>() else {
        return;
    };
    let _gate = state.gate.lock().await;
    if refresh_jump_list_locked(app, &state).await.is_err() {
        eprintln!("jump_list_unavailable");
    }
}

async fn refresh_jump_list_locked(
    app: &tauri::AppHandle,
    state: &JumpListState,
) -> Result<(), CommandError> {
    if !state.supported {
        return Ok(());
    }
    let settings = app.state::<Arc<Settings>>().inner().clone();
    let history = tauri::async_runtime::spawn_blocking(move || settings.recent_hosts())
        .await
        .map_err(|_| CommandError::new("settings_unavailable"))?
        .map_err(|error| CommandError::new(error.to_string()))?;
    let vault = app.state::<Arc<VaultManager>>();
    let hosts = match vault.hosts() {
        Ok(hosts) => hosts,
        // Preserve the existing Windows list while the vault is locked.
        Err(_) if !history.is_empty() => return Ok(()),
        Err(_) => Vec::new(),
    };
    #[cfg(windows)]
    {
        use capstan_native_windows::jump_list::{self, Entry};
        let recent = history
            .iter()
            .filter_map(|id| hosts.iter().find(|host| host.id == *id))
            .map(|host| Entry {
                title: host
                    .name
                    .chars()
                    .filter(|character| !character.is_control())
                    .take(256)
                    .collect(),
                arguments: format!("--host {}", host.id),
            })
            .filter(|entry| !entry.title.is_empty())
            .collect::<Vec<_>>();
        let executable = state.executable.clone();
        let app_id = state.app_id.clone();
        let shells = state
            .shells
            .get_or_init(|| async {
                tauri::async_runtime::spawn_blocking(capstan_pty::local_shell_profiles)
                    .await
                    .unwrap_or_default()
            })
            .await
            .clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            let tasks = shells
                .into_iter()
                .take(6)
                .filter_map(|profile| {
                    Some(Entry {
                        title: profile.name,
                        arguments: capstan_core::launch::shell_task_arguments(&profile.id)?,
                    })
                })
                .collect::<Vec<_>>();
            jump_list::publish(&executable, &app_id, &tasks, &recent, "Capstan")
        })
        .await;
        if !matches!(result, Ok(Ok(()))) {
            return Err(CommandError::new("jump_list_unavailable"));
        }
    }
    #[cfg(not(windows))]
    let _ = (hosts, &state.executable, &state.app_id, &state.shells);
    Ok(())
}

pub(crate) async fn update_recent_host(app: &tauri::AppHandle, change: HistoryChange) {
    let Some(state) = app.try_state::<Arc<JumpListState>>() else {
        return;
    };
    let _gate = state.gate.lock().await;
    if let HistoryChange::Connected(id) = change {
        // A shell that finished starting after host deletion must not restore the deleted ID.
        if !app
            .state::<Arc<VaultManager>>()
            .hosts()
            .is_ok_and(|hosts| hosts.iter().any(|host| host.id == id))
        {
            return;
        }
    }
    let settings = app.state::<Arc<Settings>>().inner().clone();
    if settings.update_recent_hosts(change).await.is_err() {
        eprintln!("jump_list_history_unavailable");
    }
    if refresh_jump_list_locked(app, &state).await.is_err() {
        eprintln!("jump_list_unavailable");
    }
}

#[tauri::command]
pub async fn clear_recent_hosts(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<(), CommandError> {
    source(&window)?;
    let state = app.state::<Arc<JumpListState>>();
    let _gate = state.gate.lock().await;
    app.state::<Arc<Settings>>()
        .inner()
        .clone()
        .update_recent_hosts(HistoryChange::Clear)
        .await
        .map_err(|error| CommandError::new(error.to_string()))?;
    refresh_jump_list_locked(&app, &state).await
}

pub async fn save_recent_hosts_preference(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    value: bool,
) -> Result<capstan_core::settings::DesktopPreferences, CommandError> {
    source(&window)?;
    let state = app.state::<Arc<JumpListState>>();
    let _gate = state.gate.lock().await;
    let preferences = app
        .state::<Arc<Settings>>()
        .inner()
        .clone()
        .save_desktop_preference(
            capstan_core::settings::DesktopPreference::JumpListRecentHosts,
            value,
        )
        .await
        .map_err(|error| CommandError::new(error.to_string()))?;
    refresh_jump_list_locked(&app, &state).await?;
    Ok(preferences)
}

#[cfg(windows)]
pub struct InstanceListener {
    name: String,
    server: Mutex<Option<tokio::net::windows::named_pipe::NamedPipeServer>>,
}

#[cfg(windows)]
fn instance_pipe_name(directory: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    let canonical = directory
        .canonicalize()
        .unwrap_or_else(|_| directory.to_owned());
    let mut hash = Sha256::new();
    hash.update(canonical.to_string_lossy().to_lowercase().as_bytes());
    let digest = hash.finalize();
    let suffix = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("\\\\.\\pipe\\capstan-launch-{suffix}")
}

/// Runs before Tauri builds any windows. Only the pipe owner may continue startup.
#[cfg(windows)]
pub async fn prepare_instance(
    directory: &std::path::Path,
    arguments: &[String],
) -> std::io::Result<Option<InstanceListener>> {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::windows::named_pipe::{ClientOptions, ServerOptions},
        time::{timeout, Duration},
    };
    let name = instance_pipe_name(directory);
    match ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .create(&name)
    {
        Ok(server) => Ok(Some(InstanceListener {
            name,
            server: Mutex::new(Some(server)),
        })),
        Err(error) if matches!(error.raw_os_error(), Some(5 | 231)) => {
            let payload = capstan_core::launch::instance::encode_arguments(arguments);
            timeout(Duration::from_secs(10), async {
                let mut client = loop {
                    match ClientOptions::new().open(&name) {
                        Ok(client) => break client,
                        Err(error) if matches!(error.raw_os_error(), Some(2 | 231)) => {
                            tokio::time::sleep(Duration::from_millis(25)).await;
                        }
                        Err(error) => return Err(error),
                    }
                };
                use std::os::windows::io::AsRawHandle;
                if !capstan_native_windows::jump_list::allow_server_foreground(
                    client.as_raw_handle() as isize,
                ) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "launch owner",
                    ));
                }
                client.write_u32_le(payload.len() as u32).await?;
                client.write_all(&payload).await?;
                if client.read_u8().await? != 1 {
                    return Err(std::io::Error::other("launch forwarding rejected"));
                }
                Ok(None)
            })
            .await
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "launch forwarding timed out")
            })?
        }
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
async fn read_forwarded(
    server: &mut tokio::net::windows::named_pipe::NamedPipeServer,
) -> std::io::Result<Vec<String>> {
    use capstan_core::launch::instance::{decode_arguments, MAX_PAYLOAD_BYTES};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let length = server.read_u32_le().await? as usize;
    if length > MAX_PAYLOAD_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "launch limit",
        ));
    }
    let mut payload = vec![0; length];
    server.read_exact(&mut payload).await?;
    let arguments = decode_arguments(&payload)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "launch frame"))?;
    server.write_u8(1).await?;
    Ok(arguments)
}

#[cfg(windows)]
pub(crate) fn install_instance(app: &tauri::AppHandle) {
    use tokio::{
        net::windows::named_pipe::ServerOptions,
        time::{timeout, Duration},
    };
    let Some(listener) = app.try_state::<InstanceListener>() else {
        return;
    };
    let Some(mut server) = listener
        .server
        .lock()
        .ok()
        .and_then(|mut server| server.take())
    else {
        return;
    };
    let name = listener.name.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            if server.connect().await.is_err() {
                break;
            }
            // Keep a pipe instance alive throughout handover, so a third process cannot become owner.
            let Ok(next) = ServerOptions::new()
                .reject_remote_clients(true)
                .create(&name)
            else {
                break;
            };
            let arguments = timeout(Duration::from_secs(2), read_forwarded(&mut server)).await;
            let _ = server.disconnect();
            server = next;
            if let Ok(Ok(arguments)) = arguments {
                receive(&app, &arguments);
            }
        }
    });
}

struct PendingLaunch {
    id: Uuid,
    request: LaunchRequest,
}

pub(crate) struct LocalLaunch {
    pub options: capstan_pty::LocalTerminalOptions,
    pub cwd_fallback: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalStartOptions {
    shell_id: Option<String>,
    working_directory: Option<String>,
    launch_request_id: Option<Uuid>,
}

impl LocalStartOptions {
    pub(crate) fn resolve(self, requests: &LaunchRequests) -> Result<LocalLaunch, CommandError> {
        if let Some(id) = self.launch_request_id {
            requests.take_local(id)
        } else {
            Ok(LocalLaunch {
                options: capstan_pty::LocalTerminalOptions {
                    shell_id: self.shell_id,
                    working_directory: self.working_directory,
                },
                cwd_fallback: false,
            })
        }
    }
}

#[derive(Default)]
struct LaunchQueue {
    pending: VecDeque<PendingLaunch>,
    // Successful local requests become bounded recipes, allowing splits/restarts to reuse only an ID.
    local_recipes: VecDeque<PendingLaunch>,
}

#[derive(Default)]
pub struct LaunchRequests {
    entries: Mutex<LaunchQueue>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum LaunchDetails {
    Focus,
    Local,
    Host { host_id: Uuid },
    Ssh { target: SshTargetView },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshTargetView {
    pub username: Option<String>,
    pub host: String,
    pub port: u16,
}

impl From<SshTarget> for SshTargetView {
    fn from(target: SshTarget) -> Self {
        Self {
            username: target.username,
            host: target.host,
            port: target.port,
        }
    }
}

impl LaunchRequests {
    fn enqueue(&self, args: &[String]) -> Option<LaunchRequestView> {
        let request = capstan_core::launch::parse_launch_args(args);
        let id = Uuid::new_v4();
        let view = request.view(id);
        let mut entries = self.entries.lock().ok()?;
        if entries.pending.len() >= MAX_PENDING_REQUESTS {
            return None;
        }
        entries.pending.push_back(PendingLaunch { id, request });
        Some(view)
    }

    fn list(&self) -> Result<Vec<LaunchRequestView>, CommandError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| CommandError::new("launch_unavailable"))?;
        Ok(entries
            .pending
            .iter()
            .map(|entry| entry.request.view(entry.id))
            .collect())
    }

    fn inspect(&self, id: Uuid) -> Result<LaunchDetails, CommandError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| CommandError::new("launch_unavailable"))?;
        let request = entries
            .pending
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| CommandError::new("launch_expired"))?;
        Ok(match &request.request {
            LaunchRequest::Focus => LaunchDetails::Focus,
            LaunchRequest::Local { .. } => LaunchDetails::Local,
            LaunchRequest::Host(host_id) => LaunchDetails::Host { host_id: *host_id },
            LaunchRequest::Ssh(target) => LaunchDetails::Ssh {
                target: target.clone().into(),
            },
        })
    }

    fn ssh_target(&self, id: Uuid) -> Result<SshTarget, CommandError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| CommandError::new("launch_unavailable"))?;
        match entries.pending.iter().find(|entry| entry.id == id) {
            Some(PendingLaunch {
                request: LaunchRequest::Ssh(target),
                ..
            }) => Ok(target.clone()),
            Some(_) => Err(CommandError::new("launch_kind")),
            None => Err(CommandError::new("launch_expired")),
        }
    }

    pub(crate) fn take_local(&self, id: Uuid) -> Result<LocalLaunch, CommandError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CommandError::new("launch_unavailable"))?;
        let entry = if let Some(index) = entries
            .local_recipes
            .iter()
            .position(|entry| entry.id == id)
        {
            entries.local_recipes.remove(index)
        } else {
            let index = entries
                .pending
                .iter()
                .position(|entry| entry.id == id)
                .ok_or_else(|| CommandError::new("launch_expired"))?;
            entries.pending.remove(index)
        }
        .ok_or_else(|| CommandError::new("launch_expired"))?;
        let launch = Self::local_options(&entry.request)?;
        entries.local_recipes.push_back(entry);
        if entries.local_recipes.len() > MAX_LOCAL_RECIPES {
            entries.local_recipes.pop_front();
        }
        Ok(launch)
    }

    fn local_options(request: &LaunchRequest) -> Result<LocalLaunch, CommandError> {
        match request {
            LaunchRequest::Local {
                shell_id,
                working_directory,
            } => {
                // The PTY adapter accepts Linux paths for WSL; external requests grant Windows paths.
                let cwd_fallback = shell_id.as_deref().is_some_and(|id| id.starts_with("wsl:"))
                    && working_directory.is_some();
                Ok(LocalLaunch {
                    cwd_fallback,
                    options: capstan_pty::LocalTerminalOptions {
                        shell_id: shell_id.clone(),
                        working_directory: if cwd_fallback {
                            None
                        } else {
                            working_directory
                                .as_ref()
                                .map(|directory| directory.to_string_lossy().into_owned())
                        },
                    },
                })
            }
            _ => Err(CommandError::new("launch_kind")),
        }
    }

    fn remove(&self, id: Uuid) -> Result<(), CommandError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CommandError::new("launch_unavailable"))?;
        let Some(index) = entries.pending.iter().position(|entry| entry.id == id) else {
            return Err(CommandError::new("launch_expired"));
        };
        entries.pending.remove(index);
        Ok(())
    }
}

pub(crate) fn receive_initial(app: &tauri::AppHandle) {
    let args = std::env::args_os()
        .skip(1)
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    receive(app, &args);
}

fn receive(app: &tauri::AppHandle, args: &[String]) {
    let view = app.state::<LaunchRequests>().enqueue(args);
    if let Some(window) = app.get_webview_window("main") {
        crate::quake::show_for_launch(&window);
        if let Some(view) = view {
            let _ = window.emit("launch-request", view);
        }
    }
}

fn source(window: &tauri::WebviewWindow) -> Result<(), CommandError> {
    source_label(window.label())
}

fn source_label(label: &str) -> Result<(), CommandError> {
    if label == "main" {
        Ok(())
    } else {
        Err(CommandError::new("window_source"))
    }
}

#[tauri::command]
pub fn list_launch_requests(
    window: tauri::WebviewWindow,
    requests: State<'_, LaunchRequests>,
) -> Result<Vec<LaunchRequestView>, CommandError> {
    source(&window)?;
    requests.list()
}

#[tauri::command]
pub fn inspect_launch_request(
    window: tauri::WebviewWindow,
    requests: State<'_, LaunchRequests>,
    request_id: Uuid,
) -> Result<LaunchDetails, CommandError> {
    source(&window)?;
    requests.inspect(request_id)
}

/// Saved hosts an SSH link may reuse; matching stays in Rust with Vault address normalization.
#[tauri::command]
pub fn match_launch_hosts(
    window: tauri::WebviewWindow,
    requests: State<'_, LaunchRequests>,
    vault: State<'_, Arc<VaultManager>>,
    request_id: Uuid,
) -> Result<Vec<Uuid>, CommandError> {
    source(&window)?;
    let target = requests.ssh_target(request_id)?;
    Ok(vault
        .hosts()?
        .iter()
        .filter(|host| {
            capstan_core::launch::ssh_target_matches(
                &target,
                &host.address,
                host.port,
                &host.username,
            )
        })
        .map(|host| host.id)
        .collect())
}

#[tauri::command]
pub fn dismiss_launch_request(
    window: tauri::WebviewWindow,
    requests: State<'_, LaunchRequests>,
    request_id: Uuid,
) -> Result<(), CommandError> {
    source(&window)?;
    requests.remove(request_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use capstan_core::launch::LaunchRequestKind;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn queue_is_bounded_and_the_ui_projection_does_not_include_local_paths() {
        let queue = LaunchRequests::default();
        for _ in 0..MAX_PENDING_REQUESTS {
            assert!(queue.enqueue(&args(&["--local"])).is_some());
        }
        assert!(queue.enqueue(&[]).is_none());
        let views = queue.list().unwrap();
        assert_eq!(views.len(), MAX_PENDING_REQUESTS);
        assert!(views.iter().all(|view| {
            view.kind == LaunchRequestKind::Local && !view.display.contains("private")
        }));
    }

    #[test]
    fn host_matching_reads_only_pending_ssh_targets() {
        let queue = LaunchRequests::default();
        let ssh = queue
            .enqueue(&args(&["ssh://alice@build.example.test:2222"]))
            .unwrap();
        let local = queue.enqueue(&args(&["--local"])).unwrap();
        let target = queue.ssh_target(ssh.request_id).unwrap();
        assert_eq!(
            (
                target.username.as_deref(),
                target.host.as_str(),
                target.port
            ),
            (Some("alice"), "build.example.test", 2222)
        );
        assert_eq!(
            queue.ssh_target(local.request_id).unwrap_err().code,
            "launch_kind"
        );
        assert_eq!(
            queue.ssh_target(Uuid::new_v4()).unwrap_err().code,
            "launch_expired"
        );
    }

    #[test]
    fn request_ids_are_required_and_local_paths_are_only_returned_to_the_start_adapter() {
        let queue = LaunchRequests::default();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_str().unwrap();
        let view = queue.enqueue(&args(&["--local", "--cwd", path])).unwrap();
        assert!(matches!(
            queue.inspect(view.request_id).unwrap(),
            LaunchDetails::Local
        ));
        assert_eq!(
            queue
                .take_local(view.request_id)
                .unwrap()
                .options
                .working_directory
                .as_deref(),
            Some(path)
        );
        assert_eq!(
            queue.inspect(view.request_id).unwrap_err().code,
            "launch_expired"
        );
        let options: LocalStartOptions = serde_json::from_value(serde_json::json!({
            "launch_request_id": view.request_id, "shell_id": "untrusted-replacement", "working_directory": "C:\\untrusted"
        })).unwrap();
        assert_eq!(
            options
                .resolve(&queue)
                .unwrap()
                .options
                .working_directory
                .as_deref(),
            Some(path)
        );
        assert!(queue.list().unwrap().is_empty());
    }

    #[test]
    fn local_recipes_support_restarts_without_occupying_or_exceeding_the_pending_queue() {
        let queue = LaunchRequests::default();
        let mut first = None;
        let mut latest = None;
        for _ in 0..=MAX_LOCAL_RECIPES {
            let view = queue
                .enqueue(&args(&["--local", "--shell", "powershell51"]))
                .unwrap();
            first.get_or_insert(view.request_id);
            latest = Some(view.request_id);
            assert_eq!(
                queue
                    .take_local(view.request_id)
                    .unwrap()
                    .options
                    .shell_id
                    .as_deref(),
                Some("powershell51")
            );
        }
        assert!(queue.list().unwrap().is_empty());
        assert_eq!(
            queue.entries.lock().unwrap().local_recipes.len(),
            MAX_LOCAL_RECIPES
        );
        assert_eq!(
            queue.take_local(first.unwrap()).err().unwrap().code,
            "launch_expired"
        );
        assert!(queue.take_local(latest.unwrap()).is_ok());
        for _ in 0..MAX_PENDING_REQUESTS {
            assert!(queue.enqueue(&args(&["--local"])).is_some());
        }
        assert!(queue.enqueue(&args(&["--local"])).is_none());
    }

    #[test]
    fn wsl_launch_falls_back_without_exposing_the_windows_directory() {
        let queue = LaunchRequests::default();
        let directory = tempfile::tempdir().unwrap();
        let view = queue
            .enqueue(&args(&[
                "--local",
                "--shell",
                "wsl:Ubuntu",
                "--cwd",
                directory.path().to_str().unwrap(),
            ]))
            .unwrap();
        let launch = queue.take_local(view.request_id).unwrap();
        assert!(launch.cwd_fallback);
        assert!(launch.options.working_directory.is_none());
        assert_eq!(launch.options.shell_id.as_deref(), Some("wsl:Ubuntu"));
    }

    #[test]
    fn malformed_inputs_are_queued_only_as_focus_requests() {
        let queue = LaunchRequests::default();
        let view = queue
            .enqueue(&args(&["--local", "--cwd", "relative"]))
            .unwrap();
        assert_eq!(view.kind, LaunchRequestKind::Focus);
        assert!(matches!(
            queue.inspect(view.request_id),
            Ok(LaunchDetails::Focus)
        ));
    }

    #[test]
    fn launch_details_and_queue_limits_match_the_shared_contract() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../docs/agent/fixtures/M4-contract.json"))
                .unwrap();
        let queue = LaunchRequests::default();
        let request = queue
            .enqueue(&args(&["ssh://user@example.test:22/?fingerprint=ignored"]))
            .unwrap();
        assert_eq!(
            serde_json::to_value(queue.inspect(request.request_id).unwrap()).unwrap(),
            fixture["launchDetails"]
        );
        assert_eq!(
            MAX_PENDING_REQUESTS,
            fixture["launchLimits"]["pendingRequests"].as_u64().unwrap() as usize
        );
        assert_eq!(
            capstan_core::launch::MAX_ARGUMENTS as u64,
            fixture["launchLimits"]["arguments"].as_u64().unwrap()
        );
        assert_eq!(
            capstan_core::launch::MAX_ARGUMENT_LENGTH as u64,
            fixture["launchLimits"]["argumentCharacters"]
                .as_u64()
                .unwrap()
        );
        assert_eq!(
            capstan_core::launch::recent::MAX_RECENT_HOSTS as u64,
            fixture["launchLimits"]["recentHosts"].as_u64().unwrap()
        );
        assert_eq!(
            MAX_LOCAL_RECIPES as u64,
            fixture["launchLimits"]["localRecipes"].as_u64().unwrap()
        );
        let nested: LocalStartOptions =
            serde_json::from_value(fixture["localLaunchOptions"].clone()).unwrap();
        assert_eq!(
            nested.launch_request_id,
            Some(Uuid::parse_str(fixture["launchRequest"]["requestId"].as_str().unwrap()).unwrap())
        );
        assert_eq!(source_label("other").unwrap_err().code, "window_source");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn second_instance_forwards_raw_arguments_and_data_directories_are_independent() {
        let first_root = tempfile::tempdir().unwrap();
        let second_root = tempfile::tempdir().unwrap();
        let owner = prepare_instance(first_root.path(), &[])
            .await
            .unwrap()
            .unwrap();
        let independent = prepare_instance(second_root.path(), &[]).await.unwrap();
        assert!(independent.is_some());
        let raw = vec!["ssh://user@[::1]:2222/?ignored=one|two".to_owned()];
        let mut server = owner.server.lock().unwrap().take().unwrap();
        let forward = prepare_instance(first_root.path(), &raw);
        let receiver = async {
            server.connect().await.unwrap();
            read_forwarded(&mut server).await.unwrap()
        };
        let (secondary, received) = tokio::join!(forward, receiver);
        assert!(secondary.unwrap().is_none());
        assert_eq!(received, raw);
        assert_ne!(
            instance_pipe_name(first_root.path()),
            instance_pipe_name(second_root.path())
        );
    }
}
