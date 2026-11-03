use capstan_vault::{Snippet, SnippetGroup, TunnelConfig, VaultData};
use uuid::Uuid;

#[test]
fn vault_records_match_the_shared_m3_fixture_and_legacy_payloads_remain_readable() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/agent/fixtures/M3-contract.json"
    ))
    .expect("fixture");
    let tunnels: Vec<TunnelConfig> =
        serde_json::from_value(fixture["tunnels"].clone()).expect("tunnel configs");
    let groups: Vec<SnippetGroup> =
        serde_json::from_value(fixture["snippetGroups"].clone()).expect("snippet groups");
    let snippets: Vec<Snippet> =
        serde_json::from_value(fixture["snippets"].clone()).expect("snippets");
    assert_eq!(tunnels.len(), 3);
    assert_eq!(groups.len(), 1);
    assert_eq!(
        snippets[0].template,
        "kubectl get pods -n {{namespace:default}}"
    );
    assert_eq!(
        serde_json::to_value(&tunnels).expect("tunnels serialize"),
        fixture["tunnels"]
    );
    let host_id = tunnels[0].host_id;
    assert!(tunnels
        .iter()
        .all(|tunnel| tunnel.validate(&[host_id]).is_ok()));
    let group_ids = groups.iter().map(|group| group.id).collect::<Vec<_>>();
    assert!(groups.iter().all(|group| group.validate().is_ok()));
    assert!(snippets
        .iter()
        .all(|snippet| snippet.validate(&group_ids).is_ok()));

    let old: VaultData =
        serde_json::from_value(fixture["legacyVaultPayload"].clone()).expect("old vault data");
    assert!(old.validate().is_ok());
    assert_eq!(old.schema, 1);
    assert!(old.snippet_groups.is_empty());
    assert!(old.snippets.is_empty());
}

#[test]
fn tunnel_endpoints_match_shared_valid_and_invalid_addresses() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/agent/fixtures/M3-contract.json"
    ))
    .unwrap();
    for tunnel in fixture["tunnels"].as_array().unwrap() {
        let host_id: Uuid = serde_json::from_value(tunnel["host_id"].clone()).unwrap();
        let keys: &[&str] = match tunnel["forward"]["kind"].as_str().unwrap() {
            "local" => &["bind_host", "destination_host"],
            "remote" => &["remote_bind_host", "local_destination_host"],
            "socks5" => &["bind_host"],
            _ => panic!("unexpected forward kind"),
        };
        for case in fixture["tunnelHostCases"].as_array().unwrap() {
            for key in keys {
                let mut value = tunnel.clone();
                value["forward"][key] = case["host"].clone();
                let config: TunnelConfig = serde_json::from_value(value).unwrap();
                assert_eq!(
                    config.validate(&[host_id]).is_ok(),
                    case["accepted"].as_bool().unwrap(),
                    "{key}: {}",
                    case["host"]
                );
            }
        }
        for key in ["id", "host_id"] {
            let mut value = tunnel.clone();
            value[key] = serde_json::to_value(Uuid::nil()).unwrap();
            let config: TunnelConfig = serde_json::from_value(value).unwrap();
            assert!(config.validate(&[host_id, Uuid::nil()]).is_err());
        }
    }
}

#[test]
fn snippet_utf8_byte_limits_match_shared_unicode_boundaries() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/agent/fixtures/M3-contract.json"
    ))
    .unwrap();
    let group: SnippetGroup = serde_json::from_value(fixture["snippetGroups"][0].clone()).unwrap();
    for case in fixture["snippetTextCases"].as_array().unwrap() {
        let content = case["text"]
            .as_str()
            .unwrap()
            .repeat(usize::try_from(case["repeat"].as_u64().unwrap()).unwrap())
            + case["suffix"].as_str().unwrap();
        let field = case["field"].as_str().unwrap();
        let accepted = if field == "group_name" {
            let mut value = fixture["snippetGroups"][0].clone();
            value["name"] = content.into();
            serde_json::from_value::<SnippetGroup>(value)
                .unwrap()
                .validate()
                .is_ok()
        } else {
            let mut value = fixture["snippets"][0].clone();
            value[field] = content.into();
            serde_json::from_value::<Snippet>(value)
                .unwrap()
                .validate(&[group.id])
                .is_ok()
        };
        assert_eq!(
            accepted,
            case["accepted"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
    }
    for key in ["id", "group_id"] {
        let mut value = fixture["snippets"][0].clone();
        value[key] = serde_json::to_value(Uuid::nil()).unwrap();
        let snippet: Snippet = serde_json::from_value(value).unwrap();
        assert!(snippet.validate(&[group.id, Uuid::nil()]).is_err());
    }
    let mut nil_group = group;
    nil_group.id = Uuid::nil();
    assert!(nil_group.validate().is_err());
    for text in [r#""\ud800""#, r#""\udfff""#, r#""ok\ud800text""#] {
        assert!(serde_json::from_str::<String>(text).is_err());
    }
}
