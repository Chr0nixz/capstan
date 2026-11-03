//! Win32 calls stay in this dependency; every entry runs on the HWND's owner thread.
use crate::Error;
use std::sync::{Arc, OnceLock};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{GetStockObject, ValidateRect, NULL_BRUSH},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT},
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

const CHILD_PROP: &[u16] = &[67, 97, 112, 115, 116, 97, 110, 83, 110, 97, 112, 0];
const CLASS_NAME: &[u16] = &[
    67, 97, 112, 115, 116, 97, 110, 83, 110, 97, 112, 79, 118, 101, 114, 108, 97, 121, 0,
];
const SUBCLASS_ID: usize = 0x4353_4e50;
const ENTER: &str = "tauri-snap://snap/mouseenter";
const LEAVE: &str = "tauri-snap://snap/mouseleave";
type Emit = Arc<dyn Fn(&'static str) + Send + Sync>;

struct SnapState {
    parent: HWND,
    hovering: bool,
    pressed: bool,
    emit: Emit,
}

fn owns_window(hwnd: HWND) -> bool {
    // SAFETY: these APIs only query an opaque handle. No pointers are dereferenced.
    unsafe {
        IsWindow(hwnd) != 0
            && GetWindowThreadProcessId(hwnd, std::ptr::null_mut()) == GetCurrentThreadId()
    }
}

fn register_class() -> Result<(), Error> {
    static REGISTERED: OnceLock<bool> = OnceLock::new();
    let registered = REGISTERED.get_or_init(|| {
        // SAFETY: static NUL-terminated class name; the module lives for the process.
        unsafe {
            let class = WNDCLASSW {
                lpfnWndProc: Some(DefWindowProcW),
                hInstance: GetModuleHandleW(std::ptr::null()),
                hbrBackground: GetStockObject(NULL_BRUSH) as _,
                lpszClassName: CLASS_NAME.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassW(&class) != 0
        }
    });
    if *registered {
        Ok(())
    } else {
        Err(Error::Native)
    }
}

pub fn update(parent: usize, rect: [i32; 4], emit: Emit) -> Result<(), Error> {
    let parent = parent as HWND;
    if !owns_window(parent) {
        return Err(Error::Closed);
    }
    let [x, y, width, height] = rect;
    // SAFETY: owner-thread checked above, RECT is initialized and writable.
    unsafe {
        let mut client: RECT = std::mem::zeroed();
        if GetClientRect(parent, &mut client) == 0
            || x < 0
            || y < 0
            || width <= 0
            || height <= 0
            || x.checked_add(width)
                .map_or(true, |right| right > client.right)
            || y.checked_add(height)
                .map_or(true, |bottom| bottom > client.bottom)
        {
            return Err(Error::Bounds);
        }
        let mut child = GetPropW(parent, CHILD_PROP.as_ptr()) as HWND;
        if child.is_null() {
            register_class()?;
            child = CreateWindowExW(
                0,
                CLASS_NAME.as_ptr(),
                std::ptr::null(),
                WS_CHILD | WS_CLIPSIBLINGS,
                x,
                y,
                width,
                height,
                parent,
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            );
            if child.is_null() {
                return Err(Error::Native);
            }
            let state = Box::into_raw(Box::new(SnapState {
                parent,
                hovering: false,
                pressed: false,
                emit,
            }));
            if SetWindowSubclass(child, Some(child_proc), SUBCLASS_ID, state as usize) == 0 {
                // The callback never acquired ownership on this failure path.
                drop(Box::from_raw(state));
                DestroyWindow(child);
                return Err(Error::Native);
            }
            if SetPropW(parent, CHILD_PROP.as_ptr(), child) == 0 {
                // WM_NCDESTROY now owns the Box and releases it exactly once.
                DestroyWindow(child);
                return Err(Error::Native);
            }
        }
        if SetWindowPos(
            child,
            HWND_TOP,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        ) == 0
        {
            detach(parent as usize)?;
            return Err(Error::Native);
        }
    }
    Ok(())
}

pub fn detach(parent: usize) -> Result<(), Error> {
    let parent = parent as HWND;
    if !owns_window(parent) {
        return Err(Error::Closed);
    }
    // SAFETY: the property is only installed here and contains our child HWND.
    unsafe {
        let child = GetPropW(parent, CHILD_PROP.as_ptr()) as HWND;
        if !child.is_null() && DestroyWindow(child) == 0 {
            return Err(Error::Native);
        }
        RemovePropW(parent, CHILD_PROP.as_ptr());
    }
    Ok(())
}

fn screen_point(lparam: LPARAM) -> (i32, i32) {
    // WM_NCHITTEST carries signed screen coordinates, including negative monitors.
    (
        i32::from(lparam as u16 as i16),
        i32::from((lparam >> 16) as u16 as i16),
    )
}

unsafe extern "system" fn child_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass: usize,
    reference: usize,
) -> LRESULT {
    // SAFETY: installation transfers one Box to this owner-thread callback. The
    // subclass is removed before WM_NCDESTROY drops it. No reference survives a
    // call that can re-enter the procedure (event emission or SendMessageW).
    unsafe {
        let pointer = reference as *mut SnapState;
        if pointer.is_null() {
            return DefSubclassProc(hwnd, msg, wparam, lparam);
        }
        match msg {
            WM_NCDESTROY => {
                RemoveWindowSubclass(hwnd, Some(child_proc), subclass);
                let state = Box::from_raw(pointer);
                if GetPropW(state.parent, CHILD_PROP.as_ptr()) == hwnd {
                    RemovePropW(state.parent, CHILD_PROP.as_ptr());
                }
                let emit = state.emit.clone();
                drop(state);
                emit(LEAVE);
                return DefSubclassProc(hwnd, msg, wparam, lparam);
            }
            // A NULL_BRUSH and validation without painting leave the WebView
            // pixels visible. A layered/alpha-zero window loses native hit tests.
            WM_ERASEBKGND => return 1,
            WM_PAINT => {
                ValidateRect(hwnd, std::ptr::null());
                return 0;
            }
            WM_NCHITTEST => {
                let first = !(*pointer).hovering;
                (*pointer).hovering = true;
                if first {
                    let emit = (*pointer).emit.clone();
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE | TME_NONCLIENT,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    TrackMouseEvent(&mut track);
                    emit(ENTER);
                }
                return HTMAXBUTTON as LRESULT;
            }
            WM_NCMOUSELEAVE => {
                (*pointer).hovering = false;
                (*pointer).pressed = false;
                let emit = (*pointer).emit.clone();
                emit(LEAVE);
            }
            WM_NCLBUTTONDOWN if wparam == HTMAXBUTTON as usize => {
                (*pointer).pressed = true;
                return 0;
            }
            WM_NCLBUTTONUP if wparam == HTMAXBUTTON as usize => {
                let pressed = (*pointer).pressed;
                (*pointer).pressed = false;
                let parent = (*pointer).parent;
                let (x, y) = screen_point(lparam);
                let mut rect: RECT = std::mem::zeroed();
                if pressed
                    && GetWindowRect(hwnd, &mut rect) != 0
                    && x >= rect.left
                    && x < rect.right
                    && y >= rect.top
                    && y < rect.bottom
                {
                    let command = if IsZoomed(parent) != 0 {
                        SC_RESTORE
                    } else {
                        SC_MAXIMIZE
                    };
                    SendMessageW(parent, WM_SYSCOMMAND, command as WPARAM, 0);
                }
                return 0;
            }
            WM_SETCURSOR => {
                SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_ARROW));
                return 1;
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn negative_monitor_coordinates_are_signed() {
        let packed = (u32::from((-50_i16) as u16) << 16) | u32::from((-1200_i16) as u16);
        assert_eq!(screen_point(packed as isize), (-1200, -50));
    }

    #[test]
    fn real_native_child_hits_maximize_reuses_and_cleans_up_without_painting() {
        // This hidden native window tests messages and ownership, not the OS flyout.
        unsafe {
            let parent = CreateWindowExW(
                0,
                [83_u16, 84, 65, 84, 73, 67, 0].as_ptr(),
                std::ptr::null(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                1280,
                800,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            );
            assert!(!parent.is_null());
            let events = Arc::new(AtomicUsize::new(0));
            let count = events.clone();
            let emit: Emit = Arc::new(move |_| {
                count.fetch_add(1, Ordering::Relaxed);
            });
            assert!(matches!(
                update(parent as usize, [2000, 0, 46, 35], emit.clone()),
                Err(Error::Bounds)
            ));
            update(parent as usize, [1150, 0, 46, 35], emit.clone()).unwrap();
            let child = GetPropW(parent, CHILD_PROP.as_ptr()) as HWND;
            assert!(!child.is_null());
            assert_eq!(
                SendMessageW(child, WM_NCHITTEST, 0, 0),
                HTMAXBUTTON as LRESULT
            );
            assert_eq!(SendMessageW(child, WM_ERASEBKGND, 0, 0), 1);
            assert_eq!(
                SendMessageW(child, WM_NCLBUTTONUP, HTMAXBUTTON as usize, 0),
                0
            );
            assert_eq!(IsZoomed(parent), 0); // A stray mouse-up cannot toggle.
            update(parent as usize, [1100, 0, 58, 44], emit).unwrap();
            assert_eq!(GetPropW(parent, CHILD_PROP.as_ptr()) as HWND, child);
            detach(parent as usize).unwrap();
            detach(parent as usize).unwrap();
            assert_eq!(IsWindow(child), 0);
            assert!(GetPropW(parent, CHILD_PROP.as_ptr()).is_null());
            assert_eq!(events.load(Ordering::Relaxed), 2);
            DestroyWindow(parent);
        }
    }
}
