//! Quick connect, SSH config import, and the native dialogs that answer the
//! server's authentication prompts. Answers and key passphrases go from the
//! native dialog straight to Rust; the WebView only sends host text, indices
//! and names.
use crate::CommandError;
use async_trait::async_trait;
use capstan_core::{
    host_import::{ImportNames, ImportPlan, ImportPreview, ImportSummary, QuickConnect},
    ssh_config::{self, Environment, Skip, SystemFiles},
    CoreError, VaultManager,
};
use capstan_ssh::{AuthPrompt, AuthPrompter, PromptField};
use secrecy::SecretString;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{Manager, State};
use uuid::Uuid;

/// Windows' credential dialog accepts at most 1024 message characters.
const MAX_MESSAGE: usize = 1000;
const PREVIEW_LIFETIME: Duration = Duration::from_secs(10 * 60);
const MAX_SELECTED: usize = 512;

/// Only one authentication dialog is shown at a time.
static PROMPTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn source(window: &tauri::WebviewWindow) -> Result<(), CommandError> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err(CommandError::new("window_source"))
    }
}

/// Answers an attempt's prompts with native secret dialogs on the main window.
pub struct NativePrompter {
    window: tauri::WebviewWindow,
    locale: String,
}

impl NativePrompter {
    pub fn new(window: tauri::WebviewWindow, locale: Option<String>) -> Arc<Self> {
        Arc::new(Self {
            window,
            locale: locale.unwrap_or_default(),
        })
    }
}

#[async_trait]
impl AuthPrompter for NativePrompter {
    async fn answer(&self, prompt: AuthPrompt) -> Option<Vec<SecretString>> {
        let _turn = PROMPTS.lock().await;
        let chinese = self.locale.starts_with("zh");
        let title = if chinese {
            "SSH 验证"
        } else {
            "SSH verification"
        };
        let mut answers = Vec::with_capacity(prompt.fields.len());
        for field in &prompt.fields {
            let message = prompt_message(&prompt, field, chinese);
            let secret = crate::native_dialog::prompt_secret(
                &self.window,
                title.to_owned(),
                message,
                &self.locale,
            )
            .await
            .ok()??;
            answers.push(SecretString::from(secret.expose().to_owned()));
        }
        Some(answers)
    }
}

fn authority(prompt: &AuthPrompt) -> String {
    let host = if prompt.target.contains(':') {
        format!("[{}]", prompt.target)
    } else {
        prompt.target.clone()
    };
    if prompt.port == 22 {
        format!("{}@{host}", prompt.username)
    } else {
        format!("{}@{host}:{}", prompt.username, prompt.port)
    }
}

fn prompt_message(prompt: &AuthPrompt, field: &PromptField, chinese: bool) -> String {
    let authority = authority(prompt);
    if prompt.password {
        return if chinese {
            format!("输入 {authority} 的 SSH 密码。")
        } else {
            format!("Enter the SSH password for {authority}.")
        };
    }
    let mut lines = vec![if chinese {
        format!("{authority} 的服务器要求验证：")
    } else {
        format!("The server for {authority} asks:")
    }];
    lines.extend(
        [&prompt.name, &prompt.instructions, &field.text]
            .into_iter()
            .filter(|text| !text.is_empty())
            .cloned(),
    );
    if field.text.is_empty() {
        lines.push(if chinese { "回答：" } else { "Response:" }.to_owned());
    }
    lines.join("\n").chars().take(MAX_MESSAGE).collect()
}

