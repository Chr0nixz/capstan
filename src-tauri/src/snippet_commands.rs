use crate::CommandError;
use capstan_core::{sftp::ExactU64, VaultManager};
use capstan_vault::{Snippet, SnippetGroup};
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;

fn source_label(label: &str) -> Result<(), CommandError> {
    if label == "main" {
        Ok(())
    } else {
        Err(CommandError::new("window_source"))
    }
}

fn source(window: &tauri::WebviewWindow) -> Result<(), CommandError> {
    source_label(window.label())
}

#[tauri::command]
pub fn list_snippets(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
) -> Result<Vec<Snippet>, CommandError> {
    source(&window)?;
    vault.snippets().map_err(Into::into)
}

#[tauri::command]
pub fn list_snippet_groups(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
) -> Result<Vec<SnippetGroup>, CommandError> {
    source(&window)?;
    vault.snippet_groups().map_err(Into::into)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn save_snippet(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
    snippet: Snippet,
    expected_revision: ExactU64,
) -> Result<ExactU64, CommandError> {
    source(&window)?;
    vault
        .save_snippet(snippet, expected_revision.0)
        .await
        .map(ExactU64)
        .map_err(Into::into)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn save_snippet_group(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
    group: SnippetGroup,
    expected_revision: ExactU64,
) -> Result<ExactU64, CommandError> {
    source(&window)?;
    vault
        .save_snippet_group(group, expected_revision.0)
        .await
        .map(ExactU64)
        .map_err(Into::into)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn delete_snippet(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
    id: Uuid,
    expected_revision: ExactU64,
) -> Result<ExactU64, CommandError> {
    source(&window)?;
    vault
        .delete_snippet(id, expected_revision.0)
        .await
        .map(ExactU64)
        .map_err(Into::into)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn delete_snippet_group(
    window: tauri::WebviewWindow,
    vault: State<'_, Arc<VaultManager>>,
    id: Uuid,
    expected_revision: ExactU64,
) -> Result<ExactU64, CommandError> {
    source(&window)?;
    vault
        .delete_snippet_group(id, expected_revision.0)
        .await
        .map(ExactU64)
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_commands_are_restricted_to_the_main_window() {
        assert!(source_label("main").is_ok());
        assert_eq!(
            source_label("other-window").unwrap_err().code,
            "window_source"
        );
    }

    #[test]
    fn snippet_revision_arguments_require_canonical_decimal_strings() {
        for revision in ["0", "18446744073709551615"] {
            assert!(serde_json::from_str::<ExactU64>(&format!("\"{revision}\"")).is_ok());
        }
        for revision in ["", "00", "+1", "1.0", "18446744073709551616"] {
            assert!(serde_json::from_str::<ExactU64>(&format!("\"{revision}\"")).is_err());
        }
        assert!(serde_json::from_str::<ExactU64>("1").is_err());
    }
}
