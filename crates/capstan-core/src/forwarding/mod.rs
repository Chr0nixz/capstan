//! Encrypted tunnel configuration and connection-generation ownership.
use crate::{CoreError, ManagedConnection, Result, VaultManager};
use capstan_ssh::{
    connection_state::ConnectionState,
    forwarding::{TunnelErrorCode, TunnelRuntimeSnapshot, TunnelRuntimeState},
    forwarding_runtime::{ForwardSpec, TunnelRuntime},
    ConnectionLease, PasswordConnection,
};
use capstan_vault::{normalize_target, TunnelConfig, TunnelForward, VaultError};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct Binding {
    connection_id: Uuid,
    generation: u64,
    config: TunnelConfig,
    runtime: Arc<TunnelRuntime>,
}
pub(crate) struct ForwardingService {
    pub(crate) gate: tokio::sync::Mutex<()>,
    bindings: Mutex<HashMap<Uuid, Binding>>,
    operations: Mutex<HashMap<Uuid, Weak<tokio::sync::Mutex<()>>>>,
    suppressed: Mutex<HashSet<(Uuid, Uuid, u64)>>,
    /// Automatic tunnels the user stopped. They stay stopped across reconnects
    /// until a manual start or an application restart.
    held: Mutex<HashSet<Uuid>>,
    events: broadcast::Sender<TunnelRuntimeSnapshot>,
}
impl Default for ForwardingService {
    fn default() -> Self {
        Self {
            gate: tokio::sync::Mutex::new(()),
            bindings: Mutex::new(HashMap::new()),
            operations: Mutex::new(HashMap::new()),
            suppressed: Mutex::new(HashSet::new()),
            held: Mutex::new(HashSet::new()),
            events: broadcast::channel(256).0,
        }
    }
}
impl ForwardingService {
    fn operation(&self, id: Uuid) -> Result<Arc<tokio::sync::Mutex<()>>> {
        let mut operations = self.operations.lock().map_err(|_| CoreError::TaskFailed)?;
        operations.retain(|_, gate| gate.strong_count() > 0);
        Ok(operations
            .entry(id)
            .or_default()
            .upgrade()
            .unwrap_or_else(|| {
                let gate = Arc::new(tokio::sync::Mutex::new(()));
                operations.insert(id, Arc::downgrade(&gate));
                gate
            }))
    }
}
fn invalid() -> CoreError {
    CoreError::Tunnel(TunnelErrorCode::InvalidConfig)
}
fn spec(config: &TunnelConfig) -> Result<ForwardSpec> {
    let normalize = |host: &str| normalize_target(host).map_err(|_| invalid());
    Ok(match &config.forward {
        TunnelForward::Local {
            bind_host,
            bind_port,
            destination_host,
            destination_port,
        } => ForwardSpec::Local {
            bind_host: normalize(bind_host)?,
            bind_port: *bind_port,
            destination_host: normalize(destination_host)?,
            destination_port: *destination_port,
        },
        TunnelForward::Remote {
            remote_bind_host,
            remote_bind_port,
            local_destination_host,
            local_destination_port,
        } => ForwardSpec::Remote {
            remote_bind_host: normalize(remote_bind_host)?,
            remote_bind_port: *remote_bind_port,
            local_destination_host: normalize(local_destination_host)?,
            local_destination_port: *local_destination_port,
        },
        TunnelForward::Socks5 {
            bind_host,
            bind_port,
        } => ForwardSpec::Socks5 {
            bind_host: normalize(bind_host)?,
            bind_port: *bind_port,
        },
    })
}

