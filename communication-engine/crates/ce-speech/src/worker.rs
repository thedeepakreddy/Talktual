//! Supervised recognition worker: one thread per recognized stream.
//!
//! * Audio input: a [`TapReceiver`] (bounded; the capture side drops
//!   copies instead of blocking, so recognition can never delay capture,
//!   playback or host delivery).
//! * Hint input: a bounded `sync_channel` of Phase 3 analysis events fed
//!   with `try_send` by the runtime; a full queue drops the hint and counts
//!   it (the control plane is never blocked by a slow recognizer).
//! * Output: [`SpeechSignal`]s on a bounded Tokio channel; the worker blocks
//!   only itself when it is full.
//! * Load: performed on the worker thread (never in a callback or on the
//!   control plane) with its start time published so the runtime can
//!   enforce `load_timeout_ms`.
//! * Supervision: the body runs under `catch_unwind`; a panic becomes
//!   `Failed` + `Stopped(Failed)`. A heartbeat lets the runtime detect a
//!   stall. Stop and reset are flags checked between blocks. An abandoned
//!   worker (stalled or load timeout) sends nothing more.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ce_audio::{TapEnd, TapReceiver, TapRecvError};
use ce_core::ids::AudioStreamId;
use ce_core::intel::AnalysisEvent;
use ce_core::speech::{RecognitionEndReason, RecognitionStatus, SpeechEvent};
use ce_core::{CeError, ErrorCategory};
use tokio::sync::mpsc;

use crate::adapter::AsrAdapter;
use crate::recognizer::{RecognizerSetup, StreamRecognizer};

/// Messages from recognition workers to the runtime.
#[derive(Clone, Debug, PartialEq)]
pub enum SpeechSignal {
    /// A speech event.
    Event(SpeechEvent),
    /// The worker finished; final status.
    Ended {
        /// Stream.
        stream: AudioStreamId,
        /// Final status.
        status: Box<RecognitionStatus>,
    },
}

const POLL: Duration = Duration::from_millis(20);
const STATUS_EVERY: u64 = 25;

fn code(r: RecognitionEndReason) -> u8 {
    match r {
        RecognitionEndReason::Stopped => 1,
        RecognitionEndReason::StreamClosed => 2,
        RecognitionEndReason::Revoked => 3,
        RecognitionEndReason::StreamInvalidated => 4,
        RecognitionEndReason::StreamFailed => 5,
        RecognitionEndReason::LoadFailed => 6,
        RecognitionEndReason::Failed => 7,
        RecognitionEndReason::Shutdown => 8,
    }
}

fn from_code(v: u8) -> Option<RecognitionEndReason> {
    Some(match v {
        1 => RecognitionEndReason::Stopped,
        2 => RecognitionEndReason::StreamClosed,
        3 => RecognitionEndReason::Revoked,
        4 => RecognitionEndReason::StreamInvalidated,
        5 => RecognitionEndReason::StreamFailed,
        6 => RecognitionEndReason::LoadFailed,
        7 => RecognitionEndReason::Failed,
        8 => RecognitionEndReason::Shutdown,
        _ => return None,
    })
}

fn tap_reason(e: TapEnd) -> RecognitionEndReason {
    match e {
        TapEnd::Closed => RecognitionEndReason::StreamClosed,
        TapEnd::Revoked => RecognitionEndReason::Revoked,
        TapEnd::Invalidated => RecognitionEndReason::StreamInvalidated,
        TapEnd::Failed => RecognitionEndReason::StreamFailed,
    }
}

#[derive(Debug)]
struct Shared {
    stop: AtomicU8,
    resets: AtomicU32,
    abandoned: AtomicBool,
    heartbeat_ms: AtomicU64,
    /// 0 = not loading; otherwise load start (ms since origin) + 1.
    loading_since: AtomicU64,
    hint_dropped: AtomicU64,
    status: Mutex<RecognitionStatus>,
    origin: Instant,
}

impl Shared {
    fn now_ms(&self) -> u64 {
        self.origin.elapsed().as_millis() as u64
    }

    fn beat(&self) {
        self.heartbeat_ms.store(self.now_ms(), Relaxed);
    }

    fn set_status(&self, s: RecognitionStatus) {
        *self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = s;
    }

