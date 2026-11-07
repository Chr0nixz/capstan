//! Status sampling on the authenticated connection, independent of terminal I/O.

#[cfg(test)]
mod contract_tests;
mod contracts;
mod frame;
#[cfg(test)]
mod tests;
mod worker;

pub use contracts::*;

use crate::{sftp::ExactU64, VaultManager};
use capstan_ssh::{connection_state::ConnectionState, ConnectionLease, PasswordConnection};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::{watch, Notify};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct Collector {
    activity: watch::Sender<CancellationToken>,
    observation: watch::Sender<StatusObservation>,
    retry: Notify,
    transport: Weak<PasswordConnection>,
}

#[derive(Default)]
struct Selection {
    connection: Option<Uuid>,
    visible: bool,
    collectors: HashMap<Uuid, Arc<Collector>>,
}

#[derive(Clone, Default)]
pub(super) struct StatusService {
    selection: Arc<Mutex<Selection>>,
    target: Arc<watch::Sender<Option<Uuid>>>,
    updates: Arc<watch::Sender<()>>,
}

impl StatusService {
    fn start(&self, lease: &ConnectionLease<PasswordConnection>) {
        let Ok(idle) = lease.idle_token() else {
            return;
        };
        let transport = lease.connection();
        let id = transport.connection_id();
        let Ok(mut selection) = self.selection.lock() else {
            return;
        };
        if let Some(collector) = selection.collectors.get(&id) {
            if collector.activity.borrow().is_cancelled() {
                if collector.observation.borrow().error_code.is_none() {
                    collector
                        .observation
                        .send_replace(StatusObservation::state(ServerStatusState::Paused, None));
                }
                collector.activity.send_replace(idle);
                self.updates.send_replace(());
            }
            return;
        }
        let collector = Arc::new(Collector {
            activity: watch::channel(idle).0,
            observation: watch::channel(StatusObservation::state(ServerStatusState::Paused, None))
                .0,
            retry: Notify::new(),
            transport: lease.downgrade_connection(),
        });
        let closed = transport.transport_close_token();
        selection.collectors.insert(id, collector.clone());
        drop(selection);
        let service = self.clone();
        tokio::spawn(async move {
            collector
                .run(id, closed, service.target.subscribe(), &service.updates)
                .await;
            if let Ok(mut selection) = service.selection.lock() {
                selection.collectors.remove(&id);
            }
            service.updates.send_replace(());
        });
        self.updates.send_replace(());
    }
}

impl VaultManager {
    pub(super) fn start_server_status(&self, lease: &ConnectionLease<PasswordConnection>) {
        self.server_status.start(lease);
        self.refresh_server_status_target();
    }

    /// The UI supplies a logical connection ID. Window visibility is set separately in Rust.
    pub fn select_server_status_connection(&self, id: Option<Uuid>) {
        self.update_server_status_selection(|selection| {
            selection.connection = id;
            true
        });
    }

    pub fn set_server_status_window_visible(&self, visible: bool) {
        self.update_server_status_selection(|selection| {
            if selection.visible == visible {
                return false;
            }
            selection.visible = visible;
            true
        });
    }

    pub fn refresh_server_status_target(&self) {
        self.update_server_status_selection(|_| true);
    }

    fn update_server_status_selection(&self, update: impl FnOnce(&mut Selection) -> bool) {
        let Ok(mut selection) = self.server_status.selection.lock() else {
            return;
        };
        if !update(&mut selection) {
            return;
        }
        let target = selection
            .connection
            .filter(|_| selection.visible)
            .and_then(|id| self.status_transport(id))
            .map(|(_, id)| id);
        self.server_status.target.send_if_modified(|current| {
            if *current == target {
                return false;
            }
            *current = target;
            true
        });
        self.server_status.updates.send_replace(());
    }

    fn status_transport(&self, id: Uuid) -> Option<(u64, Uuid)> {
        let entry = self.managed_connection(id).ok()?;
        let record = entry.record.lock().ok()?;
        if record.snapshot.state != ConnectionState::Online || entry.stop.is_cancelled() {
            return None;
        }
        let transport = record.transport.upgrade()?;
        (!transport.is_closed()).then(|| (record.snapshot.generation, transport.connection_id()))
    }

    pub fn subscribe_server_status(&self) -> watch::Receiver<()> {
        self.server_status.updates.subscribe()
    }

    #[must_use]
    pub fn server_status_snapshot(&self) -> Option<ServerStatusSnapshot> {
        let selection = self.server_status.selection.lock().ok()?;
        let connection_id = selection.connection?;
        let (generation, transport) = self.status_transport(connection_id)?;
        let collector = selection.collectors.get(&transport)?;
        if collector.activity.borrow().is_cancelled() {
            return None;
        }
        let mut observation = collector.observation.borrow().clone();
        if !selection.visible
            && matches!(
                observation.state,
                ServerStatusState::Probing | ServerStatusState::Running
            )
        {
            observation.state = ServerStatusState::Paused;
        }
        Some(ServerStatusSnapshot {
            connection_id,
            generation: ExactU64(generation),
            observation,
        })
    }

    pub fn retry_server_status(&self, connection_id: Uuid, generation: ExactU64) -> bool {
        let Ok(selection) = self.server_status.selection.lock() else {
            return false;
        };
        if selection.connection != Some(connection_id) {
            return false;
        }
        let Some((current, transport)) = self.status_transport(connection_id) else {
            return false;
        };
        let Some(collector) = selection.collectors.get(&transport) else {
            return false;
        };
        if current != generation.0
            || collector.activity.borrow().is_cancelled()
            || collector.observation.borrow().state != ServerStatusState::Unavailable
        {
            return false;
        }
        collector.retry.notify_one();
        true
    }
}
