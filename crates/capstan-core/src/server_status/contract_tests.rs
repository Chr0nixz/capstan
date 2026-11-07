use super::contracts::*;
use super::frame::{MAX_CPU_LINES, MAX_FRAME_BYTES, MAX_INTERFACES, MAX_MOUNTS};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ThemeSource {
    WindowsTerminal,
    Iterm2,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TerminalThemeDefinition {
    id: String,
    name: String,
    source: ThemeSource,
    foreground: String,
    background: String,
    cursor: String,
    selection_background: String,
    ansi: [String; 16],
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LaunchKind {
    Focus,
    Local,
    Host,
    Ssh,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LaunchRequestView {
    request_id: Uuid,
    kind: LaunchKind,
    display: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsDefaults {
    copy_on_select: bool,
    system_notifications: bool,
    jump_list_recent_hosts: bool,
    quake_enabled: bool,
    quake_hotkey: Option<String>,
    quake_height_percent: u8,
    check_updates_on_startup: bool,
}

fn round_trip<T: Serialize + serde::de::DeserializeOwned>(value: &Value) {
    let dto: T = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(&serde_json::to_value(dto).unwrap(), value);
}

#[test]
fn shared_m4_wire_contract() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../docs/agent/fixtures/M4-contract.json"
    ))
    .unwrap();
    round_trip::<ServerStatusSnapshot>(&fixture["serverStatus"]);
    use ServerStatusState::{Paused, Probing, Running, Unavailable, Unsupported};
    assert_eq!(
        fixture["serverStatusStates"],
        json!([Probing, Running, Paused, Unsupported, Unavailable])
    );
    assert_eq!(
        fixture["serverStatusLimits"],
        json!({"frameBytes": MAX_FRAME_BYTES, "interfaces": MAX_INTERFACES, "mounts": MAX_MOUNTS, "cpuLines": MAX_CPU_LINES})
    );
    assert_eq!(fixture["serverStatusEvent"], SERVER_STATUS_EVENT);
    assert_eq!(
        fixture["errorCodes"]["serverStatus"],
        json!([
            StatusError::Unsupported,
            StatusError::Rejected,
            StatusError::Parse,
            StatusError::Limit,
            StatusError::Closed
        ])
    );
    assert_eq!(
        fixture["errorCodes"]["hello"],
        json!([
            "hello_unavailable",
            "hello_canceled",
            "hello_key_missing",
            "hello_mismatch",
            "hello_data"
        ])
    );
    round_trip::<TerminalThemeDefinition>(&fixture["terminalTheme"]);
    assert_eq!(
        fixture["terminalThemeSources"],
        json!([ThemeSource::WindowsTerminal, ThemeSource::Iterm2])
    );
    round_trip::<LaunchRequestView>(&fixture["launchRequest"]);
    assert_eq!(
        fixture["launchRequestKinds"],
        json!([
            LaunchKind::Focus,
            LaunchKind::Local,
            LaunchKind::Host,
            LaunchKind::Ssh
        ])
    );
    assert_eq!(
        fixture["settingsDefaults"],
        json!(SettingsDefaults {
            copy_on_select: false,
            system_notifications: true,
            jump_list_recent_hosts: false,
            quake_enabled: false,
            quake_hotkey: None,
            quake_height_percent: 50,
            check_updates_on_startup: false,
        })
    );
}