    fn status(&self) -> RecognitionStatus {
        self.status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// The runtime's handle to a recognition worker.
#[derive(Debug)]
pub struct RecognitionHandle {
    stream: AudioStreamId,
    shared: Arc<Shared>,
    hints: SyncSender<AnalysisEvent>,
    thread: Option<JoinHandle<()>>,
    stall_timeout: Duration,
    load_timeout: Duration,
}

impl RecognitionHandle {
    /// Stream.
    pub fn stream(&self) -> AudioStreamId {
        self.stream
    }

    /// Ask the worker to finish with `reason` (first request wins).
    pub fn stop(&self, reason: RecognitionEndReason) {
        let _ = self
            .shared
            .stop
            .compare_exchange(0, code(reason), AcqRel, Acquire);
    }

    /// The stop reason requested, if any.
    pub fn stop_requested(&self) -> Option<RecognitionEndReason> {
        from_code(self.shared.stop.load(Acquire))
    }

    /// Ask the worker to reset (cancel the open utterance, reset the
    /// adapter) at its next iteration.
    pub fn request_reset(&self) {
        self.shared.resets.fetch_add(1, AcqRel);
    }

    /// Offer a Phase 3 event without blocking; false if it was dropped.
    pub fn send_hint(&self, event: &AnalysisEvent) -> bool {
        match self.hints.try_send(event.clone()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                self.shared.hint_dropped.fetch_add(1, Relaxed);
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }

    /// Hints dropped because the hint queue was full.
    pub fn hints_dropped(&self) -> u64 {
        self.shared.hint_dropped.load(Relaxed)
    }

    /// Latest status snapshot.
    pub fn status(&self) -> RecognitionStatus {
        let mut s = self.shared.status();
        s.counters.hint_dropped += self.hints_dropped();
        s
    }

    /// Whether the thread has exited.
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Join an exited thread (never blocks on a running one).
    pub fn join_if_finished(&mut self) -> Option<bool> {
        if !self.is_finished() {
            return None;
        }
        self.thread.take().map(|t| t.join().is_err())
    }

    fn loading_ms(&self) -> Option<u64> {
        match self.shared.loading_since.load(Acquire) {
            0 => None,
            v => Some(self.shared.now_ms().saturating_sub(v - 1)),
        }
    }

    /// The model load has exceeded `load_timeout_ms`.
    pub fn load_timed_out(&self) -> bool {
        !self.is_finished()
            && self
                .loading_ms()
                .is_some_and(|ms| ms > self.load_timeout.as_millis() as u64)
    }

    /// No progress for the stall timeout (outside a load).
    pub fn stalled(&self) -> bool {
        if self.is_finished() || self.loading_ms().is_some() {
            return false;
        }
        let last = self.shared.heartbeat_ms.load(Relaxed);
        self.shared.now_ms().saturating_sub(last) > self.stall_timeout.as_millis() as u64
    }

    /// Give up on the worker: it will send nothing more; the thread is
    /// detached (joined later if it ever exits).
    pub fn abandon(&mut self, reason: RecognitionEndReason) {
        self.shared.abandoned.store(true, Release);
        self.stop(reason);
    }
}

use std::sync::atomic::Ordering::Release;

/// Start recognizing `tap` on a new thread.
pub fn spawn_recognition(
    setup: RecognizerSetup,
    adapter: Box<dyn AsrAdapter>,
    tap: TapReceiver,
    sink: mpsc::Sender<SpeechSignal>,
) -> Result<RecognitionHandle, CeError> {
    let stream = setup.stream;
    let budget = setup.config.budget.clone();
    let recognizer = StreamRecognizer::new(setup, adapter)?;
    let (hint_tx, hint_rx) = sync_channel(budget.hint_queue as usize);
    let shared = Arc::new(Shared {
        stop: AtomicU8::new(0),
        resets: AtomicU32::new(0),
        abandoned: AtomicBool::new(false),
        heartbeat_ms: AtomicU64::new(0),
        loading_since: AtomicU64::new(0),
        hint_dropped: AtomicU64::new(0),
        status: Mutex::new(recognizer.status()),
        origin: Instant::now(),
    });
    let s2 = shared.clone();
    let thread = std::thread::Builder::new()
        .name(format!("ce-speech-{stream}"))
        .spawn(move || supervise(recognizer, tap, hint_rx, sink, s2))
        .map_err(|e| {
            CeError::new(
                ErrorCategory::Internal,
                "speech.spawn_failed",
                "could not start the recognition worker",
            )
            .with_source(e)
        })?;
    Ok(RecognitionHandle {
        stream,
        shared,
        hints: hint_tx,
        thread: Some(thread),
        stall_timeout: Duration::from_millis(budget.stall_timeout_ms),
        load_timeout: Duration::from_millis(budget.load_timeout_ms),
    })
}

fn send_all(
    sink: &mpsc::Sender<SpeechSignal>,
    shared: &Shared,
    out: &mut Vec<SpeechEvent>,
) -> bool {
    if shared.abandoned.load(Acquire) {
        out.clear();
        return false;
    }
    for e in out.drain(..) {
        if sink.blocking_send(SpeechSignal::Event(e)).is_err() {
            return false;
        }
    }
    true
}

fn supervise(
    mut rec: StreamRecognizer,
    tap: TapReceiver,
    hints: Receiver<AnalysisEvent>,
    sink: mpsc::Sender<SpeechSignal>,
    shared: Arc<Shared>,
) {
    let stream = rec.stream();
    let mut out = Vec::new();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        run(&mut rec, &tap, &hints, &sink, &shared, &mut out)
    }));
    if outcome.is_err() {
        out.clear();
        let err = CeError::new(
            ErrorCategory::Internal,
            "speech.worker_panicked",
            "the recognition worker panicked",
        );
        let closed = catch_unwind(AssertUnwindSafe(|| {
            let mut o = Vec::new();
            rec.fail(&err, &mut o);
            o
        }));
        match closed {
            Ok(o) => out.extend(o),
            Err(_) => {
                out.push(SpeechEvent::Failed {
                    stream,
                    error: err.report(&ce_core::privacy::RedactionPolicy::DEFAULT),
                });
                out.push(SpeechEvent::Stopped {
                    stream,
                    reason: RecognitionEndReason::Failed,
                    counters: Box::default(),
                });
            }
        }
        send_all(&sink, &shared, &mut out);
    }
    shared.loading_since.store(0, Release);
    let mut status =
        catch_unwind(AssertUnwindSafe(|| rec.status())).unwrap_or_else(|_| shared.status());
    status.running = false;
    if status.ended.is_none() {
        status.ended = Some(RecognitionEndReason::Failed);
    }
    shared.set_status(status.clone());
    if !shared.abandoned.load(Acquire) {
        let _ = sink.blocking_send(SpeechSignal::Ended {
            stream,
            status: Box::new(status),
        });
    }
}

