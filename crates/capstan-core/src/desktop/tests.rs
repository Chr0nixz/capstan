use super::*;

fn task(state: TransferState, processed: u64, total: u64) -> DesktopTransfer {
    let now = Instant::now();
    DesktopTransfer {
        id: Uuid::new_v4(),
        version: 1,
        batch_id: None,
        state,
        processed,
        total,
        attempt_started: now,
        finished: Some(now),
        awaiting_reconnect: false,
    }
}

#[test]
fn taskbar_states_and_current_batch_bytes() {
    use TransferState::{AwaitingConflict, Completed, Failed, Interrupted, Paused, Running};
    let no_ack = HashMap::new();
    let progress =
        |tasks: &[DesktopTransfer], scanning| taskbar_progress(tasks, scanning, false, &no_ack);
    assert_eq!(progress(&[], false), TaskbarProgress::default());
    assert_eq!(progress(&[], true).state, ProgressState::Indeterminate);
    assert_eq!(
        taskbar_progress(&[], false, true, &no_ack).state,
        ProgressState::Error
    );
    let batch_id = Some(Uuid::new_v4());
    let tasks = [
        DesktopTransfer {
            batch_id,
            ..task(Completed, 100, 100)
        },
        DesktopTransfer {
            batch_id,
            ..task(Running, 25, 100)
        },
        task(Running, 25, 100),
        task(Completed, 1_000, 1_000),
    ];
    assert_eq!(
        progress(&tasks, false),
        TaskbarProgress {
            state: ProgressState::Normal,
            percent: 50
        }
    );
    assert_eq!(progress(&tasks, true).state, ProgressState::Indeterminate);
    let waiting = DesktopTransfer {
        awaiting_reconnect: true,
        ..task(Interrupted, 30, 100)
    };
    assert_eq!(
        progress(&[task(Paused, 70, 100), waiting], false),
        TaskbarProgress {
            state: ProgressState::Paused,
            percent: 50
        }
    );
    assert_eq!(
        progress(&[task(Running, 0, 0)], false).state,
        ProgressState::Normal
    );
    assert_eq!(
        progress(&[task(AwaitingConflict, 0, 100)], true).state,
        ProgressState::Error
    );
    let failed = task(Failed, 20, 100);
    assert_eq!(progress(&[failed], false).state, ProgressState::Error);
    assert_eq!(
        taskbar_progress(
            &[failed],
            false,
            false,
            &HashMap::from([(failed.id, failed.version)])
        ),
        TaskbarProgress::default()
    );
    assert_eq!(
        taskbar_progress(
            &[failed],
            false,
            false,
            &HashMap::from([(failed.id, failed.version + 1)])
        )
        .state,
        ProgressState::Error
    );
    assert_eq!(
        progress(
            &[
                task(Running, u64::MAX, u64::MAX),
                task(Running, u64::MAX, u64::MAX)
            ],
            false
        )
        .percent,
        100
    );
}

#[test]
fn transfer_notice_uses_exact_batch_size_and_elapsed_time_boundaries() {
    use TransferState::{Canceled, Completed, Failed, Running};
    let mut tracker = NoticeTracker::default();
    let size = u64::try_from(MIN_TRANSFER_BYTES).unwrap();
    let batch = Some(Uuid::new_v4());
    let started = Instant::now();
    let finished = started + Duration::from_secs(1);
    let mut first = DesktopTransfer {
        batch_id: batch,
        attempt_started: started,
        finished: Some(finished),
        ..task(Completed, size / 2, size / 2)
    };
    let mut second = DesktopTransfer {
        batch_id: batch,
        attempt_started: started,
        finished: None,
        ..task(Running, 0, size / 2)
    };
    assert!(tracker.transfers(&[first, second]).is_empty());
    second.state = Failed;
    second.finished = Some(finished);
    assert_eq!(
        tracker.transfers(&[first, second]),
        [TransferNotice {
            kind: Notice::TransferFailed,
            finished,
        }]
    );
    assert!(tracker.transfers(&[first, second]).is_empty());
    first = task(Completed, size - 1, size - 1);
    first.finished = Some(first.attempt_started + MIN_TRANSFER_DURATION - Duration::from_nanos(1));
    assert!(tracker.transfers(&[first]).is_empty());
    first.finished = Some(first.attempt_started + MIN_TRANSFER_DURATION);
    let mut tracker = NoticeTracker::default();
    assert_eq!(
        tracker.transfers(&[first]),
        [TransferNotice {
            kind: Notice::TransferCompleted,
            finished: first.finished.unwrap()
        }]
    );
    first.attempt_started += Duration::from_secs(60);
    first.finished = Some(first.attempt_started + MIN_TRANSFER_DURATION);
    assert_eq!(
        tracker.transfers(&[first]),
        [TransferNotice {
            kind: Notice::TransferCompleted,
            finished: first.finished.unwrap()
        }]
    );
    assert!(tracker.transfers(&[task(Canceled, 0, size)]).is_empty());
}

