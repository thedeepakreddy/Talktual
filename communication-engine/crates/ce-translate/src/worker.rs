//! Supervised translation worker: one thread per translated stream.
//!
//! * **Input**: a bounded queue of transcript revisions, fed by the runtime
//!   with [`TranslationHandle::push`], which never blocks.
//!   * A queued partial of the same utterance is replaced by the newer one
//!     (*coalescing*: superseded partials are never translated).
//!   * When the queue is full the oldest queued partial is dropped.
//!   * Finals and cancellations may use up to twice the capacity; beyond
//!     that the input is dropped and reported.
//!   * Every drop raises `Degraded(Overload)`.
//! * **Hints**: final Phase 5 observations via [`TranslationHandle::observe`]
//!   (bounded, oldest dropped).
//! * **Output**: [`TranslationSignal`]s on a bounded Tokio channel; the
//!   worker blocks only itself when it is full.
//! * **Supervision**: adapters load on this thread; the body runs under
//!   `catch_unwind` (panic → `Failed` + `Stopped(Failed)`); a heartbeat lets
//!   the runtime detect a hung adapter and abandon the worker (an abandoned
//!   worker sends nothing more); stop and reset are checked between inputs.
//!   `SourceEnded` and `Shutdown` drain the queue first; `Stopped` and
//!   `Failed` discard it.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed, Release};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ce_core::ids::AudioStreamId;
use ce_core::semantic::{ObservationStatus, SemanticObservation};
use ce_core::speech::{RevisionStatus, TranscriptRevision};
use ce_core::translation::{TranslationEndReason, TranslationEvent, TranslationStatus};
use ce_core::{CeError, ErrorCategory};
use tokio::sync::mpsc;

use crate::processor::TranslationProcessor;

/// Messages from translation workers to the runtime.
#[derive(Clone, Debug, PartialEq)]
pub enum TranslationSignal {
    /// An event.
    Event(TranslationEvent),
    /// The worker finished; final status.
    Ended {
        /// Stream.
        stream: AudioStreamId,
        /// Final status.
        status: Box<TranslationStatus>,
    },
}

/// What happened to a pushed input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushOutcome {
    /// Queued.
    Queued,
    /// Replaced a queued partial of the same utterance.
    Coalesced,
    /// Queued after dropping an older queued partial (overload).
    QueuedDroppingOlder,
    /// Dropped (overload or the worker is gone).
    Dropped,
}

const POLL: Duration = Duration::from_millis(20);
const STATUS_EVERY: u32 = 16;
const MAX_PENDING_HINTS: usize = 64;

fn code(r: TranslationEndReason) -> u8 {
    match r {
        TranslationEndReason::Stopped => 1,
        TranslationEndReason::SourceEnded => 2,
        TranslationEndReason::Failed => 3,
        TranslationEndReason::Shutdown => 4,
    }
}

fn from_code(v: u8) -> Option<TranslationEndReason> {
    Some(match v {
        1 => TranslationEndReason::Stopped,
        2 => TranslationEndReason::SourceEnded,
        3 => TranslationEndReason::Failed,
        4 => TranslationEndReason::Shutdown,
        _ => return None,
    })
}

#[derive(Debug, Default)]
struct Queue {
    items: VecDeque<(Box<TranscriptRevision>, Instant)>,
    hints: VecDeque<SemanticObservation>,
    coalesced: u64,
    dropped: u64,
}

#[derive(Debug)]
struct Shared {
    queue: Mutex<Queue>,
    cv: Condvar,
    capacity: usize,
    stop: AtomicU8,
    resets: AtomicU32,
    abandoned: AtomicBool,
    heartbeat_ms: AtomicU64,
    status: Mutex<TranslationStatus>,
    origin: Instant,
}

