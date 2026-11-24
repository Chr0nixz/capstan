fn main() {
    if std::env::var_os("CARGO_FEATURE_TAURI_RUNTIME").is_some() {
        // On Windows this embeds icons/icon.ico as the executable and window icon.
        tauri_build::build();
    }
}
