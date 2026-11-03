//! Small Windows-only FFI boundary for atomic replacement.
#![allow(unsafe_code)]

use std::{io, path::Path};
use zeroize::Zeroize;
use zeroize::Zeroizing;

#[cfg(windows)]
pub mod editor;
#[cfg(windows)]
pub mod export;
#[cfg(windows)]
pub mod staged_drag;
#[cfg(windows)]
pub mod transfer;
#[cfg(windows)]
pub mod transfer_recovery;

pub struct PromptedSecret {
    pub username: String,
    password: Zeroizing<Vec<u16>>,
}
impl PromptedSecret {
    pub fn password(&self) -> &[u16] {
        &self.password
    }
}

#[cfg(windows)]
pub fn pick_private_key_file(title: &str, parent: isize) -> io::Result<Option<std::path::PathBuf>> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::{
        Foundation::HWND,
        UI::Controls::Dialogs::{
            CommDlgExtendedError, GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR,
            OFN_PATHMUSTEXIST, OPENFILENAMEW,
        },
    };
    const BUFFER_CAPACITY: usize = 32_768;
    let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    let filter: Vec<u16> = "Private key files\0*.*\0All files\0*.*\0\0"
        .encode_utf16()
        .collect();
    let mut file = vec![0_u16; BUFFER_CAPACITY];
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: parent as HWND,
        lpstrFilter: filter.as_ptr(),
        lpstrFile: file.as_mut_ptr(),
        nMaxFile: BUFFER_CAPACITY as u32,
        lpstrTitle: title.as_ptr(),
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    // SAFETY: all pointers refer to buffers kept alive through the synchronous common dialog call.
    let selected = unsafe { GetOpenFileNameW(&mut dialog) };
    if selected == 0 {
        // SAFETY: the common-dialog API has no pointer inputs and returns a thread-local error code.
        let error = unsafe { CommDlgExtendedError() };
        if error == 0 {
            return Ok(None);
        }
        return Err(io::Error::other(format!(
            "native file picker failed ({error})"
        )));
    }
    let end = file
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(file.len());
    Ok(Some(std::path::PathBuf::from(
        std::ffi::OsString::from_wide(&file[..end]),
    )))
}

#[cfg(not(windows))]
pub fn pick_private_key_file(_: &str, _: isize) -> io::Result<Option<std::path::PathBuf>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "native file picker is unavailable",
    ))
}

#[cfg(windows)]
fn select_vault_backup_file(
    title: &str,
    parent: isize,
    save: bool,
) -> io::Result<Option<std::path::PathBuf>> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::{
        Foundation::HWND,
        UI::Controls::Dialogs::{
            CommDlgExtendedError, GetOpenFileNameW, GetSaveFileNameW, OFN_FILEMUSTEXIST,
            OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
        },
    };
    const BUFFER_CAPACITY: usize = 32_768;
    let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    let filter: Vec<u16> = "Capstan encrypted backup\0*.cbak\0All files\0*.*\0\0"
        .encode_utf16()
        .collect();
    let default_extension: Vec<u16> = "cbak\0".encode_utf16().collect();
    let mut file = vec![0_u16; BUFFER_CAPACITY];
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: parent as HWND,
        lpstrFilter: filter.as_ptr(),
        lpstrFile: file.as_mut_ptr(),
        nMaxFile: BUFFER_CAPACITY as u32,
        lpstrTitle: title.as_ptr(),
        lpstrDefExt: default_extension.as_ptr(),
        Flags: if save {
            OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_OVERWRITEPROMPT
        } else {
            OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR
        },
        ..Default::default()
    };
    // SAFETY: dialog strings and the writable path buffer stay alive through this synchronous call.
    let selected = unsafe {
        if save {
            GetSaveFileNameW(&mut dialog)
        } else {
            GetOpenFileNameW(&mut dialog)
        }
    };
    if selected == 0 {
        // SAFETY: the common-dialog API returns a thread-local error code without pointer inputs.
        let error = unsafe { CommDlgExtendedError() };
        if error == 0 {
            return Ok(None);
        }
        return Err(io::Error::other(format!(
            "native backup file dialog failed ({error})"
        )));
    }
    let end = file
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(file.len());
    Ok(Some(std::path::PathBuf::from(
        std::ffi::OsString::from_wide(&file[..end]),
    )))
}

#[cfg(windows)]
pub fn pick_vault_backup_file(
    title: &str,
    parent: isize,
) -> io::Result<Option<std::path::PathBuf>> {
    select_vault_backup_file(title, parent, false)
}

#[cfg(windows)]
pub fn save_vault_backup_file(
    title: &str,
    parent: isize,
) -> io::Result<Option<std::path::PathBuf>> {
    select_vault_backup_file(title, parent, true)
}

