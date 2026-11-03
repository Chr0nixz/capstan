fn benchmark_preflight() -> bool {
    if std::env::args().any(|argument| argument == "--m0-check") {
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
    capstan_app::tauri_channel::register_commands(tauri::Builder::default())
        .run(tauri::generate_context!())
        .expect("failed to run Capstan Tauri application");
}
