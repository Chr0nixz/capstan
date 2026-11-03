use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch, Notify};

use crate::{Result, TerminalError};

pub const FLUSH_INTERVAL: Duration = Duration::from_millis(16);
pub const MAX_FRAME_SIZE: usize = 64 * 1024;
pub const ACK_SUSPEND_THRESHOLD: usize = 1024 * 1024;
pub const ACK_RESUME_THRESHOLD: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryFrame {
    pub sequence: u64,
    pub payload: Vec<u8>,
}

impl BinaryFrame {
    pub fn len(&self) -> usize {
        self.payload.len()
    }

    pub fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }
}

/// An application-owned sink can adapt binary frames to a Tauri Channel.  The
/// trait uses `Vec<u8>` as the payload and has no JSON representation.
pub trait FrameSink: Send + Sync + 'static {
    fn send<'a>(
        &'a self,
        frame: BinaryFrame,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
}

#[derive(Debug, Clone, Copy)]
pub struct OutputConfig {
    pub input_capacity: usize,
    pub frame_capacity: usize,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            input_capacity: 128,
            frame_capacity: 128,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputMetrics {
    pub queued_bytes: usize,
    pub in_flight_bytes: usize,
    pub max_outstanding_bytes: usize,
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub frame_count: u64,
    pub max_pending_bytes: usize,
    pub max_in_flight_bytes: usize,
    pub pause_count: u64,
    pub resume_count: u64,
    pub canceled: bool,
    pub channel_closed: bool,
}

/// Collects small PTY reads into binary frames.  A frame is emitted as soon as
/// it reaches 64 KiB; otherwise the pump flushes it after 16 ms.
#[derive(Debug)]
pub struct OutputMerger {
    pending: Vec<u8>,
    first_pending_at: Option<Instant>,
    next_sequence: u64,
}

impl Default for OutputMerger {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputMerger {
    pub fn new() -> Self {
        Self {
            pending: Vec::with_capacity(MAX_FRAME_SIZE),
            first_pending_at: None,
            next_sequence: 0,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<BinaryFrame> {
        if bytes.is_empty() {
            return Vec::new();
        }
        let mut frames = Vec::new();
        let mut offset = 0;
        while offset < bytes.len() {
            if self.pending.is_empty() {
                self.first_pending_at = Some(Instant::now());
            }
            let available = MAX_FRAME_SIZE - self.pending.len();
            let take = available.min(bytes.len() - offset);
            self.pending
                .extend_from_slice(&bytes[offset..offset + take]);
            offset += take;
            if self.pending.len() == MAX_FRAME_SIZE {
                if let Some(frame) = self.take_frame() {
                    frames.push(frame);
                }
            }
        }
        frames
    }

    pub fn flush_due(&mut self, now: Instant) -> Option<BinaryFrame> {
        let first_pending_at = self.first_pending_at?;
        if now.saturating_duration_since(first_pending_at) < FLUSH_INTERVAL {
            return None;
        }
        self.take_frame()
    }

    pub fn flush(&mut self) -> Option<BinaryFrame> {
        self.take_frame()
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    fn take_frame(&mut self) -> Option<BinaryFrame> {
        if self.pending.is_empty() {
            self.first_pending_at = None;
            return None;
        }
        let payload = std::mem::take(&mut self.pending);
        self.pending = Vec::with_capacity(MAX_FRAME_SIZE);
        self.first_pending_at = None;
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        Some(BinaryFrame { sequence, payload })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AckResult {
    pub acknowledged: usize,
    pub in_flight: usize,
    pub resumed: bool,
}

#[derive(Debug, Default)]
struct GateState {
    paused: bool,
    finishing: bool,
    closed: bool,
    metrics: OutputMetrics,
}

/// One byte budget covers admission, merging, delivery and frontend parsing.
/// Admission and finish share a lock so every accepted chunk is drained at EOF.
#[derive(Debug, Clone, Default)]
pub struct BackpressureGate {
    state: Arc<Mutex<GateState>>,
    notify: Arc<Notify>,
}

impl BackpressureGate {
    fn lock(&self) -> MutexGuard<'_, GateState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn check_open(state: &GateState) -> Result<()> {
        if state.closed {
            return Err(TerminalError::Cancelled);
        }
        if state.finishing {
            return Err(TerminalError::OutputClosed);
        }
        Ok(())
    }

    pub async fn wait_until_readable(&self) -> Result<()> {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let state = self.lock();
                Self::check_open(&state)?;
                if !state.paused {
                    return Ok(());
                }
            }
            notified.await;
        }
    }

    fn enqueue(&self, bytes: Vec<u8>, permit: mpsc::Permit<'_, Vec<u8>>) -> Result<()> {
        let mut state = self.lock();
        Self::check_open(&state)?;
        if state.paused {
            return Err(TerminalError::QueueFull);
        }
        state.metrics.input_bytes += bytes.len();
        state.metrics.queued_bytes += bytes.len();
        Self::update_watermarks(&mut state);
        permit.send(bytes);
        Ok(())
    }

    fn update_watermarks(state: &mut GateState) {
        let outstanding = state.metrics.queued_bytes + state.metrics.in_flight_bytes;
        state.metrics.max_pending_bytes = state
            .metrics
            .max_pending_bytes
            .max(state.metrics.queued_bytes);
        state.metrics.max_in_flight_bytes = state
            .metrics
            .max_in_flight_bytes
            .max(state.metrics.in_flight_bytes);
        state.metrics.max_outstanding_bytes = state.metrics.max_outstanding_bytes.max(outstanding);
        if outstanding > ACK_SUSPEND_THRESHOLD && !state.paused {
            state.paused = true;
            state.metrics.pause_count += 1;
        }
    }

    fn mark_sent(&self, bytes: usize) -> Result<()> {
        let mut state = self.lock();
        if state.closed {
            return Err(TerminalError::Cancelled);
        }
        state.metrics.queued_bytes -= bytes;
        state.metrics.in_flight_bytes += bytes;
        state.metrics.output_bytes += bytes;
        state.metrics.frame_count += 1;
        Self::update_watermarks(&mut state);
        Ok(())
    }

    #[cfg(test)]
    async fn register_sent(&self, bytes: usize) -> Result<()> {
        let mut state = self.lock();
        state.metrics.in_flight_bytes += bytes;
        Self::update_watermarks(&mut state);
        Ok(())
    }

    pub async fn acknowledge(&self, bytes: usize) -> Result<AckResult> {
        let mut state = self.lock();
        if bytes > state.metrics.in_flight_bytes {
            return Err(TerminalError::AckExceedsInFlight {
                acknowledged: bytes,
                in_flight: state.metrics.in_flight_bytes,
            });
        }
        state.metrics.in_flight_bytes -= bytes;
        let resumed = state.paused
            && state.metrics.queued_bytes + state.metrics.in_flight_bytes < ACK_RESUME_THRESHOLD;
        if resumed {
            state.paused = false;
            state.metrics.resume_count += 1;
        }
        self.notify.notify_waiters();
        Ok(AckResult {
            acknowledged: bytes,
            in_flight: state.metrics.in_flight_bytes,
            resumed,
        })
    }

    fn finish(&self) {
        self.lock().finishing = true;
        self.notify.notify_waiters();
    }

    fn close(&self, canceled: bool) {
        let mut state = self.lock();
        state.closed = true;
        state.paused = false;
        state.metrics.canceled |= canceled;
        state.metrics.channel_closed = true;
        state.metrics.queued_bytes = 0;
        self.notify.notify_waiters();
    }

    pub async fn is_paused(&self) -> bool {
        self.lock().paused
    }
    pub async fn in_flight(&self) -> usize {
        self.lock().metrics.in_flight_bytes
    }
    pub async fn metrics(&self) -> OutputMetrics {
        self.lock().metrics.clone()
    }
}

#[derive(Debug)]
struct Cancellation {
    canceled: watch::Sender<bool>,
}

impl Cancellation {
    fn new() -> Self {
        Self {
            canceled: watch::channel(false).0,
        }
    }
    fn cancel(&self) {
        self.canceled.send_replace(true);
    }
    async fn cancelled(&self) {
        let mut receiver = self.canceled.subscribe();
        while !*receiver.borrow_and_update() {
            if receiver.changed().await.is_err() {
                break;
            }
        }
    }
}

struct Completion(watch::Sender<bool>);
impl Drop for Completion {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}

#[derive(Clone, Debug)]
pub struct OutputPumpHandle {
    input: mpsc::Sender<Vec<u8>>,
    gate: BackpressureGate,
    cancellation: Arc<Cancellation>,
    finished: watch::Sender<bool>,
    done: watch::Receiver<bool>,
    admission: Arc<tokio::sync::Mutex<()>>,
}

impl OutputPumpHandle {
    pub async fn send(&self, bytes: Vec<u8>) -> Result<()> {
        if bytes.len() > MAX_FRAME_SIZE {
            return Err(TerminalError::ChunkTooLarge);
        }
        // Serial admission guarantees FIFO and prevents concurrent producers
        // from crossing the high watermark more than once.
        let _admission = self.admission.lock().await;
        self.gate.wait_until_readable().await?;
        let permit = tokio::select! {
            biased;
            () = self.cancellation.cancelled() => return Err(TerminalError::Cancelled),
            permit = self.input.reserve() => permit.map_err(|_| TerminalError::OutputClosed)?,
        };
        self.gate.enqueue(bytes, permit)
    }

    pub fn try_send(&self, bytes: Vec<u8>) -> Result<()> {
        if bytes.len() > MAX_FRAME_SIZE {
            return Err(TerminalError::ChunkTooLarge);
        }
        let _admission = self
            .admission
            .try_lock()
            .map_err(|_| TerminalError::QueueFull)?;
        let permit = self.input.try_reserve().map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => TerminalError::QueueFull,
            mpsc::error::TrySendError::Closed(_) => TerminalError::OutputClosed,
        })?;
        self.gate.enqueue(bytes, permit)
    }