impl VaultManager {
    pub fn tunnels(&self) -> Result<Vec<TunnelConfig>> {
        Ok(self.current()?.snapshot()?.tunnels)
    }
    pub fn subscribe_tunnels(&self) -> broadcast::Receiver<TunnelRuntimeSnapshot> {
        self.forwarding.events.subscribe()
    }
    pub fn tunnel_runtime_snapshots(&self) -> Result<Vec<TunnelRuntimeSnapshot>> {
        Ok(self
            .forwarding
            .bindings
            .lock()
            .map_err(|_| CoreError::TaskFailed)?
            .values()
            .map(|binding| binding.runtime.snapshot())
            .collect())
    }
    pub async fn save_tunnel(&self, config: TunnelConfig, expected_revision: u64) -> Result<u64> {
        let _gate = self.forwarding.gate.lock().await;
        let operation = self.forwarding.operation(config.id)?;
        let _operation = operation.lock().await;
        let vault = self.current()?;
        let data = vault.snapshot()?;
        if data.revision != expected_revision {
            return Err(CoreError::Conflict);
        }
        config
            .validate(&data.hosts.iter().map(|host| host.id).collect::<Vec<_>>())
            .map_err(|_| invalid())?;
        self.require_tunnel_stopped(config.id, true).await?;
        let id = config.id;
        let revision = tokio::task::spawn_blocking(move || {
            vault.transaction(expected_revision, move |data| {
                if let Some(existing) = data.tunnels.iter_mut().find(|tunnel| tunnel.id == id) {
                    *existing = config;
                } else {
                    data.tunnels.push(config);
                }
                Ok(())
            })
        })
        .await
        .map_err(|_| CoreError::TaskFailed)??;
        self.forwarding
            .bindings
            .lock()
            .map_err(|_| CoreError::TaskFailed)?
            .remove(&id);
        Ok(revision)
    }
    pub async fn delete_tunnel(&self, id: Uuid, expected_revision: u64) -> Result<u64> {
        let _gate = self.forwarding.gate.lock().await;
        let operation = self.forwarding.operation(id)?;
        let _operation = operation.lock().await;
        let vault = self.current()?;
        let data = vault.snapshot()?;
        if data.revision != expected_revision {
            return Err(CoreError::Conflict);
        }
        if !data.tunnels.iter().any(|config| config.id == id) {
            return Err(CoreError::NotFound);
        }
        self.require_tunnel_stopped(id, true).await?;
        let revision = tokio::task::spawn_blocking(move || {
            vault.transaction(expected_revision, move |data| {
                let position = data
                    .tunnels
                    .iter()
                    .position(|config| config.id == id)
                    .ok_or(VaultError::NotFound)?;
                data.tunnels.remove(position);
                Ok(())
            })
        })
        .await
        .map_err(|_| CoreError::TaskFailed)??;
        self.forwarding
            .bindings
            .lock()
            .map_err(|_| CoreError::TaskFailed)?
            .remove(&id);
        if let Ok(mut held) = self.forwarding.held.lock() {
            held.remove(&id);
        }
        Ok(revision)
    }
    /// A tunnel can only join an already authenticated, policy-compatible generation.
    pub async fn start_tunnel(
        &self,
        id: Uuid,
        connection_id: Uuid,
        generation: u64,
    ) -> Result<TunnelRuntimeSnapshot> {
        let entry = self
            .managed_connection(connection_id)
            .map_err(|_| invalid())?;
        let _tracked = entry.tasks.token();
        let _connection_gate = entry.gate.lock().await;
        let _gate = self.forwarding.gate.lock().await;
        let operation = self.forwarding.operation(id)?;
        let _operation = operation.lock().await;
        let config = self
            .tunnels()?
            .into_iter()
            .find(|config| config.id == id)
            .ok_or(CoreError::NotFound)?;
        let (policy, transport_generation) = {
            let record = entry.record.lock().map_err(|_| CoreError::TaskFailed)?;
            if record.snapshot.generation != generation
                || record.snapshot.state != ConnectionState::Online
                || entry.stop.is_cancelled()
            {
                return Err(invalid());
            }
            (record.policy.clone(), record.transport_generation)
        };
        if self
            .connections
            .locked
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(CoreError::Locked);
        }
        let (_, expected) = self.host_policy(config.host_id, Uuid::new_v4())?;
        if policy != expected {
            return Err(invalid());
        }
        let lease = self
            .ssh_pool
            .acquire_checked(
                policy,
                entry.stop.child_token(),
                |connection| !connection.is_closed(),
                |_| async { Err(capstan_ssh::SshError::Connection) },
            )
            .await?;
        if lease.connection().connection_generation() != transport_generation {
            return Err(invalid());
        }
        self.forwarding
            .held
            .lock()
            .map_err(|_| CoreError::TaskFailed)?
            .remove(&id);
        let runtime = self.start_tunnel_locked(config, &entry, lease, false)?;
        drop(_operation);
        drop(_gate);
        drop(_connection_gate);
        Ok(runtime.started().await)
    }
    fn start_tunnel_locked(
        &self,
        config: TunnelConfig,
        entry: &Arc<ManagedConnection>,
        lease: ConnectionLease<PasswordConnection>,
        automatic: bool,
    ) -> Result<Arc<TunnelRuntime>> {
        let (connection_id, generation) = {
            let record = entry.record.lock().map_err(|_| CoreError::TaskFailed)?;
            if entry.stop.is_cancelled()
                || record.snapshot.state != ConnectionState::Online
                || lease.connection().is_closed()
            {
                return Err(invalid());
            }
            (record.snapshot.connection_id, record.snapshot.generation)
        };
        let key = (config.id, connection_id, generation);
        if automatic
            && (self
                .forwarding
                .suppressed
                .lock()
                .map_err(|_| CoreError::TaskFailed)?
                .contains(&key)
                || self
                    .forwarding
                    .held
                    .lock()
                    .map_err(|_| CoreError::TaskFailed)?
                    .contains(&config.id))
        {
            return Err(CoreError::Canceled);
        }
        {
            let bindings = self
                .forwarding
                .bindings
                .lock()
                .map_err(|_| CoreError::TaskFailed)?;
            if let Some(existing) = bindings.get(&config.id) {
                if !existing.runtime.is_finished() {
                    if existing.connection_id == connection_id
                        && existing.generation == generation
                        && existing.config == config
                    {
                        return Ok(existing.runtime.clone());
                    }
                    return Err(invalid());
                }
            }
        }
        let runtime = Arc::new(TunnelRuntime::spawn(
            config.id,
            spec(&config)?,
            lease,
            self.forwarding.events.clone(),
        ));
        entry
            .record
            .lock()
            .map_err(|_| CoreError::TaskFailed)?
            .owners
            .insert(config.id, CancellationToken::new());
        self.forwarding
            .bindings
            .lock()
            .map_err(|_| CoreError::TaskFailed)?
            .insert(
                config.id,
                Binding {
                    connection_id,
                    generation,
                    config: config.clone(),
                    runtime: runtime.clone(),
                },
            );
        let vault = self.clone();
        let connection = entry.clone();
        let started = runtime.clone();
        entry.tasks.spawn(async move {
            let snapshot = started.started().await;
            if snapshot.state != TunnelRuntimeState::Running {
                started.wait_for_shutdown().await;
            }
            if snapshot.state != TunnelRuntimeState::Running && started.is_finished() {
                // A canceled/failed old start must not remove a replacement's owner.
                if let Ok(operation) = vault.forwarding.operation(config.id) {
                    let _operation = operation.lock().await;
                    let current = vault.forwarding.bindings.lock().is_ok_and(|bindings| {
                        bindings
                            .get(&config.id)
                            .is_some_and(|binding| Arc::ptr_eq(&binding.runtime, &started))
                    });
                    if current {
                        if let Ok(mut record) = connection.record.lock() {
                            record.owners.remove(&config.id);
                        }
                    }
                }
            }
        });
        Ok(runtime)
    }
    /// An explicit stop also holds an automatic tunnel until it is started again.
    pub async fn stop_tunnel(&self, id: Uuid) -> Result<TunnelRuntimeSnapshot> {
        let operation = self.forwarding.operation(id)?;
        let _operation = operation.lock().await;
        self.forwarding
            .held
            .lock()
            .map_err(|_| CoreError::TaskFailed)?
            .insert(id);
        self.stop_tunnel_locked(id, true).await
    }
    async fn require_tunnel_stopped(&self, id: Uuid, suppress: bool) -> Result<()> {
        let snapshot = self.stop_tunnel_locked(id, suppress).await?;
        if snapshot.state != TunnelRuntimeState::Stopped {
            return Err(CoreError::Tunnel(
                snapshot
                    .error_code
                    .unwrap_or(TunnelErrorCode::RemoteForwardRejected),
            ));
        }
        Ok(())
    }
    async fn stop_tunnel_locked(&self, id: Uuid, suppress: bool) -> Result<TunnelRuntimeSnapshot> {
        let binding = self
            .forwarding
            .bindings
            .lock()
            .map_err(|_| CoreError::TaskFailed)?
            .get(&id)
            .map(|binding| {
                (
                    binding.runtime.clone(),
                    binding.connection_id,
                    binding.generation,
                )
            });
        if let Some((runtime, connection_id, generation)) = binding {
            if suppress {
                self.forwarding
                    .suppressed
                    .lock()
                    .map_err(|_| CoreError::TaskFailed)?
                    .insert((id, connection_id, generation));
            }
            let snapshot = runtime.stop().await;
            if runtime.is_finished() {
                if let Ok(entry) = self.managed_connection(connection_id) {
                    if let Ok(mut record) = entry.record.lock() {
                        record.owners.remove(&id);
                    }
                }
            }
            return Ok(snapshot);
        }
        Ok(TunnelRuntime::stopped_snapshot(id))
    }
    pub(crate) async fn start_auto_tunnels(
        &self,
        entry: &Arc<ManagedConnection>,
        lease: &ConnectionLease<PasswordConnection>,
    ) {
        let _gate = self.forwarding.gate.lock().await;
        if self
            .connections
            .locked
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return;
        }
        let Ok(configs) = self.tunnels() else {
            return;
        };
        let policy = match entry.record.lock() {
            Ok(record) => record.policy.clone(),
            Err(_) => return,
        };
        for config in configs.into_iter().filter(|config| config.auto_start) {
            let compatible = self
                .host_policy(config.host_id, Uuid::new_v4())
                .is_ok_and(|(_, current)| current == policy);
            if compatible {
                let Ok(operation) = self.forwarding.operation(config.id) else {
                    continue;
                };
                // An explicit operation already owns this tunnel; don't delay siblings.
                let Ok(_operation) = operation.try_lock() else {
                    continue;
                };
                let _ = self.start_tunnel_locked(config, entry, lease.clone(), true);
            }
        }
    }
    pub(crate) async fn stop_connection_tunnels(&self, connection_id: Uuid, disconnected: bool) {
        let _gate = self.forwarding.gate.lock().await;
        let bindings = self
            .forwarding
            .bindings
            .lock()
            .map(|bindings| {
                bindings
                    .values()
                    .filter(|binding| binding.connection_id == connection_id)
                    .map(|binding| {
                        (
                            binding.config.id,
                            binding.config.auto_start,
                            binding.runtime.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (_, _, runtime) in &bindings {
            runtime.cancel();
        }
        for (id, auto_start, runtime) in bindings {
            runtime.stop().await;
            if disconnected || !auto_start {
                if let Ok(entry) = self.managed_connection(connection_id) {
                    if let Ok(mut record) = entry.record.lock() {
                        record.owners.remove(&id);
                    }
                }
            }
        }
        if let Ok(mut suppressed) = self.forwarding.suppressed.lock() {
            suppressed.retain(|(_, connection, _)| *connection != connection_id);
        }
    }
    pub(crate) async fn stop_host_tunnels_locked(&self, host_id: Uuid) -> Result<()> {
        let mut ids = self
            .forwarding
            .bindings
            .lock()
            .map(|bindings| {
                bindings
                    .values()
                    .filter(|binding| binding.config.host_id == host_id)
                    .map(|binding| {
                        (
                            matches!(&binding.config.forward, TunnelForward::Remote { .. }),
                            binding.config.id,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        // Remote cancellation can fail while its server-side listener remains active.
        // Attempt those first so a refusal cannot leave local sibling listeners stopped.
        ids.sort_unstable_by_key(|(remote, id)| (!remote, *id));
        for (_, id) in ids {
            let operation = self.forwarding.operation(id)?;
            let _operation = operation.lock().await;
            self.require_tunnel_stopped(id, false).await?;
            if let Ok(mut bindings) = self.forwarding.bindings.lock() {
                bindings.remove(&id);
            }
            if let Ok(mut suppressed) = self.forwarding.suppressed.lock() {
                suppressed.retain(|(tunnel_id, _, _)| *tunnel_id != id);
            }
            if let Ok(mut held) = self.forwarding.held.lock() {
                held.remove(&id);
            }
        }
        Ok(())
    }
    pub(crate) async fn stop_all_tunnels(&self) {
        let _gate = self.forwarding.gate.lock().await;
        let _ = self.stop_all_tunnels_locked().await;
    }
    pub(crate) async fn stop_all_tunnels_locked(&self) -> Result<()> {
        let ids = self
            .forwarding
            .bindings
            .lock()
            .map(|bindings| bindings.keys().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        for id in ids {
            let operation = self.forwarding.operation(id)?;
            let _operation = operation.lock().await;
            self.require_tunnel_stopped(id, true).await?;
        }
        Ok(())
    }
}
