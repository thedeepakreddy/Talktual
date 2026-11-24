//! Stream workers: one supervised OS thread per stream.
//!
//! The worker owns the [`BackendStream`] (so backends whose streams are not
//! `Send` work), executes lifecycle commands from the runtime, and every
//! `poll` interval moves data between the real-time ring and the host queue:
//!
//! * **Capture:** drain the ring, assemble blocks of `block_frames`, stamp
//!   sequence, sample-clock position and timestamps, place discontinuities
//!   at the exact frame where the callback dropped data, apply the delivery
//!   overflow policy.
//! * **Playback:** move host writes into the ring as space allows.
//!
//! The worker never runs inside a device callback, so it may allocate and
//! take the (non-real-time) queue locks. It reports faults, reroutes and
//! pressure to the runtime through a bounded channel, and runs its body under
//! `catch_unwind` so a panic is reported instead of silently killing the
//! stream.
//!
//! This is a separate execution domain from the Tokio control plane
//! (ADR-0011): plain threads, because device APIs block and some backend
//! stream handles are thread-bound.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::Ordering::*;
use std::sync::mpsc as std_mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ce_core::audio::frame::frames_to_duration;
use ce_core::audio::{
    AudioBlock, AudioDirection, AudioFrameMeta, DeliveredFormat, DeliveryOverflowPolicy,
    Discontinuity, DiscontinuityKind, SampleFormat, StreamCounters, StreamFormat, StreamInfo,
};
use ce_core::time::{Clock, MonotonicTime};
use ce_core::{CeError, ErrorCategory};
use rtrb::Producer;
use tokio::sync::{mpsc, oneshot};

use crate::backend::{AudioBackend, AudioSignal, BackendStream, OpenSpec};
use crate::convert::append_frames;
use crate::io::{BlockQueue, CaptureReader, PlaybackQueue, PlaybackWriter, PushOutcome};
use crate::rt::{
    CaptureDrain, CaptureEndpoint, PlaybackEndpoint, StreamFault, StreamShared, Terminal,
};
use crate::tap::{TapEnd, TapSender, TapSlot};

/// Lifecycle commands from the runtime.
#[derive(Debug)]
pub enum WorkerCommand {
    /// Build the backend stream; replies with the backend buffer size.
    Open(oneshot::Sender<Result<Option<u32>, CeError>>),
    /// Start or resume callbacks.
    Start(oneshot::Sender<Result<(), CeError>>),
    /// Pause callbacks.
    Stop(oneshot::Sender<Result<(), CeError>>),
    /// Release the device (callbacks quiesced) and exit; replies with the
    /// final counters.
    Close(oneshot::Sender<StreamCounters>),
}

/// Unsolicited reports to the runtime.
#[derive(Clone, Debug, PartialEq)]
pub enum WorkerReport {
    /// The backend reported a fault (first fault only).
    Fault {
        /// Stream.
        stream: ce_core::ids::AudioStreamId,
        /// Fault.
        fault: StreamFault,
    },
    /// The backend rerouted a default-device stream.
    Rerouted {
        /// Stream.
        stream: ce_core::ids::AudioStreamId,
    },
    /// Counters changed (sent at most once per pressure interval).
    Pressure {
        /// Stream.
        stream: ce_core::ids::AudioStreamId,
        /// Latest counters.
        counters: StreamCounters,
        /// First discontinuity since the last report: (sequence, details).
        discontinuity: Option<(u64, Discontinuity)>,
    },
    /// The worker body panicked; the stream is unusable.
    Panicked {
        /// Stream.
        stream: ce_core::ids::AudioStreamId,
    },
}

/// Per-stream static context.
#[derive(Clone, Debug)]
pub struct StreamContext {
    /// Public facts about the stream (ids, formats, block size).
    pub info: StreamInfo,
    /// Backend open parameters.
    pub open: OpenSpec,
}

