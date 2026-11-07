use super::{frame, Collector, ServerStatusState, StatusError, StatusObservation};
use capstan_ssh::exec::{ExecChannel, ExecError};
use std::{future::Future, time::Duration};
use tokio::{
    sync::watch,
    time::{timeout, Instant},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(3);
const REPLY_TIMEOUT: Duration = Duration::from_secs(8);

enum CollectFailure {
    Yielded,
    Status(StatusError),
}

impl From<StatusError> for CollectFailure {
    fn from(error: StatusError) -> Self {
        Self::Status(error)
    }
}

impl From<ExecError> for StatusError {
    fn from(error: ExecError) -> Self {
        match error {
            ExecError::Yielded => Self::Closed,
            ExecError::Rejected => Self::Rejected,
            ExecError::Limit => Self::Limit,
            ExecError::UnexpectedOutput => Self::Parse,
            ExecError::Closed | ExecError::Timeout => Self::Closed,
        }
    }
}

impl Collector {
    fn publish(&self, observation: StatusObservation, updates: &watch::Sender<()>) {
        self.observation.send_replace(observation);
        updates.send_replace(());
    }

    pub(super) async fn run(
        &self,
        id: Uuid,
        closed: CancellationToken,
        mut target: watch::Receiver<Option<Uuid>>,
        updates: &watch::Sender<()>,
    ) {
        let mut activity = self.activity.subscribe();
        let mut can_retry_automatically = true;
        loop {
            let idle = activity.borrow_and_update().clone();
            if !idle.is_cancelled() {
                self.run_busy(
                    id,
                    &idle,
                    &closed,
                    &mut target,
                    updates,
                    &mut can_retry_automatically,
                )
                .await;
                updates.send_replace(());
            }
            tokio::select! {
                biased;
                () = closed.cancelled() => return,
                changed = activity.changed() => { if changed.is_err() { return; } },
            }
        }
    }

    async fn run_busy(
        &self,
        id: Uuid,
        idle: &CancellationToken,
        closed: &CancellationToken,
        target: &mut watch::Receiver<Option<Uuid>>,
        updates: &watch::Sender<()>,
        can_retry_automatically: &mut bool,
    ) {
        let stop = async {
            tokio::select! { () = idle.cancelled() => {}, () = closed.cancelled() => {} }
        };
        tokio::pin!(stop);
        let mut error = self.observation.borrow().error_code;
        loop {
            if error == Some(StatusError::Unsupported) {
                stop.await;
                return;
            }
            if error.is_some() {
                let automatic_at = Instant::now() + SAMPLE_INTERVAL;
                let automatic = tokio::select! {
                    biased;
                    () = &mut stop => return,
                    () = self.retry.notified() => false,
                    () = async {
                        tokio::time::sleep_until(automatic_at).await;
                        let _ = target.wait_for(|selection| *selection == Some(id)).await;
                    }, if *can_retry_automatically => true,
                };
                if automatic {
                    *can_retry_automatically = false;
                }
                // A foreground handoff can interrupt this attempt before it
                // has an outcome. Continue that attempt without a new retry.
                error = None;
            }
            let mut channel = None;
            let outcome = tokio::select! {
                biased;
                () = &mut stop => None,
                result = self.collect(id, target, &mut channel, updates) => Some(result),
            };
            if let Some(channel) = channel {
                channel.close().await;
            }
            let failure = match outcome {
                None => return,
                Some(Err(CollectFailure::Yielded)) => continue,
                Some(Err(CollectFailure::Status(error))) => error,
                Some(Ok(never)) => match never {},
            };
            let state = if failure == StatusError::Unsupported {
                ServerStatusState::Unsupported
            } else {
                ServerStatusState::Unavailable
            };
            self.publish(StatusObservation::state(state, Some(failure)), updates);
            error = Some(failure);
        }
    }

    async fn collect(
        &self,
        id: Uuid,
        target: &mut watch::Receiver<Option<Uuid>>,
        channel: &mut Option<ExecChannel>,
        updates: &watch::Sender<()>,
    ) -> Result<std::convert::Infallible, CollectFailure> {
        let mut previous: Option<(frame::Sample, Instant)> = None;
        let mut next_tick = Instant::now();
        loop {
            if *target.borrow_and_update() != Some(id) {
                let mut paused = self.observation.borrow().clone();
                paused.state = ServerStatusState::Paused;
                paused.error_code = None;
                self.publish(paused, updates);
                let preempted = channel.as_ref().and_then(ExecChannel::preempted);
                tokio::select! {
                    biased;
                    () = wait_for_preemption(preempted) => return Err(CollectFailure::Yielded),
                    changed = target.changed() => { changed.map_err(|_| StatusError::Closed)?; },
                    error = wait_for_unsolicited_output(channel) => return Err(error.into()),
                }
                previous = None;
                next_tick = Instant::now();
                continue;
            }
            if channel.is_none() {
                self.publish(
                    StatusObservation::state(ServerStatusState::Probing, None),
                    updates,
                );
                let transport = self.transport.upgrade().ok_or(StatusError::Closed)?;
                *channel = Some(transport.open_status_exec("sh").await.map_err(|error| {
                    if error == ExecError::Yielded {
                        CollectFailure::Yielded
                    } else {
                        CollectFailure::Status(error.into())
                    }
                })?);
                drop(transport);
                let exec = channel.as_mut().ok_or(StatusError::Closed)?;
                preemptible(exec.preempted(), async {
                    timeout(REPLY_TIMEOUT, probe(exec))
                        .await
                        .map_err(|_| StatusError::Closed)?
                })
                .await?;
                continue;
            }
            let preempted = channel.as_ref().and_then(ExecChannel::preempted);
            tokio::select! {
                biased;
                () = wait_for_preemption(preempted) => return Err(CollectFailure::Yielded),
                changed = target.changed() => { changed.map_err(|_| StatusError::Closed)?; continue; },
                error = wait_for_unsolicited_output(channel) => return Err(error.into()),
                () = tokio::time::sleep_until(next_tick) => {},
            }
            let started = Instant::now();
            let exec = channel.as_mut().ok_or(StatusError::Closed)?;
            let sample = preemptible(exec.preempted(), async {
                timeout(REPLY_TIMEOUT, frame::sample(exec))
                    .await
                    .map_err(|_| StatusError::Closed)?
            })
            .await?;
            let observation = sample.observation(
                previous
                    .as_ref()
                    .map(|(sample, time)| (sample, started - *time)),
                crate::unix_time_seconds(),
            );
            self.publish(observation, updates);
            previous = Some((sample, started));
            next_tick = started + SAMPLE_INTERVAL;
        }
    }
}

async fn wait_for_preemption(cancel: Option<CancellationToken>) {
    match cancel {
        Some(cancel) => cancel.cancelled().await,
        None => std::future::pending().await,
    }
}

async fn preemptible<T>(
    cancel: Option<CancellationToken>,
    operation: impl Future<Output = Result<T, StatusError>>,
) -> Result<T, CollectFailure> {
    tokio::select! {
        biased;
        () = wait_for_preemption(cancel) => Err(CollectFailure::Yielded),
        result = operation => result.map_err(CollectFailure::Status),
    }
}

pub(super) async fn probe(channel: &mut ExecChannel) -> Result<(), StatusError> {
    channel.write_stdin(frame::SCRIPT.as_bytes()).await?;
    let os = channel.read_line(256).await?.ok_or(StatusError::Closed)?;
    if os == b"Linux\n" {
        return Ok(());
    }
    let name = os.strip_suffix(b"\n").ok_or(StatusError::Parse)?;
    if !name.is_empty()
        && name
            .iter()
            .all(|byte| byte.is_ascii_graphic() || *byte == b' ')
    {
        // A valid non-Linux response is followed by a successful exit. This
        // also distinguishes remote shell startup chatter from an OS name.
        if channel.read_line(256).await?.is_none() && channel.exit_status().await? == Some(0) {
            return Err(StatusError::Unsupported);
        }
    }
    Err(StatusError::Parse)
}

async fn wait_for_unsolicited_output(channel: &mut Option<ExecChannel>) -> StatusError {
    match channel {
        Some(channel) => channel.wait_while_idle().await.into(),
        None => std::future::pending().await,
    }
}
