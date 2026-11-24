//! Native pickers keep paths and secrets on the Rust side of IPC.
use capstan_vault::SecretText;
use std::{io, path::PathBuf};
use tauri::WebviewWindow;

pub enum Picker {
    /// `name` suggests a file name where the platform's save panel supports it.
    File {
        save: bool,
        #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
        name: Option<String>,
    },
    Folder,
    PrivateKey,
    Backup {
        save: bool,
    },
    Theme,
}

#[cfg(windows)]
pub async fn pick(
    window: &WebviewWindow,
    title: &'static str,
    locale: &str,
    picker: Picker,
) -> io::Result<Option<PathBuf>> {
    use capstan_native_windows::transfer::{pick_file, pick_file_with_pattern, pick_folder};

    let parent = window.hwnd().map_err(io::Error::other)?.0 as isize;
    let chinese = locale.starts_with("zh");
    let all_files = if chinese { "所有文件" } else { "All files" };
    tauri::async_runtime::spawn_blocking(move || match picker {
        Picker::File { save, .. } => pick_file(title, all_files, parent, save),
        Picker::Folder => pick_folder(title, parent),
        Picker::PrivateKey => capstan_native_windows::pick_private_key_file(
            title,
            if chinese {
                "私钥文件"
            } else {
                "Private key files"
            },
            all_files,
            parent,
        ),
        Picker::Backup { save } => {
            let filter = if chinese {
                "Capstan 加密备份"
            } else {
                "Capstan encrypted backup"
            };
            if save {
                capstan_native_windows::save_vault_backup_file(title, filter, all_files, parent)
            } else {
                capstan_native_windows::pick_vault_backup_file(title, filter, all_files, parent)
            }
        }
        Picker::Theme => pick_file_with_pattern(
            title,
            if chinese {
                "终端配色文件"
            } else {
                "Terminal color schemes"
            },
            "*.json;*.itermcolors",
            parent,
            false,
        ),
    })
    .await
    .map_err(io::Error::other)?
}

#[cfg(windows)]
pub async fn prompt_secret(
    window: &WebviewWindow,
    title: String,
    message: String,
    _locale: &str,
) -> io::Result<Option<SecretText>> {
    let parent = window.hwnd().map(|handle| handle.0 as isize).unwrap_or(0);
    tauri::async_runtime::spawn_blocking(move || {
        capstan_native_windows::prompt_secret(&title, &message, "", parent)?
            .map(|prompt| {
                String::from_utf16(prompt.password())
                    .map(SecretText::new)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
            })
            .transpose()
    })
    .await
    .map_err(io::Error::other)?
}

