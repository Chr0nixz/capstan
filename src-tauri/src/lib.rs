//! Desktop commands and Channel adapters; session ownership lives in core crates.
#![deny(unsafe_code)]
#![allow(clippy::pedantic)]

pub use capstan_terminal::encode_binary_frame;
#[cfg(feature = "m0-benchmarks")]
pub mod benchmark;
#[cfg(feature = "tauri-runtime")]
mod connection_commands;
pub mod data_directory;
#[cfg(feature = "tauri-runtime")]
mod forwarding_commands;
#[cfg(all(windows, feature = "tauri-runtime"))]
mod remote_edit_commands;
#[cfg(feature = "tauri-runtime")]
mod settings_commands;
#[cfg(feature = "tauri-runtime")]
mod sftp_commands;
#[cfg(feature = "tauri-runtime")]
mod snippet_commands;
#[cfg(feature = "tauri-runtime")]
mod transfer_commands;

#[derive(Debug, serde::Serialize)]
pub struct CommandError {
    pub code: String,
}
impl CommandError {
    pub fn new(code: impl Into<String>) -> Self {
        Self { code: code.into() }
    }
}
impl From<capstan_terminal::TerminalError> for CommandError {
    fn from(error: capstan_terminal::TerminalError) -> Self {
        Self::new(error.code())
    }
}
impl From<capstan_pty::PtyError> for CommandError {
    fn from(error: capstan_pty::PtyError) -> Self {
        Self::new(error.code())
    }
}
impl From<capstan_core::CoreError> for CommandError {
    fn from(error: capstan_core::CoreError) -> Self {
        Self::new(error.code())
    }
}

#[cfg(any(feature = "tauri-runtime", test))]
fn ssh_start_error(error: capstan_core::CoreError) -> capstan_terminal::TerminalError {
    capstan_terminal::TerminalError::BackendCode(error.code())
}

#[derive(Debug, serde::Serialize)]
pub struct RuntimeSummary {
    pub shells: Vec<capstan_pty::ShellInfo>,
}
pub fn runtime_summary() -> RuntimeSummary {
    RuntimeSummary {
        shells: capstan_pty::ShellDetector::new().detect_all(),
    }
}

#[cfg(test)]
mod startup_error_tests {
    use super::*;
    use capstan_core::CoreError;
    use capstan_ssh::SshError;
    use capstan_terminal::{BinaryFrame, FrameSink, TerminalError, TerminalService};
    use std::{future::Future, pin::Pin, sync::Arc};
    use tokio_util::sync::CancellationToken;

    #[test]
    fn release_content_policy_does_not_allow_development_or_remote_connections() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let security = &config["app"]["security"];
        let production = security["csp"].as_str().unwrap();
        let connect = production
            .split(';')
            .map(str::trim)
            .find(|directive| directive.starts_with("connect-src "))
            .unwrap();
        assert_eq!(
            connect.split_whitespace().collect::<Vec<_>>(),
            ["connect-src", "'self'", "ipc:", "http://ipc.localhost"]
        );
        assert!(!production.contains("127.0.0.1"));
        assert!(!production.contains("unsafe-eval"));
        assert!(!production.contains('*'));
        assert!(security["devCsp"]
            .as_str()
            .unwrap()
            .contains("ws://127.0.0.1:1420"));
    }

    struct QuietSink;
    impl FrameSink for QuietSink {
        fn send<'a>(
            &'a self,
            _: BinaryFrame,
        ) -> Pin<Box<dyn Future<Output = capstan_terminal::Result<()>> + Send + 'a>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn ssh_startup_codes_survive_the_terminal_service_and_command_adapter() {
        for (error, expected) in [
            (
                CoreError::Ssh(SshError::Authentication),
                "ssh_authentication",
            ),
            (CoreError::Ssh(SshError::Timeout), "ssh_timeout"),
            (CoreError::Ssh(SshError::TrustRejected), "host_key_rejected"),
            (
                CoreError::Ssh(SshError::ChannelRejected),
                "ssh_channel_rejected",
            ),
            (
                CoreError::Ssh(SshError::AgentUnavailable),
                "ssh_agent_unavailable",
            ),
            (CoreError::HostKeyChanged, "host_key_changed"),
            (CoreError::Locked, "vault_locked"),
            (CoreError::Canceled, "canceled"),
        ] {
            let terminals = Arc::new(TerminalService::default());
            let result = terminals
                .open_async::<(), _, _>(
                    move |_| async move { Err(ssh_start_error(error)) },
                    CancellationToken::new(),
                    Arc::new(QuietSink),
                )
                .await;
            assert_eq!(CommandError::from(result.unwrap_err()).code, expected);
            assert_eq!(terminals.registered_sessions(), 0);
        }
    }

    #[test]
    fn dynamic_backend_details_remain_redacted() {
        let error = CommandError::from(TerminalError::Backend(
            "private-backend-detail-with-credential-or-path".into(),
        ));
        assert_eq!(error.code, "backend");
        assert!(!format!("{error:?}").contains("private-backend-detail"));
    }
}

#[cfg(feature = "tauri-runtime")]
pub mod tauri_channel {
    use crate::{encode_binary_frame, CommandError, RuntimeSummary};
    use capstan_core::{
        CredentialUpdate, CredentialView, GroupView, HostView, TrustedHostKeyView, VaultManager,
        VaultRecoveryCandidateView, VaultStatus,
    };
    use capstan_pty::{
        local_shell_profiles, open_local_terminal, ConPtyConfig, ConPtySelection,
        LocalShellProfile, LocalTerminalOptions,
    };
    use capstan_ssh::TrustBroker;
    use capstan_terminal::{
        BinaryFrame, FrameSink, Result, TerminalError, TerminalService, TerminalSessionId,
        TerminalSnapshot,
    };
    use capstan_vault::{Group, Host, KdfParameters, Preferences, SecretText};
    use std::collections::HashMap;
    use std::io::Read;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use tauri::ipc::{Channel, InvokeResponseBody};
    use tauri::Emitter;
    use tauri::{Manager, State, WindowEvent};
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    struct SshAttempt {
        cancellation: CancellationToken,
        completed: tokio::sync::watch::Sender<bool>,
    }

