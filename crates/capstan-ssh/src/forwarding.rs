use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

pub const TUNNEL_STATE_CHANGED_EVENT: &str = "tunnel-state-changed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelRuntimeState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelErrorCode {
    BindAddressInUse,
    RemoteForwardRejected,
    DestinationUnreachable,
    ConnectionLost,
    Canceled,
    InvalidConfig,
    SshChannelRejected,
    SshProtocol,
    /// A new client was closed because the tunnel was at its client limit.
    ClientLimit,
}

impl TunnelErrorCode {
    pub const fn code(self) -> &'static str {
        match self {
            Self::BindAddressInUse => "bind_address_in_use",
            Self::RemoteForwardRejected => "remote_forward_rejected",
            Self::DestinationUnreachable => "destination_unreachable",
            Self::ConnectionLost => "connection_lost",
            Self::Canceled => "canceled",
            Self::InvalidConfig => "invalid_config",
            Self::SshChannelRejected => "ssh_channel_rejected",
            Self::SshProtocol => "ssh_protocol",
            Self::ClientLimit => "client_limit",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunnelRuntimeSnapshot {
    pub id: Uuid,
    pub state: TunnelRuntimeState,
    pub error_code: Option<TunnelErrorCode>,
    pub active_connections: u32,
    #[serde(with = "exact_u64_string")]
    pub bytes_sent: u64,
    #[serde(with = "exact_u64_string")]
    pub bytes_received: u64,
}

mod exact_u64_string {
    use super::*;
    use serde::de::Error as _;

    pub fn serialize<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.is_empty()
            || (value.len() > 1 && value.starts_with('0'))
            || !value.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(D::Error::custom("invalid exact unsigned integer"));
        }
        value
            .parse()
            .map_err(|_| D::Error::custom("exact unsigned integer is out of range"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tunnel_snapshot_matches_shared_fixture_without_losing_byte_counters() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../docs/agent/fixtures/M3-contract.json"
        ))
        .expect("fixture");
        let snapshot: TunnelRuntimeSnapshot =
            serde_json::from_value(fixture["tunnelRuntimeSnapshot"].clone())
                .expect("tunnel snapshot");
        assert_eq!(snapshot.bytes_sent, 9_007_199_254_740_993);
        assert_eq!(snapshot.bytes_received, u64::MAX);
        assert_eq!(
            serde_json::to_value(snapshot).expect("serialized snapshot"),
            fixture["tunnelRuntimeSnapshot"]
        );
    }
}
