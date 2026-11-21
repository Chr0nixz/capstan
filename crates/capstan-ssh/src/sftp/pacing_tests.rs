use super::*;
use std::{cmp::Reverse, collections::BinaryHeap};

const FILES: usize = 3;
const FILE_REQUESTS: usize = 64;

/// A FIFO bottleneck of `rate` bytes per second plus a fixed one-way delay,
/// with three files that keep their request pipelines as full as allowed.
struct Simulation {
    start: Instant,
    now: Instant,
    controller: Controller,
    rate: f64,
    one_way: Duration,
    link_free: Instant,
    replies: BinaryHeap<Reverse<(Instant, u64, usize, Instant)>>,
    per_file: [usize; FILES],
    delivered: Vec<(Instant, u64)>,
    /// How long a keystroke sent at each reply would wait behind file data.
    keystroke_waits: Vec<(Instant, Duration)>,
}

impl Simulation {
    fn new(bytes_per_second: u64, one_way: Duration) -> Self {
        let start = Instant::now();
        Self {
            start,
            now: start,
            controller: Controller::new(),
            rate: bytes_per_second as f64,
            one_way,
            link_free: start,
            replies: BinaryHeap::new(),
            per_file: [0; FILES],
            delivered: Vec::new(),
            keystroke_waits: Vec::new(),
        }
    }

    fn fill(&mut self) {
        loop {
            let mut admitted = false;
            for file in 0..FILES {
                if self.per_file[file] == FILE_REQUESTS {
                    continue;
                }
                let only = self.per_file[file] == 0;
                let Some(token) = self.controller.admit(self.now, REQUEST, only) else {
                    continue;
                };
                let departs = self.link_free.max(self.now)
                    + Duration::from_secs_f64(REQUEST as f64 / self.rate);
                self.link_free = departs;
                self.replies
                    .push(Reverse((departs + 2 * self.one_way, token, file, self.now)));
                self.per_file[file] += 1;
                admitted = true;
            }
            if !admitted {
                return;
            }
        }
    }

    fn run_for(&mut self, duration: Duration) {
        let end = self.now + duration;
        self.fill();
        while let Some(Reverse((at, token, file, admitted))) = self.replies.pop() {
            if at > end {
                self.replies.push(Reverse((at, token, file, admitted)));
                break;
            }
            self.now = at;
            self.controller
                .complete(at, token, REQUEST, Some(at - admitted));
            self.per_file[file] -= 1;
            self.delivered.push((at, REQUEST));
            self.keystroke_waits
                .push((at, self.link_free.saturating_duration_since(at)));
            self.fill();
        }
        self.now = end;
    }

    fn throughput(&self, from: Duration, to: Duration) -> f64 {
        let (from, to) = (self.start + from, self.start + to);
        let bytes: u64 = self
            .delivered
            .iter()
            .filter(|(at, _)| *at >= from && *at < to)
            .map(|(_, bytes)| bytes)
            .sum();
        bytes as f64 / (to - from).as_secs_f64() / self.rate
    }

    /// One request occupies a slow link for a while, and each file always
    /// keeps one, so the queue cannot be finer than that.
    fn granularity(&self) -> Duration {
        Duration::from_secs_f64(FILES as f64 * REQUEST as f64 / self.rate)
    }

    fn worst_wait(&self, from: Duration) -> Duration {
        let from = self.start + from;
        self.keystroke_waits
            .iter()
            .filter(|(at, _)| *at >= from)
            .map(|(_, wait)| *wait)
            .max()
            .unwrap_or_default()
    }
}

#[test]
fn a_slow_link_keeps_keystrokes_near_the_queue_target_and_stays_full() {
    // Without pacing, three 64-request pipelines queue 6 MiB: six seconds.
    // The run spans two base round-trip probes.
    let mut link = Simulation::new(1024 * 1024, Duration::from_millis(40));
    link.run_for(Duration::from_secs(70));
    assert!(link.worst_wait(Duration::from_secs(1)) <= QUEUE_TARGET + link.granularity());
    assert!(link.throughput(Duration::from_secs(1), Duration::from_secs(70)) > 0.95);
}

