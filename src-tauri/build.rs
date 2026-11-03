fn main() {
    if std::env::var_os("CARGO_FEATURE_TAURI_RUNTIME").is_some() {
        let target_is_windows =
            std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|target| target == "windows");
        if target_is_windows {
            let icon_path = std::path::PathBuf::from(
                std::env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"),
            )
            .join("capstan-m0.ico");
            std::fs::write(&icon_path, minimal_windows_icon())
                .expect("write generated M0 Windows icon");
            let attributes = tauri_build::Attributes::new().windows_attributes(
                tauri_build::WindowsAttributes::new().window_icon_path(icon_path),
            );
            tauri_build::try_build(attributes).expect("failed to build Tauri application context");
        } else {
            tauri_build::build();
        }
    }
}

fn minimal_windows_icon() -> [u8; 70] {
    let mut icon = [0_u8; 70];
    icon[2] = 1;
    icon[4] = 1;
    icon[6] = 1;
    icon[8] = 1;
    icon[10] = 1;
    icon[12] = 32;
    icon[14] = 48;
    icon[18] = 22;
    icon[22] = 40;
    icon[26] = 1;
    icon[30] = 2;
    icon[34] = 1;
    icon[36] = 32;
    icon[42] = 4;
    icon[62] = 0x2d;
    icon[63] = 0xa4;
    icon[64] = 0xd9;
    icon[65] = 0xff;
    icon
}
