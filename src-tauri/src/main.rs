fn benchmark_preflight() -> bool {
    if std::env::args_os().any(|argument| argument == "--m0-check") {
        if cfg!(feature = "m0-benchmarks") {
            println!("capstan-m0-benchmarks-v1");
        } else {
            std::process::exit(2);
        }
        return true;
    }
    false
}

#[cfg(not(feature = "tauri-runtime"))]
fn main() {
    if benchmark_preflight() {
        return;
    }
    // The full Tauri builder is enabled by the desktop packaging profile.  The
    // default binary remains useful in CI and for exercising discovery logic.
    let summary = capstan_app::runtime_summary();
    println!("detected {} local shell(s)", summary.shells.len());
}

#[cfg(feature = "tauri-runtime")]
fn main() {
    if benchmark_preflight() {
        return;
    }
    // Synchronous commands run on the AppKit main thread, and core services reach
    // tokio::spawn from them; without a runtime context that panics and aborts.
    #[cfg(not(windows))]
    let runtime = tauri::async_runtime::handle();
    #[cfg(not(windows))]
    let _runtime_context = runtime.inner().enter();
    let context = tauri::generate_context!();
    #[cfg(windows)]
    let builder = {
        let executable = std::env::current_exe().expect("application executable path unavailable");
        let roaming = std::env::var_os("APPDATA")
            .map(std::path::PathBuf::from)
            .expect("APPDATA unavailable")
            .join("Capstan");
        let data_directory =
            capstan_app::data_directory::resolve_data_directory(&executable, &roaming)
                .expect("application data directory unavailable");
        let arguments = std::env::args_os()
            .skip(1)
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        match tauri::async_runtime::block_on(capstan_app::prepare_instance(
            &data_directory,
            &arguments,
        )) {
            Ok(Some(listener)) => {
                if data_directory == roaming {
                    let _ = capstan_native_windows::jump_list::set_process_identity(
                        &context.config().identifier,
                    );
                }
                tauri::Builder::default().manage(listener)
            }
            Ok(None) => return,
            Err(_) => {
                eprintln!("launch_instance_unavailable");
                std::process::exit(1);
            }
        }
    };
    #[cfg(not(windows))]
    let builder = tauri::Builder::default();
    capstan_app::tauri_channel::register_commands(builder)
        .run(context)
        .expect("failed to run Capstan Tauri application");
}