fn run(
    rec: &mut StreamRecognizer,
    tap: &TapReceiver,
    hints: &Receiver<AnalysisEvent>,
    sink: &mpsc::Sender<SpeechSignal>,
    shared: &Shared,
    out: &mut Vec<SpeechEvent>,
) {
    rec.start(out);
    if !send_all(sink, shared, out) {
        return;
    }
    shared.loading_since.store(shared.now_ms() + 1, Release);
    let loaded = rec.load(out);
    shared.loading_since.store(0, Release);
    shared.beat();
    if shared.abandoned.load(Acquire) {
        return;
    }
    if let Err(e) = loaded {
        rec.fail(&e, out);
        send_all(sink, shared, out);
        return;
    }
    shared.set_status(rec.status());
    if !send_all(sink, shared, out) {
        return;
    }
    let mut resets_seen = 0u32;
    let mut since_status = 0u64;
    loop {
        shared.beat();
        // Requests are honoured in order: a reset requested before a stop
        // happens first.
        let resets = shared.resets.load(Acquire);
        if resets != resets_seen {
            resets_seen = resets;
            rec.reset(out);
        }
        if let Some(reason) = from_code(shared.stop.load(Acquire)) {
            // A reset requested just before the stop is still honoured.
            if shared.resets.load(Acquire) != resets_seen {
                rec.reset(out);
            }
            if reason == RecognitionEndReason::Revoked {
                tap.mark_end(TapEnd::Revoked);
                let _ = tap.try_recv(); // discards the queue
            }
            rec.finish(reason, out);
            send_all(sink, shared, out);
            return;
        }
        while let Ok(h) = hints.try_recv() {
            if let Err(e) = rec.push_hint(&h, out) {
                rec.fail(&e, out);
                send_all(sink, shared, out);
                return;
            }
        }
        match tap.recv_timeout(POLL) {
            Ok(block) => {
                let t0 = Instant::now();
                let res = rec.push_block(&block, out);
                let elapsed = t0.elapsed();
                if let Err(e) = res {
                    rec.fail(&e, out);
                    send_all(sink, shared, out);
                    return;
                }
                rec.record_processing(elapsed.as_secs_f32() * 1_000.0);
                let audio = ce_core::audio::frame::frames_to_duration(
                    u64::from(block.meta.frames),
                    block.meta.sample_rate.max(1),
                );
                let load = if audio.is_zero() {
                    0.0
                } else {
                    elapsed.as_secs_f32() / audio.as_secs_f32()
                };
                rec.note_load(load, tap.backlog(), tap.dropped(), out);
                since_status += 1;
            }
            Err(TapRecvError::Timeout) => {}
            Err(TapRecvError::Ended(e)) => {
                // A reset requested just before the stream ended still happens.
                if shared.resets.load(Acquire) != resets_seen {
                    rec.reset(out);
                }
                rec.note_load(0.0, 0, tap.dropped(), out);
                rec.finish(tap_reason(e), out);
                send_all(sink, shared, out);
                return;
            }
        }
        if !out.is_empty() || since_status >= STATUS_EVERY {
            since_status = 0;
            shared.set_status(rec.status());
        }
        if !send_all(sink, shared, out) {
            return;
        }
    }
}