    pub async fn wait_until_readable(&self) -> Result<()> {
        self.gate.wait_until_readable().await
    }
    pub async fn ack(&self, bytes: usize) -> Result<AckResult> {
        self.gate.acknowledge(bytes).await
    }
    pub async fn is_paused(&self) -> bool {
        self.gate.is_paused().await
    }
    pub async fn in_flight(&self) -> usize {
        self.gate.in_flight().await
    }
    pub async fn metrics(&self) -> OutputMetrics {
        self.gate.metrics().await
    }
    pub fn is_cancelled(&self) -> bool {
        self.gate.lock().metrics.canceled
    }
    pub fn cancel(&self) {
        self.gate.close(true);
        self.cancellation.cancel();
    }
    pub fn finish(&self) {
        self.gate.finish();
        self.finished.send_replace(true);
    }
    pub async fn cancel_and_wait(&self) {
        self.cancel();
        let mut done = self.done.clone();
        while !*done.borrow_and_update() {
            if done.changed().await.is_err() {
                break;
            }
        }
    }
}

#[derive(Debug)]
pub struct OutputStream {
    receiver: mpsc::Receiver<BinaryFrame>,
}
impl OutputStream {
    pub async fn recv(&mut self) -> Option<BinaryFrame> {
        self.receiver.recv().await
    }
}

#[derive(Debug)]
pub struct OutputPump;
impl OutputPump {
    pub fn start(
        config: OutputConfig,
    ) -> (
        OutputPumpHandle,
        OutputStream,
        tokio::task::JoinHandle<Result<OutputMetrics>>,
    ) {
        let (input, input_receiver) = mpsc::channel(config.input_capacity.max(1));
        let (output, receiver) = mpsc::channel(config.frame_capacity.max(1));
        let (finished, finish_receiver) = watch::channel(false);
        let (completed, done) = watch::channel(false);
        let gate = BackpressureGate::default();
        let cancellation = Arc::new(Cancellation::new());
        let handle = OutputPumpHandle {
            input,
            gate: gate.clone(),
            cancellation: cancellation.clone(),
            finished,
            done,
            admission: Arc::new(tokio::sync::Mutex::new(())),
        };
        let task = tokio::spawn(async move {
            let _completion = Completion(completed);
            let result = run_pump(
                input_receiver,
                output,
                finish_receiver,
                &gate,
                &cancellation,
            )
            .await;
            let canceled = matches!(result, Err(TerminalError::Cancelled));
            gate.close(canceled);
            if canceled {
                Ok(gate.metrics().await)
            } else {
                result.map(|()| gate.lock().metrics.clone())
            }
        });
        (handle, OutputStream { receiver }, task)
    }
}

async fn run_pump(
    mut input: mpsc::Receiver<Vec<u8>>,
    output: mpsc::Sender<BinaryFrame>,
    mut finished: watch::Receiver<bool>,
    gate: &BackpressureGate,
    cancellation: &Cancellation,
) -> Result<()> {
    let mut merger = OutputMerger::new();
    let mut ticker = tokio::time::interval(FLUSH_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut draining = false;
    loop {
        tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(TerminalError::Cancelled),
            () = output.closed() => return Err(TerminalError::OutputClosed),
            _ = finished.changed(), if !draining => {
                // Closing the receiver rejects outstanding/new admissions but
                // preserves every previously accepted item until recv is None.
                input.close();
                draining = true;
            }
            _ = ticker.tick() => {
                if let Some(frame) = merger.flush_due(Instant::now()) {
                    send_frame(&output, gate, cancellation, frame).await?;
                }
            }
            bytes = input.recv() => {
                let Some(bytes) = bytes else {
                    if let Some(frame) = merger.flush() {
                        send_frame(&output, gate, cancellation, frame).await?;
                    }
                    return Ok(());
                };
                let was_empty = merger.pending_len() == 0;
                let frames = merger.push(&bytes);
                let emitted = !frames.is_empty();
                for frame in frames { send_frame(&output, gate, cancellation, frame).await?; }
                if (was_empty || emitted) && merger.pending_len() > 0 {
                    ticker.reset_after(FLUSH_INTERVAL);
                }
            }
        }
    }
}

async fn send_frame(
    output: &mpsc::Sender<BinaryFrame>,
    gate: &BackpressureGate,
    cancellation: &Cancellation,
    frame: BinaryFrame,
) -> Result<()> {
    // Register before delivery; synchronous consumers may immediately ACK.
    gate.mark_sent(frame.len())?;
    tokio::select! {
        biased;
        () = cancellation.cancelled() => Err(TerminalError::Cancelled),
        result = output.send(frame) => result.map_err(|_| TerminalError::OutputClosed),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{
        BackpressureGate, OutputConfig, OutputMerger, OutputPump, ACK_RESUME_THRESHOLD,
        ACK_SUSPEND_THRESHOLD, FLUSH_INTERVAL, MAX_FRAME_SIZE,
    };

    #[tokio::test]
    async fn finish_drains_every_accepted_chunk_with_a_full_output_channel() {
        for _ in 0..100 {
            let (handle, mut stream, task) = OutputPump::start(OutputConfig {
                input_capacity: 32,
                frame_capacity: 1,
            });
            for value in 0..16 {
                handle.send(vec![value; 256]).await.expect("admit");
            }
            handle.finish();
            assert!(handle.send(vec![9]).await.is_err());
            assert!(handle.try_send(vec![9]).is_err());
            let received = tokio::time::timeout(Duration::from_secs(1), async {
                let mut received = Vec::new();
                while let Some(frame) = stream.recv().await {
                    handle.ack(frame.len()).await.expect("ack");
                    received.extend(frame.payload);
                }
                received
            })
            .await
            .expect("EOF must complete");
            let expected: Vec<u8> = (0..16).flat_map(|value| vec![value; 256]).collect();
            assert_eq!(received, expected);
            let metrics = task.await.expect("join").expect("result");
            assert_eq!(metrics.input_bytes, 4096);
            assert_eq!(metrics.output_bytes, 4096);
        }
    }

    #[tokio::test]
    async fn admission_counts_unmerged_bytes_and_resumes_only_below_low_watermark() {
        let (handle, mut stream, task) = OutputPump::start(OutputConfig::default());
        for _ in 0..17 {
            handle.send(vec![1; MAX_FRAME_SIZE]).await.expect("admit");
        }
        // The pump has not run on this current-thread runtime yet.
        let metrics = handle.metrics().await;
        assert_eq!(metrics.queued_bytes, 17 * MAX_FRAME_SIZE);
        assert_eq!(metrics.in_flight_bytes, 0);
        assert!(handle.is_paused().await);
        assert!(matches!(
            handle.try_send(vec![1]),
            Err(crate::TerminalError::QueueFull)
        ));
        for _ in 0..17 {
            stream.recv().await.expect("frame");
        }
        handle
            .ack(13 * MAX_FRAME_SIZE)
            .await
            .expect("ack to low watermark");
        assert!(handle.is_paused().await);
        handle.ack(1).await.expect("ack below watermark");
        assert!(!handle.is_paused().await);
        handle.send(vec![2]).await.expect("resumed admission");
        assert!(
            handle.metrics().await.max_outstanding_bytes <= ACK_SUSPEND_THRESHOLD + MAX_FRAME_SIZE
        );
        handle.cancel_and_wait().await;
        assert!(task.is_finished());
        task.await.expect("join").expect("cancel result");
    }

    #[tokio::test]
    async fn oversized_chunks_are_rejected_before_admission() {
        let (handle, _stream, task) = OutputPump::start(OutputConfig::default());
        assert!(matches!(
            handle.send(vec![0; MAX_FRAME_SIZE + 1]).await,
            Err(crate::TerminalError::ChunkTooLarge)
        ));
        assert_eq!(handle.metrics().await.input_bytes, 0);
        handle.cancel_and_wait().await;
        task.await.expect("join").expect("cancel result");
    }

    #[tokio::test]
    async fn pump_streams_100_mib_in_order_with_one_byte_budget() {
        let (handle, mut stream, task) = OutputPump::start(OutputConfig {
            input_capacity: 8,
            frame_capacity: 2,
        });
        let total_frames = 100 * 1024 * 1024 / MAX_FRAME_SIZE;
        let sender = handle.clone();
        let producer = tokio::spawn(async move {
            for sequence in 0..total_frames {
                sender
                    .send(vec![(sequence % 251) as u8; MAX_FRAME_SIZE])
                    .await
                    .expect("send");
            }
            sender.finish();
        });
        let mut frames = 0;
        while let Some(frame) = stream.recv().await {
            assert_eq!(frame.sequence, frames as u64);
            assert_eq!(frame.len(), MAX_FRAME_SIZE);
            assert!(frame
                .payload
                .iter()
                .all(|byte| *byte == (frames % 251) as u8));
            handle.ack(frame.len()).await.expect("ack");
            frames += 1;
        }
        producer.await.expect("producer");
        let metrics = task.await.expect("pump").expect("result");
        assert_eq!(frames, total_frames);
        assert_eq!(metrics.output_bytes, 100 * 1024 * 1024);
        assert!(metrics.max_outstanding_bytes <= ACK_SUSPEND_THRESHOLD + MAX_FRAME_SIZE);
    }

    #[tokio::test]
    async fn finish_wakes_a_producer_waiting_for_byte_budget() {
        let (handle, mut stream, task) = OutputPump::start(OutputConfig::default());
        for _ in 0..17 {
            handle.send(vec![0; MAX_FRAME_SIZE]).await.expect("admit");
        }
        let producer = {
            let handle = handle.clone();
            tokio::spawn(async move { handle.send(vec![1]).await })
        };
        tokio::task::yield_now().await;
        handle.finish();
        assert!(tokio::time::timeout(Duration::from_secs(1), producer)
            .await
            .expect("wake")
            .expect("join")
            .is_err());
        let mut received = 0;
        while let Some(frame) = stream.recv().await {
            received += frame.len();
        }
        assert_eq!(received, 17 * MAX_FRAME_SIZE);
        task.await.expect("join").expect("drain result");
    }

    #[test]
    fn merger_flushes_on_size_and_keeps_order() {
        let mut merger = OutputMerger::new();
        let first = merger.push(&vec![1_u8; MAX_FRAME_SIZE + 3]);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].payload.len(), MAX_FRAME_SIZE);
        let second = merger.flush().expect("tail frame");
        assert_eq!(second.payload, vec![1, 1, 1]);
        assert_eq!(first[0].sequence, 0);
        assert_eq!(second.sequence, 1);
    }

