//! Delay-based admission for SFTP file data on one SSH connection.
//!
//! Shell keystrokes and their echo travel in the same TCP stream as file
//! data. File data queued in socket buffers or the network ahead of a
//! keystroke delays it by its size divided by the link rate, so a fixed
//! window of several MiB stalls typing for seconds on a slow link. This
//! window follows the link instead: it lets file data queue for about
//! [`QUEUE_TARGET`] beyond the fastest recent round trip, which keeps a
//! fast link full and a slow link responsive.
use russh_sftp::client::Pacer;
use std::{
    collections::VecDeque,
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

const REQUEST: u64 = super::wire::MAX_DATA_BYTES as u64;
/// Queueing delay that file data may add to a keystroke or its echo.
pub(crate) const QUEUE_TARGET: Duration = Duration::from_millis(50);
const MIN_WINDOW: u64 = 2 * REQUEST;
const INITIAL_WINDOW: u64 = 4 * REQUEST;
/// Above every static request limit, so only the delay target binds.
const MAX_WINDOW: u64 = 16 * 1024 * 1024;
/// How often the window drops for one round to measure an unqueued round trip.
const PROBE_INTERVAL: Duration = Duration::from_secs(60);
/// Minima older than this are forgotten, so a longer route is learned. It
/// always spans at least two probes.
const BASE_HISTORY: Duration = Duration::from_secs(150);
/// After this long with no file data in flight, start again from slow start.
const IDLE_RESTART: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Doubling each round up to `ceiling` or until requests start to queue.
    SlowStart { ceiling: u64 },
    /// Holding the queue near the target.
    Steady,
    /// One round at the minimum window; the window is restored afterwards.
    Probe { saved: u64 },
}

#[derive(Debug)]
pub(crate) struct Controller {
    window: u64,
    outstanding: u64,
    phase: Phase,
    next_token: u64,
    /// The round ends when a request admitted at or after this token replies.
    round_end: u64,
    /// Sum and count of this round's round trips. The mean, not the
    /// minimum: files refill in bursts, so a round's first replies can be
    /// unqueued while later ones wait behind the whole burst.
    round_total: Duration,
    round_samples: u32,
    round_peak: u64,
    /// Skip one adjustment: that round still drains or refills earlier data.
    hold: bool,
    /// Minimum round trip per `PROBE_INTERVAL` bucket, oldest first.
    base: VecDeque<(Instant, Duration)>,
    next_probe: Option<Instant>,
    idle_since: Option<Instant>,
}

