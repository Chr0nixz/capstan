//! COM jump lists and the narrow foreground handoff used by launch forwarding.
use std::{os::windows::ffi::OsStrExt, path::Path};
use windows::{
    core::{Interface, HSTRING, PCWSTR},
    Win32::{
        Storage::EnhancedStorage::{PKEY_AppUserModel_ID, PKEY_Title},
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, StructuredStorage::PROPVARIANT,
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
        },
        UI::Shell::{
            Common::{IObjectArray, IObjectCollection},
            DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW,
            PropertiesSystem::IPropertyStore,
            SetCurrentProcessExplicitAppUserModelID, ShellLink,
        },
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub title: String,
    pub arguments: String,
}

struct Apartment;
impl Apartment {
    fn enter() -> windows::core::Result<Self> {
        // SAFETY: the guard balances this thread's successful COM initialization.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: the guard remains on the thread it initialized.
        unsafe { CoUninitialize() };
    }
}

pub fn set_process_identity(app_id: &str) -> windows::core::Result<()> {
    let app_id = HSTRING::from(app_id);
    // SAFETY: the UTF-16 input is valid for this synchronous call.
    unsafe { SetCurrentProcessExplicitAppUserModelID(PCWSTR(app_id.as_ptr())) }
}

/// Pass the foreground right from a newly launched process to the pipe owner.
pub fn allow_server_foreground(pipe: isize) -> bool {
    use windows_sys::Win32::{
        System::Pipes::GetNamedPipeServerProcessId,
        UI::WindowsAndMessaging::AllowSetForegroundWindow,
    };
    let mut process = 0;
    // SAFETY: the caller holds the open pipe handle through this call; the output is stack-owned.
    if unsafe { GetNamedPipeServerProcessId(pipe as _, &mut process) } != 0
        && process != 0
        && server_has_same_user(process)
    {
        // SAFETY: Windows validates the process ID and the caller's foreground rights.
        unsafe { AllowSetForegroundWindow(process) };
        return true;
    }
    false
}

fn server_has_same_user(process: u32) -> bool {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::HANDLE,
        Security::{EqualSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER},
        System::Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    };
    fn user_information(process: HANDLE) -> Option<Vec<usize>> {
        let mut token = std::ptr::null_mut();
        // SAFETY: the OS validates the handle, and token points to writable stack storage.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return None;
        }
        // SAFETY: OpenProcessToken returned a newly owned handle.
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        let mut length = 0;
        // SAFETY: the initial null-buffer query only obtains the required byte length.
        unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                std::ptr::null_mut(),
                0,
                &mut length,
            )
        };
        if length == 0 || length > 4096 {
            return None;
        }
        let mut buffer = vec![0_usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
        // SAFETY: the buffer is aligned for TOKEN_USER and has at least the requested capacity.
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                buffer.as_mut_ptr().cast(),
                length,
                &mut length,
            )
        } == 0
        {
            return None;
        }
        Some(buffer)
    }
    // SAFETY: requests only query access; the returned real handle is owned by the guard.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process) };
    if handle.is_null() {
        return false;
    }
    // SAFETY: OpenProcess returned a newly owned real handle.
    let process = unsafe { OwnedHandle::from_raw_handle(handle) };
    // SAFETY: the pseudo-handle remains owned by Windows and is not closed by this code.
    let own = user_information(unsafe { GetCurrentProcess() });
    let remote = user_information(process.as_raw_handle());
    let (Some(own), Some(remote)) = (own, remote) else {
        return false;
    };
    // SAFETY: TOKEN_USER is aligned and both SID pointers refer to these still-live token buffers.
    unsafe {
        let own = &*own.as_ptr().cast::<TOKEN_USER>();
        let remote = &*remote.as_ptr().cast::<TOKEN_USER>();
        EqualSid(own.User.Sid, remote.User.Sid) != 0
    }
}

fn link(executable: &Path, app_id: &str, entry: &Entry) -> windows::core::Result<IShellLinkW> {
    let executable = HSTRING::from(executable.as_os_str());
    let arguments = HSTRING::from(&entry.arguments);
    // SAFETY: COM is initialized by the caller; strings outlive every synchronous COM call.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(PCWSTR(executable.as_ptr()))?;
        link.SetArguments(PCWSTR(arguments.as_ptr()))?;
        link.SetIconLocation(PCWSTR(executable.as_ptr()), 0)?;
        let properties: IPropertyStore = link.cast()?;
        properties.SetValue(&PKEY_Title, &PROPVARIANT::from(entry.title.as_str()))?;
        properties.SetValue(&PKEY_AppUserModel_ID, &PROPVARIANT::from(app_id))?;
        properties.Commit()?;
        Ok(link)
    }
}

fn collection(
    executable: &Path,
    app_id: &str,
    entries: &[Entry],
) -> windows::core::Result<IObjectArray> {
    // SAFETY: COM is initialized; the collection retains each link until the list is committed.
    unsafe {
        let collection: IObjectCollection =
            CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
        for entry in entries {
            collection.AddObject(&link(executable, app_id, entry)?)?;
        }
        collection.cast()
    }
}

