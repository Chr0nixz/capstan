//! Quick connect, SSH config import, and the native dialogs that answer the
//! server's authentication prompts. Answers and key passphrases go from the
//! native dialog straight to Rust; the WebView only sends host text, indices
//! and names.
use crate::CommandError;
use async_trait::async_trait;
use capstan_core::VaultManager;
use capstan_ssh::{AuthPrompt, AuthPrompter, PromptField};
use secrecy::SecretString;
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

/// Windows' credential dialog accepts at most 1024 message characters.
const MAX_MESSAGE: usize = 1000;

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
}
