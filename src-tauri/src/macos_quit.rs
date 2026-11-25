//! Quitting from the Dock, at logout or at shutdown. AppKit asks the
//! application delegate first, but tao's delegate does not answer, so these
//! quits ended the app at once: no question about unfinished transfers, and no
//! orderly shutdown. This adds the answer: the quit waits while the main
//! window's close flow runs, and the flow tells AppKit how it ended.
use objc2::{
    ffi::class_addMethod,
    runtime::{AnyClass, AnyObject, Imp, Sel},
    sel, MainThreadMarker,
};
use objc2_app_kit::NSApplication;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    OnceLock,
};
use tauri::{AppHandle, Manager};

static APP: OnceLock<AppHandle> = OnceLock::new();
/// AppKit is waiting for [`answer`].
static PENDING: AtomicBool = AtomicBool::new(false);

// NSApplicationTerminateReply
const TERMINATE_CANCEL: usize = 0;
const TERMINATE_NOW: usize = 1;
const TERMINATE_LATER: usize = 2;

/// Adds `applicationShouldTerminate:` to the application delegate. Call once
/// on the main thread after the event loop exists.
pub fn install(app: &AppHandle) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if APP.set(app.clone()).is_err() {
        return;
    }
    if let Some(delegate) = NSApplication::sharedApplication(mtm).delegate() {
        add_handler(AsRef::<AnyObject>::as_ref(&*delegate).class());
    }
}

/// Returns false, changing nothing, when `class` already answers; its own
/// answer then stands.
#[allow(unsafe_code)] // One Objective-C runtime call; the handler itself is safe code.
fn add_handler(class: &AnyClass) -> bool {
    let handler: extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> usize = should_terminate;
    // SAFETY: the signature matches `-(NSApplicationTerminateReply)
    // applicationShouldTerminate:(NSApplication *)sender`, encoded "Q@:@" for
    // a 64-bit NSUInteger.
    unsafe {
        let imp = std::mem::transmute::<
            extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> usize,
            Imp,
        >(handler);
        class_addMethod(
            std::ptr::from_ref(class).cast_mut(),
            sel!(applicationShouldTerminate:),
            imp,
            c"Q@:@".as_ptr(),
        )
        .as_bool()
    }
}

extern "C-unwind" fn should_terminate(_: &AnyObject, _: Sel, _: *mut AnyObject) -> usize {
    let Some(window) = APP.get().and_then(|app| app.get_webview_window("main")) else {
        return TERMINATE_NOW;
    };
    // A second request while one waits: the first still gets its answer.
    if PENDING.swap(true, Ordering::SeqCst) {
        return TERMINATE_CANCEL;
    }
    // The same path as Quit in the menu: it asks about unfinished work, then
    // shuts down in order.
    if window.close().is_err() {
        PENDING.store(false, Ordering::SeqCst);
        return TERMINATE_NOW;
    }
    TERMINATE_LATER
}

/// Tells a waiting quit whether the close flow finished (`true`, the app
/// ends) or stopped to ask the user (`false`, the quit and any logout are
/// canceled). Returns whether a quit was waiting.
pub fn answer(quit: bool) -> bool {
    if !PENDING.swap(false, Ordering::SeqCst) {
        return false;
    }
    let Some(app) = APP.get() else {
        return false;
    };
    let replied = app.run_on_main_thread(move || {
        if let Some(mtm) = MainThreadMarker::new() {
            NSApplication::sharedApplication(mtm).replyToApplicationShouldTerminate(quit);
        }
    });
    replied.is_ok()
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;
    use objc2::{
        msg_send,
        rc::Retained,
        runtime::{ClassBuilder, NSObject},
        ClassType,
    };

    #[test]
    fn a_delegate_without_an_answer_gets_one_that_appkit_can_call() {
        let class = ClassBuilder::new(c"CapstanQuitTestDelegate", NSObject::class())
            .expect("unique test class")
            .register();
        assert!(add_handler(class));
        // A second install leaves the first handler in place.
        assert!(!add_handler(class));
        let delegate: Retained<AnyObject> = unsafe { msg_send![class, new] };
        let responds: bool =
            unsafe { msg_send![&*delegate, respondsToSelector: sel!(applicationShouldTerminate:)] };
        assert!(responds);
        // Without the running app there is no close flow to wait for.
        let reply: usize = unsafe {
            msg_send![&*delegate, applicationShouldTerminate: std::ptr::null_mut::<AnyObject>()]
        };
        assert_eq!(reply, TERMINATE_NOW);
        assert!(!PENDING.load(Ordering::SeqCst));
        assert!(!answer(true));
    }
}