/// Worker tuning (from `AudioConfig`).
#[derive(Clone, Debug)]
pub struct WorkerConfig {
    /// Poll interval.
    pub poll: Duration,
    /// Device ring capacity in frames.
    pub ring_frames: usize,
    /// Host queue capacity in blocks.
    pub queue_blocks: usize,
    /// Capture delivery overflow policy.
    pub overflow: DeliveryOverflowPolicy,
    /// Minimum interval between pressure reports.
    pub pressure_interval: Duration,
    /// Time source for block timestamps.
    pub clock: Arc<dyn Clock>,
}

/// The host-facing end of a new stream.
#[derive(Debug)]
pub enum StreamEndpoint {
    /// Capture data.
    Capture(CaptureReader),
    /// Playback data.
    Playback(PlaybackWriter),
}

impl StreamEndpoint {
    /// Replace the stream facts (used by the runtime after the device has
    /// opened, so hosts see the actual buffer size and final adjustments).
    pub fn set_info(&mut self, info: StreamInfo) {
        match self {
            StreamEndpoint::Capture(r) => r.set_info(info),
            StreamEndpoint::Playback(w) => w.set_info(info),
        }
    }
}

/// The runtime's handle to a worker thread.
#[derive(Debug)]
pub struct WorkerHandle {
    cmd: std_mpsc::SyncSender<WorkerCommand>,
    thread: Option<JoinHandle<()>>,
    shared: Arc<StreamShared>,
    capture_queue: Option<Arc<BlockQueue>>,
    playback_queue: Option<Arc<PlaybackQueue>>,
    tap: Option<Arc<TapSlot>>,
}

/// Control channel depth: the runtime sends one command at a time.
const COMMAND_CAPACITY: usize = 4;

impl WorkerHandle {
    /// Send a command without blocking.
    pub fn send(&self, command: WorkerCommand) -> Result<(), CeError> {
        self.cmd.try_send(command).map_err(|e| match e {
            std_mpsc::TrySendError::Full(_) => CeError::new(
                ErrorCategory::QueueSaturated,
                "audio.worker_busy",
                "the stream worker has too many pending commands",
            ),
            std_mpsc::TrySendError::Disconnected(_) => CeError::new(
                ErrorCategory::Internal,
                "audio.worker_gone",
                "the stream worker has exited",
            ),
        })
    }

    /// Stop capture immediately: close the callback gate, discard queued
    /// blocks and make readers fail with `Revoked`. Device release follows
    /// with `Close`.
    pub fn revoke(&self) {
        self.shared.gate_open.store(false, Release);
        self.shared.set_terminal(Terminal::Revoked);
        if let Some(t) = &self.tap {
            t.detach(TapEnd::Revoked);
        }
        if let Some(q) = &self.capture_queue {
            q.purge_and_close();
        }
        if let Some(q) = &self.playback_queue {
            q.close();
        }
    }

    /// Mark the stream invalidated. The worker, on its next poll, delivers
    /// everything already captured (ring + partial block) and then ends the
    /// data plane: readers drain and then fail with `Invalidated`; writers
    /// fail. If the worker is gone, the data plane ends immediately.
    pub fn invalidate(&self) {
        self.shared.set_terminal(Terminal::Invalidated);
        self.shared.set_fault(StreamFault::StreamInvalidated);
        if self.is_finished() {
            self.end_data_plane();
        }
    }

    /// Mark the stream failed.
    pub fn fail(&self) {
        self.shared.set_terminal(Terminal::Failed);
        self.end_data_plane();
    }

    fn end_data_plane(&self) {
        if let Some(q) = &self.capture_queue {
            q.close();
        }
        if let Some(q) = &self.playback_queue {
            q.close();
        }
    }