    #[test]
    fn merger_flushes_small_chunks_after_interval() {
        let mut merger = OutputMerger::new();
        assert!(merger.push(b"hello").is_empty());
        let now = Instant::now() + FLUSH_INTERVAL;
        let frame = merger.flush_due(now).expect("timer flush");
        assert_eq!(frame.payload, b"hello");
    }

    #[test]
    fn merger_preserves_a_100_mib_pattern() {
        let mut merger = OutputMerger::new();
        let block = (0..MAX_FRAME_SIZE)
            .map(|index| (index as u8).wrapping_mul(31))
            .collect::<Vec<_>>();
        let mut total = 0_usize;
        let mut checksum = 0_u64;
        for _ in 0..(100 * 1024 * 1024 / MAX_FRAME_SIZE) {
            for byte in &block {
                checksum = checksum.wrapping_add(u64::from(*byte));
            }
            for frame in merger.push(&block) {
                total += frame.len();
            }
        }
        if let Some(frame) = merger.flush() {
            total += frame.len();
        }
        let expected_checksum = (100 * 1024 * 1024 / MAX_FRAME_SIZE) as u64
            * block.iter().map(|byte| u64::from(*byte)).sum::<u64>();
        assert_eq!(total, 100 * 1024 * 1024);
        assert_eq!(checksum, expected_checksum);
    }