#[cfg(not(windows))]
pub fn pick_vault_backup_file(_: &str, _: isize) -> io::Result<Option<std::path::PathBuf>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "native Windows file dialogs are unavailable",
    ))
}

#[cfg(not(windows))]
pub fn save_vault_backup_file(_: &str, _: isize) -> io::Result<Option<std::path::PathBuf>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "native Windows file dialogs are unavailable",
    ))
}

#[cfg(windows)]
pub fn prompt_secret(
    title: &str,
    message: &str,
    username: &str,
    parent: isize,
) -> io::Result<Option<PromptedSecret>> {
    use std::ptr;
    use windows_sys::Win32::Foundation::{ERROR_CANCELLED, HWND};
    use windows_sys::Win32::Security::Credentials::{
        CredUIPromptForCredentialsW, CREDUI_FLAGS_ALWAYS_SHOW_UI, CREDUI_FLAGS_DO_NOT_PERSIST,
        CREDUI_FLAGS_GENERIC_CREDENTIALS, CREDUI_FLAGS_KEEP_USERNAME, CREDUI_INFOW,
    };
    const BUFFER_CAPACITY: usize = 4096;
    let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let target: Vec<u16> = "Capstan native secret input"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut username_buffer = vec![0_u16; BUFFER_CAPACITY];
    for (destination, source) in username_buffer.iter_mut().zip(username.encode_utf16()) {
        *destination = source;
    }
    let mut password_buffer = vec![0_u16; BUFFER_CAPACITY];
    let mut save = 0;
    let info = CREDUI_INFOW {
        cbSize: std::mem::size_of::<CREDUI_INFOW>() as u32,
        hwndParent: parent as HWND,
        pszMessageText: message.as_ptr(),
        pszCaptionText: title.as_ptr(),
        hbmBanner: ptr::null_mut(),
    };
    // SAFETY: all UTF-16 pointers and writable buffers remain alive for the synchronous dialog call.
    let status = unsafe {
        CredUIPromptForCredentialsW(
            &info,
            target.as_ptr(),
            ptr::null(),
            0,
            username_buffer.as_mut_ptr(),
            BUFFER_CAPACITY as u32,
            password_buffer.as_mut_ptr(),
            BUFFER_CAPACITY as u32,
            &mut save,
            CREDUI_FLAGS_GENERIC_CREDENTIALS
                | CREDUI_FLAGS_DO_NOT_PERSIST
                | CREDUI_FLAGS_KEEP_USERNAME
                | CREDUI_FLAGS_ALWAYS_SHOW_UI,
        )
    };
    if status == ERROR_CANCELLED {
        password_buffer.zeroize();
        username_buffer.zeroize();
        return Ok(None);
    }
    if status != 0 {
        password_buffer.zeroize();
        username_buffer.zeroize();
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let password_len = password_buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(password_buffer.len());
    let secret = Zeroizing::new(password_buffer[..password_len].to_vec());
    password_buffer.zeroize();
    let username_len = username_buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(username_buffer.len());
    let username = String::from_utf16_lossy(&username_buffer[..username_len]);
    username_buffer.zeroize();
    Ok(Some(PromptedSecret {
        username,
        password: secret,
    }))
}

#[cfg(not(windows))]
pub fn prompt_secret(_: &str, _: &str, _: &str, _: isize) -> io::Result<Option<PromptedSecret>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "native Windows credential UI is unavailable",
    ))
}

#[cfg(windows)]
pub fn atomic_replace(source: &Path, target: &Path, replace: bool) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let flags = MOVEFILE_WRITE_THROUGH
        | if replace {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    // SAFETY: both paths are NUL-terminated UTF-16 buffers valid for the synchronous call.
    let result = unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), flags) };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
pub fn open_external_url(url: &str) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::HWND,
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    };
    if url.is_empty() || url.len() > 8192 || url.chars().any(char::is_control) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid external URL",
        ));
    }
    let Some((scheme, _)) = url.split_once(':') else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "external URL has no scheme",
        ));
    };
    let mut chars = scheme.chars();
    if !chars
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic())
        || !chars.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
        || matches!(
            scheme.to_ascii_lowercase().as_str(),
            "javascript" | "data" | "file"
        )
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "external URL scheme is blocked",
        ));
    }
    let operation: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
    let url: Vec<u16> = std::ffi::OsStr::new(url)
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: both NUL-terminated UTF-16 arguments live for the synchronous ShellExecuteW call.
    let result = unsafe {
        ShellExecuteW(
            0 as HWND,
            operation.as_ptr(),
            url.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if (result as isize) <= 32 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn open_external_url(_: &str) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "external URL launching is only available on Windows",
    ))
}

#[cfg(not(windows))]
pub fn atomic_replace(source: &Path, target: &Path, replace: bool) -> io::Result<()> {
    if !replace && target.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "target already exists",
        ));
    }
    std::fs::rename(source, target)
}