    /// Attach an analysis tap (capture streams only). Every block delivered
    /// to the host queue from now on is also offered, without blocking, to
    /// the tap. Replaces (and ends) a previously attached tap.
    pub fn attach_tap(&self, tap: TapSender) -> Result<(), CeError> {
        let Some(slot) = &self.tap else {
            return Err(CeError::new(
                ErrorCategory::InvalidInput,
                "audio.tap_unsupported",
                "analysis taps are only available on capture streams",
            ));
        };
        if self.shared.terminal() != Terminal::None {
            tap.end(TapEnd::from_terminal(self.shared.terminal()));
            return Err(CeError::lifecycle(
                "audio.stream_ended",
                "the stream has already ended",
            ));
        }
        slot.attach(tap);
        Ok(())
    }

    /// Detach the analysis tap, if any; its receiver drains and ends.
    pub fn detach_tap(&self) {
        if let Some(t) = &self.tap {
            t.detach(TapEnd::Closed);
        }
    }

    /// Whether the thread has exited.
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Join an exited thread (never blocks on a running one). Returns
    /// whether it panicked.
    pub fn join_if_finished(&mut self) -> Option<bool> {
        if !self.is_finished() {
            return None;
        }
        self.thread.take().map(|t| t.join().is_err())
    }

    /// Live counters readable without the worker (callback-side atomics).
    pub fn callbacks(&self) -> u64 {
        self.shared.callbacks.load(Relaxed)
    }
}

/// Spawn a worker for a new stream. No device is touched until `Open`.
pub fn spawn_stream(
    backend: Arc<dyn AudioBackend>,
    context: StreamContext,
    config: WorkerConfig,
    reports: mpsc::Sender<AudioSignal>,
) -> Result<(WorkerHandle, StreamEndpoint), CeError> {
    let shared = StreamShared::new();
    let (cmd_tx, cmd_rx) = std_mpsc::sync_channel(COMMAND_CAPACITY);
    let device = context.open.format;
    let name = format!("ce-audio-{}", context.info.id);
    let spawn_err = |e: std::io::Error| {
        CeError::new(
            ErrorCategory::Internal,
            "audio.worker_spawn_failed",
            "could not start the stream worker thread",
        )
        .with_source(e)
    };
    match context.info.direction {
        AudioDirection::Capture => {
            let (endpoint, drain) =
                CaptureEndpoint::new(device.channels, config.ring_frames, shared.clone());
            let queue = BlockQueue::new(config.queue_blocks);
            let reader = CaptureReader::new(context.info.clone(), queue.clone(), shared.clone());
            let tap = Arc::new(TapSlot::default());
            let mut worker = CaptureWorker::new(
                backend,
                context,
                &config,
                endpoint,
                drain,
                queue.clone(),
                shared.clone(),
                reports,
            );
            worker.tap = tap.clone();
            let thread = std::thread::Builder::new()
                .name(name)
                .spawn(move || supervise(worker, cmd_rx))
                .map_err(spawn_err)?;
            Ok((
                WorkerHandle {
                    cmd: cmd_tx,
                    thread: Some(thread),
                    shared,
                    capture_queue: Some(queue),
                    playback_queue: None,
                    tap: Some(tap),
                },
                StreamEndpoint::Capture(reader),
            ))
        }
        AudioDirection::Playback => {
            let (endpoint, producer) =
                PlaybackEndpoint::new(device.channels, config.ring_frames, shared.clone());
            let queue = PlaybackQueue::new(config.queue_blocks);
            let writer = PlaybackWriter::new(context.info.clone(), queue.clone(), shared.clone());
            let worker = PlaybackWorker::new(
                backend,
                context,
                &config,
                endpoint,
                producer,
                queue.clone(),
                shared.clone(),
                reports,
            );
            let thread = std::thread::Builder::new()
                .name(name)
                .spawn(move || supervise(worker, cmd_rx))
                .map_err(spawn_err)?;
            Ok((
                WorkerHandle {
                    cmd: cmd_tx,
                    thread: Some(thread),
                    shared,
                    capture_queue: None,
                    playback_queue: Some(queue),
                    tap: None,
                },
                StreamEndpoint::Playback(writer),
            ))
        }
    }
}

