//! Bounded subsystem leases on one authenticated SSH transport.
use crate::{sftp, ConnectionLease, PasswordConnection};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

const SUBSYSTEMS: usize = 3;

struct CachedSftp {
    client: std::sync::Weak<sftp::SftpClient>,
    reaper: sftp::SftpReaper,
}
struct Slot {
    cached: Arc<Mutex<Option<CachedSftp>>>,
    transfer: Arc<Semaphore>,
    unavailable: AtomicBool,
}
pub(crate) struct SftpPool {
    slots: [Slot; SUBSYSTEMS],
    admission: sftp::Admission,
}
impl Default for SftpPool {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| Slot {
                cached: Arc::new(Mutex::new(None)),
                transfer: Arc::new(Semaphore::new(1)),
                unavailable: AtomicBool::new(false),
            }),
            admission: sftp::Admission::default(),
        }
    }
}
impl SftpPool {
    pub(crate) async fn wait_pending(&self) {
        for slot in &self.slots {
            let cached = slot.cached.lock().await;
            if let Some(client) = cached
                .as_ref()
                .and_then(|previous| previous.client.upgrade())
            {
                client.wait_pending().await;
            }
        }
    }
    pub(crate) async fn close(&self) {
        for slot in &self.slots {
            let mut cached = slot.cached.lock().await;
            if let Some(previous) = cached.take() {
                if let Some(client) = previous.client.upgrade() {
                    client.close().await;
                    client.wait_pending().await;
                }
                previous.reaper.close().await;
            }
        }
    }
}

/// Exclusive data assignment. Browsing still shares the primary dispatcher.
/// The assignment outlives canceled requests and pipeline cleanup.
pub struct SftpTransferLease {
    client: Arc<sftp::SftpClient>,
    primary: Arc<sftp::SftpClient>,
    permit: Option<OwnedSemaphorePermit>,
}
impl SftpTransferLease {
    pub fn client(&self) -> &Arc<sftp::SftpClient> {
        &self.client
    }
    pub async fn release(mut self) {
        self.client.wait_pending().await;
        self.permit.take();
    }
}
impl Drop for SftpTransferLease {
    fn drop(&mut self) {
        let Some(permit) = self.permit.take() else {
            return;
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let client = self.client.clone();
            let primary = self.primary.clone();
            runtime.spawn(async move {
                let _assignment = (permit, primary);
                client.wait_pending().await;
            });
        }
    }
}

impl ConnectionLease<PasswordConnection> {
    /// Browsers and control workflows coalesce onto the primary subsystem.
    pub async fn open_sftp(
        &self,
        cancel: &CancellationToken,
    ) -> Result<Arc<sftp::SftpClient>, sftp::ClientError> {
        let primary = self.open_sftp_at(0, cancel).await?;
        // A replacement primary adopts healthy companions without waiting for
        // any unrelated initialization or holding multiple setup locks.
        for (index, slot) in self.connection().sftp.slots.iter().enumerate().skip(1) {
            if let Ok(cached) = slot.cached.try_lock() {
                if let Some(client) = cached
                    .as_ref()
                    .and_then(|previous| previous.client.upgrade())
                {
                    if !client.is_closed() {
                        primary.retain_companion(index - 1, client)?;
                    }
                }
            }
        }
        Ok(primary)
    }

