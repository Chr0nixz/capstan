//! One non-interactive command on an existing authenticated transport.

use crate::PasswordConnection;
use russh::{client, Channel, ChannelMsg, Sig};
use std::{future::pending, sync::Arc, time::Duration};
use tokio::{
    sync::{watch, OwnedMutexGuard},
    time::timeout,
};
use tokio_util::sync::CancellationToken;

const OPEN_TIMEOUT: Duration = Duration::from_secs(8);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
// Leave room for the foreground retry within its 15-second setup deadline.
const STATUS_CLOSE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_STDOUT_PACKET: usize = 64 * 1024;

pub(crate) struct StatusChannelCoordinator {
    gate: Arc<tokio::sync::Mutex<()>>,
    active: watch::Sender<Option<CancellationToken>>,
}

impl Default for StatusChannelCoordinator {
    fn default() -> Self {
        Self {
            gate: Arc::new(tokio::sync::Mutex::new(())),
            active: watch::channel(None).0,
        }
    }
}

impl StatusChannelCoordinator {
    fn register(self: &Arc<Self>, gate: OwnedMutexGuard<()>) -> StatusChannelLease {
        let cancel = CancellationToken::new();
        self.active.send_replace(Some(cancel.clone()));
        StatusChannelLease {
            coordinator: self.clone(),
            cancel,
            _gate: gate,
        }
    }

    async fn yield_for_retry(&self) -> OwnedMutexGuard<()> {
        let mut active = self.active.subscribe();
        let gate = self.gate.clone().lock_owned();
        tokio::pin!(gate);
        loop {
            if let Some(token) = active.borrow_and_update().as_ref() {
                token.cancel();
            }
            tokio::select! {
                biased;
                gate = &mut gate => return gate,
                _ = active.changed() => {}
            }
        }
    }
}

struct StatusChannelLease {
    coordinator: Arc<StatusChannelCoordinator>,
    cancel: CancellationToken,
    _gate: OwnedMutexGuard<()>,
}

impl Drop for StatusChannelLease {
    fn drop(&mut self) {
        self.coordinator.active.send_replace(None);
    }
}

struct ExecLease {
    _exec_gate: OwnedMutexGuard<()>,
    status: Option<StatusChannelLease>,
}