    #[tokio::test]
    async fn gate_pauses_above_one_mib_and_resumes_below_256kib() {
        let gate = BackpressureGate::default();
        gate.register_sent(ACK_SUSPEND_THRESHOLD)
            .await
            .expect("register");
        assert!(!gate.is_paused().await);
        gate.register_sent(1).await.expect("register");
        assert!(gate.is_paused().await);
        gate.acknowledge(ACK_SUSPEND_THRESHOLD - ACK_RESUME_THRESHOLD + 2)
            .await
            .expect("ack");
        assert!(!gate.is_paused().await);
    }

    #[tokio::test]
    async fn pump_preserves_binary_payload_and_ack_bytes() {
        let (handle, mut stream, task) = OutputPump::start(OutputConfig::default());
        let payload = vec![0, 1, 2, 255, 0];
        handle.send(payload.clone()).await.expect("send");
        tokio::time::sleep(Duration::from_millis(20)).await;
        let frame = stream.recv().await.expect("frame");
        assert_eq!(frame.payload, payload);
        handle.ack(frame.len()).await.expect("ack");
        assert_eq!(handle.in_flight().await, 0);
        drop(handle);
        let metrics = task.await.expect("pump join").expect("pump result");
        assert_eq!(metrics.output_bytes, 5);
    }

