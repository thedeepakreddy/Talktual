//! Supervised analysis worker: one thread per analysed stream.
//!
//! * Input: a [`TapReceiver`] (bounded; the capture side drops instead of
//!   blocking, so analysis can never delay capture or host delivery).
//! * Output: [`IntelSignal`]s on the runtime's channel. Events are sparse;
//!   the worker blocks on a full channel (only itself — the tap absorbs the
//!   backlog and then drops).
//! * Supervision: the body runs under `catch_unwind`; a panic becomes
//!   `Failed` + `Stopped(Failed)`. A heartbeat lets the runtime detect a
//!   stall (`stalled`). Stop is a flag checked between blocks.
//! * Load: processing time per block over the block's audio duration feeds
//!   the resource governor.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed};
use std::sync::atomic::{AtomicU8, AtomicU64};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ce_audio::{TapEnd, TapReceiver, TapRecvError};
use ce_core::ids::AudioStreamId;
use ce_core::intel::{AnalysisConfig, AnalysisEndReason, AnalysisEvent, AnalysisStatus};
use ce_core::{CeError, ErrorCategory};
use tokio::sync::mpsc;

use crate::analyzer::StreamAnalyzer;
use crate::vad::VoiceActivityDetector;

/// Messages from analysis workers to the runtime.
#[derive(Clone, Debug, PartialEq)]
pub enum IntelSignal {
    /// An analysis event.
    Event(AnalysisEvent),
    /// The worker finished; final status.
    Ended {
        /// Stream.
        stream: AudioStreamId,
        /// Final status.
        status: Box<AnalysisStatus>,
    },
}

/// How long the worker waits for a block before re-checking its flags.
const POLL: Duration = Duration::from_millis(20);
/// Status snapshot refresh interval in blocks (also refreshed on events).
const STATUS_EVERY: u64 = 25;

fn reason_code(r: AnalysisEndReason) -> u8 {
    match r {
        AnalysisEndReason::Stopped => 1,
        AnalysisEndReason::StreamClosed => 2,
        AnalysisEndReason::Revoked => 3,
        AnalysisEndReason::StreamInvalidated => 4,
        AnalysisEndReason::StreamFailed => 5,
        AnalysisEndReason::Failed => 6,
        AnalysisEndReason::Shutdown => 7,
    }
}

fn reason_from(v: u8) -> Option<AnalysisEndReason> {
    Some(match v {
        1 => AnalysisEndReason::Stopped,
        2 => AnalysisEndReason::StreamClosed,
        3 => AnalysisEndReason::Revoked,
        4 => AnalysisEndReason::StreamInvalidated,
        5 => AnalysisEndReason::StreamFailed,
        6 => AnalysisEndReason::Failed,
        7 => AnalysisEndReason::Shutdown,
        _ => return None,
    })
}

fn tap_reason(e: TapEnd) -> AnalysisEndReason {
    match e {
        TapEnd::Closed => AnalysisEndReason::StreamClosed,
        TapEnd::Revoked => AnalysisEndReason::Revoked,
        TapEnd::Invalidated => AnalysisEndReason::StreamInvalidated,
        TapEnd::Failed => AnalysisEndReason::StreamFailed,
    }
}

#[derive(Debug)]
struct Shared {
    stop: AtomicU8,
    heartbeat_ms: AtomicU64,
    status: Mutex<AnalysisStatus>,
    origin: Instant,
}

impl Shared {
    fn beat(&self) {
        self.heartbeat_ms
            .store(self.origin.elapsed().as_millis() as u64, Relaxed);
    }

    fn set_status(&self, s: AnalysisStatus) {
        *self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = s;
    }
}

/// The runtime's handle to an analysis worker.
#[derive(Debug)]
pub struct AnalysisHandle {
    stream: AudioStreamId,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    stall_timeout: Duration,
}

impl AnalysisHandle {
    /// Stream analysed.
    pub fn stream(&self) -> AudioStreamId {
        self.stream
    }

    /// Ask the worker to finish with `reason` (first request wins). Does not
    /// block.
    pub fn stop(&self, reason: AnalysisEndReason) {
        let _ = self
            .shared
            .stop
            .compare_exchange(0, reason_code(reason), AcqRel, Acquire);
    }

    /// The stop reason requested, if any.
    pub fn stop_requested(&self) -> Option<AnalysisEndReason> {
        reason_from(self.shared.stop.load(Acquire))
    }