    /// At most three file workers can hold different subsystem assignments.
    /// Extra subsystems are lazy and retained only while their primary has a
    /// user. Rejected extra subsystems are skipped until this authenticated
    /// connection is replaced; one remaining subsystem serves data serially.
    pub async fn lease_sftp_transfer(
        &self,
        cancel: &CancellationToken,
    ) -> Result<SftpTransferLease, sftp::ClientError> {
        let mut primary = self.open_sftp(cancel).await?;
        let connection = self.connection();
        let slots = &connection.sftp.slots;
        loop {
            let (index, permit) = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(sftp::ClientError::Canceled),
                () = connection.closed() => return Err(sftp::ClientError::ConnectionLost),
                permit = slots[0].transfer.clone().acquire_owned() => (0, permit),
                permit = slots[1].transfer.clone().acquire_owned(), if !slots[1].unavailable.load(Ordering::SeqCst) => (1, permit),
                permit = slots[2].transfer.clone().acquire_owned(), if !slots[2].unavailable.load(Ordering::SeqCst) => (2, permit),
            };
            let permit = permit.map_err(|_| sftp::ClientError::ConnectionLost)?;
            if index > 0 && slots[index].unavailable.load(Ordering::SeqCst) {
                drop(permit);
                continue;
            }
            if primary.is_closed() {
                primary = self.open_sftp(cancel).await?;
            }
            let (client, permit) = if index == 0 {
                (primary.clone(), permit)
            } else {
                match self.open_sftp_at(index, cancel).await {
                    Ok(client) => {
                        primary.retain_companion(index - 1, client.clone())?;
                        (client, permit)
                    }
                    Err(sftp::ClientError::Unavailable) => {
                        slots[index].unavailable.store(true, Ordering::SeqCst);
                        drop(permit);
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            };
            return Ok(SftpTransferLease {
                client,
                primary,
                permit: Some(permit),
            });
        }
    }

    async fn open_sftp_at(
        &self,
        index: usize,
        cancel: &CancellationToken,
    ) -> Result<Arc<sftp::SftpClient>, sftp::ClientError> {
        let connection = self.connection();
        let cached = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(sftp::ClientError::Canceled),
            () = connection.closed() => return Err(sftp::ClientError::ConnectionLost),
            cached = connection.sftp.slots[index].cached.clone().lock_owned() => cached,
        };
        if let Some(client) = cached
            .as_ref()
            .and_then(|previous| previous.client.upgrade())
        {
            if !client.is_closed() {
                return Ok(client);
            }
        }
        // The owned task keeps setup serialized even if its caller is aborted
        // or times out. The next open must wait for complete channel cleanup.
        let setup_cancel = CancellationToken::new();
        let _cancel_on_drop = setup_cancel.clone().drop_guard();
        let lease = self.clone();
        let mut setup = tokio::spawn(async move {
            let mut cached = cached;
            if let Some(previous) = cached.take() {
                if let Some(client) = previous.client.upgrade() {
                    client.close().await;
                    client.wait_pending().await;
                }
                previous.reaper.close().await;
            }
            let client = lease.connect_sftp(&setup_cancel).await?;
            if setup_cancel.is_cancelled() || lease.connection().is_closed() {
                client.close().await;
                return Err(sftp::ClientError::ConnectionLost);
            }
            client.retain_connection(lease.clone())?;
            *cached = Some(CachedSftp {
                client: Arc::downgrade(&client),
                reaper: client.reaper(),
            });
            Ok(client)
        });
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(sftp::ClientError::Canceled),
            () = connection.closed() => Err(sftp::ClientError::ConnectionLost),
            result = tokio::time::timeout(Duration::from_secs(15), &mut setup) => result
                .map_err(|_| sftp::ClientError::Unavailable)?
                .map_err(|_| sftp::ClientError::ConnectionLost)?,
        }
    }

    async fn connect_sftp(
        &self,
        cancel: &CancellationToken,
    ) -> Result<Arc<sftp::SftpClient>, sftp::ClientError> {
        let connection = self.connection();
        // The transport mutex covers channel creation only, never handshake,
        // file I/O, admission waits or cancellation cleanup.
        let mut channel = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(sftp::ClientError::Canceled),
            channel = async { connection.handle.lock().await.channel_open_session().await } => channel.map_err(|error| match error {
                russh::Error::ChannelOpenFailure(_) => sftp::ClientError::Unavailable,
                _ => sftp::ClientError::ConnectionLost,
            })?,
        };
        let request = async {
            channel
                .request_subsystem(true, "sftp")
                .await
                .map_err(|_| sftp::ClientError::Unavailable)?;
            loop {
                match channel.wait().await {
                    Some(russh::ChannelMsg::Success) => return Ok(()),
                    Some(russh::ChannelMsg::WindowAdjusted { .. }) => {}
                    _ => return Err(sftp::ClientError::Unavailable),
                }
            }
        };
        let requested = tokio::select! {
            biased;
            () = cancel.cancelled() => Err(sftp::ClientError::Canceled),
            result = request => result,
        };
        if let Err(error) = requested {
            let _ = tokio::time::timeout(Duration::from_secs(2), channel.close()).await;
            return Err(error);
        }
        sftp::SftpClient::connect_with_admission(
            channel.into_stream(),
            cancel,
            &connection.sftp.admission,
        )
        .await
    }
}
