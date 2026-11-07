//! Shared SDK configuration for production and the isolated launch-failure probe.
use tauri_plugin_updater::UpdaterExt;

pub(super) fn builder<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    key: &str,
    target: &str,
) -> tauri_plugin_updater::UpdaterBuilder {
    app.updater_builder()
        .pubkey(key)
        .target(target)
        .clear_installer_args()
        .restart_after_install(false)
        .installer_arg("/R")
        // The SDK invokes this before ShellExecuteW, when launching can still fail.
        // Domain workflows have drained, but Tauri resources must remain usable
        // until the SDK successfully launches NSIS and immediately exits itself.
        .on_before_exit(|| {})
}