impl ExecLease {
    fn preempted(&self) -> Option<CancellationToken> {
        self.status.as_ref().map(|status| status.cancel.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ExecError {
    #[error("exec channel yielded to an interactive channel")]
    Yielded,
    #[error("exec channel rejected")]
    Rejected,
    #[error("exec channel closed")]
    Closed,
    #[error("exec channel timed out")]
    Timeout,
    #[error("exec output limit exceeded")]
    Limit,
    #[error("unexpected exec output")]
    UnexpectedOutput,
}

pub struct ExecChannel {
    channel: Option<(Channel<client::Msg>, ExecLease)>,
    pending: Vec<u8>,
    offset: usize,
    exit_status: Option<u32>,
    eof: bool,
}

impl PasswordConnection {
    pub(crate) async fn open_user_session(&self) -> Result<Channel<client::Msg>, russh::Error> {
        let mut status = self.status_channel.active.subscribe();
        let status_active = status.borrow_and_update().is_some();
        let result = self.handle.lock().await.channel_open_session().await;
        if matches!(result, Err(russh::Error::ChannelOpenFailure(_)))
            && (status_active || status.has_changed().unwrap_or(false))
        {
            // An available slot needs no coordination. After a refusal, keep
            // status out until this retry has reserved its session slot.
            let _gate = self.status_channel.yield_for_retry().await;
            return self.handle.lock().await.channel_open_session().await;
        }
        result
    }

    /// `command` must be fixed by the caller. Send protocol data via stdin.
    pub async fn open_exec(&self, command: &'static str) -> Result<ExecChannel, ExecError> {
        self.open_exec_inner(command, None).await
    }

    /// Opens the status channel, which interactive shell and SFTP opens may preempt.
    pub async fn open_status_exec(&self, command: &'static str) -> Result<ExecChannel, ExecError> {
        let gate = tokio::select! {
            biased;
            () = self.closed() => return Err(ExecError::Closed),
            gate = self.status_channel.gate.clone().lock_owned() => gate,
        };
        let status = self.status_channel.register(gate);
        self.open_exec_inner(command, Some(status)).await
    }

    async fn open_exec_inner(
        &self,
        command: &'static str,
        status: Option<StatusChannelLease>,
    ) -> Result<ExecChannel, ExecError> {
        let preempted = status.as_ref().map(|status| status.cancel.clone());
        // Clone the handle before opening so a slow session request does not hold
        // the mutex used by shell, SFTP, forwarding or the transport probe.
        let open = async move {
            let gate = self.exec_gate.clone().lock_owned().await;
            let lease = ExecLease {
                _exec_gate: gate,
                status,
            };
            let handle = self.handle.lock().await.clone();
            // A canceled caller cannot close a session open until the server
            // confirms it. Retain the gate and reap late confirmations without
            // holding a pool lease or the connection itself.
            let (mut result, receiver) = tokio::sync::oneshot::channel();
            tokio::spawn(async move {
                let opened = handle
                    .channel_open_session()
                    .await
                    .map_err(|error| match error {
                        russh::Error::ChannelOpenFailure(_) => ExecError::Rejected,
                        _ => ExecError::Closed,
                    });
                let opened = match opened {
                    Ok(channel) => {
                        let mut exec = ExecChannel {
                            channel: Some((channel, lease)),
                            pending: Vec::new(),
                            offset: 0,
                            exit_status: None,
                            eof: false,
                        };
                        let requested = tokio::select! {
                            biased;
                            () = result.closed() => {
                                exec.close().await;
                                return;
                            }
                            requested = timeout(OPEN_TIMEOUT, exec.request(command)) => requested,
                        };
                        match requested {
                            Ok(Ok(())) => Ok(exec),
                            Ok(Err(error)) => Err(error),
                            Err(_) => Err(ExecError::Timeout),
                        }
                    }
                    Err(error) => Err(error),
                };
                let _ = result.send(opened);
            });
            receiver.await.map_err(|_| ExecError::Closed)?
        };
        tokio::select! {
            biased;
            () = self.closed() => Err(ExecError::Closed),
            () = wait_for_cancel(preempted) => Err(ExecError::Yielded),
            result = timeout(OPEN_TIMEOUT, open) => result.map_err(|_| ExecError::Timeout)?,
        }
    }
}

async fn wait_for_cancel(cancel: Option<CancellationToken>) {
    match cancel {
        Some(cancel) => cancel.cancelled().await,
        None => pending().await,
    }
}

impl ExecChannel {
    fn channel(&self) -> Result<&Channel<client::Msg>, ExecError> {
        self.channel
            .as_ref()
            .map(|(channel, _)| channel)
            .ok_or(ExecError::Closed)
    }

    /// Returns the token canceled when this status channel yields to a user channel.
    pub fn preempted(&self) -> Option<CancellationToken> {
        self.channel
            .as_ref()
            .and_then(|(_, lease)| lease.preempted())
    }

    fn channel_mut(&mut self) -> Result<&mut Channel<client::Msg>, ExecError> {
        self.channel
            .as_mut()
            .map(|(channel, _)| channel)
            .ok_or(ExecError::Closed)
    }

    async fn request(&mut self, command: &'static str) -> Result<(), ExecError> {
        self.channel()?
            .exec(true, command)
            .await
            .map_err(|_| ExecError::Closed)?;
        loop {
            match self.channel_mut()?.wait().await {
                Some(ChannelMsg::Success) => return Ok(()),
                Some(ChannelMsg::Failure) => return Err(ExecError::Rejected),
                Some(ChannelMsg::WindowAdjusted { .. }) => {}
                _ => return Err(ExecError::Closed),
            }
        }
    }

    pub async fn write_stdin(&self, bytes: &[u8]) -> Result<(), ExecError> {
        self.channel()?
            .data(bytes)
            .await
            .map_err(|_| ExecError::Closed)
    }

    /// Reads one stdout line including its newline, within the caller's remaining budget.
    pub async fn read_line(&mut self, limit: usize) -> Result<Option<Vec<u8>>, ExecError> {
        let mut line = Vec::new();
        while !self.eof {
            let remaining = &self.pending[self.offset..];
            if remaining.is_empty() {
                match self.channel_mut()?.wait().await {
                    Some(ChannelMsg::Data { data }) => {
                        if data.len() > MAX_STDOUT_PACKET {
                            return Err(ExecError::Limit);
                        }
                        self.pending = data.to_vec();
                        self.offset = 0;
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        self.exit_status = Some(exit_status)
                    }
                    Some(ChannelMsg::Eof | ChannelMsg::Close) | None => self.eof = true,
                    _ => {}
                }
                continue;
            }
            let newline = remaining.iter().position(|byte| *byte == b'\n');
            let used = newline.map_or(remaining.len(), |index| index + 1);
            if used > limit.saturating_sub(line.len()) {
                return Err(ExecError::Limit);
            }
            line.extend_from_slice(&remaining[..used]);
            self.offset += used;
            if newline.is_some() {
                return Ok(Some(line));
            }
        }
        if line.is_empty() {
            Ok(None)
        } else {
            Err(ExecError::UnexpectedOutput)
        }
    }

    /// While no request is in flight, unsolicited stdout is a protocol failure.
    pub async fn wait_while_idle(&mut self) -> ExecError {
        if self.offset < self.pending.len() {
            return ExecError::UnexpectedOutput;
        }
        loop {
            match self.channel_mut() {
                Ok(channel) => match channel.wait().await {
                    Some(ChannelMsg::Data { data }) if !data.is_empty() => {
                        return ExecError::UnexpectedOutput
                    }
                    Some(
                        ChannelMsg::Eof
                        | ChannelMsg::Close
                        | ChannelMsg::ExitStatus { .. }
                        | ChannelMsg::ExitSignal { .. },
                    )
                    | None => return ExecError::Closed,
                    _ => {}
                },
                Err(error) => return error,
            }
        }
    }

    /// Waits for the remote exit code after stdout has been read to EOF.
    pub async fn exit_status(&mut self) -> Result<Option<u32>, ExecError> {
        if !self.eof {
            return Err(ExecError::UnexpectedOutput);
        }
        while self.exit_status.is_none() {
            match self.channel_mut()?.wait().await {
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    self.exit_status = Some(exit_status)
                }
                Some(ChannelMsg::Close | ChannelMsg::ExitSignal { .. }) | None => break,
                Some(ChannelMsg::Data { .. }) => return Err(ExecError::UnexpectedOutput),
                _ => {}
            }
        }
        Ok(self.exit_status)
    }

    pub async fn close(mut self) {
        if let Some(cleanup) = self.start_close() {
            let _ = cleanup.await;
        }
    }

    fn start_close(&mut self) -> Option<tokio::task::JoinHandle<()>> {
        let (mut channel, lease) = self.channel.take()?;
        let runtime = tokio::runtime::Handle::try_current().ok()?;
        Some(runtime.spawn(async move {
            let closing_lease = lease;
            if closing_lease.status.is_some() {
                let graceful = timeout(STATUS_CLOSE_TIMEOUT, async {
                    // Closing the SSH channel first can leave its running
                    // command charged against OpenSSH's session limit.
                    let _ = channel.signal(Sig::TERM).await;
                    channel.eof().await?;
                    while let Some(message) = channel.wait().await {
                        if matches!(message, ChannelMsg::Close) {
                            break;
                        }
                    }
                    Ok::<_, russh::Error>(())
                })
                .await;
                if matches!(graceful, Ok(Ok(()))) {
                    return;
                }
            }
            let _ = timeout(CLOSE_TIMEOUT, channel.close()).await;
        }))
    }
}

impl Drop for ExecChannel {
    fn drop(&mut self) {
        // The task keeps both gates until cleanup, even if close was canceled.
        drop(self.start_close());
    }
}
