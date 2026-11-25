//! Narrow Windows FFI boundary for desktop integration and protected file operations.
#![allow(unsafe_code)]

use std::{io, path::Path};
use zeroize::Zeroize;
use zeroize::Zeroizing;

#[cfg(windows)]
pub mod editor;
#[cfg(windows)]
pub mod export;
#[cfg(windows)]
pub mod hello;
#[cfg(windows)]
pub mod jump_list;
#[cfg(windows)]
pub mod staged_drag;
#[cfg(windows)]
pub mod toast;
#[cfg(windows)]
pub mod transfer;
#[cfg(windows)]
pub mod transfer_recovery;

/// Opaque normal placement; its coordinates must stay in the Windows workspace coordinate system.
#[cfg(windows)]
#[derive(Clone, Copy)]
pub struct WindowPlacement(windows_sys::Win32::UI::WindowsAndMessaging::WINDOWPLACEMENT);

/// Capture the restore rectangle even while the window is minimized or maximized.
#[cfg(windows)]
pub fn capture_window_placement(window: isize) -> io::Result<WindowPlacement> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, IsIconic, SW_SHOWMAXIMIZED, SW_SHOWNORMAL, WINDOWPLACEMENT,
        WPF_RESTORETOMAXIMIZED,
    };
    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    // SAFETY: Windows validates the borrowed HWND; the output points to an initialized stack value.
    if unsafe { GetWindowPlacement(window as _, &mut placement) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let rectangle = placement.rcNormalPosition;
    if rectangle.right <= rectangle.left || rectangle.bottom <= rectangle.top {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "window has no normal placement",
        ));
    }
    // SAFETY: IsIconic only inspects the same borrowed HWND and retains no pointer.
    let maximized = placement.showCmd == SW_SHOWMAXIMIZED as u32
        || (unsafe { IsIconic(window as _) } != 0 && placement.flags & WPF_RESTORETOMAXIMIZED != 0);
    // Quake brings minimized windows back into use, preserving their normal/maximized restore state.
    placement.showCmd = if maximized {
        SW_SHOWMAXIMIZED as u32
    } else {
        SW_SHOWNORMAL as u32
    };
    placement.flags = 0;
    Ok(WindowPlacement(placement))
}

/// Restore using the matching API, without treating an outer rectangle as a client size.
#[cfg(windows)]
pub fn restore_window_placement(window: isize, saved: WindowPlacement) -> io::Result<()> {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPlacement;
    // SAFETY: the initialized placement remains alive for this synchronous call; Windows checks HWND.
    if unsafe { SetWindowPlacement(window as _, &saved.0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub struct PromptedSecret {
    pub username: String,
    password: Zeroizing<Vec<u16>>,
}
impl PromptedSecret {
    pub fn password(&self) -> &[u16] {
        &self.password
    }
}

/// Builds a common-dialog filter: label and pattern pairs separated by NUL, closed by a double NUL.
/// Labels are localized copy, so an embedded NUL is dropped rather than splitting the list.
#[cfg(windows)]
fn dialog_filter(entries: &[(&str, &str)]) -> Vec<u16> {
    let mut filter = Vec::new();
    for (label, pattern) in entries {
        for part in [label, pattern] {
            filter.extend(part.encode_utf16().filter(|unit| *unit != 0));
            filter.push(0);
        }
    }
    filter.push(0);
    filter
}

/// `filter` and `all_files` are the localized labels of the two filter entries.
#[cfg(windows)]
pub fn pick_private_key_file(
    title: &str,
    filter: &str,
    all_files: &str,
    parent: isize,
) -> io::Result<Option<std::path::PathBuf>> {
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
    let filter = dialog_filter(&[(filter, "*.*"), (all_files, "*.*")]);
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
pub fn pick_private_key_file(
    _: &str,
    _: &str,
    _: &str,
    _: isize,
) -> io::Result<Option<std::path::PathBuf>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "native file picker is unavailable",
    ))
}

#[cfg(windows)]
fn select_vault_backup_file(
    title: &str,
    filter: &str,
    all_files: &str,
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
    let filter = dialog_filter(&[(filter, "*.cbak"), (all_files, "*.*")]);
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

/// `filter` and `all_files` are the localized labels of the two filter entries.
#[cfg(windows)]
pub fn pick_vault_backup_file(
    title: &str,
    filter: &str,
    all_files: &str,
    parent: isize,
) -> io::Result<Option<std::path::PathBuf>> {
    select_vault_backup_file(title, filter, all_files, parent, false)
}

/// `filter` and `all_files` are the localized labels of the two filter entries.
#[cfg(windows)]
pub fn save_vault_backup_file(
    title: &str,
    filter: &str,
    all_files: &str,
    parent: isize,
) -> io::Result<Option<std::path::PathBuf>> {
    select_vault_backup_file(title, filter, all_files, parent, true)
}

#[cfg(not(windows))]
pub fn pick_vault_backup_file(
    _: &str,
    _: &str,
    _: &str,
    _: isize,
) -> io::Result<Option<std::path::PathBuf>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "native Windows file dialogs are unavailable",
    ))
}