/// What both worker kinds provide to the shared command loop.
trait Pump {
    fn stream_id(&self) -> ce_core::ids::AudioStreamId;
    fn base(&mut self) -> &mut Base;
    fn open(&mut self) -> Result<Box<dyn BackendStream>, CeError>;
    fn on_start(&mut self) {}
    fn on_stop(&mut self) {}
    fn pump(&mut self);
    fn counters(&mut self) -> StreamCounters;
    fn on_close(&mut self);
}

/// State common to capture and playback workers.
struct Base {
    backend: Arc<dyn AudioBackend>,
    open: OpenSpec,
    shared: Arc<StreamShared>,
    reports: mpsc::Sender<AudioSignal>,
    poll: Duration,
    pressure_interval: Duration,
    last_report: Instant,
    last_reported: StreamCounters,
    fault_reported: bool,
    pending_discontinuity: Option<(u64, Discontinuity)>,
}

impl Base {
    fn report_blocking(&self, report: WorkerReport) {
        // Fault reports matter; the runtime drains this channel even while it
        // waits on workers, so a blocking send cannot deadlock. If the
        // runtime is gone the send fails and there is nobody to tell.
        let _ = self.reports.blocking_send(AudioSignal::Worker(report));
    }

    fn check_flags(&mut self, stream: ce_core::ids::AudioStreamId) {
        if !self.fault_reported {
            if let Some(fault) = self.shared.fault() {
                self.fault_reported = true;
                self.report_blocking(WorkerReport::Fault { stream, fault });
            }
        }
        if self.shared.rerouted.swap(false, AcqRel) {
            let _ = self
                .reports
                .try_send(AudioSignal::Worker(WorkerReport::Rerouted { stream }));
        }
    }

    fn maybe_report_pressure(
        &mut self,
        stream: ce_core::ids::AudioStreamId,
        counters: StreamCounters,
    ) {
        if self.last_report.elapsed() < self.pressure_interval {
            return;
        }
        let changed = counters.ring_overflow_frames != self.last_reported.ring_overflow_frames
            || counters.queue_dropped_frames != self.last_reported.queue_dropped_frames
            || counters.underrun_frames != self.last_reported.underrun_frames
            || counters.discontinuities != self.last_reported.discontinuities
            || counters.backend_xruns != self.last_reported.backend_xruns
            || counters.blocks != self.last_reported.blocks;
        if !changed {
            return;
        }
        let report = WorkerReport::Pressure {
            stream,
            counters,
            discontinuity: self.pending_discontinuity,
        };
        // Pressure is advisory: a full channel skips this report and the
        // next interval carries the totals.
        if self.reports.try_send(AudioSignal::Worker(report)).is_ok() {
            self.last_reported = counters;
            self.last_report = Instant::now();
            self.pending_discontinuity = None;
        }
    }
}

fn not_open() -> CeError {
    CeError::lifecycle("audio.stream_not_open", "the device stream is not open")
}

/// Command loop shared by both worker kinds, run under `catch_unwind`.
fn supervise<W: Pump>(mut worker: W, commands: std_mpsc::Receiver<WorkerCommand>) {
    let stream = worker.stream_id();
    // The backend stream is a local of `run`, so unwinding drops it (and
    // the adapter quiesces callbacks) before the panic is reported.
    let outcome = catch_unwind(AssertUnwindSafe(|| run(&mut worker, &commands)));
    if outcome.is_err() {
        let base = worker.base();
        base.shared.set_terminal(Terminal::Failed);
        base.shared.gate_open.store(false, Release);
        base.report_blocking(WorkerReport::Panicked { stream });
        worker.on_close();
    }
}

