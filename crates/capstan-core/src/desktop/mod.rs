//! Desktop policy over a privacy-minimized transfer and connection view.
use crate::transfer::TransferState;
use capstan_ssh::connection_state::{ConnectionErrorCode, ConnectionSnapshot, ConnectionState};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};
use uuid::Uuid;

#[cfg(test)]
mod tests;

pub const NOTICE_WINDOW: Duration = Duration::from_secs(10);
const MIN_TRANSFER_BYTES: u128 = 100 * 1024 * 1024;
const MIN_TRANSFER_DURATION: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug)]
pub struct DesktopTransfer {
    pub id: Uuid,
    pub version: u64,
    pub batch_id: Option<Uuid>,
    pub state: TransferState,
    pub processed: u64,
    pub total: u64,
    pub attempt_started: Instant,
    pub finished: Option<Instant>,
    pub awaiting_reconnect: bool,
}

impl DesktopTransfer {
    #[must_use]
    pub fn failed(self) -> bool {
        self.state == TransferState::Failed
            || (self.state == TransferState::Interrupted && !self.awaiting_reconnect)
    }

    fn active(self) -> bool {
        self.awaiting_reconnect
            || !matches!(
                self.state,
                TransferState::Completed
                    | TransferState::Canceled
                    | TransferState::Interrupted
                    | TransferState::Failed
            )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProgressState {
    #[default]
    None,
    Normal,
    Indeterminate,
    Paused,
    Error,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaskbarProgress {
    pub state: ProgressState,
    pub percent: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum BatchKey {
    Folder(Uuid),
    File(Uuid),
}

fn batches(tasks: &[DesktopTransfer]) -> HashMap<BatchKey, Vec<&DesktopTransfer>> {
    let mut batches = HashMap::<_, Vec<_>>::new();
    for task in tasks {
        let key = task
            .batch_id
            .map_or(BatchKey::File(task.id), BatchKey::Folder);
        batches.entry(key).or_default().push(task);
    }
    batches
}

/// Completed members contribute to their own active batch. Terminal history
/// from older, unrelated batches does not affect the progress percentage.
#[must_use]
pub fn taskbar_progress(
    tasks: &[DesktopTransfer],
    scanning: bool,
    has_tree_conflict: bool,
    acknowledged_failures: &HashMap<Uuid, u64>,
) -> TaskbarProgress {
    let attention = |task: &DesktopTransfer| {
        task.state == TransferState::AwaitingConflict
            || (task.failed() && acknowledged_failures.get(&task.id) != Some(&task.version))
    };
    let mut error = has_tree_conflict;
    let mut active = false;
    let mut paused = true;
    let mut processed = 0_u128;
    let mut total = 0_u128;
    for batch in batches(tasks).into_values() {
        if !batch.iter().any(|task| task.active() || attention(task)) {
            continue;
        }
        for task in batch {
            error |= attention(task);
            if task.active() {
                active = true;
                paused &= task.awaiting_reconnect || task.state == TransferState::Paused;
            }
            if task.state != TransferState::Canceled {
                processed += u128::from(task.processed.min(task.total));
                total += u128::from(task.total);
            }
        }
    }
    let state = if error {
        ProgressState::Error
    } else if scanning {
        ProgressState::Indeterminate
    } else if !active {
        ProgressState::None
    } else if paused {
        ProgressState::Paused
    } else {
        ProgressState::Normal
    };
    let percent = if matches!(state, ProgressState::None | ProgressState::Indeterminate) {
        None
    } else {
        (processed * 100)
            .checked_div(total)
            .and_then(|value| u8::try_from(value).ok())
    };
    TaskbarProgress {
        state,
        percent: percent.unwrap_or(0),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    TransferCompleted,
    TransferFailed,
    ConnectionLost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferNotice {
    pub kind: Notice,
    pub finished: Instant,
}

struct TransferRound {
    started: Instant,
    latest_start: Instant,
    finished: Instant,
    bytes: u128,
    failed: bool,
}

impl TransferRound {
    fn notice(&self) -> Option<TransferNotice> {
        let elapsed = self.finished.saturating_duration_since(self.started);
        (self.bytes >= MIN_TRANSFER_BYTES || elapsed >= MIN_TRANSFER_DURATION).then_some(
            TransferNotice {
                kind: if self.failed {
                    Notice::TransferFailed
                } else {
                    Notice::TransferCompleted
                },
                finished: self.finished,
            },
        )
    }
}

fn latest_transfer_round(batch: &mut [&DesktopTransfer]) -> Option<TransferRound> {
    if batch.iter().any(|task| task.active()) {
        return None;
    }
    batch.sort_unstable_by_key(|task| task.attempt_started);
    let mut latest = None::<TransferRound>;
    for task in batch
        .iter()
        .filter(|task| task.state != TransferState::Canceled)
    {
        let Some(finished) = task.finished else {
            continue;
        };
        // Members enqueued together and overlapping retries share a round.
        // A retry started after the previous round ended excludes its idle gap and results.
        if let Some(round) = latest
            .as_mut()
            .filter(|round| task.attempt_started <= round.finished)
        {
            round.latest_start = task.attempt_started;
            round.finished = round.finished.max(finished);
            round.bytes += u128::from(task.total);
            round.failed |= task.failed();
        } else {
            latest = Some(TransferRound {
                started: task.attempt_started,
                latest_start: task.attempt_started,
                finished,
                bytes: u128::from(task.total),
                failed: task.failed(),
            });
        }
    }
    latest
}

#[must_use]
pub fn unexpected_disconnect(previous: &ConnectionSnapshot, current: &ConnectionSnapshot) -> bool {
    previous.connection_id == current.connection_id
        && previous.generation == current.generation
        && previous.state == ConnectionState::Online
        && matches!(
            current.state,
            ConnectionState::Reconnecting | ConnectionState::Failed
        )
        && !matches!(
            current.error_code,
            Some(ConnectionErrorCode::VaultLocked | ConnectionErrorCode::Canceled)
        )
}

/// Keep the latest attempt and generation per live item even while notifications
/// are suppressed. A foreground event can never be replayed in the background.
#[derive(Default)]
pub struct NoticeTracker {
    transfers: HashMap<BatchKey, Instant>,
    connections: HashMap<Uuid, (ConnectionSnapshot, Option<u64>)>,
}

impl NoticeTracker {
    pub fn transfers(&mut self, tasks: &[DesktopTransfer]) -> Vec<TransferNotice> {
        let batches = batches(tasks);
        self.transfers.retain(|key, _| batches.contains_key(key));
        let mut notices = Vec::new();
        for (key, mut batch) in batches {
            let Some(round) = latest_transfer_round(&mut batch) else {
                continue;
            };
            if self
                .transfers
                .get(&key)
                .is_none_or(|seen| round.latest_start > *seen)
            {
                self.transfers.insert(key, round.latest_start);
                notices.extend(round.notice());
            }
        }
        notices
    }

    pub fn connection(&mut self, current: ConnectionSnapshot) -> Option<Notice> {
        let id = current.connection_id;
        if self
            .connections
            .get(&id)
            .is_some_and(|(previous, _)| current.generation < previous.generation)
        {
            return None;
        }
        if current.state == ConnectionState::Disconnected {
            self.connections.remove(&id);
            return None;
        }
        if current.state != ConnectionState::Online && !self.connections.contains_key(&id) {
            return None;
        }
        let (previous, notified) = self
            .connections
            .entry(id)
            .or_insert_with(|| (current.clone(), None));
        let notify =
            *notified != Some(current.generation) && unexpected_disconnect(previous, &current);
        if notify {
            *notified = Some(current.generation);
        }
        *previous = current;
        notify.then_some(Notice::ConnectionLost)
    }

    pub fn retain_connections(&mut self, ids: &HashSet<Uuid>) {
        self.connections.retain(|id, _| ids.contains(id));
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoticeSummary {
    pub completed: u32,
    pub failed: u32,
    pub disconnected: u32,
}

#[derive(Default)]
pub struct NoticeQueue {
    pending: Option<(Instant, NoticeSummary)>,
}

impl NoticeQueue {
    #[must_use]
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.map(|(deadline, _)| deadline)
    }

    pub fn push(&mut self, notice: Notice, now: Instant, allowed: bool) {
        if !allowed {
            return;
        }
        let (_, summary) = self
            .pending
            .get_or_insert_with(|| (now + NOTICE_WINDOW, NoticeSummary::default()));
        let count = match notice {
            Notice::TransferCompleted => &mut summary.completed,
            Notice::TransferFailed => &mut summary.failed,
            Notice::ConnectionLost => &mut summary.disconnected,
        };
        *count = count.saturating_add(1);
    }

    /// Check permission again before delivery and drop pending notifications
    /// as soon as the app returns to the foreground or notifications are disabled.
    pub fn take_due(&mut self, now: Instant, allowed: bool) -> Option<NoticeSummary> {
        if !allowed {
            self.pending = None;
        }
        if self.pending.is_some_and(|(deadline, _)| now >= deadline) {
            return self.pending.take().map(|(_, summary)| summary);
        }
        None
    }
}