#[test]
fn a_very_slow_link_is_limited_by_request_granularity_only() {
    // 24 seconds of queue without pacing.
    let mut link = Simulation::new(256 * 1024, Duration::from_millis(40));
    link.run_for(Duration::from_secs(40));
    assert!(link.worst_wait(Duration::from_secs(1)) <= QUEUE_TARGET + 2 * link.granularity());
    assert!(link.throughput(Duration::from_secs(1), Duration::from_secs(40)) > 0.95);
}

#[test]
fn a_fast_long_link_still_fills_after_slow_start() {
    let mut link = Simulation::new(50 * 1024 * 1024, Duration::from_millis(50));
    link.run_for(Duration::from_secs(70));
    assert!(link.throughput(Duration::from_secs(1), Duration::from_secs(70)) > 0.95);
    assert!(link.worst_wait(Duration::from_secs(1)) <= QUEUE_TARGET * 5 / 4);
}

#[test]
fn a_local_network_still_fills() {
    let mut link = Simulation::new(110 * 1024 * 1024, Duration::from_micros(250));
    link.run_for(Duration::from_secs(70));
    assert!(link.throughput(Duration::from_secs(1), Duration::from_secs(70)) > 0.95);
    assert!(link.worst_wait(Duration::from_secs(1)) <= QUEUE_TARGET * 5 / 4);
}

#[test]
fn a_longer_route_is_learned_within_the_base_history() {
    let mut link = Simulation::new(2 * 1024 * 1024, Duration::from_millis(20));
    link.run_for(Duration::from_secs(20));
    link.one_way = Duration::from_millis(120);
    link.run_for(BASE_HISTORY + 2 * PROBE_INTERVAL);
    let end = link.now - link.start;
    assert!(link.throughput(end - PROBE_INTERVAL, end) > 0.9);
}

#[test]
fn dropped_requests_return_their_bytes_without_a_sample() {
    let start = Instant::now();
    let mut controller = Controller::new();
    let tokens = (0..INITIAL_WINDOW / REQUEST)
        .map(|_| controller.admit(start, REQUEST, false).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(controller.admit(start, REQUEST, false), None);
    for token in tokens {
        controller.complete(start, token, REQUEST, None);
    }
    assert_eq!(controller.outstanding(), 0);
    assert_eq!(controller.window(), INITIAL_WINDOW);
}

#[test]
fn a_files_only_request_is_admitted_beyond_the_window() {
    let start = Instant::now();
    let mut controller = Controller::new();
    while controller.admit(start, REQUEST, false).is_some() {}
    assert!(controller.admit(start, REQUEST, true).is_some());
    assert_eq!(controller.outstanding(), INITIAL_WINDOW + REQUEST);
}

#[test]
fn an_idle_transport_starts_again_from_slow_start() {
    let mut link = Simulation::new(50 * 1024 * 1024, Duration::from_millis(50));
    link.run_for(Duration::from_secs(5));
    // Every request replies and nothing new is issued.
    let mut now = link.now;
    while let Some(Reverse((at, token, _, admitted))) = link.replies.pop() {
        now = at;
        link.controller
            .complete(at, token, REQUEST, Some(at - admitted));
    }
    let grown = link.controller.window();
    assert!(grown > INITIAL_WINDOW);
    assert_eq!(link.controller.outstanding(), 0);
    // A short pause keeps what was learned.
    let soon = now + IDLE_RESTART / 2;
    let token = link.controller.admit(soon, REQUEST, true).unwrap();
    assert_eq!(link.controller.window(), grown);
    link.controller.complete(soon, token, REQUEST, None);
    link.controller
        .admit(soon + IDLE_RESTART, REQUEST, true)
        .unwrap();
    assert_eq!(link.controller.window(), INITIAL_WINDOW);
}