fn run<W: Pump>(worker: &mut W, commands: &std_mpsc::Receiver<WorkerCommand>) {
    let stream = worker.stream_id();
    let mut device: Option<Box<dyn BackendStream>> = None;
    let mut fault_drained = false;
    loop {
        let poll = worker.base().poll;
        match commands.recv_timeout(poll) {
            Ok(WorkerCommand::Open(reply)) => {
                let result = match worker.open() {
                    Ok(s) => {
                        let frames = s.buffer_frames();
                        device = Some(s);
                        Ok(frames)
                    }
                    Err(e) => Err(e),
                };
                let _ = reply.send(result);
            }
            Ok(WorkerCommand::Start(reply)) => {
                let result = match device.as_mut() {
                    Some(s) => s.start(),
                    None => Err(not_open()),
                };
                if result.is_ok() {
                    worker.on_start();
                }
                let _ = reply.send(result);
            }
            Ok(WorkerCommand::Stop(reply)) => {
                let result = match device.as_mut() {
                    Some(s) => s.stop(),
                    None => Err(not_open()),
                };
                worker.pump();
                worker.on_stop();
                let _ = reply.send(result);
            }
            Ok(WorkerCommand::Close(reply)) => {
                let base = worker.base();
                base.shared.gate_open.store(false, Release);
                // Dropping the backend stream guarantees callback quiescence
                // (adapter contract) before anything else is released.
                drop(device.take());
                base.shared.set_terminal(Terminal::Closed);
                worker.on_close();
                let counters = worker.counters();
                let _ = reply.send(counters);
                return;
            }
            Err(std_mpsc::RecvTimeoutError::Timeout) => {}
            Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                // The runtime dropped the handle: release and exit.
                let base = worker.base();
                base.shared.gate_open.store(false, Release);
                drop(device.take());
                base.shared.set_terminal(Terminal::Closed);
                worker.on_close();
                return;
            }
        }
        worker.pump();
        worker.base().check_flags(stream);
        if !fault_drained && worker.base().shared.fault().is_some() {
            // The stream can no longer run: deliver what was captured before
            // the fault, then end the data plane. The device is released
            // when the runtime closes the stream (no silent restart).
            fault_drained = true;
            worker.on_stop();
            worker.base().shared.gate_open.store(false, Release);
            worker.on_close();
        }
        let counters = worker.counters();
        worker.base().maybe_report_pressure(stream, counters);
    }
}

// ---------------------------------------------------------------- capture

struct CaptureWorker {
    base: Base,
    info: StreamInfo,
    endpoint: Option<CaptureEndpoint>,
    drain: CaptureDrain,
    queue: Arc<BlockQueue>,
    overflow: DeliveryOverflowPolicy,
    clock: Arc<dyn Clock>,
    device: StreamFormat,
    delivered: DeliveredFormat,
    downmix: bool,
    block_frames: u32,
    ring_capacity_frames: usize,
    // assembly state
    read_pos: u64,
    frame_index: u64,
    sequence: u64,
    pending: Vec<f32>,
    pending_frames: u32,
    pending_start_index: u64,
    pending_start_read_pos: u64,
    next_discontinuity: Option<Discontinuity>,
    anchor: Option<(MonotonicTime, u64)>,
    started_once: bool,
    last_timing: Option<crate::rt::TimingMarker>,
    counters: StreamCounters,
    tap: Arc<TapSlot>,
}