#[tauri::command]
pub async fn create_interactive_credential(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
    name: String,
) -> Result<Uuid, CommandError> {
    source(&window)?;
    vault
        .add_interactive_credential(name)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn quick_connect_host(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
    text: String,
    credential_name: String,
) -> Result<QuickConnect, CommandError> {
    source(&window)?;
    let result = vault
        .quick_connect_host(&text, credential_name)
        .await
        .map_err(CommandError::from)?;
    if result.created {
        crate::launch_commands::refresh_jump_list(window.app_handle()).await;
    }
    Ok(result)
}

struct PendingImport {
    token: Uuid,
    expires: Instant,
    plan: ImportPlan,
}

/// The last SSH config preview, so an import uses exactly what was shown.
#[derive(Default)]
pub struct SshConfigImports(Mutex<Option<PendingImport>>);

impl SshConfigImports {
    fn store(&self, plan: ImportPlan) -> Uuid {
        let token = Uuid::new_v4();
        if let Ok(mut pending) = self.0.lock() {
            *pending = Some(PendingImport {
                token,
                expires: Instant::now() + PREVIEW_LIFETIME,
                plan,
            });
        }
        token
    }
    fn plan(&self, token: Uuid) -> Result<ImportPlan, CommandError> {
        let pending = self
            .0
            .lock()
            .map_err(|_| CommandError::new("task_failed"))?;
        pending
            .as_ref()
            .filter(|pending| pending.token == token && pending.expires > Instant::now())
            .map(|pending| pending.plan.clone())
            .ok_or_else(|| CommandError::new("ssh_config_expired"))
    }
    fn clear(&self, token: Uuid) {
        if let Ok(mut pending) = self.0.lock() {
            if pending
                .as_ref()
                .is_some_and(|pending| pending.token == token)
            {
                *pending = None;
            }
        }
    }
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfigPreview {
    /// `None` when there is no readable `~/.ssh/config`.
    token: Option<Uuid>,
    hosts: Vec<ImportPreview>,
    skipped: Vec<Skip>,
}

#[tauri::command]
pub async fn preview_ssh_config_import(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
    imports: State<'_, SshConfigImports>,
) -> Result<SshConfigPreview, CommandError> {
    source(&window)?;
    let vault = vault.inner().clone();
    let preview = tauri::async_runtime::spawn_blocking(move || {
        let Some((home, user)) = ssh_config::user_environment() else {
            return Ok(None);
        };
        let Some(parsed) = ssh_config::parse(&Environment {
            home: &home,
            local_user: user.as_deref(),
            files: &SystemFiles,
        }) else {
            return Ok(None);
        };
        let (plan, hosts) = vault.plan_ssh_config_import(parsed.candidates)?;
        Ok::<_, CoreError>(Some((plan, hosts, parsed.skipped)))
    })
    .await
    .map_err(|_| CommandError::new("task_failed"))?
    .map_err(CommandError::from)?;
    Ok(match preview {
        Some((plan, hosts, skipped)) => SshConfigPreview {
            token: Some(imports.store(plan)),
            hosts,
            skipped,
        },
        None => SshConfigPreview {
            token: None,
            hosts: Vec::new(),
            skipped: Vec::new(),
        },
    })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri injects services as separate command parameters.
pub async fn import_ssh_config(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
    imports: State<'_, SshConfigImports>,
    token: Uuid,
    indices: Vec<u32>,
    locale: String,
    credential_name: String,
    group_name: String,
) -> Result<ImportSummary, CommandError> {
    source(&window)?;
    if indices.is_empty() || indices.len() > MAX_SELECTED {
        return Err(CommandError::new("invalid_input"));
    }
    let plan = imports.plan(token)?;
    let mut passphrases = Vec::new();
    for path in plan.encrypted_keys(&indices) {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
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
        let secret = crate::native_dialog::prompt_secret(&window, title, message, &locale)
            .await
            .map_err(|_| CommandError::new("native_credential_ui"))?
            .ok_or_else(|| CommandError::new("canceled"))?;
        passphrases.push((path, secret.expose().to_owned()));
    }
    let summary = vault
        .import_ssh_config_hosts(
            &plan,
            &indices,
            passphrases,
            ImportNames {
                interactive_credential: credential_name,
                group: group_name,
            },
        )
        .await
        .map_err(CommandError::from)?;
    imports.clear(token);
    if summary.created > 0 {
        crate::launch_commands::refresh_jump_list(window.app_handle()).await;
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(password: bool) -> AuthPrompt {
        AuthPrompt {
            attempt_id: Uuid::new_v4(),
            host_id: Uuid::new_v4(),
            target: "2001:db8::1".into(),
            port: 2222,
            username: "alice".into(),
            password,
            name: "Duo".into(),
            instructions: "Choose a device".into(),
            fields: Vec::new(),
        }
    }

    #[test]
    fn prompt_messages_name_the_target_and_keep_server_text_bounded() {
        let field = PromptField {
            text: "Passcode:".into(),
            echo: true,
        };
        assert_eq!(
            prompt_message(&prompt(true), &field, false),
            "Enter the SSH password for alice@[2001:db8::1]:2222."
        );
        assert_eq!(
            prompt_message(&prompt(false), &field, true),
            "alice@[2001:db8::1]:2222 的服务器要求验证：\nDuo\nChoose a device\nPasscode:"
        );
        let mut long = prompt(false);
        long.instructions = "x".repeat(4000);
        let empty = PromptField {
            text: String::new(),
            echo: false,
        };
        let message = prompt_message(&long, &empty, false);
        assert_eq!(message.chars().count(), MAX_MESSAGE);
    }

    #[test]
    fn previews_expire_and_are_single_use() {
        let imports = SshConfigImports::default();
        let token = imports.store(ImportPlan::default());
        assert!(imports.plan(token).is_ok());
        assert_eq!(
            imports.plan(Uuid::new_v4()).unwrap_err().code,
            "ssh_config_expired"
        );
        imports.clear(token);
        assert!(imports.plan(token).is_err());
        let token = imports.store(ImportPlan::default());
        if let Ok(mut pending) = imports.0.lock() {
            if let Some(pending) = pending.as_mut() {
                pending.expires = Instant::now() - Duration::from_secs(1);
            }
        }
        assert!(imports.plan(token).is_err());
    }
}
