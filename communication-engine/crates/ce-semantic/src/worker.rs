//! Supervised semantic worker: one thread per interpreted stream.
//!
//! * **Input**: a bounded queue of transcript revisions, fed by the runtime
//!   with [`SemanticHandle::push`], which never blocks (the control plane is
//!   never delayed by a slow adapter).
//!   * A queued partial revision of the same utterance is replaced by the
//!     newer one (*coalescing*; superseded partials are never interpreted).
//!   * When the queue is full the oldest queued partial is dropped.
//!   * Finals and cancellations are never dropped while there is room for
//!     them (up to twice the capacity). Beyond that the input is dropped and
//!     reported.
//!   * Every drop raises `Degraded(Overload)`.
//! * **Output**: [`SemanticSignal`]s on a bounded Tokio channel; the worker
//!   blocks only itself when it is full.
//! * **Supervision**:
//!   * the body runs under `catch_unwind`: a panic becomes `Failed` and then
//!     `Stopped(Failed)`;
//!   * a heartbeat lets the runtime detect a hung adapter and abandon it
//!     (an abandoned worker sends nothing more);
//!   * stop, reset and confirmation requests are checked between inputs.
//!     `SourceEnded` and `Shutdown` drain the queue first (final transcripts
//!     are interpreted); `Stopped` and `Failed` discard it.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed, Release};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ce_core::ids::AudioStreamId;
use ce_core::semantic::{
    ObservationId, SemanticEndReason, SemanticEvent, SemanticInput, SemanticStatus,
};
use ce_core::speech::RevisionStatus;
use ce_core::{CeError, ErrorCategory};
use tokio::sync::mpsc;

use crate::processor::SemanticProcessor;