impl CaptureWorker {
    #[allow(clippy::too_many_arguments)]
    fn new(
        backend: Arc<dyn AudioBackend>,
        context: StreamContext,
        config: &WorkerConfig,
        endpoint: CaptureEndpoint,
        drain: CaptureDrain,
        queue: Arc<BlockQueue>,
        shared: Arc<StreamShared>,
        reports: mpsc::Sender<AudioSignal>,
    ) -> Self {
        let delivered = context.info.format.delivered;
        let device = context.open.format;
        let block_frames = context.info.block_frames.max(1);
        Self {
            base: Base {
                backend,
                open: context.open,
                shared,
                reports,
                poll: config.poll,
                pressure_interval: config.pressure_interval,
                last_report: Instant::now(),
                last_reported: StreamCounters::default(),
                fault_reported: false,
                pending_discontinuity: None,
            },
            downmix: delivered.channels != device.channels,
            info: context.info,
            endpoint: Some(endpoint),
            drain,
            queue,
            overflow: config.overflow,
            clock: config.clock.clone(),
            device,
            delivered,
            block_frames,
            ring_capacity_frames: config.ring_frames,
            read_pos: 0,
            frame_index: 0,
            sequence: 0,
            pending: Vec::with_capacity(block_frames as usize * usize::from(delivered.channels)),
            pending_frames: 0,
            pending_start_index: 0,
            pending_start_read_pos: 0,
            next_discontinuity: None,
            anchor: None,
            started_once: false,
            last_timing: None,
            counters: StreamCounters::default(),
            tap: Arc::new(TapSlot::default()),
        }
    }

    fn add_discontinuity(&mut self, kind: DiscontinuityKind, lost: Option<u64>) {
        let lost_frames = match (self.next_discontinuity.and_then(|d| d.lost_frames), lost) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        };
        // A restart dominates: it explains a real-time gap regardless of loss.
        let kind = match self.next_discontinuity {
            Some(d) if d.kind == DiscontinuityKind::StreamRestart => {
                DiscontinuityKind::StreamRestart
            }
            _ => kind,
        };
        self.next_discontinuity = Some(Discontinuity { kind, lost_frames });
    }

    fn device_time_at(&self, read_pos: u64) -> Option<Duration> {
        let m = self.last_timing?;
        if m.pos > read_pos {
            return None;
        }
        let offset = frames_to_duration(read_pos - m.pos, self.device.sample_rate);
        Some(Duration::from_nanos(m.device_nanos) + offset)
    }

    fn emit(&mut self) {
        if self.pending_frames == 0 {
            return;
        }
        let now = self.clock.monotonic();
        let rate = self.device.sample_rate;
        if self.anchor.is_none() {
            // First data after a start: the first frame of this block was
            // captured roughly (block + ring backlog) ago.
            let backlog =
                (self.drain.samples.slots() / usize::from(self.device.channels.max(1))) as u64;
            let age = frames_to_duration(u64::from(self.pending_frames) + backlog, rate);
            let origin = MonotonicTime::from_nanos(
                now.as_nanos()
                    .saturating_sub(u64::try_from(age.as_nanos()).unwrap_or(u64::MAX)),
            );
            self.anchor = Some((origin, self.pending_start_index));
        }
        let (anchor_time, anchor_index) = self.anchor.unwrap_or((now, self.pending_start_index));
        let since = frames_to_duration(self.pending_start_index.saturating_sub(anchor_index), rate);
        let timestamp = MonotonicTime::from_nanos(
            anchor_time
                .as_nanos()
                .saturating_add(u64::try_from(since.as_nanos()).unwrap_or(u64::MAX)),
        );
        let discontinuity = self.next_discontinuity.take();
        let meta = AudioFrameMeta {
            stream_id: self.info.id,
            session_id: self.info.session_id,
            correlation_id: self.info.correlation_id,
            sequence: self.sequence,
            frame_index: self.pending_start_index,
            frames: self.pending_frames,
            sample_rate: rate,
            channels: self.delivered.channels,
            layout: self.delivered.layout,
            sample_format: SampleFormat::F32,
            timestamp,
            host_time: now,
            device_time: self.device_time_at(self.pending_start_read_pos),
            discontinuity,
        };
        let samples = std::mem::replace(
            &mut self.pending,
            Vec::with_capacity(self.block_frames as usize * usize::from(self.delivered.channels)),
        );
        let frames = u64::from(self.pending_frames);
        let sequence = self.sequence;
        self.sequence += 1;
        self.pending_frames = 0;
        if let Some(d) = discontinuity {
            self.counters.discontinuities += 1;
            if self.base.pending_discontinuity.is_none() {
                self.base.pending_discontinuity = Some((sequence, d));
            }
        }
        let block = AudioBlock { meta, samples };
        self.tap.offer(&block);
        match self.queue.push(block, self.overflow) {
            PushOutcome::Queued => {
                self.counters.blocks += 1;
                self.counters.frames += frames;
            }
            PushOutcome::EvictedOldest(lost) => {
                self.counters.blocks += 1;
                self.counters.frames += frames;
                self.counters.queue_dropped_blocks += 1;
                self.counters.queue_dropped_frames += lost;
                // The drop is a gap in the delivered stream (flagged on the
                // block after it, possibly merged with other gaps).
                self.counters.discontinuities += 1;
            }
            PushOutcome::DroppedNewest(lost) => {
                self.counters.queue_dropped_blocks += 1;
                self.counters.queue_dropped_frames += lost;
                self.counters.discontinuities += 1;
            }
        }
    }

    fn handle_gap(&mut self, lost: u64) {
        // Blocks are contiguous: flush what precedes the gap.
        self.emit();
        self.frame_index += lost;
        self.add_discontinuity(DiscontinuityKind::DeviceOverflow, Some(lost));
    }
}

