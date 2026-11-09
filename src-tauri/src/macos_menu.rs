//! The macOS application menu. A WKWebView only sees Cmd+C/V/X/A through the Edit
//! menu's responder-chain actions, so these items are what make them work at all.
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    menu::{AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
    AppHandle, Manager, Runtime,
};

const QUIT: &str = "capstan-quit";
/// The predefined items have no localized titles of their own.
static CHINESE: AtomicBool = AtomicBool::new(false);

struct Labels {
    about: &'static str,
    services: &'static str,
    hide: &'static str,
    hide_others: &'static str,
    show_all: &'static str,
    quit: &'static str,
    edit: &'static str,
    undo: &'static str,
    redo: &'static str,
    cut: &'static str,
    copy: &'static str,
    paste: &'static str,
    select_all: &'static str,
    view: &'static str,
    full_screen: &'static str,
    window: &'static str,
    minimize: &'static str,
    zoom: &'static str,
}

fn labels(chinese: bool) -> Labels {
    if chinese {
        Labels {
            about: "关于 Capstan",
            services: "服务",
            hide: "隐藏 Capstan",
            hide_others: "隐藏其他",
            show_all: "全部显示",
            quit: "退出 Capstan",
            edit: "编辑",
            undo: "撤销",
            redo: "重做",
            cut: "剪切",
            copy: "拷贝",
            paste: "粘贴",
            select_all: "全选",
            view: "显示",
            full_screen: "进入全屏幕",
            window: "窗口",
            minimize: "最小化",
            zoom: "缩放",
        }
    } else {
        Labels {
            about: "About Capstan",
            services: "Services",
            hide: "Hide Capstan",
            hide_others: "Hide Others",
            show_all: "Show All",
            quit: "Quit Capstan",
            edit: "Edit",
            undo: "Undo",
            redo: "Redo",
            cut: "Cut",
            copy: "Copy",
            paste: "Paste",
            select_all: "Select All",
            view: "View",
            full_screen: "Enter Full Screen",
            window: "Window",
            minimize: "Minimize",
            zoom: "Zoom",
        }
    }
}

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let text = labels(CHINESE.load(Ordering::Relaxed));
    let separator = || PredefinedMenuItem::separator(app);
    let info = app.package_info();
    let about = AboutMetadata {
        name: Some(info.name.clone()),
        version: Some(info.version.to_string()),
        copyright: app.config().bundle.copyright.clone(),
        ..Default::default()
    };
    // Quit runs the window's close path, which owns the unfinished-transfer
    // prompt and session cleanup; the predefined item would terminate directly.
    let quit = MenuItem::with_id(app, QUIT, text.quit, true, Some("Cmd+Q"))?;
    let application = Submenu::with_items(
        app,
        "Capstan",
        true,
        &[
            &PredefinedMenuItem::about(app, Some(text.about), Some(about))?,
            &separator()?,
            &PredefinedMenuItem::services(app, Some(text.services))?,
            &separator()?,
            &PredefinedMenuItem::hide(app, Some(text.hide))?,
            &PredefinedMenuItem::hide_others(app, Some(text.hide_others))?,
            &PredefinedMenuItem::show_all(app, Some(text.show_all))?,
            &separator()?,
            &quit,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        text.edit,
        true,
        &[
            &PredefinedMenuItem::undo(app, Some(text.undo))?,
            &PredefinedMenuItem::redo(app, Some(text.redo))?,
            &separator()?,
            &PredefinedMenuItem::cut(app, Some(text.cut))?,
            &PredefinedMenuItem::copy(app, Some(text.copy))?,
            &PredefinedMenuItem::paste(app, Some(text.paste))?,
            &PredefinedMenuItem::select_all(app, Some(text.select_all))?,
        ],
    )?;
    let view = Submenu::with_items(
        app,
        text.view,
        true,
        &[&PredefinedMenuItem::fullscreen(
            app,
            Some(text.full_screen),
        )?],
    )?;
    // No Close Window item: Cmd+W belongs to the focused terminal tab, and its
    // key equivalent would close every session once the WebView lets it through.
    let window = Submenu::with_items(
        app,
        text.window,
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some(text.minimize))?,
            &PredefinedMenuItem::maximize(app, Some(text.zoom))?,
        ],
    )?;
    let menu = Menu::with_items(app, &[&application, &edit, &view, &window])?;
    window.set_as_windows_menu_for_nsapp()?;
    Ok(menu)
}

pub fn handle_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    if event.id() == QUIT {
        match app.get_webview_window("main") {
            Some(window) => {
                let _ = window.close();
            }
            None => app.exit(0),
        }
    }
}

/// The WebView reports its locale after the initial menu is already on screen.
pub fn set_locale<R: Runtime>(app: &AppHandle<R>, locale: &str) {
    let chinese = locale.starts_with("zh");
    if CHINESE.swap(chinese, Ordering::Relaxed) != chinese {
        if let Ok(menu) = build(app) {
            let _ = app.set_menu(menu);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::labels;

    #[test]
    fn both_locales_label_the_commands_that_cannot_fall_back_to_the_system() {
        for (chinese, quit, paste) in [
            (false, "Quit Capstan", "Paste"),
            (true, "退出 Capstan", "粘贴"),
        ] {
            let text = labels(chinese);
            assert_eq!(text.quit, quit);
            assert_eq!(text.paste, paste);
        }
    }
}
