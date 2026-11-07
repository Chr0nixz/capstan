use crate::{Result, VaultError};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::HashSet, fmt};
use uuid::Uuid;

pub struct SecretText(SecretString);
impl SecretText {
    pub fn new(value: String) -> Self {
        Self(SecretString::from(value))
    }
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }
}
impl Clone for SecretText {
    fn clone(&self) -> Self {
        Self::new(self.expose().to_owned())
    }
}
impl fmt::Debug for SecretText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}
// This serializer is exclusively used inside encrypted Rust-side storage.
impl Serialize for SecretText {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.expose())
    }
}
impl<'de> Deserialize<'de> for SecretText {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    OpenSsh,
    Pageant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CredentialMaterial {
    Password {
        value: SecretText,
    },
    ImportedKey {
        content: SecretText,
        passphrase: Option<SecretText>,
    },
    KeyFile {
        path: String,
        passphrase: Option<SecretText>,
    },
    Agent {
        agent: AgentKind,
        fingerprint: String,
    },
}
impl CredentialMaterial {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Password { .. } => "password",
            Self::ImportedKey { .. } => "imported_key",
            Self::KeyFile { .. } => "key_file",
            Self::Agent { .. } => "agent",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credential {
    pub id: Uuid,
    pub name: String,
    pub revision: u64,
    pub material: CredentialMaterial,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialView {
    pub id: Uuid,
    pub name: String,
    pub revision: u64,
    pub kind: String,
}
impl Credential {
    pub fn view(&self) -> CredentialView {
        CredentialView {
            id: self.id,
            name: self.name.clone(),
            revision: self.revision,
            kind: self.material.kind().to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub id: Uuid,
    pub name: String,
    pub parent_id: Option<Uuid>,
    /// Marks every host in this group and its subgroups as production.
    #[serde(default)]
    pub production: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Host {
    pub id: Uuid,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub credential_id: Uuid,
    pub group_id: Option<Uuid>,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub revision: u64,
    pub working_directory: Option<String>,
    pub startup_command: Option<String>,
    pub encoding: String,
    pub theme: String,
    pub agent_forwarding: bool,
    /// Production hosts carry red identity marks and stricter confirmations.
    #[serde(default)]
    pub production: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustRecord {
    pub target: String,
    pub port: u16,
    pub algorithm: String,
    pub public_key: String,
    pub fingerprint: String,
    pub first_trusted_at: u64,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub auto_lock_minutes: Option<u16>,
    pub disconnect_on_lock: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            auto_lock_minutes: Some(30),
            disconnect_on_lock: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunnelConfig {
    pub id: Uuid,
    pub host_id: Uuid,
    pub auto_start: bool,
    pub forward: TunnelForward,
}
impl TunnelConfig {
    pub fn validate(&self, host_ids: &[Uuid]) -> Result<()> {
        let valid = match &self.forward {
            TunnelForward::Local {
                bind_host,
                bind_port,
                destination_host,
                destination_port,
            } => {
                *bind_port != 0
                    && *destination_port != 0
                    && normalize_target(bind_host).is_ok()
                    && normalize_target(destination_host).is_ok()
            }
            TunnelForward::Remote {
                remote_bind_host,
                remote_bind_port,
                local_destination_host,
                local_destination_port,
            } => {
                *remote_bind_port != 0
                    && *local_destination_port != 0
                    && normalize_target(remote_bind_host).is_ok()
                    && normalize_target(local_destination_host).is_ok()
            }
            TunnelForward::Socks5 {
                bind_host,
                bind_port,
            } => *bind_port != 0 && normalize_target(bind_host).is_ok(),
        };
        if self.id.is_nil() || self.host_id.is_nil() || !host_ids.contains(&self.host_id) || !valid
        {
            return Err(VaultError::InvalidData);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TunnelForward {
    Local {
        bind_host: String,
        bind_port: u16,
        destination_host: String,
        destination_port: u16,
    },
    Remote {
        remote_bind_host: String,
        remote_bind_port: u16,
        local_destination_host: String,
        local_destination_port: u16,
    },
    Socks5 {
        bind_host: String,
        bind_port: u16,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnippetGroup {
    pub id: Uuid,
    /// At most 128 UTF-8 bytes, excluding blank and control-only names.
    pub name: String,
}
impl SnippetGroup {
    pub fn validate(&self) -> Result<()> {
        if self.id.is_nil()
            || !bounded(&self.name, 128, true)
            || self.name.chars().any(char::is_control)
        {
            return Err(VaultError::InvalidData);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snippet {
    pub id: Uuid,
    pub group_id: Option<Uuid>,
    /// At most 256 UTF-8 bytes.
    pub name: String,
    /// At most 8192 UTF-8 bytes, including variable placeholders and defaults.
    pub template: String,
    pub auto_enter: bool,
}
impl Snippet {
    pub fn validate(&self, group_ids: &[Uuid]) -> Result<()> {
        if self.id.is_nil()
            || !bounded(&self.name, 256, true)
            || !bounded(&self.template, 8192, true)
            || self.name.chars().any(char::is_control)
            || self
                .group_id
                .is_some_and(|id| id.is_nil() || !group_ids.contains(&id))
        {
            return Err(VaultError::InvalidData);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultData {
    pub schema: u16,
    pub revision: u64,
    pub hosts: Vec<Host>,
    pub groups: Vec<Group>,
    pub credentials: Vec<Credential>,
    pub trust: Vec<TrustRecord>,
    pub preferences: Preferences,
    // Empty lists are omitted so a Vault without M3 data stays readable by
    // earlier builds, which reject unknown fields.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub snippet_groups: Vec<SnippetGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub snippets: Vec<Snippet>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tunnels: Vec<TunnelConfig>,
}
impl Default for VaultData {
    fn default() -> Self {
        Self {
            schema: 1,
            revision: 0,
            hosts: Vec::new(),
            groups: Vec::new(),
            credentials: Vec::new(),
            trust: Vec::new(),
            preferences: Preferences::default(),
            snippet_groups: Vec::new(),
            snippets: Vec::new(),
            tunnels: Vec::new(),
        }
    }
}

pub fn normalize_target(address: &str) -> Result<String> {
    let value = address.trim();
    if value.is_empty() || value.len() > 253 || value.chars().any(char::is_whitespace) {
        return Err(VaultError::InvalidData);
    }
    let ip_value = value
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(value);
    if let Some((address, scope)) = ip_value.split_once('%') {
        let address = address
            .parse::<std::net::Ipv6Addr>()
            .map_err(|_| VaultError::InvalidData)?;
        if scope.is_empty()
            || scope.len() > 64
            || !scope
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        {
            return Err(VaultError::InvalidData);
        }
        let scope = if scope.bytes().all(|byte| byte.is_ascii_digit()) {
            scope
                .parse::<u32>()
                .map_err(|_| VaultError::InvalidData)?
                .to_string()
        } else {
            scope.to_owned()
        };
        return Ok(format!("{address}%{scope}"));
    }
    if let Ok(ip) = ip_value.parse::<std::net::IpAddr>() {
        return Ok(ip.to_string());
    }
    let host = value.trim_end_matches('.').to_ascii_lowercase();
    if !host.is_ascii()
        || host.split('.').any(|part| {
            part.is_empty()
                || part.len() > 63
                || part.starts_with('-')
                || part.ends_with('-')
                || part.starts_with('_')
                || part.ends_with('_')
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
    {
        return Err(VaultError::InvalidData);
    }
    Ok(host)
}

fn bounded(value: &str, maximum: usize, required: bool) -> bool {
    (!required || !value.trim().is_empty()) && value.len() <= maximum && !value.contains('\0')
}

/// A missing custom definition is valid in a host record (for example after a backup import).
#[must_use]
pub fn valid_custom_theme_id(id: &str) -> bool {
    id.strip_prefix("custom:").is_some_and(|value| {
        value.len() == 36
            && Uuid::parse_str(value)
                .is_ok_and(|id| !id.is_nil() && id.hyphenated().to_string() == value)
    })
}

impl Host {
    pub fn validate(&self) -> Result<()> {
        normalize_target(&self.address)?;
        if self.id.is_nil()
            || self.port == 0
            || !bounded(&self.name, 256, true)
            || !bounded(&self.username, 256, true)
            || self.username.chars().any(char::is_control)
            || self.tags.len() > 32
            || self.tags.iter().any(|tag| !bounded(tag, 64, true))
            || self.encoding != "utf-8"
            || !(matches!(self.theme.as_str(), "graphite" | "light")
                || valid_custom_theme_id(&self.theme))
            || self.working_directory.as_deref().is_some_and(|value| {
                !bounded(value, 4096, true) || value.chars().any(char::is_control)
            })
            || self
                .startup_command
                .as_deref()
                .is_some_and(|value| !bounded(value, 8192, true))
        {
            return Err(VaultError::InvalidData);
        }
        Ok(())
    }
}
impl VaultData {
    pub fn validate(&self) -> Result<()> {
        if self.schema != 1
            || self.hosts.len() > 4096
            || self.groups.len() > 1024
            || self.credentials.len() > 4096
            || self.trust.len() > 4096
            || self.snippet_groups.len() > 1024
            || self.snippets.len() > 4096
            || self.tunnels.len() > 4096
            || self
                .preferences
                .auto_lock_minutes
                .is_some_and(|minutes| minutes == 0 || minutes > 1440)
        {
            return Err(VaultError::InvalidData);
        }
        let credentials: HashSet<_> = self
            .credentials
            .iter()
            .map(|credential| credential.id)
            .collect();
        let groups: HashSet<_> = self.groups.iter().map(|group| group.id).collect();
        let hosts: HashSet<_> = self.hosts.iter().map(|host| host.id).collect();
        let snippet_groups: HashSet<_> = self.snippet_groups.iter().map(|group| group.id).collect();
        let snippet_ids: HashSet<_> = self.snippets.iter().map(|snippet| snippet.id).collect();
        let snippet_group_ids = snippet_groups.iter().copied().collect::<Vec<_>>();
        let tunnel_ids: HashSet<_> = self.tunnels.iter().map(|tunnel| tunnel.id).collect();
        if tunnel_ids.len() != self.tunnels.len() {
            return Err(VaultError::InvalidData);
        }
        let host_ids = hosts.iter().copied().collect::<Vec<_>>();
        for tunnel in &self.tunnels {
            tunnel.validate(&host_ids)?;
        }
        if credentials.len() != self.credentials.len()
            || groups.len() != self.groups.len()
            || hosts.len() != self.hosts.len()
            || snippet_groups.len() != self.snippet_groups.len()
            || snippet_ids.len() != self.snippets.len()
        {
            return Err(VaultError::InvalidData);
        }
        for group in &self.snippet_groups {
            group.validate()?;
        }
        for snippet in &self.snippets {
            snippet.validate(&snippet_group_ids)?;
        }
        for credential in &self.credentials {
            if credential.id.is_nil() || !bounded(&credential.name, 256, true) {
                return Err(VaultError::InvalidData);
            }
            let valid = match &credential.material {
                CredentialMaterial::Password { value } => bounded(value.expose(), 8192, false),
                CredentialMaterial::ImportedKey {
                    content,
                    passphrase,
                } => {
                    bounded(content.expose(), 1024 * 1024, true)
                        && passphrase
                            .as_ref()
                            .is_none_or(|p| bounded(p.expose(), 8192, false))
                }
                CredentialMaterial::KeyFile { path, passphrase } => {
                    bounded(path, 4096, true)
                        && std::path::Path::new(path).is_absolute()
                        && passphrase
                            .as_ref()
                            .is_none_or(|p| bounded(p.expose(), 8192, false))
                }
                CredentialMaterial::Agent { fingerprint, .. } => {
                    bounded(fingerprint, 256, true) && fingerprint.starts_with("SHA256:")
                }
            };
            if !valid {
                return Err(VaultError::InvalidData);
            }
        }
        for group in &self.groups {
            if group.id.is_nil() || !bounded(&group.name, 256, true) {
                return Err(VaultError::InvalidData);
            }
            let mut seen = HashSet::new();
            let mut parent = group.parent_id;
            seen.insert(group.id);
            while let Some(id) = parent {
                if !seen.insert(id) {
                    return Err(VaultError::InvalidData);
                }
                parent = self
                    .groups
                    .iter()
                    .find(|candidate| candidate.id == id)
                    .ok_or(VaultError::InvalidData)?
                    .parent_id;
            }
        }
        for host in &self.hosts {
            host.validate()?;
            if !credentials.contains(&host.credential_id)
                || host.group_id.is_some_and(|id| !groups.contains(&id))
            {
                return Err(VaultError::InvalidData);
            }
        }
        let mut targets = HashSet::new();
        for trust in &self.trust {
            if normalize_target(&trust.target)? != trust.target
                || trust.port == 0
                || !targets.insert((&trust.target, trust.port))
                || !bounded(&trust.algorithm, 128, true)
                || !bounded(&trust.public_key, 16384, true)
                || !bounded(&trust.fingerprint, 256, true)
            {
                return Err(VaultError::InvalidData);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod target_tests {
    use super::*;

    #[test]
    fn targets_preserve_ipv6_scope_and_normalize_internal_host_names() {
        for (input, expected) in [
            ("Dev_Web_1.LOCAL.", "dev_web_1.local"),
            ("[FE80:0::1%0012]", "fe80::1%12"),
            ("fe80::1%ethernet_2", "fe80::1%ethernet_2"),
            ("[2001:db8::1]", "2001:db8::1"),
        ] {
            assert_eq!(normalize_target(input).unwrap(), expected);
        }
        assert_ne!(
            normalize_target("fe80::1%2").unwrap(),
            normalize_target("fe80::1%3").unwrap()
        );
        for invalid in [
            "127.0.0.1%2",
            "[fe80::1%]",
            "fe80::1%2%3",
            "fe80::1%4294967296",
            "fe80::1%a/b",
            "[fe80::1%2]:22",
            "bad name",
            "host;command",
            "_bad",
            "bad_",
        ] {
            assert!(
                normalize_target(invalid).is_err(),
                "invalid target accepted: {invalid}"
            );
        }
    }
}

#[cfg(test)]
mod record_tests {
    use super::*;

    #[test]
    fn custom_theme_ids_are_bounded_and_canonical() {
        assert!(valid_custom_theme_id(
            "custom:11111111-1111-4111-8111-111111111111"
        ));
        for invalid in [
            "custom:../file",
            "custom:00000000-0000-0000-0000-000000000000",
            "custom:11111111111141118111111111111111",
            "graphite",
        ] {
            assert!(!valid_custom_theme_id(invalid));
        }
    }

    #[test]
    fn production_flags_default_to_false_for_older_records() {
        let host: Host = serde_json::from_value(serde_json::json!({
            "id": "6f1d2c9a-1b5e-4c7d-9a10-000000000001",
            "name": "build-01",
            "address": "10.0.4.21",
            "port": 22,
            "username": "deploy",
            "credential_id": "6f1d2c9a-1b5e-4c7d-9a10-0000000000aa",
            "group_id": null,
            "tags": [],
            "favorite": false,
            "revision": 0,
            "working_directory": null,
            "startup_command": null,
            "encoding": "utf-8",
            "theme": "graphite",
            "agent_forwarding": false
        }))
        .unwrap();
        assert!(!host.production);
        let mut themed_host = host.clone();
        themed_host.theme = "custom:11111111-1111-4111-8111-111111111111".into();
        themed_host.validate().unwrap();
        let reopened: Host =
            serde_json::from_value(serde_json::to_value(&themed_host).unwrap()).unwrap();
        assert_eq!(reopened.theme, themed_host.theme);
        themed_host.theme = "custom:invalid".into();
        assert!(themed_host.validate().is_err());
        let group: Group = serde_json::from_value(serde_json::json!({
            "id": "6f1d2c9a-1b5e-4c7d-9a10-0000000000bb",
            "name": "Platform",
            "parent_id": null
        }))
        .unwrap();
        assert!(!group.production);
        let round_trip: Host = serde_json::from_value(
            serde_json::to_value(Host {
                production: true,
                ..host
            })
            .unwrap(),
        )
        .unwrap();
        assert!(round_trip.production);
    }
}

#[cfg(test)]
mod m3_validation_tests {
    use super::*;

    fn data() -> VaultData {
        let host_id = Uuid::from_u128(1);
        let credential_id = Uuid::from_u128(2);
        VaultData {
            hosts: vec![Host {
                id: host_id,
                name: "build".into(),
                address: "build.example.test".into(),
                port: 22,
                username: "deploy".into(),
                credential_id,
                group_id: None,
                tags: Vec::new(),
                favorite: false,
                revision: 0,
                working_directory: None,
                startup_command: None,
                encoding: "utf-8".into(),
                theme: "graphite".into(),
                agent_forwarding: false,
                production: false,
            }],
            credentials: vec![Credential {
                id: credential_id,
                name: "deploy".into(),
                revision: 0,
                material: CredentialMaterial::Password {
                    value: SecretText::new(String::new()),
                },
            }],
            ..VaultData::default()
        }
    }

    #[test]
    fn validates_tunnel_endpoints_and_referenced_hosts() {
        let value = data();
        let mut tunnel = TunnelConfig {
            id: Uuid::from_u128(3),
            host_id: Uuid::from_u128(1),
            auto_start: false,
            forward: TunnelForward::Local {
                bind_host: "127.0.0.1".into(),
                bind_port: 8080,
                destination_host: "service.internal".into(),
                destination_port: 80,
            },
        };
        let hosts = value.hosts.iter().map(|host| host.id).collect::<Vec<_>>();
        assert!(tunnel.validate(&hosts).is_ok());

        tunnel.forward = TunnelForward::Local {
            bind_host: "0.0.0.0".into(),
            bind_port: 0,
            destination_host: "service.internal".into(),
            destination_port: 80,
        };
        assert!(tunnel.validate(&hosts).is_err());

        tunnel.host_id = Uuid::from_u128(99);
        tunnel.forward = TunnelForward::Socks5 {
            bind_host: "127.0.0.1".into(),
            bind_port: 1080,
        };
        assert!(tunnel.validate(&hosts).is_err());
    }

    #[test]
    fn validates_snippet_ids_groups_names_and_template_bounds() {
        let group = SnippetGroup {
            id: Uuid::from_u128(4),
            name: "Operations".into(),
        };
        assert!(group.validate().is_ok());
        let mut snippet = Snippet {
            id: Uuid::from_u128(5),
            group_id: Some(Uuid::from_u128(4)),
            name: "List pods".into(),
            template: "kubectl get pods -n {{namespace:default}}".into(),
            auto_enter: true,
        };
        let groups = vec![group.id];
        assert!(snippet.validate(&groups).is_ok());

        snippet.group_id = Some(Uuid::from_u128(6));
        assert!(snippet.validate(&groups).is_err());
        snippet.group_id = Some(Uuid::from_u128(4));
        snippet.template = "x".repeat(8193);
        assert!(snippet.validate(&groups).is_err());
    }

    #[test]
    fn empty_m3_lists_keep_the_earlier_payload_shape() {
        let mut value = data();
        let encoded = serde_json::to_value(&value).unwrap();
        let keys = encoded.as_object().unwrap();
        for key in ["snippet_groups", "snippets", "tunnels"] {
            assert!(!keys.contains_key(key), "{key} must be omitted when empty");
        }
        let decoded: VaultData = serde_json::from_value(encoded).unwrap();
        decoded.validate().unwrap();

        value.tunnels.push(TunnelConfig {
            id: Uuid::from_u128(3),
            host_id: Uuid::from_u128(1),
            auto_start: true,
            forward: TunnelForward::Socks5 {
                bind_host: "127.0.0.1".into(),
                bind_port: 1080,
            },
        });
        let encoded = serde_json::to_value(&value).unwrap();
        assert!(encoded.as_object().unwrap().contains_key("tunnels"));
        let decoded: VaultData = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.tunnels, value.tunnels);
    }
}
