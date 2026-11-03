use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use uuid::Uuid;

pub const CONNECTION_STATE_CHANGED_EVENT: &str = "connection-state-changed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Connecting,
    Authenticating,
    Online,
    Reconnecting,
    Disconnected,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionErrorCode {
    SshTimeout,
    SshConnection,
    SshAuthentication,
    SshProtocol,
    SshChannelRejected,
    SshPrivateKeyInvalid,
    SshAgentUnavailable,
    SshAgentIdentityUnavailable,
    HostKeyUnknown,
    HostKeyChanged,
    HostKeyRejected,
    VaultLocked,
    ConnectionLost,
    ReconnectExhausted,
    Canceled,
    InvalidInput,
}

impl ConnectionErrorCode {
    pub const fn code(self) -> &'static str {
        match self {
            Self::SshTimeout => "ssh_timeout",
            Self::SshConnection => "ssh_connection",
            Self::SshAuthentication => "ssh_authentication",
            Self::SshProtocol => "ssh_protocol",
            Self::SshChannelRejected => "ssh_channel_rejected",
            Self::SshPrivateKeyInvalid => "ssh_private_key_invalid",
            Self::SshAgentUnavailable => "ssh_agent_unavailable",
            Self::SshAgentIdentityUnavailable => "ssh_agent_identity_unavailable",
            Self::HostKeyUnknown => "host_key_unknown",
            Self::HostKeyChanged => "host_key_changed",
            Self::HostKeyRejected => "host_key_rejected",
            Self::VaultLocked => "vault_locked",
            Self::ConnectionLost => "connection_lost",
            Self::ReconnectExhausted => "reconnect_exhausted",
            Self::Canceled => "canceled",
            Self::InvalidInput => "invalid_input",
        }
    }
}

