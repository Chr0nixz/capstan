use capstan_ssh::{
    connection_state::{
        validate_snapshot, validate_successor, ConnectionErrorCode, ConnectionSnapshot,
        CONNECTION_STATE_CHANGED_EVENT,
    },
    forwarding::{TunnelRuntimeSnapshot, TUNNEL_STATE_CHANGED_EVENT},
};

#[test]
fn connection_snapshots_and_generation_transitions_match_the_shared_fixture() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/agent/fixtures/M3-contract.json"
    ))
    .expect("fixture");
    let snapshot: ConnectionSnapshot =
        serde_json::from_value(fixture["connectionSnapshot"].clone()).expect("connection snapshot");
    assert_eq!(snapshot.generation, 9_007_199_254_740_993);
    assert_eq!(
        CONNECTION_STATE_CHANGED_EVENT,
        fixture["eventNames"]["connectionStateChanged"]
    );
    assert_eq!(
        TUNNEL_STATE_CHANGED_EVENT,
        fixture["eventNames"]["tunnelStateChanged"]
    );
    assert_eq!(
        serde_json::to_value(snapshot).expect("snapshot serialize"),
        fixture["connectionSnapshot"]
    );
    assert_eq!(
        serde_json::to_value(ConnectionErrorCode::HostKeyChanged).expect("error code"),
        "host_key_changed"
    );

    for transition in fixture["connectionTransitions"]
        .as_array()
        .expect("transitions")
    {
        let current: ConnectionSnapshot =
            serde_json::from_value(transition["current"].clone()).expect("current snapshot");
        let next: ConnectionSnapshot =
            serde_json::from_value(transition["next"].clone()).expect("next snapshot");
        assert_eq!(
            validate_successor(&current, &next).is_ok(),
            transition["accepted"].as_bool().expect("expected result")
        );
    }
}

#[test]
fn snapshot_validation_matches_shared_boundary_and_malformed_cases() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/agent/fixtures/M3-contract.json"
    ))
    .expect("fixture");
    for case in fixture["connectionSnapshotCases"].as_array().unwrap() {
        let mut value = fixture["connectionSnapshot"].clone();
        value
            .as_object_mut()
            .unwrap()
            .extend(case["patch"].as_object().unwrap().clone());
        let accepted = serde_json::from_value::<ConnectionSnapshot>(value)
            .is_ok_and(|snapshot| validate_snapshot(&snapshot).is_ok());
        assert_eq!(
            accepted,
            case["accepted"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
    }
    for case in fixture["tunnelSnapshotCases"].as_array().unwrap() {
        let mut value = fixture["tunnelRuntimeSnapshot"].clone();
        value
            .as_object_mut()
            .unwrap()
            .extend(case["patch"].as_object().unwrap().clone());
        assert_eq!(
            serde_json::from_value::<TunnelRuntimeSnapshot>(value).is_ok(),
            case["accepted"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
    }
}