    #[derive(Clone, Default)]
    pub struct SshAttempts {
        entries: Arc<Mutex<HashMap<Uuid, SshAttempt>>>,
        locked: Arc<AtomicBool>,
    }
    impl SshAttempts {
        fn begin(&self, attempt_id: Uuid) -> std::result::Result<CancellationToken, CommandError> {
            if self.locked.load(Ordering::Acquire) {
                return Err(CommandError::new("vault_locked"));
            }
            let mut attempts = self
                .entries
                .lock()
                .map_err(|_| CommandError::new("task_failed"))?;
            if attempts.contains_key(&attempt_id) {
                return Err(CommandError::new("attempt_exists"));
            }
            let token = CancellationToken::new();
            let (completed, _) = tokio::sync::watch::channel(false);
            attempts.insert(
                attempt_id,
                SshAttempt {
                    cancellation: token.clone(),
                    completed,
                },
            );
            if self.locked.load(Ordering::Acquire) {
                if let Some(attempt) = attempts.remove(&attempt_id) {
                    attempt.cancellation.cancel();
                }
                return Err(CommandError::new("vault_locked"));
            }
            Ok(token)
        }
        fn finish(&self, attempt_id: Uuid) {
            if let Ok(mut attempts) = self.entries.lock() {
                if let Some(attempt) = attempts.remove(&attempt_id) {
                    attempt.completed.send_replace(true);
                }
            }
        }
        async fn cancel(&self, attempt_id: Uuid) {
            let completed = if let Ok(attempts) = self.entries.lock() {
                if let Some(attempt) = attempts.get(&attempt_id) {
                    attempt.cancellation.cancel();
                    Some(attempt.completed.subscribe())
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(mut completed) = completed {
                while !*completed.borrow_and_update() {
                    if completed.changed().await.is_err() {
                        break;
                    }
                }
            }
        }
        fn allow_new(&self) {
            self.locked.store(false, Ordering::Release);
        }
        async fn block_and_cancel_all(&self, broker: &TrustBroker) {
            self.locked.store(true, Ordering::Release);
            let pending = if let Ok(attempts) = self.entries.lock() {
                attempts
                    .iter()
                    .map(|(attempt_id, attempt)| {
                        attempt.cancellation.cancel();
                        broker.cancel_attempt(*attempt_id);
                        (*attempt_id, attempt.completed.subscribe())
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            for (_, mut completed) in pending {
                while !*completed.borrow_and_update() {
                    if completed.changed().await.is_err() {
                        break;
                    }
                }
            }
        }
    }
    struct SshAttemptGuard {
        attempts: SshAttempts,
        attempt_id: Uuid,
    }
    impl Drop for SshAttemptGuard {
        fn drop(&mut self) {
            self.attempts.finish(self.attempt_id);
        }
    }

    #[derive(Clone, Default)]
    pub struct SshSessions(Arc<Mutex<HashMap<TerminalSessionId, Option<Uuid>>>>);
    impl SshSessions {
        #[cfg(test)]
        fn insert(&self, id: TerminalSessionId) {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(id, None);
        }
        fn bind(&self, id: TerminalSessionId, owner: Uuid) {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(id, Some(owner));
        }
        fn remove(&self, id: &TerminalSessionId) -> Option<Uuid> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(id)
                .flatten()
        }
        fn take_all(&self) -> Vec<TerminalSessionId> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .drain()
                .map(|(id, _)| id)
                .collect()
        }
    }

    pub struct PendingBackupImport {
        source: std::path::PathBuf,
        password: SecretText,
        expected_revision: u64,
        expected_epoch: u64,
        expires_at: Instant,
    }

    #[derive(Clone, Default)]
    pub struct BackupImportSessions(Arc<Mutex<HashMap<Uuid, PendingBackupImport>>>);
    impl BackupImportSessions {
        fn insert(
            &self,
            source: std::path::PathBuf,
            password: SecretText,
            expected_revision: u64,
            expected_epoch: u64,
        ) -> std::result::Result<Uuid, CommandError> {
            let mut pending = self
                .0
                .lock()
                .map_err(|_| CommandError::new("task_failed"))?;
            pending.retain(|_, value| value.expires_at > Instant::now());
            if pending.len() >= 16 {
                return Err(CommandError::new("backup_import_busy"));
            }
            let token = Uuid::new_v4();
            pending.insert(
                token,
                PendingBackupImport {
                    source,
                    password,
                    expected_revision,
                    expected_epoch,
                    expires_at: Instant::now() + Duration::from_secs(600),
                },
            );
            Ok(token)
        }
        fn take(
            &self,
            token: Uuid,
            expected_revision: u64,
            expected_epoch: u64,
        ) -> std::result::Result<PendingBackupImport, CommandError> {
            let pending = self
                .0
                .lock()
                .map_err(|_| CommandError::new("task_failed"))?
                .remove(&token)
                .ok_or_else(|| CommandError::new("backup_import_expired"))?;
            if pending.expires_at <= Instant::now()
                || pending.expected_revision != expected_revision
                || pending.expected_epoch != expected_epoch
            {
                return Err(CommandError::new("backup_import_expired"));
            }
            Ok(pending)
        }
        fn cancel(&self, token: Uuid) {
            if let Ok(mut pending) = self.0.lock() {
                pending.remove(&token);
            }
        }
        fn clear(&self) {
            if let Ok(mut pending) = self.0.lock() {
                pending.clear();
            }
        }
    }

    #[derive(Clone)]
    pub struct UserActivity(Arc<Mutex<tokio::time::Instant>>);
    impl Default for UserActivity {
        fn default() -> Self {
            Self(Arc::new(Mutex::new(tokio::time::Instant::now())))
        }
    }
    impl UserActivity {
        fn record(&self) {
            if let Ok(mut last_activity) = self.0.lock() {
                *last_activity = tokio::time::Instant::now();
            }
        }
        fn claim_if_idle(&self, timeout: Duration) -> bool {
            let Ok(mut last_activity) = self.0.lock() else {
                return false;
            };
            if last_activity.elapsed() < timeout {
                return false;
            }
            *last_activity = tokio::time::Instant::now();
            true
        }
    }

    async fn close_ssh_sessions(terminals: Arc<TerminalService>, sessions: &SshSessions) {
        let mut tasks = tokio::task::JoinSet::new();
        for id in sessions.take_all() {
            let terminals = terminals.clone();
            tasks.spawn(async move { terminals.close(&id).await });
        }
        while tasks.join_next().await.is_some() {}
    }

    #[allow(clippy::too_many_arguments)] // Managed workflow dependencies are injected separately.
    async fn lock_vault_and_cleanup(
        vault: &VaultManager,
        preferences: &Preferences,
        attempts: &SshAttempts,
        broker: &TrustBroker,
        terminals: Arc<TerminalService>,
        ssh_sessions: &SshSessions,
        transfers: &capstan_core::transfer::TransferService,
        #[cfg(windows)] editors: Option<&capstan_core::remote_edit::RemoteEditorService>,
    ) -> std::result::Result<(), CommandError> {
        attempts.block_and_cancel_all(broker).await;
        terminals.wait_for_starts().await;
        vault.lock().await.map_err(CommandError::from)?;
        if preferences.disconnect_on_lock {
            #[cfg(windows)]
            if let Some(editors) = editors {
                editors.cancel_window("main");
            }
            transfers.cancel_window("main");
            close_ssh_sessions(terminals, ssh_sessions).await;
            vault.disconnect_ssh().await;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // Managed workflow dependencies are injected separately.
    async fn maybe_auto_lock(
        vault: &VaultManager,
        activity: &UserActivity,
        attempts: &SshAttempts,
        broker: &TrustBroker,
        terminals: Arc<TerminalService>,
        ssh_sessions: &SshSessions,
        transfers: &capstan_core::transfer::TransferService,
        #[cfg(windows)] editors: Option<&capstan_core::remote_edit::RemoteEditorService>,
    ) -> std::result::Result<bool, CommandError> {
        let preferences = match vault.preferences() {
            Ok(preferences) => preferences,
            Err(capstan_core::CoreError::Locked) => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        let Some(minutes) = preferences.auto_lock_minutes else {
            return Ok(false);
        };
        if !activity.claim_if_idle(Duration::from_secs(u64::from(minutes) * 60)) {
            return Ok(false);
        }
        lock_vault_and_cleanup(
            vault,
            &preferences,
            attempts,
            broker,
            terminals,
            ssh_sessions,
            transfers,
            #[cfg(windows)]
            editors,
        )
        .await?;
        Ok(true)
    }

    #[derive(Clone)]
    pub struct TauriBinaryChannel(Channel<InvokeResponseBody>);
    impl TauriBinaryChannel {
        pub fn new(channel: Channel<InvokeResponseBody>) -> Self {
            Self(channel)
        }
    }

    // Tauri 2.12.1 serializes Raw bodies smaller than 1024 bytes into inline JS
    // arrays. Zero padding keeps every terminal frame on the binary fetch path.
    // ACK/budget/checksum still count only the declared terminal payload.
    fn encode_channel_frame(frame: BinaryFrame) -> Result<Vec<u8>> {
        let mut bytes = encode_binary_frame(frame)?;
        bytes.resize(bytes.len().max(1024), 0);
        Ok(bytes)
    }

    impl FrameSink for TauriBinaryChannel {
        fn send<'a>(
            &'a self,
            frame: BinaryFrame,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
            Box::pin(async move {
                self.0
                    .send(InvokeResponseBody::Raw(encode_channel_frame(frame)?))
                    .map_err(|_| TerminalError::OutputSend("channel_closed".to_owned()))
            })
        }
    }

    #[derive(serde::Serialize)]
    pub struct StartedTerminal {
        pub kind: &'static str,
        pub id: TerminalSessionId,
        pub runtime: ConPtySelection,
    }

    #[tauri::command]
    pub async fn list_local_shells(
        window: tauri::WebviewWindow,
    ) -> std::result::Result<Vec<LocalShellProfile>, CommandError> {
        if window.label() != "main" {
            return Err(CommandError::new("window_source"));
        }
        tokio::task::spawn_blocking(local_shell_profiles)
            .await
            .map_err(|_| CommandError::new("task_failed"))
    }

    #[tauri::command]
    pub async fn start_local_terminal(
        manager: State<'_, Arc<TerminalService>>,
        options: LocalTerminalOptions,
        on_output: Channel<InvokeResponseBody>,
        on_status: Channel<TerminalSnapshot>,
    ) -> std::result::Result<StartedTerminal, CommandError> {
        let executable = std::env::current_exe().map_err(|_| CommandError::new("runtime_path"))?;
        let (id, runtime) = manager
            .open(
                move || {
                    open_local_terminal(options, ConPtyConfig::for_executable(&executable))
                        .map_err(|error| TerminalError::BackendCode(error.code()))
                },
                Arc::new(TauriBinaryChannel(on_output)),
            )
            .await?;
        let session = manager.get(&id)?;
        forward_status(session, on_status);
        Ok(StartedTerminal {
            kind: "local",
            id,
            runtime,
        })
    }

    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct StartedSshTerminal {
        pub kind: &'static str,
        pub id: TerminalSessionId,
        pub host_id: Uuid,
        pub connection_id: Uuid,
        pub generation: capstan_core::sftp::ExactU64,
    }

    #[tauri::command]
    #[allow(clippy::too_many_arguments)] // Tauri injects services and channels as separate command parameters.
    pub async fn start_ssh_terminal(
        window: tauri::WebviewWindow,
        manager: State<'_, Arc<TerminalService>>,
        vault: State<'_, Arc<VaultManager>>,
        broker: State<'_, Arc<TrustBroker>>,
        attempts: State<'_, SshAttempts>,
        ssh_sessions: State<'_, SshSessions>,
        sftp: State<'_, Arc<capstan_core::sftp::SftpService>>,
        host_id: Uuid,
        attempt_id: Uuid,
        reconnect: Option<crate::connection_commands::ConnectionTarget>,
        on_output: Channel<InvokeResponseBody>,
        on_status: Channel<TerminalSnapshot>,
    ) -> std::result::Result<StartedSshTerminal, CommandError> {
        if window.label() != "main" {
            return Err(CommandError::new("window_source"));
        }
        let cancellation = attempts.begin(attempt_id)?;
        let _attempt_guard = SshAttemptGuard {
            attempts: attempts.inner().clone(),
            attempt_id,
        };
        let vault = vault.inner().clone();
        let broker = broker.inner().clone();
        let terminals = manager.inner().clone();
        let connect_vault = vault.clone();
        // TerminalService may drop its factory on cancellation. Keep the owned
        // core operation alive long enough to cancel and join its pool waiter.
        let connect_tasks = tokio_util::task::TaskTracker::new();
        let factory_tasks = connect_tasks.clone();
        let result = manager
            .inner()
            .clone()
            .open_async(
                move |cancel| async move {
                    factory_tasks
                        .spawn(async move {
                            let (host_id, shell, lease) = if let Some(target) = reconnect {
                                connect_vault
                                    .open_reconnected_shell(
                                        target.connection_id,
                                        target.expected_generation.0,
                                        attempt_id,
                                        cancel,
                                    )
                                    .await
                                    .map_err(crate::ssh_start_error)?
                            } else {
                                let (host, shell, lease) = connect_vault
                                    .connect_password_host_with_lease(
                                        host_id, attempt_id, broker, cancel,
                                    )
                                    .await
                                    .map_err(crate::ssh_start_error)?;
                                (host.id, shell, lease)
                            };
                            let snapshot = connect_vault
                                .connection_for_owner(attempt_id)
                                .map_err(crate::ssh_start_error)?;
                            let metadata = (host_id, shell.connection_id(), lease, snapshot);
                            Ok((shell.into_terminal_parts(), metadata))
                        })
                        .await
                        .map_err(|_| TerminalError::TaskFailed)?
                },
                cancellation,
                Arc::new(TauriBinaryChannel(on_output)),
            )
            .await;
        connect_tasks.close();
        connect_tasks.wait().await;
        let (id, (host_id, physical_connection_id, lease, connection)) = match result {
            Ok(result) => result,
            Err(error) => {
                vault.release_connection_owner_and_wait(attempt_id).await;
                return Err(error.into());
            }
        };
        ssh_sessions.bind(id.clone(), attempt_id);
        if !vault.status().unlocked {
            ssh_sessions.remove(&id);
            terminals.close(&id).await?;
            vault.release_connection_owner(attempt_id);
            return Err(CommandError::new("vault_locked"));
        }
        if let Err(error) = sftp.register(
            window.label(),
            &id.0,
            host_id,
            physical_connection_id,
            lease,
        ) {
            ssh_sessions.remove(&id);
            terminals.close(&id).await?;
            vault.release_connection_owner(attempt_id);
            return Err(CommandError::new(error.to_string()));
        }
        let sftp_lifetime = sftp.inner().clone();
        let terminal_id = id.0.clone();
        let mut terminal_status = terminals.get(&id)?.subscribe();
        tauri::async_runtime::spawn(async move {
            loop {
                if terminal_status.borrow_and_update().is_finished() {
                    break;
                }
                if terminal_status.changed().await.is_err() {
                    break;
                }
            }
            sftp_lifetime.revoke_terminal(&terminal_id);
            if terminal_status.borrow().state == capstan_terminal::TerminalState::Exited {
                vault.release_connection_owner_and_wait(attempt_id).await;
            }
        });
        forward_status(terminals.get(&id)?, on_status);
        Ok(StartedSshTerminal {
            kind: "ssh",
            id,
            host_id,
            connection_id: connection.connection_id,
            generation: capstan_core::sftp::ExactU64(connection.generation),
        })
    }

    #[tauri::command]
    pub async fn cancel_ssh_attempt(
        window: tauri::WebviewWindow,
        vault: State<'_, Arc<VaultManager>>,
        attempts: State<'_, SshAttempts>,
        broker: State<'_, Arc<TrustBroker>>,
        attempt_id: Uuid,
    ) -> std::result::Result<(), CommandError> {
        if window.label() != "main" {
            return Err(CommandError::new("window_source"));
        }
        broker.cancel_attempt(attempt_id);
        attempts.cancel(attempt_id).await;
        vault.cancel_connection_attempt(attempt_id).await;
        Ok(())
    }

    #[tauri::command]
    pub fn answer_host_trust(
        broker: State<'_, Arc<TrustBroker>>,
        challenge_id: Uuid,
        accepted: bool,
    ) -> std::result::Result<(), CommandError> {
        broker
            .respond(challenge_id, accepted)
            .map_err(|error| CommandError::new(error.code()))
    }

    #[tauri::command]
    pub fn vault_status(vault: State<'_, Arc<VaultManager>>) -> VaultStatus {
        vault.status()
    }

    #[cfg(windows)]
    async fn prompt_native_secret(
        window: &tauri::WebviewWindow<tauri::Wry>,
        title: String,
        message: String,
    ) -> std::result::Result<SecretText, CommandError> {
        let parent = window.hwnd().map(|handle| handle.0 as isize).unwrap_or(0);
        let prompt = tauri::async_runtime::spawn_blocking(move || {
            capstan_native_windows::prompt_secret(&title, &message, "", parent)
        })
        .await
        .map_err(|_| CommandError::new("task_failed"))?
        .map_err(|_| CommandError::new("native_credential_ui"))?
        .ok_or_else(|| CommandError::new("canceled"))?;
        let password = String::from_utf16(prompt.password())
            .map_err(|_| CommandError::new("invalid_input"))?;
        Ok(SecretText::new(password))
    }

    #[cfg(not(windows))]
    async fn prompt_native_secret(
        _: &tauri::WebviewWindow<tauri::Wry>,
        _: String,
        _: String,
    ) -> std::result::Result<SecretText, CommandError> {
        Err(CommandError::new("native_credential_ui"))
    }

    fn native_prompt_copy(
        locale: &str,
        master: bool,
        credential_name: Option<&str>,
    ) -> (String, String) {
        if locale.starts_with("zh") {
            if master {
                (
                    "Capstan 加密库".into(),
                    "输入加密库主密码；此窗口不会保存凭据。".into(),
                )
            } else {
                (
                    "Capstan SSH 凭据".into(),
                    format!(
                        "输入凭据“{}”的 SSH 密码。",
                        credential_name.unwrap_or("SSH")
                    ),
                )
            }
        } else if master {
            (
                "Capstan Vault".into(),
                "Enter the master password. This native prompt does not save credentials.".into(),
            )
        } else {
            (
                "Capstan SSH credential".into(),
                format!(
                    "Enter the SSH password for “{}”.",
                    credential_name.unwrap_or("SSH")
                ),
            )
        }
    }

    fn native_recovery_prompt_copy(locale: &str, restoring: bool) -> (String, String) {
        if locale.starts_with("zh") {
            if restoring {
                (
                    "恢复 Capstan 加密库".into(),
                    "再次输入保护所选恢复文件的密码。验证通过后，此文件将替换当前加密库；恢复候选不会自动覆盖文件。".into(),
                )
            } else {
                (
                    "查找 Capstan 加密库恢复文件".into(),
                    "输入恢复文件使用的主密码。更改主密码前创建的备份可能需要旧密码；密码只在 Rust 中用于验证。".into(),
                )
            }
        } else if restoring {
            (
                "Restore Capstan Vault".into(),
                "Enter the password protecting this recovery file again. After validation, this file will replace the current vault; candidates are never restored automatically.".into(),
            )
        } else {
            (
                "Find Capstan Vault recovery files".into(),
                "Enter the master password used by recovery files. Backups created before a password change may require the previous password. The password is used only in Rust for verification.".into(),
            )
        }
    }

    #[tauri::command]
    pub async fn initialize_vault(
        window: tauri::WebviewWindow<tauri::Wry>,
        vault: State<'_, Arc<VaultManager>>,
        activity: State<'_, UserActivity>,
        attempts: State<'_, SshAttempts>,
        locale: String,
    ) -> std::result::Result<(), CommandError> {
        let (title, message) = native_prompt_copy(&locale, true, None);
        vault
            .initialize(
                prompt_native_secret(&window, title, message).await?,
                KdfParameters::default(),
            )
            .await?;
        attempts.allow_new();
        activity.record();
        Ok(())
    }

    #[tauri::command]
    pub async fn unlock_vault(
        window: tauri::WebviewWindow<tauri::Wry>,
        vault: State<'_, Arc<VaultManager>>,
        activity: State<'_, UserActivity>,
        attempts: State<'_, SshAttempts>,
        locale: String,
    ) -> std::result::Result<(), CommandError> {
        let (title, message) = native_prompt_copy(&locale, true, None);
        vault
            .unlock(prompt_native_secret(&window, title, message).await?)
            .await?;
        attempts.allow_new();
        activity.record();
        Ok(())
    }

    #[tauri::command]
    pub async fn list_vault_recovery_candidates(
        window: tauri::WebviewWindow<tauri::Wry>,
        vault: State<'_, Arc<VaultManager>>,
        locale: String,
    ) -> std::result::Result<Vec<VaultRecoveryCandidateView>, CommandError> {
        let (title, message) = native_recovery_prompt_copy(&locale, false);
        vault
            .list_recovery_candidates(prompt_native_secret(&window, title, message).await?)
            .await
            .map_err(Into::into)
    }

    #[tauri::command]
    pub async fn restore_vault_recovery_candidate(
        window: tauri::WebviewWindow<tauri::Wry>,
        vault: State<'_, Arc<VaultManager>>,
        activity: State<'_, UserActivity>,
        attempts: State<'_, SshAttempts>,
        token: Uuid,
        locale: String,
    ) -> std::result::Result<(), CommandError> {
        let (title, message) = native_recovery_prompt_copy(&locale, true);
        vault
            .restore_recovery_candidate(token, prompt_native_secret(&window, title, message).await?)
            .await?;
        attempts.allow_new();
        activity.record();
        Ok(())
    }

    #[tauri::command]
    #[allow(clippy::too_many_arguments)] // Managed workflow services are injected separately.
    pub async fn lock_vault(
        vault: State<'_, Arc<VaultManager>>,
        attempts: State<'_, SshAttempts>,
        broker: State<'_, Arc<TrustBroker>>,
        manager: State<'_, Arc<TerminalService>>,
        ssh_sessions: State<'_, SshSessions>,
        backup_imports: State<'_, BackupImportSessions>,
        transfers: State<'_, Arc<capstan_core::transfer::TransferService>>,
        #[cfg(windows)] editors: State<'_, Arc<capstan_core::remote_edit::RemoteEditorService>>,
    ) -> std::result::Result<(), CommandError> {
        backup_imports.clear();
        let preferences = vault.preferences().map_err(CommandError::from)?;
        lock_vault_and_cleanup(
            vault.inner(),
            &preferences,
            attempts.inner(),
            broker.inner(),
            manager.inner().clone(),
            ssh_sessions.inner(),
            transfers.inner(),
            #[cfg(windows)]
            Some(editors.inner()),
        )
        .await
    }

    #[tauri::command]
    pub fn vault_preferences(
        vault: State<'_, Arc<VaultManager>>,
    ) -> std::result::Result<Preferences, CommandError> {
        vault.preferences().map_err(Into::into)
    }

    #[tauri::command]
    pub async fn update_vault_preferences(
        vault: State<'_, Arc<VaultManager>>,
        auto_lock_minutes: Option<u16>,
        disconnect_on_lock: bool,
        expected_revision: u64,
    ) -> std::result::Result<u64, CommandError> {
        vault
            .update_preferences(
                Preferences {
                    auto_lock_minutes,
                    disconnect_on_lock,
                },
                expected_revision,
            )
            .await
            .map_err(Into::into)
    }

    #[tauri::command]
    pub fn record_user_activity(activity: State<'_, UserActivity>) {
        activity.record();
    }

    #[tauri::command]
    #[allow(clippy::too_many_arguments)] // Tauri injects each managed service separately.
    pub async fn confirm_transfer_exit(
        window: tauri::WebviewWindow,
        transfers: State<'_, Arc<capstan_core::transfer::TransferService>>,
        sftp: State<'_, Arc<capstan_core::sftp::SftpService>>,
        terminals: State<'_, Arc<TerminalService>>,
        attempts: State<'_, SshAttempts>,
        broker: State<'_, Arc<TrustBroker>>,
        vault: State<'_, Arc<VaultManager>>,
        backup_imports: State<'_, BackupImportSessions>,
        settings: State<'_, Arc<capstan_core::settings::Settings>>,
        #[cfg(windows)] editors: State<'_, Arc<capstan_core::remote_edit::RemoteEditorService>>,
    ) -> std::result::Result<(), CommandError> {
        if window.label() != "main" {
            return Err(CommandError::new("window_source"));
        }
        #[cfg(windows)]
        editors
            .prepare_exit(window.label())
            .map_err(|error| CommandError::new(error.to_string()))?;
        let result = capstan_core::application::prepare_file_workflow_exit(
            &settings,
            &transfers,
            &sftp,
            window.label(),
        )
        .await
        .map_err(|error| CommandError::new(error.to_string()));
        if let Err(error) = result {
            #[cfg(windows)]
            editors.abort_exit();
            return Err(error);
        }
        backup_imports.clear();
        attempts.block_and_cancel_all(&broker).await;
        terminals.close_all().await;
        vault.disconnect_ssh().await;
        window
            .destroy()
            .map_err(|_| CommandError::new("window_source"))
    }

    #[tauri::command]
    #[allow(clippy::too_many_arguments)] // Tauri injects each managed service separately.
    pub async fn check_vault_auto_lock(
        vault: State<'_, Arc<VaultManager>>,
        activity: State<'_, UserActivity>,
        attempts: State<'_, SshAttempts>,
        broker: State<'_, Arc<TrustBroker>>,
        manager: State<'_, Arc<TerminalService>>,
        ssh_sessions: State<'_, SshSessions>,
        backup_imports: State<'_, BackupImportSessions>,
        transfers: State<'_, Arc<capstan_core::transfer::TransferService>>,
        #[cfg(windows)] editors: State<'_, Arc<capstan_core::remote_edit::RemoteEditorService>>,
    ) -> std::result::Result<bool, CommandError> {
        let locked = maybe_auto_lock(
            vault.inner(),
            activity.inner(),
            attempts.inner(),
            broker.inner(),
            manager.inner().clone(),
            ssh_sessions.inner(),
            transfers.inner(),
            #[cfg(windows)]
            Some(editors.inner()),
        )
        .await?;
        if locked {
            backup_imports.clear();
        }
        Ok(locked)
    }

    fn validate_external_url(
        url: &str,
        confirmed_non_http: bool,
    ) -> std::result::Result<(), CommandError> {
        if url.is_empty() || url.len() > 8192 || url.chars().any(char::is_control) {
            return Err(CommandError::new("external_link_blocked"));
        }
        let scheme = url
            .split_once(':')
            .map(|(scheme, _)| scheme)
            .ok_or_else(|| CommandError::new("external_link_blocked"))?;
        let mut characters = scheme.chars();
        if !characters
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic())
            || !characters.all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
            })
            || matches!(
                scheme.to_ascii_lowercase().as_str(),
                "javascript" | "data" | "file"
            )
        {
            return Err(CommandError::new("external_link_blocked"));
        }
        if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") && !confirmed_non_http
        {
            return Err(CommandError::new("external_link_confirmation_required"));
        }
        Ok(())
    }

    #[tauri::command]
    pub fn open_external_url(
        url: String,
        confirmed_non_http: bool,
    ) -> std::result::Result<(), CommandError> {
        validate_external_url(&url, confirmed_non_http)?;
        #[cfg(windows)]
        {
            capstan_native_windows::open_external_url(&url)
                .map_err(|_| CommandError::new("external_open_failed"))
        }
        #[cfg(not(windows))]
        {
            Err(CommandError::new("external_open_unsupported"))
        }
    }

    #[tauri::command]
    pub async fn list_hosts(
        vault: State<'_, Arc<VaultManager>>,
    ) -> std::result::Result<Vec<HostView>, CommandError> {
        vault.hosts().map_err(Into::into)
    }

    #[tauri::command]
    pub async fn list_credentials(
        vault: State<'_, Arc<VaultManager>>,
    ) -> std::result::Result<Vec<CredentialView>, CommandError> {
        vault.credentials().map_err(Into::into)
    }

    #[tauri::command]
    pub async fn list_groups(
        vault: State<'_, Arc<VaultManager>>,
    ) -> std::result::Result<Vec<GroupView>, CommandError> {
        vault.groups().map_err(Into::into)
    }

    #[tauri::command]
    pub async fn list_agent_identities(
        provider: capstan_ssh::AgentProvider,
    ) -> std::result::Result<Vec<capstan_ssh::AgentIdentityView>, CommandError> {
        capstan_ssh::list_agent_identities(provider)
            .await
            .map_err(|error| CommandError::new(error.code()))
    }

    #[tauri::command]
    pub async fn create_agent_credential(
        vault: State<'_, Arc<VaultManager>>,
        name: String,
        agent: capstan_vault::AgentKind,
        fingerprint: String,
    ) -> std::result::Result<Uuid, CommandError> {
        vault
            .add_agent_credential(name, agent, fingerprint)
            .await
            .map_err(Into::into)
    }

    #[tauri::command]
    pub async fn create_password_credential(
        window: tauri::WebviewWindow<tauri::Wry>,
        vault: State<'_, Arc<VaultManager>>,
        name: String,
        locale: String,
    ) -> std::result::Result<Uuid, CommandError> {
        let (title, message) = native_prompt_copy(&locale, false, Some(&name));
        let secret = prompt_native_secret(&window, title, message).await?;
        let password = secret.expose().to_owned();
        vault
            .add_password_credential(name, password)
            .await
            .map_err(Into::into)
    }

    #[tauri::command]
    pub async fn import_private_key_credential(
        window: tauri::WebviewWindow<tauri::Wry>,
        vault: State<'_, Arc<VaultManager>>,
        name: String,
        locale: String,
    ) -> std::result::Result<Uuid, CommandError> {
        if name.trim().is_empty() || name.len() > 256 {
            return Err(CommandError::new("invalid_input"));
        }
        let parent = window.hwnd().map(|handle| handle.0 as isize).unwrap_or(0);
        let title = if locale.starts_with("zh") {
            "选择 SSH 私钥"
        } else {
            "Select SSH private key"
        };
        let path = tauri::async_runtime::spawn_blocking(move || {
            capstan_native_windows::pick_private_key_file(title, parent)
        })
        .await
        .map_err(|_| CommandError::new("task_failed"))?
        .map_err(|_| CommandError::new("native_file_picker"))?
        .ok_or_else(|| CommandError::new("canceled"))?;
        let file = std::fs::File::open(path).map_err(|_| CommandError::new("credential_file"))?;
        let metadata = file
            .metadata()
            .map_err(|_| CommandError::new("credential_file"))?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 1024 * 1024 {
            return Err(CommandError::new("credential_file"));
        }
        let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(metadata.len() as usize));
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| CommandError::new("credential_file"))?;
        if bytes.is_empty() || bytes.len() > 1024 * 1024 {
            return Err(CommandError::new("credential_file"));
        }
        let mut content = match String::from_utf8(std::mem::take(&mut *bytes)) {
            Ok(content) => zeroize::Zeroizing::new(content),
            Err(error) => {
                let _bytes = zeroize::Zeroizing::new(error.into_bytes());
                return Err(CommandError::new("credential_file"));
            }
        };
        let passphrase = if capstan_ssh::private_key_requires_passphrase(&content) {
            let (title, message) = if locale.starts_with("zh") {
                (
                    "SSH 私钥口令".to_owned(),
                    format!("输入私钥“{name}”的口令。"),
                )
            } else {
                (
                    "SSH private key passphrase".to_owned(),
                    format!("Enter the passphrase for “{name}”."),
                )
            };
            Some(prompt_native_secret(&window, title, message).await?)
        } else {
            None
        };
        let passphrase = passphrase.map(|secret| secret.expose().to_owned());
        let result = vault
            .add_imported_key_credential(name, std::mem::take(&mut *content), passphrase)
            .await;
        result.map_err(Into::into)
    }

