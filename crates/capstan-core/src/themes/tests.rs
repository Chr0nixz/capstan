use super::*;
use serde_json::{json, Value};

fn windows_theme(name: &str) -> Value {
    let mut value = json!({"name": name, "foreground": "#F0F0F0", "background": "#ABC"});
    for key in [
        "black",
        "red",
        "green",
        "yellow",
        "blue",
        "purple",
        "cyan",
        "white",
        "brightBlack",
        "brightRed",
        "brightGreen",
        "brightYellow",
        "brightBlue",
        "brightPurple",
        "brightCyan",
        "brightWhite",
    ] {
        value[key] = json!("#12345678");
    }
    value
}

fn iterm_theme() -> String {
    let mut xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>"#.to_owned();
    for key in (0..16)
        .map(|index| format!("Ansi {index} Color"))
        .chain(["Foreground Color".to_owned(), "Background Color".to_owned()])
    {
        use std::fmt::Write as _;
        write!(xml, "<key>{key}</key><dict><key>Red Component</key><real>0.5</real><key>Green Component</key><integer>1</integer><key>Blue Component</key><real>0</real><key>Alpha Component</key><real>0.1</real></dict>").unwrap();
    }
    xml.push_str("</dict></plist>");
    xml
}

#[test]
fn theme_definition_matches_shared_fixture() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../docs/agent/fixtures/M4-contract.json"
    ))
    .unwrap();
    let theme: TerminalThemeDefinition =
        serde_json::from_value(fixture["terminalTheme"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(&theme).unwrap(),
        fixture["terminalTheme"]
    );
    assert_eq!(
        serde_json::to_value([ThemeSource::WindowsTerminal, ThemeSource::Iterm2]).unwrap(),
        fixture["terminalThemeSources"]
    );
    assert!(
        decode_catalog(&serde_json::to_vec(&[fixture["terminalTheme"].clone()]).unwrap()).is_ok()
    );
    let preview = ThemePreview {
        token: serde_json::from_value(fixture["themePreviewToken"].clone()).unwrap(),
        themes: vec![theme],
    };
    assert_eq!(
        serde_json::to_value(preview).unwrap(),
        json!({ "token": fixture["themePreviewToken"], "themes": [fixture["terminalTheme"]] })
    );
    assert_eq!(
        json!([
            ThemeError::Limit,
            ThemeError::Invalid,
            ThemeError::Json,
            ThemeError::Xml,
            ThemeError::Unavailable,
            ThemeError::Changed,
        ]
        .map(|error| error.to_string())),
        fixture["errorCodes"]["terminalTheme"]
    );
}

#[test]
fn parses_windows_terminal_scheme_and_settings_fragment() {
    let raw = windows_theme("  Sample\t  ");
    let single =
        parse_theme_file(ThemeSource::WindowsTerminal, raw.to_string().as_bytes(), "").unwrap();
    assert_eq!(single[0].name, "Sample");
    assert_eq!(single[0].background, "#aabbcc");
    assert_eq!(single[0].cursor, "#f0f0f0");
    assert_eq!(single[0].selection_background, "#404040");
    assert_eq!(
        single[0].ansi,
        std::array::from_fn::<_, 16, _>(|_| "#123456".to_owned())
    );
    let fragment = json!({"profiles": [], "schemes": [raw, windows_theme("Next")]});
    assert_eq!(
        parse_theme_file(
            ThemeSource::WindowsTerminal,
            fragment.to_string().as_bytes(),
            ""
        )
        .unwrap()
        .len(),
        2
    );
}