impl Pump for CaptureWorker {
    fn stream_id(&self) -> ce_core::ids::AudioStreamId {
        self.info.id
    }

    fn base(&mut self) -> &mut Base {
        &mut self.base
    }

    fn open(&mut self) -> Result<Box<dyn BackendStream>, CeError> {
        let endpoint = self
            .endpoint
            .take()
            .ok_or_else(|| CeError::lifecycle("audio.already_opened", "stream already opened"))?;
        self.base.backend.open_capture(&self.base.open, endpoint)
    }

    fn on_start(&mut self) {
        self.anchor = None;
        if self.started_once {
            self.add_discontinuity(DiscontinuityKind::StreamRestart, None);
        }
        self.started_once = true;
    }

    fn on_stop(&mut self) {
        // Deliver everything captured before the stop.
        self.emit();
    }

    fn pump(&mut self) {
        let ch = usize::from(self.device.channels.max(1));
        loop {
            // Observe the sample ring *before* the gap ring: a gap marker is
            // pushed before any later samples, so it is visible here.
            let available = self.drain.samples.slots() / ch;
            let fill = available * 1000 / self.ring_capacity_frames.max(1);
            self.counters.max_ring_fill_permille = self
                .counters
                .max_ring_fill_permille
                .max(u16::try_from(fill.min(1000)).unwrap_or(1000));
            while let Ok(m) = self.drain.timing.peek() {
                if m.pos > self.read_pos {
                    break;
                }
                self.last_timing = Some(*m);
                let _ = self.drain.timing.pop();
            }
            let gap = self.drain.gaps.peek().ok().copied();
            if let Some(g) = gap {
                if g.at <= self.read_pos {
                    let _ = self.drain.gaps.pop();
                    self.handle_gap(g.lost);
                    continue;
                }
            }
            if available == 0 {
                break;
            }
            let mut take = available;
            if let Some(g) = gap {
                take = take.min(usize::try_from(g.at - self.read_pos).unwrap_or(usize::MAX));
            }
            if self.pending_frames == 0 {
                self.pending_start_index = self.frame_index;
                self.pending_start_read_pos = self.read_pos;
            }
            take = take.min((self.block_frames - self.pending_frames) as usize);
            match self.drain.samples.read_chunk(take * ch) {
                Ok(chunk) => {
                    let (a, b) = chunk.as_slices();
                    append_frames(a, ch, self.downmix, &mut self.pending);
                    append_frames(b, ch, self.downmix, &mut self.pending);
                    chunk.commit_all();
                }
                Err(_) => break,
            }
            self.read_pos += take as u64;
            self.frame_index += take as u64;
            self.pending_frames += take as u32;
            if self.pending_frames >= self.block_frames {
                self.emit();
            }
        }
    }