    #[tauri::command]
    pub async fn add_private_key_file_credential(
        window: tauri::WebviewWindow<tauri::Wry>,
        vault: State<'_, Arc<VaultManager>>,
        name: String,
        locale: String,
    ) -> std::result::Result<Uuid, CommandError> {
        if name.trim().is_empty() || name.len() > 256 {
            return Err(CommandError::new("invalid_input"));
        }
        let parent = window.hwnd().map(|handle| handle.0 as isize).unwrap_or(0);
        let title = if locale.starts_with("zh") {
            "选择 SSH 私钥文件"
        } else {
            "Select SSH private key file"
        };
        let path = tauri::async_runtime::spawn_blocking(move || {
            capstan_native_windows::pick_private_key_file(title, parent)
        })
        .await
        .map_err(|_| CommandError::new("task_failed"))?
        .map_err(|_| CommandError::new("native_file_picker"))?
        .ok_or_else(|| CommandError::new("canceled"))?;
        let file = std::fs::File::open(&path).map_err(|_| CommandError::new("credential_file"))?;
        let metadata = file
            .metadata()
            .map_err(|_| CommandError::new("credential_file"))?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 1024 * 1024 {
            return Err(CommandError::new("credential_file"));
        }
        let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(metadata.len() as usize));
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| CommandError::new("credential_file"))?;
        if bytes.is_empty() || bytes.len() > 1024 * 1024 {
            return Err(CommandError::new("credential_file"));
        }
        let content = match String::from_utf8(std::mem::take(&mut *bytes)) {
            Ok(content) => zeroize::Zeroizing::new(content),
            Err(error) => {
                let _bytes = zeroize::Zeroizing::new(error.into_bytes());
                return Err(CommandError::new("credential_file"));
            }
        };
        let passphrase = if capstan_ssh::private_key_requires_passphrase(&content) {
            let (title, message) = if locale.starts_with("zh") {
                (
                    "SSH 私钥口令".to_owned(),
                    format!("输入私钥“{name}”的口令。"),
                )
            } else {
                (
                    "SSH private key passphrase".to_owned(),
                    format!("Enter the passphrase for “{name}”."),
                )
            };
            Some(prompt_native_secret(&window, title, message).await?)
        } else {
            None
        };
        let passphrase =
            passphrase.map(|secret| zeroize::Zeroizing::new(secret.expose().to_owned()));
        let result = vault
            .add_key_file_credential(
                name,
                path,
                passphrase.map(|mut value| std::mem::take(&mut *value)),
            )
            .await;
        drop(content);
        result.map_err(Into::into)
    }

    #[tauri::command]
    pub async fn rename_credential(
        vault: State<'_, Arc<VaultManager>>,
        id: Uuid,
        name: String,
        expected_revision: u64,
    ) -> std::result::Result<Uuid, CommandError> {
        vault
            .rename_credential(id, name, expected_revision)
            .await
            .map_err(Into::into)
    }

    #[tauri::command]
    pub async fn update_password_credential(
        window: tauri::WebviewWindow<tauri::Wry>,
        vault: State<'_, Arc<VaultManager>>,
        id: Uuid,
        expected_revision: u64,
        locale: String,
    ) -> std::result::Result<Uuid, CommandError> {
        let credential = vault
            .credentials()
            .map_err(CommandError::from)?
            .into_iter()
            .find(|candidate| candidate.id == id)
            .ok_or_else(|| CommandError::new("not_found"))?;
        if credential.kind != "password" {
            return Err(CommandError::new("invalid_input"));
        }
        let (title, message) = native_prompt_copy(&locale, false, Some(&credential.name));
        let secret = prompt_native_secret(&window, title, message).await?;
        vault
            .save_password_credential(
                credential.name,
                secret.expose().to_owned(),
                Some(CredentialUpdate {
                    id,
                    expected_revision,
                }),
            )
            .await
            .map_err(Into::into)
    }

    #[tauri::command]
    pub async fn delete_credential(
        vault: State<'_, Arc<VaultManager>>,
        id: Uuid,
        expected_revision: u64,
    ) -> std::result::Result<u64, CommandError> {
        vault
            .delete_credential(id, expected_revision)
            .await
            .map_err(Into::into)
    }

    #[tauri::command]
    pub async fn change_vault_master_password(
        window: tauri::WebviewWindow<tauri::Wry>,
        vault: State<'_, Arc<VaultManager>>,
        activity: State<'_, UserActivity>,
        expected_revision: u64,
        locale: String,
    ) -> std::result::Result<(), CommandError> {
        let (title, message) = if locale.starts_with("zh") {
            (
                "更改加密库主密码".to_owned(),
                "输入新的加密库主密码（至少 12 个字符）。".to_owned(),
            )
        } else {
            (
                "Change Vault master password".to_owned(),
                "Enter a new Vault master password (at least 12 characters).".to_owned(),
            )
        };
        let password = prompt_native_secret(&window, title, message).await?;
        vault
            .change_master_password(password, expected_revision)
            .await?;
        activity.record();
        Ok(())
    }

    #[tauri::command]
    pub async fn export_vault_backup(
        window: tauri::WebviewWindow<tauri::Wry>,
        vault: State<'_, Arc<VaultManager>>,
        activity: State<'_, UserActivity>,
        locale: String,
    ) -> std::result::Result<(), CommandError> {
        let (title, message) = if locale.starts_with("zh") {
            (
                "导出加密库备份".to_owned(),
                "为此 .cbak 文件设置独立口令。".to_owned(),
            )
        } else {
            (
                "Export encrypted Vault backup".to_owned(),
                "Choose a separate passphrase for this .cbak backup.".to_owned(),
            )
        };
        let password = prompt_native_secret(&window, title, message).await?;
        let parent = window.hwnd().map(|handle| handle.0 as isize).unwrap_or(0);
        let title = if locale.starts_with("zh") {
            "保存 Capstan 加密备份"
        } else {
            "Save Capstan encrypted backup"
        };
        let destination = tauri::async_runtime::spawn_blocking(move || {
            capstan_native_windows::save_vault_backup_file(title, parent)
        })
        .await
        .map_err(|_| CommandError::new("task_failed"))?
        .map_err(|_| CommandError::new("native_file_picker"))?
        .ok_or_else(|| CommandError::new("canceled"))?;
        vault
            .export_backup_replace(destination, password)
            .await
            .map_err(CommandError::from)?;
        activity.record();
        Ok(())
    }

    #[tauri::command]
    pub async fn preview_vault_backup(
        window: tauri::WebviewWindow<tauri::Wry>,
        vault: State<'_, Arc<VaultManager>>,
        sessions: State<'_, BackupImportSessions>,
        expected_revision: u64,
        expected_epoch: u64,
        locale: String,
    ) -> std::result::Result<PendingBackupPreview, CommandError> {
        let parent = window.hwnd().map(|handle| handle.0 as isize).unwrap_or(0);
        let title = if locale.starts_with("zh") {
            "选择 Capstan 加密备份"
        } else {
            "Select Capstan encrypted backup"
        };
        let source = tauri::async_runtime::spawn_blocking(move || {
            capstan_native_windows::pick_vault_backup_file(title, parent)
        })
        .await
        .map_err(|_| CommandError::new("task_failed"))?
        .map_err(|_| CommandError::new("native_file_picker"))?
        .ok_or_else(|| CommandError::new("canceled"))?;
        let (title, message) = if locale.starts_with("zh") {
            (
                "预览加密库备份".to_owned(),
                "输入此 .cbak 文件的独立口令以查看合并预览。".to_owned(),
            )
        } else {
            (
                "Preview Vault backup".to_owned(),
                "Enter this .cbak file's passphrase to preview its merge.".to_owned(),
            )
        };
        let password = prompt_native_secret(&window, title, message).await?;
        let preview = vault
            .preview_backup(source.clone(), password.clone())
            .await
            .map_err(CommandError::from)?;
        if preview.base_revision != expected_revision {
            return Err(CommandError::new("vault_conflict"));
        }
        let token = sessions.insert(source, password, expected_revision, expected_epoch)?;
        Ok(PendingBackupPreview { token, preview })
    }

    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct PendingBackupPreview {
        pub token: Uuid,
        pub preview: capstan_core::BackupPreview,
    }

    #[tauri::command]
    pub fn cancel_vault_backup_preview(sessions: State<'_, BackupImportSessions>, token: Uuid) {
        sessions.cancel(token);
    }

    #[tauri::command]
    pub async fn import_vault_backup(
        vault: State<'_, Arc<VaultManager>>,
        activity: State<'_, UserActivity>,
        sessions: State<'_, BackupImportSessions>,
        token: Uuid,
        mode: capstan_core::BackupImportMode,
        expected_revision: u64,
        expected_epoch: u64,
    ) -> std::result::Result<u64, CommandError> {
        let pending = sessions.take(token, expected_revision, expected_epoch)?;
        let revision = vault
            .import_backup(
                pending.source,
                pending.password,
                mode,
                expected_revision,
                expected_epoch,
            )
            .await
            .map_err(CommandError::from)?;
        activity.record();
        Ok(revision)
    }

    #[tauri::command]
    pub fn list_trusted_host_keys(
        vault: State<'_, Arc<VaultManager>>,
    ) -> std::result::Result<Vec<TrustedHostKeyView>, CommandError> {
        vault.trusted_host_keys().map_err(Into::into)
    }

    #[tauri::command]
    pub async fn forget_trusted_host_key(
        vault: State<'_, Arc<VaultManager>>,
        activity: State<'_, UserActivity>,
        target: String,
        port: u16,
        fingerprint: String,
        expected_revision: u64,
    ) -> std::result::Result<u64, CommandError> {
        let revision = vault
            .forget_trusted_host_key(target, port, fingerprint, expected_revision)
            .await
            .map_err(CommandError::from)?;
        activity.record();
        Ok(revision)
    }

    #[tauri::command]
    pub async fn save_host(
        vault: State<'_, Arc<VaultManager>>,
        host: Host,
        expected_revision: u64,
    ) -> std::result::Result<u64, CommandError> {
        vault
            .save_host(host, expected_revision)
            .await
            .map_err(Into::into)
    }

    #[tauri::command]
    pub async fn delete_host(
        vault: State<'_, Arc<VaultManager>>,
        id: Uuid,
        expected_revision: u64,
    ) -> std::result::Result<u64, CommandError> {
        vault
            .delete_host(id, expected_revision)
            .await
            .map_err(Into::into)
    }

    #[tauri::command]
    pub async fn save_group(
        vault: State<'_, Arc<VaultManager>>,
        group: Group,
        expected_revision: u64,
    ) -> std::result::Result<u64, CommandError> {
        vault
            .save_group(group, expected_revision)
            .await
            .map_err(Into::into)
    }

    #[tauri::command]
    pub async fn delete_group(
        vault: State<'_, Arc<VaultManager>>,
        id: Uuid,
        expected_revision: u64,
    ) -> std::result::Result<u64, CommandError> {
        vault
            .delete_group(id, expected_revision)
            .await
            .map_err(Into::into)
    }

    pub fn forward_status(
        session: Arc<capstan_terminal::TerminalSession>,
        on_status: Channel<TerminalSnapshot>,
    ) {
        tauri::async_runtime::spawn(async move {
            let mut status = session.subscribe();
            loop {
                let snapshot = status.borrow_and_update().clone();
                let done = snapshot.is_finished();
                if on_status.send(snapshot).is_err() {
                    session.close().await;
                    break;
                }
                if done || status.changed().await.is_err() {
                    break;
                }
            }
        });
    }

    #[tauri::command]
    pub async fn write_terminal_input(
        manager: State<'_, Arc<TerminalService>>,
        id: TerminalSessionId,
        input: String,
    ) -> std::result::Result<(), CommandError> {
        #[cfg(feature = "m0-benchmarks")]
        crate::benchmark::trace_input(&id, "begin");
        let result = manager
            .get(&id)?
            .write(input.into_bytes())
            .await
            .map_err(Into::into);
        #[cfg(feature = "m0-benchmarks")]
        crate::benchmark::trace_input(&id, if result.is_ok() { "ok" } else { "error" });
        result
    }

    #[tauri::command]
    pub async fn resize_terminal(
        manager: State<'_, Arc<TerminalService>>,
        id: TerminalSessionId,
        columns: u16,
        rows: u16,
    ) -> std::result::Result<(), CommandError> {
        manager
            .get(&id)?
            .resize(columns, rows)
            .await
            .map_err(Into::into)
    }

    #[tauri::command]
    pub async fn ack_terminal_output(
        manager: State<'_, Arc<TerminalService>>,
        id: TerminalSessionId,
        bytes: usize,
    ) -> std::result::Result<(), CommandError> {
        manager.get(&id)?.ack(bytes).await.map_err(Into::into)
    }

    #[tauri::command]
    pub async fn stop_terminal(
        vault: State<'_, Arc<VaultManager>>,
        manager: State<'_, Arc<TerminalService>>,
        ssh_sessions: State<'_, SshSessions>,
        sftp: State<'_, Arc<capstan_core::sftp::SftpService>>,
        id: TerminalSessionId,
    ) -> std::result::Result<(), CommandError> {
        let owner = ssh_sessions.remove(&id);
        sftp.revoke_terminal(&id.0);
        let result = manager.close(&id).await.map_err(Into::into);
        if let Some(owner) = owner {
            vault.release_connection_owner_and_wait(owner).await;
        }
        result
    }

    #[tauri::command]
    async fn runtime_summary_command() -> std::result::Result<RuntimeSummary, CommandError> {
        tauri::async_runtime::spawn_blocking(crate::runtime_summary)
            .await
            .map_err(|_| CommandError::new("task_failed"))
    }

    #[tauri::command]
    fn minimize_window(
        window: tauri::WebviewWindow<tauri::Wry>,
    ) -> std::result::Result<(), CommandError> {
        window
            .minimize()
            .map_err(|_| CommandError::new("window_operation"))
    }
    #[tauri::command]
    fn toggle_maximize_window(
        window: tauri::WebviewWindow<tauri::Wry>,
    ) -> std::result::Result<(), CommandError> {
        (if window
            .is_maximized()
            .map_err(|_| CommandError::new("window_operation"))?
        {
            window.unmaximize()
        } else {
            window.maximize()
        })
        .map_err(|_| CommandError::new("window_operation"))
    }
    #[tauri::command]
    fn close_window(
        window: tauri::WebviewWindow<tauri::Wry>,
    ) -> std::result::Result<(), CommandError> {
        window
            .close()
            .map_err(|_| CommandError::new("window_operation"))
    }

    pub fn register_commands(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
        #[cfg(feature = "m0-benchmarks")]
        let builder = builder.manage(crate::benchmark::BenchmarkState::default());
        builder
            .plugin(tauri_plugin_snap_layout::init())
            .manage(Arc::new(TerminalService::default()))
            .manage(SshSessions::default())
            .manage(Arc::new(capstan_core::sftp::SftpService::default()))
            .manage(Arc::new(
                capstan_core::shell_integration::PreviewRegistry::default(),
            ))
            .manage(Arc::new(capstan_core::transfer::TransferService::default()))
            .manage(BackupImportSessions::default())
            .manage(UserActivity::default())
            .setup(|app| {
                #[cfg(windows)]
                let roaming_data_directory = std::env::var_os("APPDATA")
                    .map(std::path::PathBuf::from)
                    .ok_or_else(|| {
                        Box::<dyn std::error::Error>::from(std::io::Error::other(
                            "APPDATA unavailable",
                        ))
                    })?
                    .join("Capstan");
                #[cfg(not(windows))]
                let roaming_data_directory = app.path().app_data_dir().map_err(|_| {
                    Box::<dyn std::error::Error>::from(std::io::Error::other(
                        "application data directory unavailable",
                    ))
                })?;
                let executable = std::env::current_exe()?;
                let data_directory = crate::data_directory::resolve_data_directory(
                    &executable,
                    &roaming_data_directory,
                )?;
                app.state::<Arc<capstan_core::transfer::TransferService>>()
                    .configure_recovery_directory(data_directory.clone())
                    .map_err(|_| {
                        Box::<dyn std::error::Error>::from(std::io::Error::other(
                            "transfer recovery storage unavailable",
                        ))
                    })?;
                let settings = Arc::new(capstan_core::settings::Settings::new(
                    data_directory.clone(),
                ));
                app.manage(settings.clone());
                #[cfg(windows)]
                app.state::<Arc<capstan_core::transfer::TransferService>>()
                    .configure_export_directory(
                        std::env::temp_dir().join("Capstan").join("exports"),
                    )?;
                #[cfg(windows)]
                app.manage(capstan_core::remote_edit::RemoteEditorService::new(
                    settings,
                    capstan_native_windows::editor::EditorStore::default(),
                    app.state::<Arc<capstan_core::transfer::TransferService>>()
                        .publication_gate(),
                ));
                app.manage(Arc::new(VaultManager::new(
                    data_directory.join("vault.capstan"),
                )));
                app.manage(SshAttempts::default());
                let auto_lock_backup_imports = app.state::<BackupImportSessions>().inner().clone();
                let auto_lock_vault = app.state::<Arc<VaultManager>>().inner().clone();
                let auto_lock_activity = app.state::<UserActivity>().inner().clone();
                let auto_lock_attempts = app.state::<SshAttempts>().inner().clone();
                let (broker, mut challenges) = TrustBroker::new();
                app.manage(broker.clone());
                // TaskTracker::spawn requires an active Tokio runtime during synchronous setup.
                tauri::async_runtime::block_on(async {
                    crate::connection_commands::install(app)?;
                    crate::forwarding_commands::install(app);
                    Ok::<(), Box<dyn std::error::Error>>(())
                })?;
                let auto_lock_broker = broker.clone();
                let auto_lock_terminals = app.state::<Arc<TerminalService>>().inner().clone();
                let auto_lock_sessions = app.state::<SshSessions>().inner().clone();
                let auto_lock_transfers = app
                    .state::<Arc<capstan_core::transfer::TransferService>>()
                    .inner()
                    .clone();
                #[cfg(windows)]
                let auto_lock_editors = app
                    .state::<Arc<capstan_core::remote_edit::RemoteEditorService>>()
                    .inner()
                    .clone();
                let auto_lock_window = app.get_webview_window("main");
                tauri::async_runtime::spawn(async move {
                    let mut ticker = tokio::time::interval(Duration::from_secs(15));
                    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    loop {
                        ticker.tick().await;
                        if let Ok(true) = maybe_auto_lock(
                            &auto_lock_vault,
                            &auto_lock_activity,
                            &auto_lock_attempts,
                            &auto_lock_broker,
                            auto_lock_terminals.clone(),
                            &auto_lock_sessions,
                            &auto_lock_transfers,
                            #[cfg(windows)]
                            Some(&auto_lock_editors),
                        )
                        .await
                        {
                            auto_lock_backup_imports.clear();
                            if let Some(window) = &auto_lock_window {
                                let _ = window.emit("vault-auto-locked", ());
                            }
                        }
                    }
                });
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    while let Some(challenge) = challenges.recv().await {
                        let attempt_id = challenge.attempt_id;
                        if handle.emit("ssh-trust-challenge", challenge).is_err() {
                            broker.cancel_attempt(attempt_id);
                        }
                    }
                });
                // Builder::setup replaces earlier hooks; initialize state and the window together.
                if let Some(window) = app.get_webview_window("main") {
                    #[cfg(feature = "m0-benchmarks")]
                    if std::env::var("CAPSTAN_M0_VISIBLE").is_ok_and(|value| value == "1") {
                        window.show()?;
                        window.set_focus()?;
                    }
                    #[cfg(target_os = "windows")]
                    let _ = window.set_effects(
                        tauri::window::EffectsBuilder::new()
                            .effect(tauri::window::Effect::Mica)
                            .build(),
                    );
                }
                Ok(())
            })
            .invoke_handler(tauri::generate_handler![
                runtime_summary_command,
                vault_status,
                initialize_vault,
                unlock_vault,
                list_vault_recovery_candidates,
                restore_vault_recovery_candidate,
                lock_vault,
                vault_preferences,
                update_vault_preferences,
                record_user_activity,
                check_vault_auto_lock,
                open_external_url,
                list_hosts,
                list_credentials,
                list_groups,
                crate::snippet_commands::list_snippets,
                crate::snippet_commands::list_snippet_groups,
                crate::snippet_commands::save_snippet,
                crate::snippet_commands::save_snippet_group,
                crate::snippet_commands::delete_snippet,
                crate::snippet_commands::delete_snippet_group,
                list_trusted_host_keys,
                list_agent_identities,
                create_password_credential,
                create_agent_credential,
                import_private_key_credential,
                add_private_key_file_credential,
                rename_credential,
                update_password_credential,
                delete_credential,
                change_vault_master_password,
                export_vault_backup,
                preview_vault_backup,
                cancel_vault_backup_preview,
                import_vault_backup,
                save_host,
                delete_host,
                save_group,
                delete_group,
                forget_trusted_host_key,
                start_ssh_terminal,
                crate::connection_commands::connection_snapshots,
                crate::connection_commands::reconnect_connection,
                crate::connection_commands::disconnect_connection,
                crate::forwarding_commands::list_tunnels,
                crate::forwarding_commands::tunnel_runtime_snapshots,
                crate::forwarding_commands::save_tunnel,
                crate::forwarding_commands::delete_tunnel,
                crate::forwarding_commands::start_tunnel,
                crate::forwarding_commands::stop_tunnel,
                crate::sftp_commands::attach_sftp,
                crate::settings_commands::load_sftp_panel_preferences,
                crate::settings_commands::save_sftp_panel_preferences,
                crate::sftp_commands::detach_sftp,
                crate::sftp_commands::update_remote_cwd,
                crate::sftp_commands::configure_remote_cwd_aliases,
                crate::sftp_commands::list_remote_directory,
                crate::sftp_commands::next_remote_directory_page,
                crate::sftp_commands::release_remote_entries,
                crate::sftp_commands::create_remote_entry,
                crate::sftp_commands::prepare_remote_mutation,
                crate::sftp_commands::apply_remote_mutation,
                crate::sftp_commands::prepare_remote_tree_delete,
                crate::sftp_commands::apply_remote_tree_delete,
                crate::sftp_commands::cancel_remote_tree_delete,
                crate::sftp_commands::preview_shell_integration,
                crate::sftp_commands::apply_shell_integration,
                #[cfg(windows)]
                crate::remote_edit_commands::select_remote_editor,
                #[cfg(windows)]
                crate::remote_edit_commands::begin_remote_edit,
                #[cfg(windows)]
                crate::remote_edit_commands::remote_editor_configuration,
                #[cfg(windows)]
                crate::remote_edit_commands::remote_edit_snapshot,
                #[cfg(windows)]
                crate::remote_edit_commands::discover_remote_edit_residuals,
                #[cfg(windows)]
                crate::remote_edit_commands::retry_remote_edit,
                crate::remote_edit_commands::cleanup_remote_edit_temporaries,
                #[cfg(windows)]
                crate::remote_edit_commands::resolve_remote_edit_conflict,
                #[cfg(windows)]
                crate::remote_edit_commands::close_remote_edit,
                crate::transfer_commands::select_transfer_source,
                crate::transfer_commands::enqueue_native_drop,
                crate::transfer_commands::arm_native_drop,
                crate::transfer_commands::disarm_native_drop,
                crate::transfer_commands::prepare_native_export,
                crate::transfer_commands::native_export_snapshot,
                crate::transfer_commands::release_native_export,
                crate::transfer_commands::start_native_export,
                crate::transfer_commands::select_transfer_folder_plan,
                crate::transfer_commands::select_transfer_folder_download_plan,
                crate::transfer_commands::resolve_transfer_tree_plan,
                crate::transfer_commands::release_transfer_tree_plan,
                crate::transfer_commands::select_transfer_destination,
                crate::transfer_commands::release_transfer_selection,
                crate::transfer_commands::enqueue_transfer,
                crate::transfer_commands::transfer_snapshot,
                crate::transfer_commands::transfer_recovery_snapshot,
                crate::transfer_commands::retry_recovered_transfer_upload,
                crate::transfer_commands::cleanup_recovered_transfer_upload,
                crate::transfer_commands::select_recovered_transfer_destination,
                crate::transfer_commands::retry_recovered_transfer_download,
                crate::transfer_commands::cleanup_recovered_transfer_download,
                crate::transfer_commands::cancel_transfer,
                crate::transfer_commands::pause_transfer,
                crate::transfer_commands::resume_transfer,
                crate::transfer_commands::retry_transfer,
                crate::transfer_commands::retry_transfer_on_session,
                crate::transfer_commands::retry_transfer_with_selection,
                crate::transfer_commands::resolve_transfer_conflict,
                confirm_transfer_exit,
                cancel_ssh_attempt,
                answer_host_trust,
                list_local_shells,
                start_local_terminal,
                write_terminal_input,
                resize_terminal,
                ack_terminal_output,
                stop_terminal,
                minimize_window,
                toggle_maximize_window,
                close_window,
                #[cfg(feature = "m0-benchmarks")]
                crate::benchmark::m0_benchmark_enabled,
                #[cfg(feature = "m0-benchmarks")]
                crate::benchmark::m0_benchmark_mixed,
                #[cfg(feature = "m0-benchmarks")]
                crate::benchmark::m0_start_fixture,
                #[cfg(feature = "m0-benchmarks")]
                crate::benchmark::m0_source_sample,
                #[cfg(feature = "m0-benchmarks")]
                crate::benchmark::m0_registered_sessions,
                #[cfg(all(windows, feature = "m0-benchmarks"))]
                crate::benchmark::transfer_load::m2_benchmark_round,
                #[cfg(all(not(windows), feature = "m0-benchmarks"))]
                crate::benchmark::m2_benchmark_round,
                #[cfg(all(windows, feature = "m0-benchmarks"))]
                crate::benchmark::transfer_load::m2_start_load,
                #[cfg(all(windows, feature = "m0-benchmarks"))]
                crate::benchmark::transfer_load::m2_load_active,
                #[cfg(all(windows, feature = "m0-benchmarks"))]
                crate::benchmark::transfer_load::m2_finish_load,
                #[cfg(all(windows, feature = "m0-benchmarks"))]
                crate::benchmark::transfer_load::m2_cancel_load,
                #[cfg(feature = "m0-benchmarks")]
                crate::benchmark::m0_finish_report
            ])
            .on_webview_event(|webview, event| {
                #[cfg(windows)]
                if webview.label() == "main" {
                    if let tauri::WebviewEvent::DragDrop(tauri::DragDropEvent::Drop {
                        paths,
                        position,
                    }) = event
                    {
                        let transfers =
                            webview.state::<Arc<capstan_core::transfer::TransferService>>();
                        if let Ok(token) =
                            transfers.capture_native_drop(webview.label(), paths.clone())
                        {
                            #[derive(Clone, serde::Serialize)]
                            struct DropReceipt {
                                token: uuid::Uuid,
                                x: f64,
                                y: f64,
                            }
                            let scale = webview.window().scale_factor().unwrap_or(1.0);
                            let _ = webview.emit(
                                "native-transfer-drop",
                                DropReceipt {
                                    token,
                                    x: position.x / scale,
                                    y: position.y / scale,
                                },
                            );
                        }
                    }
                }
            })
            .on_window_event(|window, event| {
                if window.label() == "main" && matches!(event, WindowEvent::Focused(true)) {
                    let vault = window.state::<Arc<VaultManager>>().inner().clone();
                    let activity = window.state::<UserActivity>().inner().clone();
                    let attempts = window.state::<SshAttempts>().inner().clone();
                    let broker = window.state::<Arc<TrustBroker>>().inner().clone();
                    let terminals = window.state::<Arc<TerminalService>>().inner().clone();
                    let ssh_sessions = window.state::<SshSessions>().inner().clone();
                    let backup_imports = window.state::<BackupImportSessions>().inner().clone();
                    let transfers = window
                        .state::<Arc<capstan_core::transfer::TransferService>>()
                        .inner()
                        .clone();
                    #[cfg(windows)]
                    let editors = window
                        .state::<Arc<capstan_core::remote_edit::RemoteEditorService>>()
                        .inner()
                        .clone();
                    let window = window.clone();
                    tauri::async_runtime::spawn(async move {
                        if matches!(
                            maybe_auto_lock(
                                &vault,
                                &activity,
                                &attempts,
                                &broker,
                                terminals,
                                &ssh_sessions,
                                &transfers,
                                #[cfg(windows)]
                                Some(&editors),
                            )
                            .await,
                            Ok(true)
                        ) {
                            backup_imports.clear();
                            let _ = window.emit("vault-auto-locked", ());
                        }
                    });
                }
                if let WindowEvent::CloseRequested { api, .. } = event {
                    if window.label() == "main" {
                        api.prevent_close();
                        #[cfg(windows)]
                        {
                            let editors = window
                                .state::<Arc<capstan_core::remote_edit::RemoteEditorService>>();
                            if editors.has_open(window.label()) {
                                #[derive(Clone, serde::Serialize)]
                                struct EditorExit {
                                    error: &'static str,
                                }
                                let _ = window.emit(
                                    "transfer-exit-requested",
                                    EditorExit {
                                        error: "editor_residual",
                                    },
                                );
                                return;
                            }
                        }
                        let transfers = window
                            .state::<Arc<capstan_core::transfer::TransferService>>()
                            .inner()
                            .clone();
                        let sftp = window
                            .state::<Arc<capstan_core::sftp::SftpService>>()
                            .inner()
                            .clone();
                        if transfers.has_active(window.label()).unwrap_or(true)
                            || sftp.has_active_tree_deletes(window.label()).unwrap_or(true)
                        {
                            let _ = window.emit("transfer-exit-requested", ());
                            return;
                        }
                        #[cfg(windows)]
                        let editors = window
                            .state::<Arc<capstan_core::remote_edit::RemoteEditorService>>()
                            .inner()
                            .clone();
                        #[cfg(windows)]
                        if let Err(error) = editors.prepare_exit(window.label()) {
                            #[derive(Clone, serde::Serialize)]
                            struct EditorExit {
                                error: String,
                            }
                            let _ = window.emit(
                                "transfer-exit-requested",
                                EditorExit {
                                    error: error.to_string(),
                                },
                            );
                            return;
                        }
                        window.state::<BackupImportSessions>().inner().clear();
                        let manager = window.state::<Arc<TerminalService>>().inner().clone();
                        let attempts = window.state::<SshAttempts>().inner().clone();
                        let broker = window.state::<Arc<TrustBroker>>().inner().clone();
                        let settings = window
                            .state::<Arc<capstan_core::settings::Settings>>()
                            .inner()
                            .clone();
                        let window = window.clone();
                        tauri::async_runtime::spawn(async move {
                            let result = capstan_core::application::prepare_file_workflow_exit(
                                &settings,
                                &transfers,
                                &sftp,
                                window.label(),
                            )
                            .await
                            .map_err(|error| error.to_string());
                            if let Err(error) = result {
                                #[cfg(windows)]
                                editors.abort_exit();
                                #[derive(Clone, serde::Serialize)]
                                struct ExitFailure {
                                    error: String,
                                }
                                let _ = window.emit(
                                    "transfer-exit-requested",
                                    ExitFailure {
                                        error: error.to_string(),
                                    },
                                );
                                return;
                            }
                            attempts.block_and_cancel_all(&broker).await;
                            crate::forwarding_commands::shutdown(&window).await;
                            crate::connection_commands::shutdown(&window).await;
                            manager.close_all().await;
                            let _ = window.destroy();
                        });
                    }
                }
            })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use capstan_terminal::{BackendControl, BackendParts, FrameSink};
        use std::io::Cursor;

        struct QuietSink;
        impl FrameSink for QuietSink {
            fn send<'a>(
                &'a self,
                _: BinaryFrame,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>>
            {
                Box::pin(async { Ok(()) })
            }
        }

        struct TestControl;
        impl BackendControl for TestControl {
            fn resize(&self, _: u16, _: u16) -> Result<()> {
                Ok(())
            }
            fn try_wait(&self) -> Result<Option<u32>> {
                Ok(None)
            }
            fn close(&self, _: bool) -> Result<Option<u32>> {
                Ok(None)
            }
        }

        #[test]
        fn channel_frames_avoid_the_inline_json_path_and_preserve_payload() {
            for length in [4, 1004, 64 * 1024] {
                let frame = BinaryFrame {
                    sequence: 42,
                    payload: vec![17; length],
                };
                let canonical = encode_binary_frame(frame.clone()).unwrap();
                let wire = encode_channel_frame(frame).unwrap();
                assert_eq!(wire.len(), canonical.len().max(1024));
                assert_eq!(wire[..canonical.len()], canonical);
                assert!(wire[canonical.len()..].iter().all(|byte| *byte == 0));
            }
        }

        #[tokio::test(start_paused = true)]
        async fn user_activity_resets_the_idle_deadline() {
            let activity = UserActivity::default();
            tokio::time::advance(Duration::from_secs(60)).await;
            assert!(activity.claim_if_idle(Duration::from_secs(30)));
            assert!(!activity.claim_if_idle(Duration::from_secs(30)));
            activity.record();
            assert!(!activity.claim_if_idle(Duration::from_secs(30)));
        }

        #[test]
        fn backup_import_preview_is_revision_bound_single_use_and_cleared_on_lock() {
            let sessions = BackupImportSessions::default();
            let source = std::path::PathBuf::from("C:\\backups\\transfer.cbak");
            let token = sessions
                .insert(
                    source.clone(),
                    SecretText::new("separate backup passphrase".into()),
                    4,
                    7,
                )
                .unwrap();
            assert_eq!(
                sessions
                    .take(token, 4, 6)
                    .err()
                    .expect("revision mismatch must expire the token")
                    .code,
                "backup_import_expired"
            );
            assert_eq!(
                sessions
                    .take(token, 4, 7)
                    .err()
                    .expect("mismatched token must be single-use")
                    .code,
                "backup_import_expired"
            );

            let token = sessions
                .insert(
                    source.clone(),
                    SecretText::new("separate backup passphrase".into()),
                    4,
                    7,
                )
                .unwrap();
            let pending = sessions.take(token, 4, 7).unwrap();
            assert_eq!(pending.source, source);
            assert_eq!(pending.expected_revision, 4);
            assert_eq!(pending.expected_epoch, 7);
            assert_eq!(pending.password.expose(), "separate backup passphrase");
            assert_eq!(
                sessions
                    .take(token, 4, 7)
                    .err()
                    .expect("token must not be reused")
                    .code,
                "backup_import_expired"
            );

            let token = sessions
                .insert(
                    source,
                    SecretText::new("another separate phrase".into()),
                    4,
                    7,
                )
                .unwrap();
            sessions.clear();
            assert_eq!(
                sessions
                    .take(token, 4, 7)
                    .err()
                    .expect("lock must clear pending imports")
                    .code,
                "backup_import_expired"
            );
        }

        #[test]
        fn external_links_require_confirmation_and_reject_active_or_file_schemes() {
            assert!(validate_external_url("https://example.test/path", false).is_ok());
            assert_eq!(
                validate_external_url("mailto:user@example.test", false)
                    .unwrap_err()
                    .code,
                "external_link_confirmation_required"
            );
            assert!(validate_external_url("mailto:user@example.test", true).is_ok());
            for url in [
                "javascript:alert(1)",
                "data:text/html,hello",
                "file:///C:/secret.txt",
                "1bad:thing",
                "https://example.test/\nmalformed",
            ] {
                assert_eq!(
                    validate_external_url(url, true).unwrap_err().code,
                    "external_link_blocked"
                );
            }
            assert_eq!(
                validate_external_url(&format!("https://example.test/{}", "a".repeat(8192)), false)
                    .unwrap_err()
                    .code,
                "external_link_blocked"
            );
        }

        #[tokio::test(start_paused = true)]
        async fn auto_lock_uses_saved_timeout_and_honors_disable_setting() {
            let directory = tempfile::tempdir().unwrap();
            let vault = VaultManager::new(directory.path().join("vault.capstan"));
            vault
                .initialize(
                    SecretText::new("master password for tests".into()),
                    KdfParameters {
                        memory_kib: 19_456,
                        iterations: 2,
                        parallelism: 1,
                    },
                )
                .await
                .unwrap();
            let attempts = SshAttempts::default();
            let (broker, _) = TrustBroker::new();
            let attempt_id = Uuid::new_v4();
            let pending_attempt = attempts.begin(attempt_id).unwrap();
            let finish_attempts = attempts.clone();
            let canceled_attempt = pending_attempt.clone();
            let finish_task = tokio::spawn(async move {
                canceled_attempt.cancelled().await;
                finish_attempts.finish(attempt_id);
            });
            let terminals = Arc::new(TerminalService::default());
            let sessions = SshSessions::default();
            let activity = UserActivity::default();
            tokio::time::advance(Duration::from_secs(31 * 60)).await;

            assert!(maybe_auto_lock(
                &vault,
                &activity,
                &attempts,
                &broker,
                terminals.clone(),
                &sessions,
                &capstan_core::transfer::TransferService::default(),
                #[cfg(windows)]
                None,
            )
            .await
            .unwrap());
            assert!(pending_attempt.is_cancelled());
            finish_task.await.unwrap();
            assert!(!vault.status().unlocked);

            vault
                .unlock(SecretText::new("master password for tests".into()))
                .await
                .unwrap();
            let revision = vault.revision().unwrap();
            vault
                .update_preferences(
                    Preferences {
                        auto_lock_minutes: None,
                        disconnect_on_lock: false,
                    },
                    revision,
                )
                .await
                .unwrap();
            tokio::time::advance(Duration::from_secs(31 * 60)).await;
            assert!(!maybe_auto_lock(
                &vault,
                &activity,
                &attempts,
                &broker,
                terminals,
                &sessions,
                &capstan_core::transfer::TransferService::default(),
                #[cfg(windows)]
                None,
            )
            .await
            .unwrap());
            assert!(vault.status().unlocked);
        }

        #[tokio::test]
        async fn disconnect_on_lock_closes_only_registered_ssh_terminals() {
            let terminals = Arc::new(TerminalService::default());
            let backend = || BackendParts {
                reader: Box::new(Cursor::new(Vec::<u8>::new())),
                writer: Box::new(Cursor::new(Vec::<u8>::new())),
                control: Arc::new(TestControl),
            };
            let ssh_id = terminals
                .start(backend(), Arc::new(QuietSink))
                .await
                .unwrap();
            let local_id = terminals
                .start(backend(), Arc::new(QuietSink))
                .await
                .unwrap();
            let ssh_sessions = SshSessions::default();
            ssh_sessions.insert(ssh_id.clone());

            close_ssh_sessions(terminals.clone(), &ssh_sessions).await;

            assert!(terminals.get(&ssh_id).is_err());
            assert!(terminals.get(&local_id).is_ok());
            terminals.close(&local_id).await.unwrap();
            assert!(ssh_sessions.take_all().is_empty());
        }

        #[tokio::test]
        async fn locking_waits_for_ssh_attempts_and_blocks_new_ones_until_unlock() {
            let attempts = SshAttempts::default();
            let (broker, _challenges) = TrustBroker::new();
            let attempt_id = Uuid::new_v4();
            let token = attempts.begin(attempt_id).expect("begin SSH attempt");
            let waiting = attempts.clone();
            let broker = broker.clone();
            let lock = tokio::spawn(async move { waiting.block_and_cancel_all(&broker).await });
            tokio::task::yield_now().await;
            assert!(token.is_cancelled());
            assert!(attempts.begin(Uuid::new_v4()).is_err());
            assert!(!lock.is_finished());
            attempts.finish(attempt_id);
            lock.await.unwrap();
            attempts.allow_new();
            assert!(attempts.begin(attempt_id).is_ok());
        }
    }
}
