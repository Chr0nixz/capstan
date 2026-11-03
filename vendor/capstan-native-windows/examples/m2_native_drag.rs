//! Standalone M2-01 platform probe, never linked into the application.
#![allow(clippy::pedantic)]
#[cfg(windows)]
mod native {
    use capstan_native_windows::staged_drag::{DragOutcome, StagedDrag};
    use std::{
        io::{self, Write},
        path::{Path, PathBuf},
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{
            BeginPaint, DrawFrameControl, DrawTextW, EndPaint, InvalidateRect, DFCS_BUTTONPUSH,
            DFC_BUTTON, DT_LEFT, DT_WORDBREAK, PAINTSTRUCT,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    };

    #[derive(Clone, Copy)]
    pub(super) enum Language {
        English,
        Chinese,
    }
    impl Language {
        fn text(self, english: &'static str, chinese: &'static str) -> &'static str {
            match self {
                Self::English => english,
                Self::Chinese => chinese,
            }
        }
    }
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    pub(super) fn paths(root: &Path, choice: usize) -> Vec<PathBuf> {
        let file = root.join("staging").join("sample.txt");
        let directory = root.join("staging").join("folder with spaces");
        match choice {
            0 => vec![file],
            1 => vec![directory],
            _ => vec![file, directory],
        }
    }
    pub(super) fn validate(root: &Path) -> io::Result<()> {
        let name = root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let suffix = name.strip_prefix("m2-drag-").unwrap_or("");
        let target = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .canonicalize()?;
        if !root.is_absolute()
            || suffix.len() != 12
            || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
            || root.canonicalize()?.parent() != Some(target.as_path())
            || std::fs::read(root.join("probe.marker"))? != b"CAPSTAN_M2_DRAG_PROBE_V1"
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid native drag fixture",
            ));
        }
        Ok(())
    }
    struct State {
        root: PathBuf,
        language: Language,
        status: &'static str,
        failed: bool,
    }
    // SAFETY: the window owns no Rust allocation. State is a Box retained by
    // run() through the complete native message loop, including modal OLE.
    unsafe extern "system" fn procedure(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_NCCREATE {
            let create = &*(lparam as *const CREATESTRUCTW);
            SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut State;
        match message {
            WM_PAINT if !state.is_null() => {
                let state = &*state;
                let mut paint: PAINTSTRUCT = std::mem::zeroed();
                let context = BeginPaint(window, &mut paint);
                let instructions = wide(state.language.text(
                    "Drag each box to its matching Explorer folder. Hold the left button, then drop. Also test Escape cancellation. This is a staged local-copy probe, not SFTP download.",
                    "按住左键从下方区域拖到对应的资源管理器目录。另做一次按 Esc 取消。这里只验证本地暂存数据的拖出能力，不代表 SFTP 拖拽下载。"));
                let mut area = RECT {
                    left: 20,
                    top: 15,
                    right: 540,
                    bottom: 86,
                };
                DrawTextW(
                    context,
                    instructions.as_ptr(),
                    -1,
                    &mut area,
                    DT_LEFT | DT_WORDBREAK,
                );
                let labels = [
                    state.language.text(
                        "1. File -> received/single-file",
                        "1. 文件 → received/single-file",
                    ),
                    state.language.text(
                        "2. Folder -> received/folder",
                        "2. 文件夹 → received/folder",
                    ),
                    state.language.text(
                        "3. Both -> received/multiple",
                        "3. 两项一起 → received/multiple",
                    ),
                ];
                for (index, label) in labels.iter().enumerate() {
                    let mut rect = RECT {
                        left: 20,
                        top: 96 + index as i32 * 52,
                        right: 540,
                        bottom: 136 + index as i32 * 52,
                    };
                    DrawFrameControl(context, &mut rect, DFC_BUTTON, DFCS_BUTTONPUSH);
                    rect.left += 12;
                    rect.top += 10;
                    let text = wide(label);
                    DrawTextW(context, text.as_ptr(), -1, &mut rect, DT_LEFT);
                }
                let mut area = RECT {
                    left: 20,
                    top: 264,
                    right: 540,
                    bottom: 315,
                };
                let status = wide(state.status);
                DrawTextW(
                    context,
                    status.as_ptr(),
                    -1,
                    &mut area,
                    DT_LEFT | DT_WORDBREAK,
                );
                EndPaint(window, &paint);
                0
            }
            WM_LBUTTONDOWN if !state.is_null() => {
                let x = (lparam & 0xffff) as i16;
                let y = ((lparam >> 16) & 0xffff) as i16;
                if !(20..=540).contains(&x) {
                    return 0;
                }
                let choice = (0..3)
                    .find(|index| (96 + index * 52..=136 + index * 52).contains(&i32::from(y)));
                if let Some(choice) = choice {
                    // Do not retain a mutable Rust borrow across the reentrant
                    // OLE message pump; paint and window messages may recur.
                    let root = (*state).root.clone();
                    let result =
                        StagedDrag::prepare(&root.join("staging"), &paths(&root, choice as usize))
                            .and_then(|drag| drag.start(window as isize));
                    let state = &mut *state;
                    state.status = match result {
                        Ok(DragOutcome::Copied) => state.language.text(
                            "Copy accepted; verify Explorer files after closing this probe.",
                            "复制已接受；关闭此探针后核对资源管理器中的实际文件。",
                        ),
                        Ok(DragOutcome::Canceled) => state.language.text(
                            "Canceled; staged sources remain available.",
                            "已取消；暂存来源仍保留。",
                        ),
                        Err(_) => {
                            state.failed = true;
                            state.language.text(
                                "Native drag failed; retain the probe report.",
                                "原生拖拽失败；请保留探针报告。",
                            )
                        }
                    };
                    let outcome = match result {
                        Ok(DragOutcome::Copied) => "copied",
                        Ok(DragOutcome::Canceled) => "canceled",
                        Err(_) => "failed",
                    };
                    let _ = writeln!(io::stdout(), "{{\"choice\":{choice},\"outcome\":\"{outcome}\",\"desktopAcceptance\":false}}");
                    InvalidateRect(window, ptr::null(), 1);
                }
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
    pub(super) fn run(root: PathBuf, language: Language) -> io::Result<()> {
        validate(&root)?;
        let mut state = Box::new(State {
            root,
            language,
            status: language.text("Ready", "就绪"),
            failed: false,
        });
        let class = wide("CapstanM2NativeDragProbe");
        let title = wide(language.text("Capstan native drag probe", "Capstan 原生拖出探针"));
        // SAFETY: class/title buffers and boxed State live until all windows
        // are destroyed; procedure is this module's correctly typed callback.
        unsafe {
            let instance = GetModuleHandleW(ptr::null());
            let definition = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
                hbrBackground: (6_usize) as _,
                ..std::mem::zeroed()
            };
            if RegisterClassW(&definition) == 0 {
                return Err(io::Error::last_os_error());
            }
            let window = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                590,
                380,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                (&mut *state as *mut State).cast(),
            );
            if window.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut message: MSG = std::mem::zeroed();
            loop {
                let result = GetMessageW(&mut message, ptr::null_mut(), 0, 0);
                if result == 0 {
                    break;
                }
                if result < 0 {
                    DestroyWindow(window);
                    return Err(io::Error::last_os_error());
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if state.failed {
            Err(io::Error::other("native drag probe failed"))
        } else {
            Ok(())
        }
    }
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !(2..=3).contains(&args.len()) {
        return Err("usage: m2_native_drag ABSOLUTE_ROOT en|zh-CN [--self-test]".into());
    }
    let root = std::path::PathBuf::from(&args[0]);
    native::validate(&root)?;
    let language = match args[1].to_str() {
        Some("en") => native::Language::English,
        Some("zh-CN") => native::Language::Chinese,
        _ => return Err("invalid locale".into()),
    };
    if args.len() == 3 {
        if args[2] != "--self-test" {
            return Err("invalid probe option".into());
        }
        for choice in 0..3 {
            let drag = capstan_native_windows::staged_drag::StagedDrag::prepare(
                &root.join("staging"),
                &native::paths(&root, choice),
            )?;
            drop(drag);
        }
        println!("{{\"dataObjectsPrepared\":3,\"interactiveDragExecuted\":false,\"desktopAcceptance\":false}}");
        return Ok(());
    }
    native::run(root, language)?;
    Ok(())
}
#[cfg(not(windows))]
fn main() {
    eprintln!("The native drag probe requires Windows.");
}
