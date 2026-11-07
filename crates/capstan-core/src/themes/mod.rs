//! Bounded native theme previews and a separate, atomically updated catalog.
mod parse;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Mutex,
};
use thiserror::Error;
use uuid::Uuid;

pub use parse::parse_theme_file;

pub const MAX_IMPORT_BYTES: usize = 1024 * 1024;
const MAX_CATALOG_BYTES: usize = 128 * 1024;
const MAX_THEMES: usize = 32;
const MAX_NAME: usize = 64;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThemeSource {
    WindowsTerminal,
    Iterm2,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalThemeDefinition {
    pub id: String,
    pub name: String,
    pub source: ThemeSource,
    pub foreground: String,
    pub background: String,
    pub cursor: String,
    pub selection_background: String,
    pub ansi: [String; 16],
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemePreview {
    pub token: Uuid,
    pub themes: Vec<TerminalThemeDefinition>,
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum ThemeError {
    #[error("theme_limit")]
    Limit,
    #[error("theme_invalid")]
    Invalid,
    #[error("theme_json")]
    Json,
    #[error("theme_xml")]
    Xml,
    #[error("theme_unavailable")]
    Unavailable,
    #[error("theme_changed")]
    Changed,
}

pub struct ThemeCatalog {
    path: PathBuf,
    pending: Mutex<Option<ThemePreview>>,
}

impl ThemeCatalog {
    #[must_use]
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join("terminal-themes.json"),
            pending: Mutex::new(None),
        }
    }

    pub fn list(&self) -> Result<Vec<TerminalThemeDefinition>, ThemeError> {
        let _guard = self.pending.lock().map_err(|_| ThemeError::Unavailable)?;
        self.read().map(|(themes, _)| themes)
    }

    /// Only the path returned by the native picker may be used here.
    #[cfg(windows)]
    pub fn preview(&self, path: &Path) -> Result<ThemePreview, ThemeError> {
        let extension = path
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .ok_or(ThemeError::Invalid)?;
        let source = if extension.eq_ignore_ascii_case("json") {
            ThemeSource::WindowsTerminal
        } else if extension.eq_ignore_ascii_case("itermcolors") {
            ThemeSource::Iterm2
        } else {
            return Err(ThemeError::Invalid);
        };
        let bytes = read_bounded(path, MAX_IMPORT_BYTES)?;
        let name = path
            .file_stem()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default();
        let mut themes = parse_theme_file(source, &bytes, name)?;
        let mut pending = self.pending.lock().map_err(|_| ThemeError::Unavailable)?;
        let (current, _) = self.read()?;
        if current.len() + themes.len() > MAX_THEMES {
            return Err(ThemeError::Limit);
        }
        let mut names = catalog_names(&current);
        for theme in &mut themes {
            theme.name = unique_name(&theme.name, &mut names);
        }
        let preview = ThemePreview {
            token: Uuid::new_v4(),
            themes,
        };
        *pending = Some(preview.clone());
        Ok(preview)
    }

    pub fn save_preview(&self, token: Uuid) -> Result<Vec<TerminalThemeDefinition>, ThemeError> {
        let mut pending = self.pending.lock().map_err(|_| ThemeError::Unavailable)?;
        let preview = pending
            .as_ref()
            .filter(|item| item.token == token)
            .ok_or(ThemeError::Changed)?;
        let (mut themes, storage) = self.read()?;
        let mut names = catalog_names(&themes);
        // Preserve exactly the names previewed, even if another edit raced it.
        for theme in &preview.themes {
            if !names.insert(theme.name.to_lowercase()) {
                return Err(ThemeError::Changed);
            }
        }
        themes.extend_from_slice(&preview.themes);
        storage.commit(&themes)?;
        *pending = None;
        Ok(themes)
    }

    pub fn discard_preview(&self, token: Uuid) -> Result<(), ThemeError> {
        let mut pending = self.pending.lock().map_err(|_| ThemeError::Unavailable)?;
        if pending.as_ref().is_some_and(|item| item.token == token) {
            *pending = None;
        }
        Ok(())
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<Vec<TerminalThemeDefinition>, ThemeError> {
        if !capstan_vault::valid_custom_theme_id(id) || name.len() > 4 * MAX_NAME {
            return Err(ThemeError::Invalid);
        }
        let name = clean_name(name)?;
        let _guard = self.pending.lock().map_err(|_| ThemeError::Unavailable)?;
        let (mut themes, storage) = self.read()?;
        let mut names = reserved_names();
        names.extend(
            themes
                .iter()
                .filter(|theme| theme.id != id)
                .map(|theme| theme.name.to_lowercase()),
        );
        let theme = themes
            .iter_mut()
            .find(|theme| theme.id == id)
            .ok_or(ThemeError::Changed)?;
        theme.name = unique_name(&name, &mut names);
        storage.commit(&themes)?;
        Ok(themes)
    }

    pub fn delete(&self, id: &str) -> Result<Vec<TerminalThemeDefinition>, ThemeError> {
        if !capstan_vault::valid_custom_theme_id(id) {
            return Err(ThemeError::Invalid);
        }
        let _guard = self.pending.lock().map_err(|_| ThemeError::Unavailable)?;
        let (mut themes, storage) = self.read()?;
        let index = themes
            .iter()
            .position(|theme| theme.id == id)
            .ok_or(ThemeError::Changed)?;
        themes.remove(index);
        storage.commit(&themes)?;
        Ok(themes)
    }

    #[cfg(windows)]
    fn read(&self) -> Result<(Vec<TerminalThemeDefinition>, Storage), ThemeError> {
        let destination = capstan_native_windows::transfer::Destination::select(&self.path)
            .map_err(storage_error)?;
        let expected = destination.snapshot().map_err(storage_error)?;
        let themes = if expected.is_some() {
            let themes = decode_catalog(&read_bounded(&self.path, MAX_CATALOG_BYTES)?)?;
            if destination.snapshot().map_err(storage_error)? != expected {
                return Err(ThemeError::Changed);
            }
            themes
        } else {
            Vec::new()
        };
        Ok((
            themes,
            Storage {
                destination,
                expected,
            },
        ))
    }

    #[cfg(not(windows))]
    fn read(&self) -> Result<(Vec<TerminalThemeDefinition>, Storage), ThemeError> {
        let _ = &self.path;
        Err(ThemeError::Unavailable)
    }
}

fn clean_name(name: &str) -> Result<String, ThemeError> {
    let name: String = name
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    let name = name.trim();
    if name.is_empty() {
        return Err(ThemeError::Invalid);
    }
    if name.chars().count() > MAX_NAME {
        return Err(ThemeError::Limit);
    }
    Ok(name.to_owned())
}

fn reserved_names() -> HashSet<String> {
    ["graphite", "light", "paper", "石墨", "浅色"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn catalog_names(themes: &[TerminalThemeDefinition]) -> HashSet<String> {
    let mut names = reserved_names();
    names.extend(themes.iter().map(|theme| theme.name.to_lowercase()));
    names
}

fn unique_name(name: &str, names: &mut HashSet<String>) -> String {
    if names.insert(name.to_lowercase()) {
        return name.to_owned();
    }
    for index in 2.. {
        let suffix = format!(" ({index})");
        let prefix: String = name.chars().take(MAX_NAME - suffix.len()).collect();
        let candidate = format!("{}{suffix}", prefix.trim_end());
        if names.insert(candidate.to_lowercase()) {
            return candidate;
        }
    }
    unreachable!("bounded catalog names")
}

#[cfg(any(windows, test))]
fn decode_catalog(bytes: &[u8]) -> Result<Vec<TerminalThemeDefinition>, ThemeError> {
    if bytes.len() > MAX_CATALOG_BYTES {
        return Err(ThemeError::Limit);
    }
    let themes: Vec<TerminalThemeDefinition> =
        serde_json::from_slice(bytes).map_err(|_| ThemeError::Invalid)?;
    if themes.len() > MAX_THEMES {
        return Err(ThemeError::Limit);
    }
    let mut names = reserved_names();
    let mut ids = HashSet::new();
    for theme in &themes {
        if !capstan_vault::valid_custom_theme_id(&theme.id)
            || !ids.insert(&theme.id)
            || clean_name(&theme.name)? != theme.name
            || !names.insert(theme.name.to_lowercase())
            || [
                &theme.foreground,
                &theme.background,
                &theme.cursor,
                &theme.selection_background,
            ]
            .into_iter()
            .chain(&theme.ansi)
            .any(|color| parse::hex_color(color).as_ref() != Ok(color))
        {
            return Err(ThemeError::Invalid);
        }
    }
    Ok(themes)
}

#[cfg(windows)]
fn storage_error(error: std::io::Error) -> ThemeError {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        ThemeError::Changed
    } else {
        ThemeError::Unavailable
    }
}

#[cfg(windows)]
fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, ThemeError> {
    let file = capstan_native_windows::transfer::Source::open(path).map_err(storage_error)?;
    let size = usize::try_from(file.size())
        .ok()
        .filter(|size| *size <= limit)
        .ok_or(ThemeError::Limit)?;
    let mut bytes = Vec::with_capacity(size);
    while bytes.len() < size {
        let chunk = file
            .read(bytes.len() as u64, (size - bytes.len()).min(32_768))
            .map_err(storage_error)?;
        if chunk.is_empty() {
            return Err(ThemeError::Changed);
        }
        bytes.extend(chunk);
    }
    file.verify().map_err(storage_error)?;
    Ok(bytes)
}

#[cfg(windows)]
struct Storage {
    destination: capstan_native_windows::transfer::Destination,
    expected: Option<capstan_native_windows::transfer::Version>,
}

#[cfg(windows)]
impl Storage {
    fn commit(mut self, themes: &[TerminalThemeDefinition]) -> Result<(), ThemeError> {
        if themes.len() > MAX_THEMES {
            return Err(ThemeError::Limit);
        }
        let bytes = serde_json::to_vec(themes).map_err(|_| ThemeError::Invalid)?;
        if bytes.len() > MAX_CATALOG_BYTES {
            return Err(ThemeError::Limit);
        }
        self.destination
            .begin(&format!(".capstan-themes-{}.partial", Uuid::new_v4()))
            .map_err(storage_error)?;
        let mut offset = 0;
        for chunk in bytes.chunks(32_768) {
            self.destination
                .write(offset, chunk)
                .map_err(storage_error)?;
            offset += chunk.len() as u64;
        }
        self.destination
            .commit(self.expected)
            .map_err(storage_error)
    }
}

#[cfg(not(windows))]
struct Storage;
#[cfg(not(windows))]
impl Storage {
    fn commit(self, _: &[TerminalThemeDefinition]) -> Result<(), ThemeError> {
        Err(ThemeError::Unavailable)
    }
}
