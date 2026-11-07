use crate::sftp::ExactU64;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const SERVER_STATUS_EVENT: &str = "server-status-changed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerStatusState {
    Probing,
    Running,
    Paused,
    Unsupported,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum StatusError {
    #[serde(rename = "status_unsupported")]
    #[error("status_unsupported")]
    Unsupported,
    #[serde(rename = "status_rejected")]
    #[error("status_rejected")]
    Rejected,
    #[serde(rename = "status_parse")]
    #[error("status_parse")]
    Parse,
    #[serde(rename = "status_limit")]
    #[error("status_limit")]
    Limit,
    #[serde(rename = "status_closed")]
    #[error("status_closed")]
    Closed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerStatusSnapshot {
    pub connection_id: Uuid,
    pub generation: ExactU64,
    #[serde(flatten)]
    pub observation: StatusObservation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusObservation {
    pub state: ServerStatusState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<StatusError>,
    pub sampled_at: ExactU64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_percent: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<Memory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk: Option<Disk>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkRate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Details>,
}

impl StatusObservation {
    pub(super) fn state(state: ServerStatusState, error_code: Option<StatusError>) -> Self {
        Self {
            state,
            error_code,
            sampled_at: ExactU64(0),
            cpu_percent: None,
            load: None,
            memory: None,
            disk: None,
            network: None,
            details: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Memory {
    pub total: ExactU64,
    pub available: ExactU64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Disk {
    pub mount: String,
    pub total: ExactU64,
    pub used: ExactU64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkRate {
    pub rx_per_sec: ExactU64,
    pub tx_per_sec: ExactU64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Details {
    pub disks: Vec<Disk>,
    pub interfaces: Vec<Interface>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Interface {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate: Option<NetworkRate>,
}