impl Shared {
    fn now_ms(&self) -> u64 {
        self.origin.elapsed().as_millis() as u64
    }
    fn beat(&self) {
        self.heartbeat_ms.store(self.now_ms(), Relaxed);
    }
    fn queue(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn set_status(&self, s: TranslationStatus) {
        *self.status.lock().unwrap_or_else(PoisonError::into_inner) = s;
    }
    fn status(&self) -> TranslationStatus {
        self.status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The runtime's handle to a translation worker.
#[derive(Debug)]
pub struct TranslationHandle {
    stream: AudioStreamId,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    stall_timeout: Duration,
}

impl TranslationHandle {
    /// Stream.
    pub fn stream(&self) -> AudioStreamId {
        self.stream
    }

    /// Offer a transcript revision without blocking.
    pub fn push(&self, revision: TranscriptRevision) -> PushOutcome {
        if self.is_finished() || self.shared.stop.load(Acquire) != 0 {
            return PushOutcome::Dropped;
        }
        let outcome = {
            let mut q = self.shared.queue();
            let partial = revision.status == RevisionStatus::Partial;
            if partial {
                if let Some(slot) = q.items.iter_mut().find(|(i, _)| {
                    i.meta.utterance == revision.meta.utterance
                        && i.status == RevisionStatus::Partial
                }) {
                    // Keep the original enqueue time (queue delay from the
                    // first revision the slot held). Never replace a newer
                    // revision with an older one.
                    if revision.revision > slot.0.revision {
                        *slot.0 = revision;
                    }
                    q.coalesced += 1;
                    drop(q);
                    self.shared.cv.notify_one();
                    return PushOutcome::Coalesced;
                }
            }
            if q.items.len() < self.shared.capacity {
                q.items.push_back((Box::new(revision), Instant::now()));
                PushOutcome::Queued
            } else if let Some(pos) = q
                .items
                .iter()
                .position(|(i, _)| i.status == RevisionStatus::Partial)
            {
                q.items.remove(pos);
                q.items.push_back((Box::new(revision), Instant::now()));
                q.dropped += 1;
                PushOutcome::QueuedDroppingOlder
            } else if !partial && q.items.len() < self.shared.capacity * 2 {
                q.items.push_back((Box::new(revision), Instant::now()));
                PushOutcome::Queued
            } else {
                q.dropped += 1;
                PushOutcome::Dropped
            }
        };
        self.shared.cv.notify_one();
        outcome
    }

    /// Offer a Phase 5 observation as a potential hint (bounded; never
    /// blocks).
    pub fn observe(&self, observation: SemanticObservation) {
        // Only finals (admissible hints) and retractions (which remove one)
        // matter; dropping the rest here keeps a burst of provisional
        // observations from evicting pending finals.
        if !matches!(
            observation.status,
            ObservationStatus::Final | ObservationStatus::Retracted
        ) {
            return;
        }
        let mut q = self.shared.queue();
        if q.hints.len() == MAX_PENDING_HINTS {
            q.hints.pop_front();
        }
        q.hints.push_back(observation);
    }

    /// Ask the worker to finish (first request wins).
    pub fn stop(&self, reason: TranslationEndReason) {
        let _ = self
            .shared
            .stop
            .compare_exchange(0, code(reason), AcqRel, Acquire);
        self.shared.cv.notify_one();
    }

    /// The stop reason requested, if any.
    pub fn stop_requested(&self) -> Option<TranslationEndReason> {
        from_code(self.shared.stop.load(Acquire))
    }

    /// Discard pending work and withdraw open results at the next iteration.
    pub fn request_reset(&self) {
        self.shared.resets.fetch_add(1, AcqRel);
        self.shared.cv.notify_one();
    }

    /// Inputs waiting.
    pub fn backlog(&self) -> usize {
        self.shared.queue().items.len()
    }

    /// Latest status.
    pub fn status(&self) -> TranslationStatus {
        let mut s = self.shared.status();
        s.backlog = self.backlog() as u32;
        s
    }

    /// Whether the thread exited.
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

    /// No progress for the stall timeout.
    pub fn stalled(&self) -> bool {
        !self.is_finished()
            && self
                .shared
                .now_ms()
                .saturating_sub(self.shared.heartbeat_ms.load(Relaxed))
                > self.stall_timeout.as_millis() as u64
    }

    /// Give up on the worker: it sends nothing more.
    pub fn abandon(&mut self, reason: TranslationEndReason) {
        self.shared.abandoned.store(true, Release);
        self.stop(reason);
    }
}

/// Start a worker for `processor`.
pub fn spawn_translation(
    processor: TranslationProcessor,
    sink: mpsc::Sender<TranslationSignal>,
) -> Result<TranslationHandle, CeError> {
    let stream = processor.stream();
    let budget = processor.config().budget.clone();
    let initial = processor.status();
    let shared = Arc::new(Shared {
        queue: Mutex::new(Queue::default()),
        cv: Condvar::new(),
        capacity: budget.queue as usize,
        stop: AtomicU8::new(0),
        resets: AtomicU32::new(0),
        abandoned: AtomicBool::new(false),
        heartbeat_ms: AtomicU64::new(0),
        status: Mutex::new(initial),
        origin: Instant::now(),
    });
    let s2 = shared.clone();
    let thread = std::thread::Builder::new()
        .name(format!("ce-translate-{stream}"))
        .spawn(move || supervise(processor, sink, s2))
        .map_err(|e| {
            CeError::new(
                ErrorCategory::Internal,
                "translation.spawn_failed",
                "could not start the translation worker",
            )
            .with_source(e)
        })?;
    Ok(TranslationHandle {
        stream,
        shared,
        thread: Some(thread),
        stall_timeout: Duration::from_millis(budget.stall_timeout_ms),
    })
}

fn send_all(
    sink: &mpsc::Sender<TranslationSignal>,
    shared: &Shared,
    out: &mut Vec<TranslationEvent>,
) -> bool {
    if shared.abandoned.load(Acquire) {
        out.clear();
        return false;
    }
    for e in out.drain(..) {
        if sink.blocking_send(TranslationSignal::Event(e)).is_err() {
            return false;
        }
    }
    true
}

fn supervise(
    mut p: TranslationProcessor,
    sink: mpsc::Sender<TranslationSignal>,
    shared: Arc<Shared>,
) {
    let stream = p.stream();
    let mut out = Vec::new();
    let outcome = catch_unwind(AssertUnwindSafe(|| run(&mut p, &sink, &shared, &mut out)));
    if outcome.is_err() {
        out.clear();
        let err = CeError::new(
            ErrorCategory::Internal,
            "translation.worker_panicked",
            "the translation worker panicked",
        );
        let closed = catch_unwind(AssertUnwindSafe(|| {
            let mut o = Vec::new();
            p.fail(&err, &mut o);
            o
        }));
        match closed {
            Ok(o) => out.extend(o),
            Err(_) => {
                out.push(TranslationEvent::Failed {
                    stream,
                    error: err.report(&ce_core::privacy::RedactionPolicy::DEFAULT),
                });
                out.push(TranslationEvent::Stopped {
                    stream,
                    reason: TranslationEndReason::Failed,
                    counters: Box::default(),
                });
            }
        }
        send_all(&sink, &shared, &mut out);
    }
    let mut status =
        catch_unwind(AssertUnwindSafe(|| p.status())).unwrap_or_else(|_| shared.status());
    status.running = false;
    if status.ended.is_none() {
        status.ended = Some(TranslationEndReason::Failed);
    }
    shared.set_status(status.clone());
    if !shared.abandoned.load(Acquire) {
        let _ = sink.blocking_send(TranslationSignal::Ended {
            stream,
            status: Box::new(status),
        });
    }
}

fn run(
    p: &mut TranslationProcessor,
    sink: &mpsc::Sender<TranslationSignal>,
    shared: &Shared,
    out: &mut Vec<TranslationEvent>,
) {
    shared.beat();
    if let Err(e) = p.load() {
        p.fail(&e, out);
        send_all(sink, shared, out);
        return;
    }
    shared.beat();
    p.start(out);
    shared.set_status(p.status());
    if !send_all(sink, shared, out) {
        return;
    }
    let mut resets_seen = 0u32;
    let mut since_status = 0u32;
    loop {
        shared.beat();
        let resets = shared.resets.load(Acquire);
        if resets != resets_seen {
            resets_seen = resets;
            let discarded = {
                let mut q = shared.queue();
                let n = q.items.len() as u64;
                q.items.clear();
                q.hints.clear();
                n
            };
            p.note_coalesced(discarded);
            p.reset(out);
        }
        let (next, hints, coalesced, dropped, backlog) = {
            let mut q = shared.queue();
            if q.items.is_empty()
                && shared.stop.load(Acquire) == 0
                && shared.resets.load(Acquire) == resets_seen
            {
                q = shared
                    .cv
                    .wait_timeout(q, POLL)
                    .map(|(g, _)| g)
                    .unwrap_or_else(|e| e.into_inner().0);
            }
            let hints: Vec<SemanticObservation> = q.hints.drain(..).collect();
            let next = match from_code(shared.stop.load(Acquire)) {
                Some(TranslationEndReason::SourceEnded | TranslationEndReason::Shutdown) | None => {
                    q.items.pop_front()
                }
                Some(_) => {
                    let n = q.items.len() as u64;
                    q.items.clear();
                    q.coalesced += n;
                    None
                }
            };
            (
                next,
                hints,
                std::mem::take(&mut q.coalesced),
                std::mem::take(&mut q.dropped),
                q.items.len() as u32,
            )
        };
        for h in &hints {
            p.observe(h);
        }
        if coalesced > 0 {
            p.note_coalesced(coalesced);
        }
        if dropped > 0 {
            p.note_overload(dropped, out);
        }
        p.set_backlog(backlog);
        if let Some((rev, queued)) = next {
            let ms = queued.elapsed().as_secs_f32() * 1_000.0;
            if let Err(e) = p.process(&rev, ms, out) {
                p.fail(&e, out);
                send_all(sink, shared, out);
                return;
            }
            since_status += 1;
        } else if let Some(reason) = from_code(shared.stop.load(Acquire)) {
            p.finish(reason, out);
            send_all(sink, shared, out);
            return;
        }
        shared.beat();
        if !out.is_empty() || since_status >= STATUS_EVERY {
            since_status = 0;
            shared.set_status(p.status());
        }
        if !send_all(sink, shared, out) {
            return;
        }
    }
}