    fn counters(&mut self) -> StreamCounters {
        let s = &self.base.shared;
        StreamCounters {
            callbacks: s.callbacks.load(Relaxed),
            ring_overflow_frames: s.ring_overflow_frames.load(Relaxed),
            backend_xruns: s.backend_xruns.load(Relaxed),
            ..self.counters
        }
    }

    fn on_close(&mut self) {
        self.queue.close();
        self.tap.detach(TapEnd::at_close(&self.base.shared));
    }
}

// ---------------------------------------------------------------- playback

struct PlaybackWorker {
    base: Base,
    info: StreamInfo,
    endpoint: Option<PlaybackEndpoint>,
    producer: Producer<f32>,
    queue: Arc<PlaybackQueue>,
    channels: usize,
    current: Option<(Vec<f32>, usize)>,
    counters: StreamCounters,
}

impl PlaybackWorker {
    #[allow(clippy::too_many_arguments)]
    fn new(
        backend: Arc<dyn AudioBackend>,
        context: StreamContext,
        config: &WorkerConfig,
        endpoint: PlaybackEndpoint,
        producer: Producer<f32>,
        queue: Arc<PlaybackQueue>,
        shared: Arc<StreamShared>,
        reports: mpsc::Sender<AudioSignal>,
    ) -> Self {
        Self {
            base: Base {
                backend,
                open: context.open,
                shared,
                reports,
                poll: config.poll,
                pressure_interval: config.pressure_interval,
                last_report: Instant::now(),
                last_reported: StreamCounters::default(),
                fault_reported: false,
                pending_discontinuity: None,
            },
            channels: usize::from(context.info.format.device.channels.max(1)),
            info: context.info,
            endpoint: Some(endpoint),
            producer,
            queue,
            current: None,
            counters: StreamCounters::default(),
        }
    }
}

impl Pump for PlaybackWorker {
    fn stream_id(&self) -> ce_core::ids::AudioStreamId {
        self.info.id
    }

    fn base(&mut self) -> &mut Base {
        &mut self.base
    }

    fn open(&mut self) -> Result<Box<dyn BackendStream>, CeError> {
        let endpoint = self
            .endpoint
            .take()
            .ok_or_else(|| CeError::lifecycle("audio.already_opened", "stream already opened"))?;
        self.base.backend.open_playback(&self.base.open, endpoint)
    }

    fn pump(&mut self) {
        loop {
            if self.current.is_none() {
                match self.queue.pop() {
                    Some((_, samples)) => {
                        self.counters.blocks += 1;
                        self.current = Some((samples, 0));
                    }
                    None => break,
                }
            }
            let Some((samples, offset)) = self.current.as_mut() else {
                break;
            };
            let space = (self.producer.slots() / self.channels) * self.channels;
            let n = space.min(samples.len() - *offset);
            if n == 0 {
                break;
            }
            if let Ok(chunk) = self.producer.write_chunk_uninit(n) {
                chunk.fill_from_iter(samples[*offset..*offset + n].iter().copied());
                *offset += n;
            }
            if *offset >= samples.len() {
                self.current = None;
            }
        }
        let pending = self.current.is_some() || !self.queue.is_empty();
        self.base.shared.upstream_pending.store(pending, Release);
    }

    fn counters(&mut self) -> StreamCounters {
        let s = &self.base.shared;
        StreamCounters {
            callbacks: s.callbacks.load(Relaxed),
            frames: s.played_frames.load(Relaxed),
            underrun_frames: s.underrun_frames.load(Relaxed),
            backend_xruns: s.backend_xruns.load(Relaxed),
            ..self.counters
        }
    }

    fn on_close(&mut self) {
        self.queue.close();
        self.current = None;
    }
}