/// Replace the app's custom list. No known-category or automatic recent-document tracking is used.
pub fn publish(
    executable: &Path,
    app_id: &str,
    tasks: &[Entry],
    recent: &[Entry],
    recent_label: &str,
) -> windows::core::Result<()> {
    if tasks.len() > 6
        || recent.len() > 10
        || tasks.iter().chain(recent).any(|entry| {
            entry.title.is_empty()
                || entry.title.chars().count() > 256
                || entry.title.chars().any(char::is_control)
                || entry.arguments.len() > 4096
                || entry.arguments.contains('\0')
        })
        || executable.as_os_str().encode_wide().any(|value| value == 0)
    {
        return Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_INVALIDARG,
        ));
    }
    let _apartment = Apartment::enter()?;
    let app_id_wide = HSTRING::from(app_id);
    let category = HSTRING::from(recent_label);
    // SAFETY: all COM interfaces and strings remain alive until CommitList/AbortList completes.
    unsafe {
        let list: ICustomDestinationList =
            CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;
        list.SetAppID(PCWSTR(app_id_wide.as_ptr()))?;
        let mut slots = 0;
        let removed: IObjectArray = list.BeginList(&mut slots)?;
        let result = (|| {
            if !tasks.is_empty() {
                list.AddUserTasks(&collection(executable, app_id, tasks)?)?;
            }
            let recent = recent
                .iter()
                .filter(|entry| !was_removed(&removed, &entry.arguments))
                .take(slots as usize)
                .cloned()
                .collect::<Vec<_>>();
            if !recent.is_empty() {
                list.AppendCategory(
                    PCWSTR(category.as_ptr()),
                    &collection(executable, app_id, &recent)?,
                )?;
            }
            list.CommitList()
        })();
        if result.is_err() {
            let _ = list.AbortList();
        }
        result
    }
}

fn was_removed(removed: &IObjectArray, arguments: &str) -> bool {
    // SAFETY: the OS-provided array is valid for the current BeginList transaction.
    unsafe {
        let Ok(count) = removed.GetCount() else {
            return false;
        };
        for index in 0..count {
            if let Ok(link) = removed.GetAt::<IShellLinkW>(index) {
                let mut buffer = vec![0; 4096];
                if link.GetArguments(&mut buffer).is_ok() {
                    let end = buffer
                        .iter()
                        .position(|value| *value == 0)
                        .unwrap_or(buffer.len());
                    if String::from_utf16_lossy(&buffer[..end]) == arguments {
                        return true;
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::BSTR;

    #[test]
    fn forwarding_authorizes_the_current_user_and_rejects_invalid_handles() {
        assert!(server_has_same_user(std::process::id()));
        assert!(!server_has_same_user(u32::MAX));
        assert!(!allow_server_foreground(0));
    }

    #[test]
    fn jump_links_keep_literal_arguments_and_only_the_supplied_display_name() {
        let _apartment = Apartment::enter().unwrap();
        let exe = std::env::current_exe().unwrap();
        let entry = Entry {
            title: "Build server".into(),
            arguments: "--host 22222222-2222-4222-8222-222222222222".into(),
        };
        let link = link(&exe, "dev.capstan.app", &entry).unwrap();
        // SAFETY: COM and the output buffer are initialized and valid for these synchronous calls.
        unsafe {
            let mut arguments = vec![0; 4096];
            link.GetArguments(&mut arguments).unwrap();
            let end = arguments.iter().position(|value| *value == 0).unwrap();
            assert_eq!(String::from_utf16_lossy(&arguments[..end]), entry.arguments);
            let properties: IPropertyStore = link.cast().unwrap();
            assert_eq!(
                BSTR::try_from(&properties.GetValue(&PKEY_Title).unwrap())
                    .unwrap()
                    .to_string(),
                entry.title
            );
            assert_eq!(
                BSTR::try_from(&properties.GetValue(&PKEY_AppUserModel_ID).unwrap())
                    .unwrap()
                    .to_string(),
                "dev.capstan.app"
            );
        }
    }

    #[test]
    #[ignore = "publishes and removes an isolated Windows jump list; run explicitly on a desktop"]
    fn publishes_and_removes_an_isolated_jump_list() {
        let app_id = format!(
            "dev.capstan.jump-list-probe.{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let executable = std::env::current_exe().unwrap();
        let tasks = [Entry {
            title: "Local shell probe".into(),
            arguments: "--local --shell powershell51".into(),
        }];
        let recent = [Entry {
            title: "Host display name probe".into(),
            arguments: "--host 22222222-2222-4222-8222-222222222222".into(),
        }];
        let result = publish(&executable, &app_id, &tasks, &recent, "Capstan");
        let _apartment = Apartment::enter().unwrap();
        let id = HSTRING::from(&app_id);
        // SAFETY: removes only the list generated under this test's unique application ID.
        unsafe {
            let list: ICustomDestinationList =
                CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER).unwrap();
            list.DeleteList(PCWSTR(id.as_ptr())).unwrap();
        }
        result.unwrap();
    }
}
