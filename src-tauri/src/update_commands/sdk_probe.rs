//! Isolated MockRuntime regression, compiled by Test-M4Update.ps1 without Wry.
//! It includes production SDK configuration, rather than duplicating its callback.
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    borrow::Cow,
    io::Cursor,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tauri::Manager;

#[path = "@SDK_MODULE@"]
mod sdk;

struct ObservableResource(Arc<AtomicUsize>);
impl tauri::Resource for ObservableResource {
    fn name(&self) -> Cow<'_, str> {
        "update-launch-regression".into()
    }
}
impl Drop for ObservableResource {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn main() {
    let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    // Signed, disposable, invalid PE bytes force the real Windows launcher to
    // reject installation after verification. They can never run an installer.
    let bytes = b"MZ invalid disposable executable for launch rejection regression";
    let signature = minisign::sign(
        Some(&pair.pk),
        &pair.sk,
        Cursor::new(bytes),
        Some("timestamp:1\tfile:fixture.exe\tversion:9.9.9"),
        None,
    )
    .unwrap();
    let key = STANDARD.encode(pair.pk.to_box().unwrap().to_string());
    let manifest = serde_json::json!({"version": "9.9.9", "platforms": {"windows-x86_64": {
        "url": "https://github.com/Chr0nixz/capstan/releases/download/v9.9.9/Capstan_9.9.9_x64-setup.exe",
        "signature": STANDARD.encode(signature.to_string())
    }}});
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().plugins.0.insert("updater".into(), serde_json::json!({
        "pubkey": key, "requireSignedVersion": true,
        "endpoints": ["https://github.com/Chr0nixz/capstan/releases/latest/download/latest.json"],
        "windows": {"installMode": "passive"}
    }));
    let app = tauri::test::mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(context)
        .unwrap();
    let window =
        tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::App("index.html".into()))
            .build()
            .unwrap();
    window.show().unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let app_resource = app.resources_table().add(ObservableResource(drops.clone()));
    let window_resource = window
        .resources_table()
        .add(ObservableResource(drops.clone()));
    let webview_resource = window
        .as_ref()
        .resources_table()
        .add(ObservableResource(drops.clone()));
    for _ in 0..2 {
        let update = sdk::builder(app.handle(), &key, "windows-x86_64")
            .build()
            .unwrap()
            .restore_update(manifest.clone())
            .unwrap()
            .unwrap();
        update.verify(bytes).unwrap();
        assert!(matches!(
            update.install(bytes),
            Err(tauri_plugin_updater::Error::Io(_))
        ));
        assert!(app.get_webview_window("main").is_some());
        assert!(window.is_visible().unwrap());
        assert!(app
            .resources_table()
            .get::<ObservableResource>(app_resource)
            .is_ok());
        assert!(window
            .resources_table()
            .get::<ObservableResource>(window_resource)
            .is_ok());
        assert!(window
            .as_ref()
            .resources_table()
            .get::<ObservableResource>(webview_resource)
            .is_ok());
        assert_eq!(drops.load(Ordering::SeqCst), 0);
    }
    println!("SDK launch rejection: 2 attempts, app/window/webview resources preserved, resource drops=0.");
}
