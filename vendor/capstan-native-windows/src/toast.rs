//! Read the installed shortcut's identity and deliver a text-only WinRT toast.
use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::Path, ptr};
use windows::{
    core::{Interface, BSTR, HSTRING, PCWSTR},
    Data::Xml::Dom::XmlDocument,
    Win32::{
        Storage::EnhancedStorage::PKEY_AppUserModel_ID,
        System::{
            Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ},
            WinRT::{RoInitialize, RoUninitialize, RO_INIT_SINGLETHREADED},
        },
        UI::Shell::{IShellLinkW, PropertiesSystem::IPropertyStore, ShellLink, SLGP_RAWPATH},
    },
    UI::Notifications::{ToastNotification, ToastNotificationManager},
};

struct Apartment;

impl Apartment {
    fn enter() -> windows::core::Result<Self> {
        // SAFETY: initializes this calling thread. The guard stays on this thread.
        unsafe { RoInitialize(RO_INIT_SINGLETHREADED) }?;
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: balances this guard's successful RoInitialize.
        unsafe { RoUninitialize() };
    }
}

/// Match the default NSIS Start menu shortcut to this executable and bundle ID.
/// The shortcut is inspected without resolving it or writing to it.
#[must_use]
pub fn has_installed_identity(shortcut: &Path, executable: &Path, app_id: &str) -> bool {
    read_shortcut(shortcut).is_ok_and(|(target, id)| {
        id == app_id
            && target
                .canonicalize()
                .ok()
                .zip(executable.canonicalize().ok())
                .is_some_and(|(target, executable)| target == executable)
    })
}

fn read_shortcut(shortcut: &Path) -> windows::core::Result<(std::path::PathBuf, String)> {
    let _apartment = Apartment::enter()?;
    let filename = HSTRING::from(shortcut.as_os_str());
    let mut path = vec![0; 32_768];
    // SAFETY: COM is initialized. The input string and output buffer are valid
    // for the calls, and a null find-data pointer is permitted by IShellLinkW.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.cast::<IPersistFile>()?
            .Load(PCWSTR(filename.as_ptr()), STGM_READ)?;
        link.GetPath(&mut path, ptr::null_mut(), SLGP_RAWPATH.0 as u32)?;
        let properties: IPropertyStore = link.cast()?;
        let property = properties.GetValue(&PKEY_AppUserModel_ID)?;
        let id = BSTR::try_from(&property)?.to_string();
        let end = path
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(path.len());
        Ok((OsString::from_wide(&path[..end]).into(), id))
    }
}

fn toast_xml(title: &str, body: &str) -> windows::core::Result<XmlDocument> {
    let xml = XmlDocument::new()?;
    xml.LoadXml(&HSTRING::from(
        r#"<toast><visual><binding template="ToastGeneric"><text/><text/></binding></visual></toast>"#,
    ))?;
    let text = xml.GetElementsByTagName(&HSTRING::from("text"))?;
    text.Item(0)?.SetInnerText(&HSTRING::from(title))?;
    text.Item(1)?.SetInnerText(&HSTRING::from(body))?;
    Ok(xml)
}

pub fn show(app_id: &str, title: &str, body: &str) -> windows::core::Result<()> {
    let _apartment = Apartment::enter()?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id))?;
    let notification = ToastNotification::CreateToastNotification(&toast_xml(title, body)?)?;
    notifier.Show(&notification)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;

    #[test]
    fn verifies_the_shortcut_aumid_and_its_target_without_installing_it() {
        let _apartment = Apartment::enter().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let shortcut = directory.path().join("Capstan.lnk");
        let exe = std::env::current_exe().unwrap();
        let other = directory.path().join("other.exe");
        std::fs::write(&other, []).unwrap();
        let app_id = "dev.capstan.app";
        assert!(!has_installed_identity(&shortcut, &exe, app_id));
        let filename = HSTRING::from(shortcut.as_os_str());
        let target = HSTRING::from(exe.as_os_str());
        // SAFETY: COM is initialized and both UTF-16 strings live through the calls.
        unsafe {
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
            link.SetPath(PCWSTR(target.as_ptr())).unwrap();
            let properties: IPropertyStore = link.cast().unwrap();
            properties
                .SetValue(&PKEY_AppUserModel_ID, &PROPVARIANT::from(app_id))
                .unwrap();
            properties.Commit().unwrap();
            link.cast::<IPersistFile>()
                .unwrap()
                .Save(PCWSTR(filename.as_ptr()), true)
                .unwrap();
        }
        assert!(has_installed_identity(&shortcut, &exe, app_id));
        assert!(!has_installed_identity(&shortcut, &other, app_id));
        assert!(!has_installed_identity(&shortcut, &exe, "other.app"));
    }

    #[test]
    fn toast_xml_inserts_text_without_interpreting_it_as_markup() {
        let _apartment = Apartment::enter().unwrap();
        let xml = toast_xml("Capstan", "<text> & 中文").unwrap();
        let nodes = xml.GetElementsByTagName(&HSTRING::from("text")).unwrap();
        assert_eq!(nodes.Length().unwrap(), 2);
        assert_eq!(nodes.Item(1).unwrap().InnerText().unwrap(), "<text> & 中文");
        assert!(ToastNotification::CreateToastNotification(&xml).is_ok());
    }
}