    #[tokio::test]
    async fn cancellation_wakes_pump_and_drops_pending_data() {
        let (handle, _stream, task) = OutputPump::start(OutputConfig::default());
        handle.send(vec![1, 2, 3]).await.expect("send");
        handle.cancel_and_wait().await;
        let metrics = task.await.expect("pump join").expect("pump result");
        assert!(metrics.canceled);
    }

    #[tokio::test]
    async fn synchronous_cancel_wakes_a_paused_reader() {
        let (handle, mut stream, task) = OutputPump::start(OutputConfig::default());
        for _ in 0..17 {
            handle
                .send(vec![42_u8; MAX_FRAME_SIZE])
                .await
                .expect("queue output");
        }
        for _ in 0..17 {
            stream.recv().await.expect("frame");
        }
        assert!(handle.is_paused().await);

        let waiter = {
            let handle = handle.clone();
            tokio::spawn(async move { handle.wait_until_readable().await })
        };
        tokio::task::yield_now().await;
        handle.cancel();
        let result = tokio::time::timeout(Duration::from_millis(100), waiter)
            .await
            .expect("cancel must wake reader")
            .expect("reader task");
        assert!(matches!(result, Err(crate::TerminalError::Cancelled)));
        let _ = task.await;
    }

    #[tokio::test]
    async fn cancellation_wakes_a_pump_blocked_on_a_full_channel() {
        let (handle, _stream, task) = OutputPump::start(OutputConfig {
            input_capacity: 4,
            frame_capacity: 1,
        });
        handle.send(vec![7; MAX_FRAME_SIZE]).await.expect("send");
        handle.send(vec![7; MAX_FRAME_SIZE]).await.expect("send");
        tokio::time::sleep(Duration::from_millis(5)).await;
        handle.cancel_and_wait().await;
        let metrics = tokio::time::timeout(Duration::from_millis(100), task)
            .await
            .expect("pump cancellation timeout")
            .expect("pump join")
            .expect("pump result");
        assert!(metrics.canceled);
    }

    #[tokio::test]
    async fn dropping_output_stream_while_paused_wakes_reader() {
        let (handle, mut stream, task) = OutputPump::start(OutputConfig::default());
        for _ in 0..17 {
            handle
                .send(vec![42_u8; MAX_FRAME_SIZE])
                .await
                .expect("queue output");
        }

        for _ in 0..17 {
            tokio::time::timeout(Duration::from_secs(1), stream.recv())
                .await
                .expect("frame delivery timeout")
                .expect("frame");
        }
        assert!(handle.is_paused().await);

        drop(stream);
        let readable =
            tokio::time::timeout(Duration::from_millis(100), handle.wait_until_readable())
                .await
                .expect("closed channel must wake the PTY reader");
        assert!(matches!(readable, Err(crate::TerminalError::Cancelled)));

        let pump_result = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("pump close timeout")
            .expect("pump join");
        assert!(matches!(
            pump_result,
            Err(crate::TerminalError::OutputClosed)
        ));
    }
}