impl From<&crate::SshError> for ConnectionErrorCode {
    fn from(error: &crate::SshError) -> Self {
        match error {
            crate::SshError::Timeout => Self::SshTimeout,
            crate::SshError::Connection => Self::SshConnection,
            crate::SshError::Authentication => Self::SshAuthentication,
            crate::SshError::TrustRequired => Self::HostKeyUnknown,
            crate::SshError::HostKeyChanged => Self::HostKeyChanged,
            crate::SshError::TrustRejected => Self::HostKeyRejected,
            crate::SshError::Canceled => Self::Canceled,
            crate::SshError::ChannelRejected => Self::SshChannelRejected,
            crate::SshError::Protocol => Self::SshProtocol,
            crate::SshError::InvalidInput => Self::InvalidInput,
            crate::SshError::InvalidPrivateKey => Self::SshPrivateKeyInvalid,
            crate::SshError::AgentUnavailable => Self::SshAgentUnavailable,
            crate::SshError::AgentIdentityUnavailable => Self::SshAgentIdentityUnavailable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ConnectionSnapshot {
    pub connection_id: Uuid,
    #[serde(with = "exact_u64_string")]
    pub generation: u64,
    pub state: ConnectionState,
    pub error_code: Option<ConnectionErrorCode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ConnectionTransitionError {
    #[error("connection snapshot has an invalid identity or generation")]
    InvalidSnapshot,
    #[error("connection snapshot belongs to a different logical connection")]
    IdentityChanged,
    #[error("connection snapshot is from an older generation")]
    StaleGeneration,
    #[error("connection state transition is not allowed")]
    InvalidTransition,
}

pub fn validate_snapshot(snapshot: &ConnectionSnapshot) -> Result<(), ConnectionTransitionError> {
    if snapshot.connection_id.is_nil() || snapshot.generation == 0 {
        return Err(ConnectionTransitionError::InvalidSnapshot);
    }
    Ok(())
}

pub fn validate_successor(
    current: &ConnectionSnapshot,
    next: &ConnectionSnapshot,
) -> Result<(), ConnectionTransitionError> {
    validate_snapshot(current)?;
    validate_snapshot(next)?;
    if current.connection_id != next.connection_id {
        return Err(ConnectionTransitionError::IdentityChanged);
    }
    if next.generation < current.generation {
        return Err(ConnectionTransitionError::StaleGeneration);
    }
    if next.generation > current.generation {
        return if next.state == ConnectionState::Connecting {
            Ok(())
        } else {
            Err(ConnectionTransitionError::InvalidTransition)
        };
    }
    if current.state == next.state && current.error_code == next.error_code {
        return Ok(());
    }
    let allowed = matches!(
        (current.state, next.state),
        (ConnectionState::Connecting, ConnectionState::Authenticating)
            | (ConnectionState::Connecting, ConnectionState::Online)
            | (ConnectionState::Connecting, ConnectionState::Reconnecting)
            | (ConnectionState::Connecting, ConnectionState::Disconnected)
            | (ConnectionState::Connecting, ConnectionState::Failed)
            | (ConnectionState::Authenticating, ConnectionState::Online)
            | (
                ConnectionState::Authenticating,
                ConnectionState::Reconnecting
            )
            | (
                ConnectionState::Authenticating,
                ConnectionState::Disconnected
            )
            | (ConnectionState::Authenticating, ConnectionState::Failed)
            | (ConnectionState::Online, ConnectionState::Reconnecting)
            | (ConnectionState::Online, ConnectionState::Disconnected)
            | (ConnectionState::Reconnecting, ConnectionState::Disconnected)
            | (ConnectionState::Reconnecting, ConnectionState::Failed)
            | (ConnectionState::Failed, ConnectionState::Disconnected)
    );
    if allowed {
        Ok(())
    } else {
        Err(ConnectionTransitionError::InvalidTransition)
    }
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

    fn snapshot(generation: u64, state: ConnectionState) -> ConnectionSnapshot {
        ConnectionSnapshot {
            connection_id: Uuid::from_u128(1),
            generation,
            state,
            error_code: None,
        }
    }

    #[test]
    fn rejects_late_events_from_an_older_connection_generation() {
        let current = snapshot(8, ConnectionState::Online);
        let stale = snapshot(7, ConnectionState::Disconnected);
        assert_eq!(
            validate_successor(&current, &stale),
            Err(ConnectionTransitionError::StaleGeneration)
        );
    }

    #[test]
    fn new_generation_must_begin_connecting_and_keep_logical_identity() {
        let current = snapshot(8, ConnectionState::Reconnecting);
        assert!(validate_successor(&current, &snapshot(9, ConnectionState::Connecting)).is_ok());
        assert_eq!(
            validate_successor(&current, &snapshot(9, ConnectionState::Online)),
            Err(ConnectionTransitionError::InvalidTransition)
        );
        let mut changed_id = snapshot(9, ConnectionState::Connecting);
        changed_id.connection_id = Uuid::from_u128(2);
        assert_eq!(
            validate_successor(&current, &changed_id),
            Err(ConnectionTransitionError::IdentityChanged)
        );
    }

    #[test]
    fn accepts_only_declared_same_generation_transitions() {
        let connecting = snapshot(1, ConnectionState::Connecting);
        let authenticating = snapshot(1, ConnectionState::Authenticating);
        let online = snapshot(1, ConnectionState::Online);
        assert!(validate_successor(&connecting, &authenticating).is_ok());
        assert!(validate_successor(&authenticating, &online).is_ok());
        assert_eq!(
            validate_successor(&online, &authenticating),
            Err(ConnectionTransitionError::InvalidTransition)
        );
        assert_eq!(
            validate_successor(
                &snapshot(1, ConnectionState::Disconnected),
                &snapshot(1, ConnectionState::Online)
            ),
            Err(ConnectionTransitionError::InvalidTransition)
        );
    }

    #[test]
    fn generation_uses_an_exact_decimal_string_on_the_wire() {
        let value = snapshot(u64::MAX, ConnectionState::Online);
        let json = serde_json::to_value(&value).expect("serialize connection snapshot");
        assert_eq!(json["generation"], u64::MAX.to_string());
        assert_eq!(
            serde_json::from_value::<ConnectionSnapshot>(json).expect("deserialize snapshot"),
            value
        );
        for invalid in ["01", "-1", "18446744073709551616", "1.0"] {
            let json = serde_json::json!({
                "connectionId": Uuid::from_u128(1),
                "generation": invalid,
                "state": "online",
                "errorCode": null
            });
            assert!(serde_json::from_value::<ConnectionSnapshot>(json).is_err());
        }
    }

    #[test]
    fn existing_ssh_errors_map_to_stable_connection_codes() {
        assert_eq!(
            ConnectionErrorCode::from(&crate::SshError::TrustRequired),
            ConnectionErrorCode::HostKeyUnknown
        );
        assert_eq!(
            ConnectionErrorCode::from(&crate::SshError::HostKeyChanged).code(),
            "host_key_changed"
        );
        assert_eq!(
            ConnectionErrorCode::from(&crate::SshError::Canceled).code(),
            "canceled"
        );
    }
}