#[cfg(target_os = "macos")]
pub use macos::{open_folder, open_url, pick, prompt_secret, ModalDialog};

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use objc2::{rc::autoreleasepool, MainThreadMarker};
    use objc2_app_kit::{
        NSAlert, NSAlertFirstButtonReturn, NSModalResponseOK, NSOpenPanel, NSSavePanel,
        NSSecureTextField, NSWorkspace,
    };
    use objc2_foundation::{ns_string, NSArray, NSPoint, NSRect, NSSize, NSString, NSURL};
    use tauri::Manager;

    #[derive(Default)]
    pub struct ModalDialog(std::sync::Arc<tokio::sync::Mutex<()>>);

    async fn show<T: Send + 'static>(
        window: &WebviewWindow,
        operation: impl FnOnce(MainThreadMarker) -> io::Result<T> + Send + 'static,
    ) -> io::Result<T> {
        let state = window.state::<ModalDialog>();
        let guard = state.0.clone().try_lock_owned().map_err(io::Error::other)?;
        let (send, receive) = tokio::sync::oneshot::channel();
        window
            .run_on_main_thread(move || {
                let _guard = guard;
                let result = autoreleasepool(|_| {
                    let main =
                        MainThreadMarker::new().ok_or_else(|| io::Error::other("AppKit thread"))?;
                    operation(main)
                });
                let _ = send.send(result);
            })
            .map_err(io::Error::other)?;
        receive.await.map_err(io::Error::other)?
    }

    pub async fn pick(
        window: &WebviewWindow,
        title: &'static str,
        _locale: &str,
        picker: Picker,
    ) -> io::Result<Option<PathBuf>> {
        show(window, move |main| {
            let panel = match &picker {
                Picker::File { save: true, .. } | Picker::Backup { save: true } => {
                    let panel = NSSavePanel::savePanel(main);
                    panel.setCanCreateDirectories(true);
                    if matches!(picker, Picker::Backup { .. }) {
                        set_extensions(&panel, &["cbak"]);
                        panel.setNameFieldStringValue(ns_string!("Capstan.cbak"));
                    }
                    if let Picker::File {
                        name: Some(name), ..
                    } = &picker
                    {
                        panel.setNameFieldStringValue(&NSString::from_str(name));
                    }
                    panel
                }
                _ => {
                    let panel = NSOpenPanel::openPanel(main);
                    let folder = matches!(picker, Picker::Folder);
                    panel.setCanChooseFiles(!folder);
                    panel.setCanChooseDirectories(folder);
                    panel.setAllowsMultipleSelection(false);
                    panel.setResolvesAliases(false);
                    match &picker {
                        Picker::Backup { .. } => set_extensions(&panel, &["cbak"]),
                        Picker::Theme => set_extensions(&panel, &["json", "itermcolors"]),
                        // Finder hides ~/.ssh, the one place these keys normally live.
                        Picker::PrivateKey => {
                            panel.setShowsHiddenFiles(true);
                            if let Some(home) = std::env::var_os("HOME") {
                                let ssh = PathBuf::from(home).join(".ssh");
                                let path = NSString::from_str(&ssh.to_string_lossy());
                                panel.setDirectoryURL(Some(&NSURL::fileURLWithPath(&path)));
                            }
                        }
                        _ => {}
                    }
                    objc2::rc::Retained::into_super(panel)
                }
            };
            panel.setTitle(Some(&NSString::from_str(title)));
            if panel.runModal() != NSModalResponseOK {
                return Ok(None);
            }
            panel
                .URL()
                .and_then(|url| url.path())
                .map(|path| Some(PathBuf::from(path.to_string())))
                .ok_or_else(|| io::Error::other("missing picker path"))
        })
        .await
    }

    // This API accepts filename extensions directly and works on the macOS deployment target.
    #[allow(deprecated)]
    fn set_extensions(panel: &NSSavePanel, extensions: &[&str]) {
        let types = extensions
            .iter()
            .map(|value| NSString::from_str(value))
            .collect::<Vec<_>>();
        panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&types)));
    }

    pub async fn prompt_secret(
        window: &WebviewWindow,
        title: String,
        message: String,
        locale: &str,
    ) -> io::Result<Option<SecretText>> {
        let chinese = locale.starts_with("zh");
        show(window, move |main| {
            let alert = NSAlert::new(main);
            alert.setMessageText(&NSString::from_str(&title));
            alert.setInformativeText(&NSString::from_str(&message));
            let field = NSSecureTextField::initWithFrame(
                main.alloc(),
                NSRect::new(NSPoint::ZERO, NSSize::new(280.0, 24.0)),
            );
            alert.setAccessoryView(Some(&field));
            let _ = alert.addButtonWithTitle(if chinese {
                ns_string!("确定")
            } else {
                ns_string!("OK")
            });
            let _ = alert.addButtonWithTitle(if chinese {
                ns_string!("取消")
            } else {
                ns_string!("Cancel")
            });
            alert.layout();
            alert.window().setInitialFirstResponder(Some(&field));
            let accepted = alert.runModal() == NSAlertFirstButtonReturn;
            let secret = accepted.then(|| SecretText::new(field.stringValue().to_string()));
            field.setStringValue(ns_string!(""));
            Ok(secret)
        })
        .await
    }

    pub fn open_url(url: &str) -> bool {
        NSURL::URLWithString(&NSString::from_str(url))
            .is_some_and(|url| NSWorkspace::sharedWorkspace().openURL(&url))
    }

    /// Show an application-owned directory in Finder.
    pub fn open_folder(path: &std::path::Path) -> bool {
        path.to_str().is_some_and(|path| {
            let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(path), true);
            NSWorkspace::sharedWorkspace().openURL(&url)
        })
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub async fn pick(
    _: &WebviewWindow,
    _: &'static str,
    _: &str,
    picker: Picker,
) -> io::Result<Option<PathBuf>> {
    match picker {
        Picker::File { save, .. } | Picker::Backup { save } => {
            let _ = save;
        }
        _ => {}
    }
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub async fn prompt_secret(
    _: &WebviewWindow,
    _: String,
    _: String,
    _: &str,
) -> io::Result<Option<SecretText>> {
    Err(io::ErrorKind::Unsupported.into())
}