#[cfg(not(windows))]
pub fn save_vault_backup_file(
    _: &str,
    _: &str,
    _: &str,
    _: isize,
) -> io::Result<Option<std::path::PathBuf>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "native Windows file dialogs are unavailable",
    ))
}

/// Closes a dialog that a blocking call shows on its own thread, for a caller
/// that stopped waiting for the answer (a timeout, a lost connection).
/// Otherwise the dialog stays open and its answer goes nowhere.
#[derive(Clone, Debug, Default)]
pub struct DialogCloser(std::sync::Arc<DialogThread>);

#[derive(Debug, Default)]
struct DialogThread {
    /// The thread showing the dialog, 0 when none is.
    thread: std::sync::atomic::AtomicU32,
    closed: std::sync::atomic::AtomicBool,
}

/// Closes the dialog when dropped; after an answer there is nothing to close.
#[derive(Debug)]
pub struct CloseOnDrop(DialogCloser);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        self.0.close();
    }
}

impl DialogCloser {
    pub fn close_on_drop(&self) -> CloseOnDrop {
        CloseOnDrop(self.clone())
    }

    /// Cancels the dialog if it is open, and keeps one that has not opened
    /// yet from opening.
    pub fn close(&self) {
        use std::sync::atomic::Ordering;
        self.0.closed.store(true, Ordering::SeqCst);
        let thread = self.0.thread.load(Ordering::SeqCst);
        if thread != 0 {
            close_thread_windows(thread);
        }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    fn is_closed(&self) -> bool {
        self.0.closed.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Marks the calling thread as the one showing the dialog; false when
    /// the dialog was closed before it opened.
    #[cfg(windows)]
    fn enter(&self) -> bool {
        use std::sync::atomic::Ordering;
        // SAFETY: GetCurrentThreadId has no preconditions.
        let thread = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
        self.0.thread.store(thread, Ordering::SeqCst);
        if self.is_closed() {
            self.leave();
            return false;
        }
        true
    }

    #[cfg(windows)]
    fn leave(&self) {
        self.0.thread.store(0, std::sync::atomic::Ordering::SeqCst);
    }
}

/// WM_CLOSE cancels a dialog the way its Cancel button does.
#[cfg(windows)]
fn close_thread_windows(thread: u32) {
    use windows_sys::Win32::Foundation::{HWND, LPARAM, TRUE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{EnumThreadWindows, PostMessageW, WM_CLOSE};

    unsafe extern "system" fn post_close(window: HWND, _: LPARAM) -> i32 {
        // SAFETY: the window handle comes from EnumThreadWindows; posting
        // to a window that has since closed fails harmlessly.
        unsafe { PostMessageW(window, WM_CLOSE, 0, 0) };
        TRUE
    }
    // SAFETY: the callback matches WNDENUMPROC and keeps no state.
    unsafe { EnumThreadWindows(thread, Some(post_close), 0) };
}

#[cfg(not(windows))]
fn close_thread_windows(_: u32) {}

#[cfg(windows)]
pub fn prompt_secret(
    title: &str,
    message: &str,
    username: &str,
    parent: isize,
    closer: &DialogCloser,
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
    if !closer.enter() {
        return Ok(None);
    }
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
    closer.leave();
    // An answer given just as the caller gave up goes nowhere either.
    if status == ERROR_CANCELLED || (status == 0 && closer.is_closed()) {
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
pub fn prompt_secret(
    _: &str,
    _: &str,
    _: &str,
    _: isize,
    _: &DialogCloser,
) -> io::Result<Option<PromptedSecret>> {
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

/// Show an application-owned directory in Explorer. The path comes from
/// Rust, never from the WebView, and must be an absolute plain directory.
#[cfg(windows)]
pub fn open_folder(path: &Path) -> io::Result<()> {
    use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};
    use windows_sys::Win32::{
        Foundation::HWND,
        Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT,
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    };
    let metadata = std::fs::symlink_metadata(path)?;
    if !path.is_absolute()
        || !metadata.is_dir()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "folder to open is not a plain directory",
        ));
    }
    let operation: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
    let folder: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: both NUL-terminated UTF-16 arguments live for the synchronous ShellExecuteW call.
    let result = unsafe {
        ShellExecuteW(
            0 as HWND,
            operation.as_ptr(),
            folder.as_ptr(),
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

#[cfg(test)]
mod dialog_closer_tests {
    use super::*;

    #[test]
    fn a_dialog_closed_before_it_opens_never_opens() {
        let closer = DialogCloser::default();
        drop(closer.close_on_drop());
        assert!(closer.is_closed());
        #[cfg(windows)]
        {
            assert!(!closer.enter());
            assert!(prompt_secret("title", "message", "", 0, &closer)
                .unwrap()
                .is_none());
        }
    }

    #[cfg(windows)]
    #[test]
    fn closing_reaches_the_thread_that_shows_the_dialog() {
        use std::sync::atomic::Ordering;
        let closer = DialogCloser::default();
        assert!(closer.enter());
        assert_ne!(closer.0.thread.load(Ordering::SeqCst), 0);
        // No windows on this thread: closing finds nothing and does no harm.
        closer.close();
        closer.leave();
        assert_eq!(closer.0.thread.load(Ordering::SeqCst), 0);
    }
}

#[cfg(all(test, windows))]
mod filter_tests {
    use super::dialog_filter;

    #[test]
    fn dialog_filter_pairs_localized_labels_and_drops_embedded_nul() {
        let filter = dialog_filter(&[("加密备份", "*.cbak"), ("All\0 files", "*.*")]);
        let expected: Vec<u16> = "加密备份\0*.cbak\0All files\0*.*\0\0"
            .encode_utf16()
            .collect();
        assert_eq!(filter, expected);
    }
}

#[cfg(all(test, windows))]
mod window_placement_tests {
    use super::{capture_window_placement, restore_window_placement};
    use std::ptr;
    use windows_sys::Win32::{
        Foundation::{HINSTANCE, HWND, RECT},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetWindowRect, IsIconic,
            IsZoomed, RegisterClassW, SetWindowPos, ShowWindow, UnregisterClassW, SWP_NOACTIVATE,
            SWP_NOZORDER, SW_HIDE, SW_RESTORE, SW_SHOWMAXIMIZED, SW_SHOWMINNOACTIVE, WNDCLASSW,
            WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP,
            WS_THICKFRAME,
        },
    };

    struct TestWindow {
        handle: HWND,
        instance: HINSTANCE,
        class: Vec<u16>,
    }

    impl TestWindow {
        fn new() -> Self {
            let class: Vec<u16> = "Capstan isolated placement regression\0"
                .encode_utf16()
                .collect();
            // SAFETY: module lookup takes no borrowed pointers.
            let instance = unsafe { GetModuleHandleW(ptr::null()) };
            let definition = WNDCLASSW {
                lpfnWndProc: Some(DefWindowProcW),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            // SAFETY: the class name stays allocated until after the window and class are destroyed.
            assert_ne!(unsafe { RegisterClassW(&definition) }, 0);
            // SAFETY: all strings live through creation; no application callback data is supplied.
            let handle = unsafe {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    class.as_ptr(),
                    class.as_ptr(),
                    WS_POPUP | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX,
                    180,
                    160,
                    818,
                    610,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    instance,
                    ptr::null(),
                )
            };
            assert!(!handle.is_null());
            Self {
                handle,
                instance,
                class,
            }
        }

        fn geometry(&self) -> ([i32; 4], [i32; 2]) {
            let mut outer = RECT::default();
            let mut client = RECT::default();
            // SAFETY: this test owns a live window on the calling thread and stack output buffers.
            unsafe {
                assert_ne!(GetWindowRect(self.handle, &mut outer), 0);
                assert_ne!(GetClientRect(self.handle, &mut client), 0);
            }
            (
                [outer.left, outer.top, outer.right, outer.bottom],
                [client.right - client.left, client.bottom - client.top],
            )
        }
    }

    impl Drop for TestWindow {
        fn drop(&mut self) {
            // SAFETY: the window and class are test-owned and destroyed on their creation thread.
            unsafe {
                DestroyWindow(self.handle);
                UnregisterClassW(self.class.as_ptr(), self.instance);
            }
        }
    }

    #[test]
    fn real_window_restores_normal_geometry_from_normal_minimized_and_maximized_states() {
        let window = TestWindow::new();
        let handle = window.handle as isize;
        let baseline = capture_window_placement(handle).unwrap();
        let geometry = window.geometry();
        assert!(geometry.0[2] - geometry.0[0] > geometry.1[0]);
        assert!(geometry.0[3] - geometry.0[1] > geometry.1[1]);
        for (maximized, minimized) in [(false, false), (false, true), (true, false), (true, true)] {
            for _ in 0..3 {
                restore_window_placement(handle, baseline).unwrap();
                // SAFETY: this test owns the window; NOACTIVATE prevents taking the user's focus.
                unsafe {
                    if maximized {
                        ShowWindow(window.handle, SW_SHOWMAXIMIZED);
                    }
                    if minimized {
                        ShowWindow(window.handle, SW_SHOWMINNOACTIVE);
                    }
                    assert_eq!(IsIconic(window.handle) != 0, minimized);
                }
                let saved = capture_window_placement(handle).unwrap();
                // SAFETY: mutate only the isolated test window to simulate Quake geometry/hiding.
                unsafe {
                    ShowWindow(window.handle, SW_RESTORE);
                    assert_ne!(
                        SetWindowPos(
                            window.handle,
                            ptr::null_mut(),
                            60,
                            80,
                            1000,
                            500,
                            SWP_NOZORDER | SWP_NOACTIVATE
                        ),
                        0
                    );
                    ShowWindow(window.handle, SW_HIDE);
                }
                restore_window_placement(handle, saved).unwrap();
                // SAFETY: these getters inspect the test's live window without retaining pointers.
                unsafe {
                    assert_eq!(IsIconic(window.handle), 0);
                    assert_eq!(IsZoomed(window.handle) != 0, maximized);
                }
                // Returning from an originally maximized window must still recover its normal size.
                // SAFETY: SW_RESTORE targets only the isolated test window.
                unsafe { ShowWindow(window.handle, SW_RESTORE) };
                assert_eq!(
                    window.geometry(),
                    geometry,
                    "maximized={maximized}, minimized={minimized}"
                );
                // SAFETY: hide the test-owned window between scenarios.
                unsafe { ShowWindow(window.handle, SW_HIDE) };
            }
        }
        assert!(restore_window_placement(0, baseline).is_err());
    }

    #[test]
    fn invalid_window_has_no_capturable_placement() {
        assert!(capture_window_placement(0).is_err());
    }
}