#[test]
fn rejects_incomplete_unsafe_or_oversized_json() {
    let parse = |raw: &[u8]| parse_theme_file(ThemeSource::WindowsTerminal, raw, "");
    let mut value = windows_theme("Sample");
    value.as_object_mut().unwrap().remove("brightBlue");
    assert_eq!(
        parse(value.to_string().as_bytes()),
        Err(ThemeError::Invalid)
    );
    value["brightBlue"] = json!("url(https://example.test/)");
    assert_eq!(
        parse(value.to_string().as_bytes()),
        Err(ThemeError::Invalid)
    );
    assert_eq!(
        parse(b"{// settings.json\n\"schemes\":[]}"),
        Err(ThemeError::Json)
    );
    assert_eq!(
        parse(format!("{}0{}", "[".repeat(256), "]".repeat(256)).as_bytes()),
        Err(ThemeError::Json)
    );
    assert_eq!(
        parse(&vec![0; MAX_IMPORT_BYTES + 1]),
        Err(ThemeError::Limit)
    );
    assert_eq!(
        parse(
            json!({"schemes": vec![windows_theme("Theme"); MAX_THEMES + 1]})
                .to_string()
                .as_bytes()
        ),
        Err(ThemeError::Limit)
    );
    assert_eq!(
        parse(windows_theme(&"中".repeat(65)).to_string().as_bytes()),
        Err(ThemeError::Limit)
    );
}

#[test]
fn parses_iterm_xml_and_rejects_missing_components_entities_and_depth() {
    let xml = iterm_theme();
    let parse = |text: &str| parse_theme_file(ThemeSource::Iterm2, text.as_bytes(), "Color sample");
    let theme = parse(&xml).unwrap().remove(0);
    assert_eq!(theme.name, "Color sample");
    assert_eq!(theme.foreground, "#80ff00");
    assert_eq!(theme.ansi[15], "#80ff00");
    assert_eq!(
        parse(&xml.replace("Ansi 15 Color", "Other Color")),
        Err(ThemeError::Invalid)
    );
    assert_eq!(
        parse(&xml.replace("<real>0.5</real>", "<real>1.1</real>")),
        Err(ThemeError::Invalid)
    );
    assert_eq!(
        parse(&xml.replace("Red Component", "Missing")),
        Err(ThemeError::Invalid)
    );
    assert_eq!(
        parse(&xml.replace("Ansi 0 Color", "&secret;")),
        Err(ThemeError::Xml)
    );
    let evil = xml.replace(
        "PropertyList-1.0.dtd\">",
        "PropertyList-1.0.dtd\" [<!ENTITY secret SYSTEM \"file:///secret\">]>",
    );
    assert_eq!(parse(&evil), Err(ThemeError::Xml));
    assert_eq!(
        parse(&format!(
            "<plist>{}{}</plist>",
            "<array>".repeat(25),
            "</array>".repeat(25)
        )),
        Err(ThemeError::Limit)
    );
    assert_eq!(
        parse(&" ".repeat(MAX_IMPORT_BYTES + 1)),
        Err(ThemeError::Limit)
    );
}

#[cfg(windows)]
#[test]
fn preview_is_read_only_until_confirmed_and_catalog_updates_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let selected = directory.path().join("selected.json");
    let path = directory.path().join("terminal-themes.json");
    std::fs::write(&selected, windows_theme("Graphite").to_string()).unwrap();
    let catalog = ThemeCatalog::new(directory.path());
    let preview = catalog.preview(&selected).unwrap();
    assert_eq!(preview.themes[0].name, "Graphite (2)");
    assert!(!path.exists());
    assert!(!serde_json::to_string(&preview)
        .unwrap()
        .contains("selected.json"));
    assert_eq!(
        catalog.save_preview(Uuid::new_v4()),
        Err(ThemeError::Changed)
    );
    let id = &preview.themes[0].id;
    assert_eq!(catalog.save_preview(preview.token).unwrap(), preview.themes);
    assert_eq!(
        catalog.save_preview(preview.token),
        Err(ThemeError::Changed)
    );
    let reopened = ThemeCatalog::new(directory.path());
    assert_eq!(reopened.list().unwrap(), preview.themes);
    assert_eq!(reopened.rename(id, "Paper").unwrap()[0].name, "Paper (2)");
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(&selected, "bad").unwrap();
    assert!(catalog.preview(&selected).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    std::fs::write(&selected, vec![b' '; MAX_IMPORT_BYTES + 1]).unwrap();
    assert_eq!(catalog.preview(&selected).unwrap_err(), ThemeError::Limit);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(reopened.delete("graphite"), Err(ThemeError::Invalid));
    assert!(reopened.delete(id).unwrap().is_empty());
}