    /// Latest status snapshot (refreshed every few blocks and on events).
    pub fn status(&self) -> AnalysisStatus {
        self.shared
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
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

    /// Whether the worker has not made progress for the stall timeout.
    pub fn stalled(&self) -> bool {
        if self.is_finished() {
            return false;
        }
        let last = self.shared.heartbeat_ms.load(Relaxed);
        let now = self.shared.origin.elapsed().as_millis() as u64;
        now.saturating_sub(last) > self.stall_timeout.as_millis() as u64
    }

    /// Give up on a stalled thread: request a stop and detach it.
    pub fn abandon(&mut self) {
        self.stop(AnalysisEndReason::Failed);
        self.thread.take();
    }
}

/// Start analysing `tap` on a new thread. `detector` overrides the
/// configured detector (tests, fault injection).
pub fn spawn_analysis(
    stream: AudioStreamId,
    cfg: AnalysisConfig,
    tap: TapReceiver,
    sink: mpsc::Sender<IntelSignal>,
    detector: Option<Box<dyn VoiceActivityDetector>>,
) -> Result<AnalysisHandle, CeError> {
    let stall_timeout = Duration::from_millis(cfg.budget.stall_timeout_ms);
    let analyzer = match detector {
        Some(d) => StreamAnalyzer::with_detector(stream, cfg, d)?,
        None => StreamAnalyzer::new(stream, cfg)?,
    };
    let shared = Arc::new(Shared {
        stop: AtomicU8::new(0),
        heartbeat_ms: AtomicU64::new(0),
        status: Mutex::new(analyzer.status(true)),
        origin: Instant::now(),
    });
    let s2 = shared.clone();
    let thread = std::thread::Builder::new()
        .name(format!("ce-intel-{stream}"))
        .spawn(move || supervise(analyzer, tap, sink, s2))
        .map_err(|e| {
            CeError::new(
                ErrorCategory::Internal,
                "analysis.spawn_failed",
                "could not start the analysis worker",
            )
            .with_source(e)
        })?;
    Ok(AnalysisHandle {
        stream,
        shared,
        thread: Some(thread),
        stall_timeout,
    })
}

fn send_all(sink: &mpsc::Sender<IntelSignal>, out: &mut Vec<AnalysisEvent>) -> bool {
    for e in out.drain(..) {
        if sink.blocking_send(IntelSignal::Event(e)).is_err() {
            return false;
        }
    }
    true
}

fn supervise(
    mut analyzer: StreamAnalyzer,
    tap: TapReceiver,
    sink: mpsc::Sender<IntelSignal>,
    shared: Arc<Shared>,
) {
    let stream = analyzer.stream();
    let mut out = Vec::new();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        run(&mut analyzer, &tap, &sink, &shared, &mut out)
    }));
    if outcome.is_err() {
        out.clear();
        let err = CeError::new(
            ErrorCategory::Internal,
            "analysis.worker_panicked",
            "the analysis worker panicked",
        );
        // The analyzer may be mid-update; closing it must not take the
        // supervisor down with it.
        let closed = catch_unwind(AssertUnwindSafe(|| {
            let mut o = Vec::new();
            analyzer.fail(&err, &mut o);
            o
        }));
        match closed {
            Ok(o) => out.extend(o),
            Err(_) => {
                out.push(AnalysisEvent::Failed {
                    stream,
                    error: err.report(&ce_core::privacy::RedactionPolicy::DEFAULT),
                });
                out.push(AnalysisEvent::Stopped {
                    stream,
                    reason: AnalysisEndReason::Failed,
                    counters: Default::default(),
                });
            }
        }
        send_all(&sink, &mut out);
    }
    let mut status =
        catch_unwind(AssertUnwindSafe(|| analyzer.status(false))).unwrap_or_else(|_| {
            shared
                .status
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        });
    status.running = false;
    if status.ended.is_none() {
        status.ended = Some(AnalysisEndReason::Failed);
    }
    shared.set_status(status.clone());
    let _ = sink.blocking_send(IntelSignal::Ended {
        stream,
        status: Box::new(status),
    });
}

fn run(
    analyzer: &mut StreamAnalyzer,
    tap: &TapReceiver,
    sink: &mpsc::Sender<IntelSignal>,
    shared: &Shared,
    out: &mut Vec<AnalysisEvent>,
) {
    analyzer.start(out);
    if !send_all(sink, out) {
        return;
    }
    let mut since_status = 0u64;
    loop {
        shared.beat();
        if let Some(reason) = reason_from(shared.stop.load(Acquire)) {
            if reason == AnalysisEndReason::Revoked {
                tap.mark_end(TapEnd::Revoked);
                let _ = tap.try_recv(); // discards the queue
            }
            analyzer.finish(reason, out);
            send_all(sink, out);
            return;
        }
        match tap.recv_timeout(POLL) {
            Ok(block) => {
                let t0 = Instant::now();
                let res = analyzer.push_block(&block, out);
                let elapsed = t0.elapsed();
                if let Err(e) = res {
                    analyzer.fail(&e, out);
                    send_all(sink, out);
                    return;
                }
                let audio = ce_core::audio::frame::frames_to_duration(
                    u64::from(block.meta.frames),
                    block.meta.sample_rate.max(1),
                );
                let load = if audio.is_zero() {
                    0.0
                } else {
                    elapsed.as_secs_f32() / audio.as_secs_f32()
                };
                analyzer.set_tap_dropped(tap.dropped());
                analyzer.note_load(load, tap.backlog(), out);
                since_status += 1;
            }
            Err(TapRecvError::Timeout) => {}
            Err(TapRecvError::Ended(e)) => {
                analyzer.set_tap_dropped(tap.dropped());
                analyzer.finish(tap_reason(e), out);
                send_all(sink, out);
                return;
            }
        }
        if !out.is_empty() || since_status >= STATUS_EVERY {
            since_status = 0;
            shared.set_status(analyzer.status(true));
        }
        if !send_all(sink, out) {
            return;
        }
    }
}
