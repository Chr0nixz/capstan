use super::*;
use base64::Engine;
use std::io::Cursor;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn signed_package(version: &str) -> (String, String, Vec<u8>) {
    let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let bytes = b"MZ disposable installer fixture".to_vec();
    let signature = minisign::sign(
        Some(&pair.pk),
        &pair.sk,
        Cursor::new(&bytes),
        Some(&format!("timestamp:1\tfile:fixture.exe\tversion:{version}")),
        None,
    )
    .unwrap();
    (
        BASE64.encode(pair.pk.to_box().unwrap().to_string()),
        BASE64.encode(signature.to_string()),
        bytes,
    )
}

fn manifest(version: &str, signature: &str, size: u64) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "version": version, "notes": "<script>alert(1)</script>\nRelease notes",
        "platforms": { TARGET: { "url": format!("https://github.com/Chr0nixz/capstan/releases/download/v{version}/Capstan_{version}_x64-setup.exe"), "signature": signature, "size": size } }
    })).unwrap()
}

#[test]
fn policy_rejects_downgrades_same_version_wrong_platform_and_untrusted_urls() {
    let (_, signature, bytes) = signed_package("1.2.3");
    let good = manifest("1.2.3", &signature, bytes.len() as u64);
    let release = parse_manifest(&good, "1.2.2").unwrap();
    assert!(release.notes.starts_with("<script>"));
    for current in ["1.2.3", "1.2.4", "9.0.0"] {
        assert_eq!(
            parse_manifest(&good, current).unwrap_err(),
            UpdateError::Version
        );
    }
    for url in [
        "http://github.com/Chr0nixz/capstan/releases/download/v1.2.3/a.exe",
        "https://github.com/evil/capstan/releases/download/v1.2.3/a.exe",
        "https://github.com.evil.test/Chr0nixz/capstan/releases/download/v1.2.3/a.exe",
        "https://user@github.com/Chr0nixz/capstan/releases/download/v1.2.3/a.exe",
        "https://github.com/Chr0nixz/capstan/releases/download/v1.2.2/Capstan_1.2.3_x64-setup.exe",
    ] {
        let mut value: serde_json::Value = serde_json::from_slice(&good).unwrap();
        value["platforms"][TARGET]["url"] = url.into();
        assert_eq!(
            parse_manifest(&serde_json::to_vec(&value).unwrap(), "1.2.2").unwrap_err(),
            UpdateError::Manifest
        );
    }
    let mut value: serde_json::Value = serde_json::from_slice(&good).unwrap();
    value["platforms"]["linux-x86_64"] = value["platforms"][TARGET].take();
    assert_eq!(
        parse_manifest(&serde_json::to_vec(&value).unwrap(), "1.2.2").unwrap_err(),
        UpdateError::Manifest
    );
}

#[test]
fn signature_binds_artifact_key_and_announced_version() {
    let (key, signature, bytes) = signed_package("1.2.3");
    let mut release =
        parse_manifest(&manifest("1.2.3", &signature, bytes.len() as u64), "1.2.2").unwrap();
    verify_package(&release, &key, &bytes).unwrap();
    let (other_key, _, _) = signed_package("1.2.3");
    assert_eq!(
        verify_package(&release, &other_key, &bytes),
        Err(UpdateError::Signature)
    );
    let mut damaged = bytes.clone();
    damaged[3] ^= 1;
    assert_eq!(
        verify_package(&release, &key, &damaged),
        Err(UpdateError::Signature)
    );
    release.version = "1.2.4".into();
    assert_eq!(
        verify_package(&release, &key, &bytes),
        Err(UpdateError::Version)
    );
    release.artifact.signature.clear();
    assert_eq!(
        verify_package(&release, &key, &bytes),
        Err(UpdateError::Signature)
    );
}

#[test]
fn malformed_limits_and_redirects_fail_closed() {
    let (_, signature, bytes) = signed_package("1.2.3");
    for size in [0, MAX_PACKAGE_BYTES + 1, u64::MAX] {
        assert_eq!(
            parse_manifest(&manifest("1.2.3", &signature, size), "1.2.2").unwrap_err(),
            UpdateError::Limit
        );
    }
    assert_eq!(
        parse_manifest(&vec![b' '; MAX_MANIFEST_BYTES + 1], "1.2.2").unwrap_err(),
        UpdateError::Limit
    );
    let mut value: serde_json::Value =
        serde_json::from_slice(&manifest("1.2.3", &signature, bytes.len() as u64)).unwrap();
    value["notes"] = "x".repeat(MAX_NOTES_BYTES + 1).into();
    assert_eq!(
        parse_manifest(&serde_json::to_vec(&value).unwrap(), "1.2.2").unwrap_err(),
        UpdateError::Limit
    );
    for url in [
        "http://release-assets.githubusercontent.com/a",
        "https://evil.test/a",
        "https://github.com/other/repo/releases/a",
        "https://release-assets.githubusercontent.com:444/a",
    ] {
        assert!(!allowed_redirect(&reqwest::Url::parse(url).unwrap()));
    }
    assert!(allowed_redirect(&reqwest::Url::parse("https://release-assets.githubusercontent.com/github-production-release-asset/1/file?sig=sample").unwrap()));
}