#[test]
fn fast_folder_retry_ignores_earlier_time_bytes_and_failures() {
    let start = Instant::now();
    let initial_end = start + Duration::from_secs(1);
    let batch_id = Some(Uuid::new_v4());
    let size = u64::try_from(MIN_TRANSFER_BYTES).unwrap();
    let completed = DesktopTransfer {
        batch_id,
        attempt_started: start,
        finished: Some(initial_end),
        ..task(TransferState::Completed, size - 1, size - 1)
    };
    let failed = DesktopTransfer {
        id: Uuid::new_v4(),
        state: TransferState::Failed,
        processed: 0,
        total: 1,
        ..completed
    };
    let mut tracker = NoticeTracker::default();
    assert_eq!(
        tracker.transfers(&[completed, failed]),
        [TransferNotice {
            kind: Notice::TransferFailed,
            finished: initial_end,
        }]
    );

    let retry_start = start + Duration::from_secs(60);
    let retry = DesktopTransfer {
        state: TransferState::Completed,
        processed: 1,
        attempt_started: retry_start,
        finished: Some(retry_start + Duration::from_secs(1)),
        ..failed
    };
    assert!(tracker.transfers(&[completed, retry]).is_empty());

    let large_retry = DesktopTransfer {
        total: size,
        processed: size,
        ..retry
    };
    let old_large_member = DesktopTransfer {
        total: size,
        processed: size,
        ..completed
    };
    let mut tracker = NoticeTracker::default();
    assert!(tracker.transfers(&[old_large_member, retry]).is_empty());
    assert!(tracker.transfers(&[old_large_member]).is_empty());

    let old_failure = DesktopTransfer {
        id: Uuid::new_v4(),
        ..failed
    };
    let mut tracker = NoticeTracker::default();
    assert_eq!(
        tracker.transfers(&[old_failure, large_retry]),
        [TransferNotice {
            kind: Notice::TransferCompleted,
            finished: retry.finished.unwrap(),
        }]
    );
}

#[test]
fn overlapping_folder_retries_share_a_round() {
    let start = Instant::now();
    let batch_id = Some(Uuid::new_v4());
    let half = u64::try_from(MIN_TRANSFER_BYTES).unwrap() / 2;
    let earlier = DesktopTransfer {
        batch_id,
        attempt_started: start,
        finished: Some(start + Duration::from_secs(2)),
        ..task(TransferState::Completed, half, half)
    };
    let later = DesktopTransfer {
        batch_id,
        attempt_started: start + Duration::from_secs(1),
        finished: Some(start + Duration::from_secs(3)),
        ..task(TransferState::Completed, half, half)
    };
    assert_eq!(
        NoticeTracker::default().transfers(&[later, earlier]),
        [TransferNotice {
            kind: Notice::TransferCompleted,
            finished: later.finished.unwrap(),
        }]
    );
}

#[test]
fn disconnect_is_generation_scoped_and_excludes_intentional_changes() {
    let id = Uuid::new_v4();
    let snapshot = |generation, state, error_code| ConnectionSnapshot {
        connection_id: id,
        generation,
        state,
        error_code,
    };
    let mut tracker = NoticeTracker::default();
    let online = |generation| snapshot(generation, ConnectionState::Online, None);
    let lost = |generation| {
        snapshot(
            generation,
            ConnectionState::Reconnecting,
            Some(ConnectionErrorCode::ConnectionLost),
        )
    };
    assert_eq!(tracker.connection(lost(1)), None);
    assert_eq!(tracker.connection(online(1)), None);
    assert_eq!(tracker.connection(lost(1)), Some(Notice::ConnectionLost));
    assert_eq!(tracker.connection(online(1)), None);
    assert_eq!(tracker.connection(lost(1)), None);
    tracker.connection(online(2));
    assert_eq!(
        tracker.connection(snapshot(
            2,
            ConnectionState::Failed,
            Some(ConnectionErrorCode::VaultLocked)
        )),
        None
    );
    tracker.connection(online(3));
    assert_eq!(
        tracker.connection(snapshot(
            3,
            ConnectionState::Failed,
            Some(ConnectionErrorCode::Canceled)
        )),
        None
    );
    tracker.connection(online(4));
    assert_eq!(
        tracker.connection(snapshot(4, ConnectionState::Disconnected, None)),
        None
    );
    tracker.connection(online(5));
    assert_eq!(tracker.connection(lost(4)), None);
    assert_eq!(
        tracker.connection(snapshot(4, ConnectionState::Disconnected, None)),
        None
    );
    assert_eq!(tracker.connection(lost(5)), Some(Notice::ConnectionLost));
}

#[test]
fn background_notices_coalesce_and_are_discarded_when_suppressed() {
    let mut queue = NoticeQueue::default();
    let now = Instant::now();
    queue.push(Notice::TransferFailed, now, false);
    queue.push(Notice::TransferCompleted, now, true);
    queue.push(Notice::ConnectionLost, now + Duration::from_secs(9), true);
    assert_eq!(queue.take_due(now + Duration::from_secs(9), true), None);
    assert_eq!(
        queue.take_due(now + NOTICE_WINDOW, true),
        Some(NoticeSummary {
            completed: 1,
            failed: 0,
            disconnected: 1
        })
    );
    queue.push(Notice::TransferFailed, now, true);
    assert_eq!(queue.take_due(now, false), None);
    assert_eq!(queue.take_due(now + NOTICE_WINDOW, true), None);
}