impl Controller {
    pub(crate) fn new() -> Self {
        Self {
            window: INITIAL_WINDOW,
            outstanding: 0,
            phase: Phase::SlowStart {
                ceiling: MAX_WINDOW,
            },
            next_token: 0,
            round_end: 0,
            round_total: Duration::ZERO,
            round_samples: 0,
            round_peak: 0,
            hold: false,
            base: VecDeque::new(),
            next_probe: None,
            idle_since: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn window(&self) -> u64 {
        self.window
    }

    #[cfg(test)]
    pub(crate) fn outstanding(&self) -> u64 {
        self.outstanding
    }

    pub(crate) fn admit(&mut self, now: Instant, bytes: u64, force: bool) -> Option<u64> {
        if let Some(idle) = self.idle_since {
            if now.saturating_duration_since(idle) >= IDLE_RESTART {
                self.restart(now, idle);
            }
        }
        let limit = match self.phase {
            Phase::Probe { .. } => MIN_WINDOW,
            Phase::SlowStart { .. } | Phase::Steady => self.window,
        };
        if !force && self.outstanding.saturating_add(bytes) > limit {
            return None;
        }
        self.outstanding = self.outstanding.saturating_add(bytes);
        self.round_peak = self.round_peak.max(self.outstanding);
        self.idle_since = None;
        let token = self.next_token;
        self.next_token = self.next_token.saturating_add(1);
        Some(token)
    }

    pub(crate) fn complete(
        &mut self,
        now: Instant,
        token: u64,
        bytes: u64,
        round_trip: Option<Duration>,
    ) {
        self.outstanding = self.outstanding.saturating_sub(bytes);
        if self.outstanding == 0 {
            self.idle_since = Some(now);
        }
        // Dropped, failed and timed-out requests return their bytes only.
        let Some(round_trip) = round_trip else {
            return;
        };
        self.remember_base(now, round_trip);
        if matches!(self.phase, Phase::SlowStart { .. })
            && self.leave_slow_start(now, round_trip, bytes)
        {
            return;
        }
        self.round_total = self.round_total.saturating_add(round_trip);
        self.round_samples = self.round_samples.saturating_add(1);
        if token >= self.round_end {
            self.end_round(now);
        }
    }

    /// Doubling overshoots by up to a window, so slow start ends at the first
    /// queued reply rather than at the end of its round. The window is cut to
    /// what this reply's rate needs and the queue already built drains.
    fn leave_slow_start(&mut self, now: Instant, round_trip: Duration, bytes: u64) -> bool {
        let Some(base) = self.base_rtt() else {
            return false;
        };
        if round_trip.saturating_sub(base) < QUEUE_TARGET {
            return false;
        }
        let in_flight = self.outstanding.saturating_add(bytes);
        let target = (base + QUEUE_TARGET).as_nanos();
        let needed = u64::try_from(u128::from(in_flight) * target / round_trip.as_nanos().max(1))
            .unwrap_or(MAX_WINDOW);
        self.window = needed.clamp(MIN_WINDOW, self.window.max(MIN_WINDOW));
        self.enter_steady(now);
        trace("slow_start_end", base, round_trip, self.window);
        true
    }

    fn enter_steady(&mut self, now: Instant) {
        self.phase = Phase::Steady;
        self.next_probe = Some(now + PROBE_INTERVAL);
        self.round_end = self.next_token;
        self.round_total = Duration::ZERO;
        self.round_samples = 0;
        self.round_peak = self.outstanding;
        self.hold = true;
    }

    fn restart(&mut self, now: Instant, idle: Instant) {
        self.window = INITIAL_WINDOW;
        self.phase = Phase::SlowStart {
            ceiling: MAX_WINDOW,
        };
        self.next_probe = None;
        self.round_end = self.next_token;
        self.round_total = Duration::ZERO;
        self.round_samples = 0;
        self.round_peak = 0;
        self.hold = false;
        if now.saturating_duration_since(idle) >= PROBE_INTERVAL {
            // The route may have changed while idle; the next reply is unqueued.
            self.base.clear();
        }
    }

    fn remember_base(&mut self, now: Instant, round_trip: Duration) {
        match self.base.back_mut() {
            Some((start, min)) if now.saturating_duration_since(*start) < PROBE_INTERVAL => {
                *min = (*min).min(round_trip);
            }
            _ => self.base.push_back((now, round_trip)),
        }
        while self
            .base
            .front()
            .is_some_and(|(start, _)| now.saturating_duration_since(*start) > BASE_HISTORY)
        {
            self.base.pop_front();
        }
    }

    fn base_rtt(&self) -> Option<Duration> {
        self.base.iter().map(|(_, min)| *min).min()
    }

    fn end_round(&mut self, now: Instant) {
        let sample = (self.round_samples > 0).then(|| self.round_total / self.round_samples);
        self.round_total = Duration::ZERO;
        self.round_samples = 0;
        let peak = std::mem::replace(&mut self.round_peak, self.outstanding);
        self.round_end = self.next_token;
        let (Some(sample), Some(base)) = (sample, self.base_rtt()) else {
            return;
        };
        let queue = sample.saturating_sub(base);
        // Only a window that was nearly full has shown what the link can take.
        let used = peak.saturating_add(REQUEST) >= self.window;
        if std::mem::take(&mut self.hold) {
            return;
        }
        match self.phase {
            Phase::Probe { saved } => {
                trace("probe", base, sample, saved);
                // The pipe is empty now; refilling it with the whole window
                // at once would queue that burst ahead of the next keystroke.
                self.window = (2 * MIN_WINDOW).min(saved);
                self.phase = Phase::SlowStart { ceiling: saved };
            }
            Phase::SlowStart { .. } if queue >= QUEUE_TARGET / 2 => {
                // Close to the target: grow gently from here.
                self.enter_steady(now);
                self.hold = false;
                trace("slow_start_end", base, sample, self.window);
            }
            Phase::SlowStart { ceiling } => {
                if used {
                    self.window = self.window.saturating_mul(2).min(ceiling);
                }
                if self.window >= ceiling {
                    self.enter_steady(now);
                }
            }
            Phase::Steady if self.next_probe.is_some_and(|probe| now >= probe) => {
                self.phase = Phase::Probe { saved: self.window };
            }
            Phase::Steady => self.adjust(base, queue, peak, used),
        }
    }

    /// The round delivered `peak` bytes per `base + queue`; that rate with
    /// only `QUEUE_TARGET` of queue needs `estimate` bytes in flight. Files'
    /// first requests can exceed the window, so the window moves by the
    /// difference instead of to the estimate: at most a quarter up or a half
    /// down per round.
    fn adjust(&mut self, base: Duration, queue: Duration, peak: u64, used: bool) {
        let target = (base + QUEUE_TARGET).as_nanos();
        let observed = (base + queue).as_nanos().max(1);
        let estimate = u64::try_from(u128::from(peak) * target / observed).unwrap_or(MAX_WINDOW);
        if queue > QUEUE_TARGET {
            let excess = peak.saturating_sub(estimate).min(self.window / 2);
            self.window = self.window.saturating_sub(excess).max(MIN_WINDOW);
        } else if used {
            let room = estimate.saturating_sub(peak).min(self.window / 4);
            self.window = self.window.saturating_add(room).min(MAX_WINDOW);
        }
    }
}

/// At most about once a minute per busy connection: enough to see what the
/// controller learned about a real link.
fn trace(phase: &'static str, base: Duration, round_trip: Duration, window: u64) {
    capstan_diagnostics::event("sftp_pacing")
        .code("phase", phase)
        .millis("base_rtt_ms", base)
        .millis("round_trip_ms", round_trip)
        .number("window_kib", window / 1024)
        .record();
}

/// One controller shared by every SFTP subsystem on a transport.
pub(crate) struct TransportPacer(Mutex<Controller>);

impl TransportPacer {
    pub(crate) fn new() -> Self {
        Self(Mutex::new(Controller::new()))
    }
}

impl Pacer for TransportPacer {
    fn admit(&self, bytes: u32, force: bool) -> Option<u64> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).admit(
            Instant::now(),
            u64::from(bytes),
            force,
        )
    }
    fn complete(&self, token: u64, bytes: u32, round_trip: Option<Duration>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .complete(Instant::now(), token, u64::from(bytes), round_trip);
    }
}

#[cfg(test)]
#[path = "pacing_tests.rs"]
mod tests;