#[tokio::test]
async fn bounded_http_faults_cancel_without_touching_installation_or_data() {
    let root = tempfile::tempdir().unwrap();
    let install = root.path().join("capstan-app.exe");
    let data = root.path().join("vault.capstan");
    std::fs::write(&install, b"old executable").unwrap();
    std::fs::write(&data, b"untouched vault").unwrap();
    let (key, signature, bytes) = signed_package("1.2.3");
    let release =
        parse_manifest(&manifest("1.2.3", &signature, bytes.len() as u64), "1.2.2").unwrap();
    // HTTP is injected only at this private transport seam; production accepts a fixed HTTPS endpoint.
    for mode in ["valid", "tampered", "interrupted", "oversized", "cancel"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let body = bytes.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            let _ = socket.read(&mut request).await;
            let length = if mode == "oversized" {
                MAX_PACKAGE_BYTES + 1
            } else {
                body.len() as u64
            };
            let header =
                format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n");
            let _ = socket.write_all(header.as_bytes()).await;
            if mode == "cancel" {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            let mut body = body;
            if mode == "tampered" {
                body[3] ^= 1;
            }
            if mode == "interrupted" {
                body.truncate(3);
            }
            let _ = socket.write_all(&body).await;
        });
        let token = CancellationToken::new();
        if mode == "cancel" {
            let cancel = token.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                cancel.cancel();
            });
        }
        let response = reqwest::Client::new()
            .get(format!("http://{address}/package"))
            .send()
            .await
            .unwrap();
        let result = receive_package(
            response,
            &release,
            &key,
            &token,
            Some(root.path()),
            |_, _| {},
        )
        .await;
        if mode == "valid" {
            assert_eq!(result.unwrap(), bytes);
        } else {
            assert_eq!(
                result.unwrap_err(),
                match mode {
                    "tampered" => UpdateError::Signature,
                    "oversized" => UpdateError::Limit,
                    "cancel" => UpdateError::Canceled,
                    _ => UpdateError::Network,
                }
            );
        }
        server.abort();
        assert_eq!(std::fs::read(&install).unwrap(), b"old executable");
        assert_eq!(std::fs::read(&data).unwrap(), b"untouched vault");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
    }
}

#[test]
fn daily_schedule_and_contract_are_stable() {
    assert!(!startup_check_due(false, None, 200_000));
    assert!(startup_check_due(true, None, 200_000));
    assert!(!startup_check_due(true, Some(199_999), 200_000));
    assert!(startup_check_due(true, Some(100_000), 200_000));
    assert!(!startup_check_due(true, Some(300_000), 200_000));
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../docs/agent/fixtures/M4-contract.json"
    ))
    .unwrap();
    assert_eq!(fixture["updateLimits"]["packageBytes"], MAX_PACKAGE_BYTES);
    assert_eq!(fixture["updateLimits"]["manifestBytes"], MAX_MANIFEST_BYTES);
    assert_eq!(fixture["updateTarget"], TARGET);
    assert_eq!(fixture["updateEndpoint"], ENDPOINT);
}

#[test]
fn startup_claim_is_persistent_and_never_rewrites_settings_or_vault() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("settings.json"), b"{}\n").unwrap();
    std::fs::write(root.path().join("vault.capstan"), b"vault").unwrap();
    assert!(!claim_startup_check(root.path(), false, 100_000).unwrap());
    assert!(!root.path().join("update-check.json").exists());
    assert!(claim_startup_check(root.path(), true, 100_000).unwrap());
    assert!(!claim_startup_check(root.path(), true, 100_001).unwrap());
    assert!(!claim_startup_check(root.path(), true, 99_999).unwrap());
    assert!(claim_startup_check(root.path(), true, 186_400).unwrap());
    assert_eq!(
        std::fs::read(root.path().join("settings.json")).unwrap(),
        b"{}\n"
    );
    assert_eq!(
        std::fs::read(root.path().join("vault.capstan")).unwrap(),
        b"vault"
    );
    std::fs::write(root.path().join("update-check.json"), b"bad").unwrap();
    assert_eq!(
        claim_startup_check(root.path(), true, 300_000),
        Err(UpdateError::Storage)
    );
}

#[cfg(windows)]
#[tokio::test]
async fn update_setting_preserves_other_keys_and_uses_existing_write_admission() {
    use crate::settings::{Settings, SettingsError};
    use std::sync::Arc;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("settings.json");
    let settings = Arc::new(Settings::new(root.path().to_owned()));
    assert!(!settings.check_updates_on_startup().unwrap());
    assert!(!path.exists());
    std::fs::write(
        &path,
        br#"{"schemaVersion":1,"font":{"size":14},"copyOnSelect":true}"#,
    )
    .unwrap();
    settings.clone().save_update_preference(true).await.unwrap();
    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(document["font"]["size"], 14);
    assert_eq!(document["copyOnSelect"], true);
    assert_eq!(document["schemaVersion"], 1);
    assert!(Settings::new(root.path().to_owned())
        .check_updates_on_startup()
        .unwrap());
    settings.drain().await.unwrap();
    let before = std::fs::read(&path).unwrap();
    assert_eq!(
        settings.clone().save_update_preference(false).await,
        Err(SettingsError::Unavailable)
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[tokio::test]
async fn streaming_limits_apply_without_content_length_and_cleanup_partial_files() {
    let (key, signature, bytes) = signed_package("1.2.3");
    let release =
        parse_manifest(&manifest("1.2.3", &signature, bytes.len() as u64), "1.2.2").unwrap();
    let root = tempfile::tempdir().unwrap();
    for is_manifest in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            let _ = socket.read(&mut request).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            let _ = socket.write_all(&vec![b'x'; MAX_MANIFEST_BYTES + 1]).await;
        });
        let response = reqwest::Client::new()
            .get(format!("http://{address}/fixture"))
            .send()
            .await
            .unwrap();
        let cancel = CancellationToken::new();
        let result = if is_manifest {
            receive_manifest(response, &cancel).await
        } else {
            receive_package(
                response,
                &release,
                &key,
                &cancel,
                Some(root.path()),
                |_, _| {},
            )
            .await
        };
        assert_eq!(result.unwrap_err(), UpdateError::Limit);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        server.await.unwrap();
    }
}