/// Messages from semantic workers to the runtime.
#[derive(Clone, Debug, PartialEq)]
pub enum SemanticSignal {
    /// An event.
    Event(SemanticEvent),
    /// The worker finished; final status.
    Ended {
        /// Stream.
        stream: AudioStreamId,
        /// Final status.
        status: Box<SemanticStatus>,
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

fn code(r: SemanticEndReason) -> u8 {
    match r {
        SemanticEndReason::Stopped => 1,
        SemanticEndReason::SourceEnded => 2,
        SemanticEndReason::Failed => 3,
        SemanticEndReason::Shutdown => 4,
    }
}

fn from_code(v: u8) -> Option<SemanticEndReason> {
    Some(match v {
        1 => SemanticEndReason::Stopped,
        2 => SemanticEndReason::SourceEnded,
        3 => SemanticEndReason::Failed,
        4 => SemanticEndReason::Shutdown,
        _ => return None,
    })
}

#[derive(Debug, Default)]
struct Queue {
    items: VecDeque<(SemanticInput, Instant)>,
    coalesced: u64,
    overloads: u32,
    confirms: Vec<ObservationId>,
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
    status: Mutex<SemanticStatus>,
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
    fn set_status(&self, s: SemanticStatus) {
        *self.status.lock().unwrap_or_else(PoisonError::into_inner) = s;
    }
    fn status(&self) -> SemanticStatus {
        self.status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The runtime's handle to a semantic worker.
#[derive(Debug)]
pub struct SemanticHandle {
    stream: AudioStreamId,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    stall_timeout: Duration,
}

impl SemanticHandle {
    /// Stream.
    pub fn stream(&self) -> AudioStreamId {
        self.stream
    }

    /// Offer a revision without blocking.
    pub fn push(&self, input: SemanticInput) -> PushOutcome {
        if self.is_finished() || self.shared.stop.load(Acquire) != 0 {
            return PushOutcome::Dropped;
        }
        let outcome = {
            let mut q = self.shared.queue();
            let partial = input.status == RevisionStatus::Partial;
            if partial {
                if let Some(slot) = q.items.iter_mut().find(|(i, _)| {
                    i.utterance == input.utterance && i.status == RevisionStatus::Partial
                }) {
                    // Keep the original enqueue time: queue delay is measured
                    // from the first revision the slot held.
                    slot.0 = input;
                    q.coalesced += 1;
                    drop(q);
                    self.shared.cv.notify_one();
                    return PushOutcome::Coalesced;
                }
            }
            if q.items.len() < self.shared.capacity {
                q.items.push_back((input, Instant::now()));
                PushOutcome::Queued
            } else if let Some(pos) = q
                .items
                .iter()
                .position(|(i, _)| i.status == RevisionStatus::Partial)
            {
                q.items.remove(pos);
                q.items.push_back((input, Instant::now()));
                q.coalesced += 1;
                q.overloads += 1;
                PushOutcome::QueuedDroppingOlder
            } else if !partial && q.items.len() < self.shared.capacity * 2 {
                q.items.push_back((input, Instant::now()));
                PushOutcome::Queued
            } else {
                q.overloads += 1;
                PushOutcome::Dropped
            }
        };
        self.shared.cv.notify_one();
        outcome
    }

    /// Ask the worker to finish (first request wins).
    pub fn stop(&self, reason: SemanticEndReason) {
        let _ = self
            .shared
            .stop
            .compare_exchange(0, code(reason), AcqRel, Acquire);
        self.shared.cv.notify_one();
    }

    /// The stop reason requested, if any.
    pub fn stop_requested(&self) -> Option<SemanticEndReason> {
        from_code(self.shared.stop.load(Acquire))
    }

    /// Clear the context at the worker's next iteration.
    pub fn request_reset(&self) {
        self.shared.resets.fetch_add(1, AcqRel);
        self.shared.cv.notify_one();
    }

    /// Confirm an observation (host action) at the next iteration.
    pub fn confirm(&self, id: ObservationId) {
        self.shared.queue().confirms.push(id);
        self.shared.cv.notify_one();
    }

    /// Inputs waiting.
    pub fn backlog(&self) -> usize {
        self.shared.queue().items.len()
    }

    /// Latest status.
    pub fn status(&self) -> SemanticStatus {
        self.shared.status()
    }

    /// Whether the thread exited.
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Join an exited thread (never blocks on a running one); `Some(true)`
    /// if it panicked outside supervision.
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
    pub fn abandon(&mut self, reason: SemanticEndReason) {
        self.shared.abandoned.store(true, Release);
        self.stop(reason);
    }
}

/// Start a worker for `processor`.
pub fn spawn_semantic(
    processor: SemanticProcessor,
    sink: mpsc::Sender<SemanticSignal>,
) -> Result<SemanticHandle, CeError> {
    let stream = processor.stream();
    let budget = processor.config().budget.clone();
    let mut initial = processor.status();
    initial.running = true;
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
        .name(format!("ce-semantic-{stream}"))
        .spawn(move || supervise(processor, sink, s2))
        .map_err(|e| {
            CeError::new(
                ErrorCategory::Internal,
                "semantic.spawn_failed",
                "could not start the semantic worker",
            )
            .with_source(e)
        })?;
    Ok(SemanticHandle {
        stream,
        shared,
        thread: Some(thread),
        stall_timeout: Duration::from_millis(budget.stall_timeout_ms),
    })
}

fn send_all(
    sink: &mpsc::Sender<SemanticSignal>,
    shared: &Shared,
    out: &mut Vec<SemanticEvent>,
) -> bool {
    if shared.abandoned.load(Acquire) {
        out.clear();
        return false;
    }
    for e in out.drain(..) {
        if sink.blocking_send(SemanticSignal::Event(e)).is_err() {
            return false;
        }
    }
    true
}

fn supervise(mut p: SemanticProcessor, sink: mpsc::Sender<SemanticSignal>, shared: Arc<Shared>) {
    let stream = p.stream();
    let mut out = Vec::new();
    let outcome = catch_unwind(AssertUnwindSafe(|| run(&mut p, &sink, &shared, &mut out)));
    if outcome.is_err() {
        out.clear();
        let err = CeError::new(
            ErrorCategory::Internal,
            "semantic.worker_panicked",
            "the semantic worker panicked",
        );
        let closed = catch_unwind(AssertUnwindSafe(|| {
            let mut o = Vec::new();
            p.fail(&err, &mut o);
            o
        }));
        match closed {
            Ok(o) => out.extend(o),
            Err(_) => {
                out.push(SemanticEvent::Failed {
                    stream,
                    error: err.report(&ce_core::privacy::RedactionPolicy::DEFAULT),
                });
                out.push(SemanticEvent::Stopped {
                    stream,
                    reason: SemanticEndReason::Failed,
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
        status.ended = Some(SemanticEndReason::Failed);
    }
    shared.set_status(status.clone());
    if !shared.abandoned.load(Acquire) {
        let _ = sink.blocking_send(SemanticSignal::Ended {
            stream,
            status: Box::new(status),
        });
    }
}

fn run(
    p: &mut SemanticProcessor,
    sink: &mpsc::Sender<SemanticSignal>,
    shared: &Shared,
    out: &mut Vec<SemanticEvent>,
) {
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
            p.reset_context(out);
        }
        let (next, coalesced, overloads, confirms) = {
            let mut q = shared.queue();
            if q.items.is_empty()
                && q.confirms.is_empty()
                && shared.stop.load(Acquire) == 0
                && shared.resets.load(Acquire) == resets_seen
            {
                q = shared
                    .cv
                    .wait_timeout(q, POLL)
                    .map(|(g, _)| g)
                    .unwrap_or_else(|e| e.into_inner().0);
            }
            let stop = from_code(shared.stop.load(Acquire));
            let next = match stop {
                Some(SemanticEndReason::SourceEnded | SemanticEndReason::Shutdown) | None => {
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
                std::mem::take(&mut q.coalesced),
                std::mem::take(&mut q.overloads),
                std::mem::take(&mut q.confirms),
            )
        };
        if coalesced > 0 {
            p.note_coalesced(coalesced);
        }
        if overloads > 0 {
            p.note_overload(out);
        }
        for id in confirms {
            // Unknown ids (already retracted) are ignored: confirmation is
            // best effort and reported only when it happens.
            let _ = p.confirm(&id, out);
        }
        if let Some((input, queued)) = next {
            let ms = queued.elapsed().as_secs_f32() * 1_000.0;
            if let Err(e) = p.process(input, ms, out) {
                p.fail(&e, out);
                send_all(sink, shared, out);
                return;
            }
            since_status += 1;
        } else if let Some(reason) = from_code(shared.stop.load(Acquire)) {
            if shared.resets.load(Acquire) != resets_seen {
                p.reset_context(out);
            }
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
