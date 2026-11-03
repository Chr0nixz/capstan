use capstan_terminal::{
    encode_binary_frame, BinaryFrame, OutputMetrics, TerminalSessionId, TerminalSnapshot,
    TerminalState,
};

#[test]
fn rust_encoding_and_status_match_the_shared_frontend_fixture() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/agent/fixtures/terminal-contract.json"
    ))
    .expect("fixture");
    let payload: Vec<u8> = serde_json::from_value(fixture["payload"].clone()).expect("payload");
    let encoded = encode_binary_frame(BinaryFrame {
        sequence: 1,
        payload,
    })
    .expect("encode");
    assert_eq!(
        serde_json::to_value(encoded).expect("encoded bytes"),
        fixture["encoded"]
    );
    let snapshot = TerminalSnapshot {
        id: TerminalSessionId("session-0".to_owned()),
        state: TerminalState::Running,
        exit_code: None,
        error_code: None,
        metrics: OutputMetrics::default(),
    };
    assert_eq!(
        serde_json::to_value(snapshot).expect("snapshot"),
        fixture["snapshot"]
    );
}
