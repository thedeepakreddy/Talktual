//! Audio-intelligence orchestration inside the runtime core (Phase 3).
//!
//! The core owns which streams are analysed. Each analysis runs on its own
//! supervised thread (`ce-audio-intel`), fed by a bounded, non-blocking tap
//! on the capture worker: analysis can fall behind or fail without touching
//! capture, host delivery or the control plane.
//!
//! * Events arrive on one channel and are published as
//!   `EventPayload::Analysis` with the runtime's single sequence, in the
//!   stream's session.
//! * Stream end (close, revoke, invalidation, failure) reaches the analysis
//!   through the tap itself, so the end reason is exact and a revocation
//!   discards queued audio.
//! * Housekeeping detects stalled workers (no heartbeat within
//!   `stall_timeout_ms`), reports them `Failed` and abandons them.
//! * Shutdown stops every analysis first and waits (bounded) for each to
//!   report its end.

use std::collections::BTreeMap;

use ce_audio::analysis_tap;
use ce_audio_intel::VoiceActivityDetector;
use ce_audio_intel::worker::{AnalysisHandle, IntelSignal, spawn_analysis};
use ce_core::CeResult;
use ce_core::audio::{AudioDirection, AudioStreamState};
use ce_core::command::CommandOutcome;
use ce_core::error::{CeError, ErrorCategory};
use ce_core::event::EventPayload;
use ce_core::ids::{AudioStreamId, SessionId};
use ce_core::intel::{AnalysisConfig, AnalysisEndReason, AnalysisEvent, AnalysisStatus};
use ce_core::observability::{Level, Observation};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::Core;
use crate::runtime::metrics::Metrics;

/// Capacity of the analysis event channel.
pub(crate) const INTEL_SIGNAL_CAPACITY: usize = 1024;
/// Ended analyses kept for `QueryAnalysis`/health.
const FINISHED_KEPT: usize = 16;

struct Running {
    handle: AnalysisHandle,
    session: SessionId,
}

/// Analysis bookkeeping owned by the core.
pub(crate) struct IntelState {
    tx: mpsc::Sender<IntelSignal>,
    running: BTreeMap<AudioStreamId, Running>,
    finished: BTreeMap<AudioStreamId, (SessionId, AnalysisStatus)>,
    finished_order: Vec<AudioStreamId>,
    abandoned: Vec<AnalysisHandle>,
}

impl std::fmt::Debug for IntelState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntelState")
            .field("running", &self.running.len())
            .field("finished", &self.finished.len())
            .field("abandoned", &self.abandoned.len())
            .finish_non_exhaustive()
    }
}

impl IntelState {
    pub(crate) fn new(tx: mpsc::Sender<IntelSignal>) -> Self {
        Self {
            tx,
            running: BTreeMap::new(),
            finished: BTreeMap::new(),
            finished_order: Vec::new(),
            abandoned: Vec::new(),
        }
    }

    fn remember(&mut self, stream: AudioStreamId, session: SessionId, status: AnalysisStatus) {
        self.finished_order.retain(|s| *s != stream);
        self.finished_order.push(stream);
        self.finished.insert(stream, (session, status));
        while self.finished_order.len() > FINISHED_KEPT {
            let old = self.finished_order.remove(0);
            self.finished.remove(&old);
        }
    }
}

fn no_analysis(id: AudioStreamId) -> CeError {
    CeError::invalid_input("analysis.not_found", "no analysis exists for this stream")
        .with_context("stream", id.to_string())
}

impl Core {
    fn intel_mut(&mut self) -> CeResult<&mut IntelState> {
        self.intel.as_mut().ok_or_else(|| {
            CeError::new(
                ErrorCategory::CapabilityResolution,
                "analysis.unavailable",
                "audio analysis needs an audio backend",
            )
        })
    }

    /// Start analysing a capture stream.
    pub(super) fn start_analysis(
        &mut self,
        id: AudioStreamId,
        config: Option<AnalysisConfig>,
        detector: Option<Box<dyn VoiceActivityDetector>>,
    ) -> CeResult<CommandOutcome> {
        let cfg = config.unwrap_or_else(|| self.config.analysis.clone());
        cfg.validate()?;
        let session = {
            let audio = self.audio.as_ref().ok_or_else(|| {
                CeError::new(
                    ErrorCategory::CapabilityResolution,
                    "analysis.unavailable",
                    "audio analysis needs an audio backend",
                )
            })?;
            let (info, state, ended) = audio.stream(id).ok_or_else(|| {
                CeError::invalid_input("audio.unknown_stream", "no such audio stream")
                    .with_context("stream", id.to_string())
            })?;
            if info.direction != AudioDirection::Capture {
                return Err(CeError::invalid_input(
                    "analysis.not_capture",
                    "only capture streams can be analysed",
                ));
            }
            if ended
                || matches!(
                    state,
                    AudioStreamState::Closed
                        | AudioStreamState::Failed
                        | AudioStreamState::Invalidated
                )
            {
                return Err(CeError::lifecycle(
                    "analysis.stream_ended",
                    "the stream has ended",
                ));
            }
            info.session_id
        };
        let intel = self.intel_mut()?;
        if intel.running.contains_key(&id) {
            return Err(CeError::lifecycle(
                "analysis.already_running",
                "this stream is already being analysed",
            ));
        }
        let tx = intel.tx.clone();
        let (tap_tx, tap_rx) = analysis_tap(cfg.budget.queue_blocks as usize);
        let handle = spawn_analysis(id, cfg, tap_rx, tx, detector)?;
        let attached = self
            .audio
            .as_ref()
            .map(|a| a.attach_tap(id, tap_tx).map(|_| ()))
            .unwrap_or_else(|| {
                Err(CeError::internal(
                    "analysis.no_audio",
                    "audio state vanished",
                ))
            });
        if let Err(e) = attached {
            handle.stop(AnalysisEndReason::StreamClosed);
            if let Some(intel) = self.intel.as_mut() {
                intel.abandoned.push(handle);
            }
            return Err(e);
        }
        let status = handle.status();
        self.intel_mut()?
            .running
            .insert(id, Running { handle, session });
        Metrics::inc(&self.metrics.analyses_started, 1);
        self.log(
            Observation::log(Level::Info, "analysis.started")
                .field("stream", id)
                .field("vad", status.algorithm.vad.as_str())
                .field("config", status.algorithm.config_fingerprint.as_str()),
        );
        Ok(CommandOutcome::Analysis { status })
    }

    /// Stop analysing a stream; waits (bounded) for the final status.
    pub(super) async fn stop_analysis(&mut self, id: AudioStreamId) -> CeResult<CommandOutcome> {
        {
            let intel = self.intel_mut()?;
            match intel.running.get(&id) {
                Some(r) => r.handle.stop(AnalysisEndReason::Stopped),
                None => {
                    return match intel.finished.get(&id) {
                        Some((_, s)) => Ok(CommandOutcome::Analysis { status: s.clone() }),
                        None => Err(no_analysis(id)),
                    };
                }
            }
        }
        let deadline = Instant::now() + self.config.audio.operation_timeout();
        self.await_analysis_end(&[id], deadline).await;
        self.query_analysis(id)
    }

    /// Whether an analysis is running on `id`.
    pub(super) fn analysis_running(&self, id: AudioStreamId) -> bool {
        self.intel
            .as_ref()
            .is_some_and(|i| i.running.contains_key(&id))
    }

    /// Current or final status.
    pub(super) fn query_analysis(&mut self, id: AudioStreamId) -> CeResult<CommandOutcome> {
        let intel = self.intel_mut()?;
        if let Some(r) = intel.running.get(&id) {
            return Ok(CommandOutcome::Analysis {
                status: r.handle.status(),
            });
        }
        intel
            .finished
            .get(&id)
            .map(|(_, s)| CommandOutcome::Analysis { status: s.clone() })
            .ok_or_else(|| no_analysis(id))
    }

    /// Process one analysis signal.
    pub(super) fn handle_intel_signal(&mut self, signal: IntelSignal) {
        match signal {
            IntelSignal::Event(event) => {
                let stream = event.stream();
                let session = self.intel.as_ref().and_then(|i| {
                    i.running
                        .get(&stream)
                        .map(|r| r.session)
                        .or_else(|| i.finished.get(&stream).map(|f| f.0))
                });
                self.forward_hint(&event);
                if matches!(event, AnalysisEvent::Failed { .. }) {
                    Metrics::inc(&self.metrics.analysis_failures, 1);
                    self.log(
                        Observation::log(Level::Warn, "analysis.failed").field("stream", stream),
                    );
                }
                self.emit(EventPayload::Analysis(event), session);
            }
            IntelSignal::Ended { stream, status } => {
                let Some(intel) = self.intel.as_mut() else {
                    return;
                };
                if let Some(mut r) = intel.running.remove(&stream) {
                    let _ = r.handle.join_if_finished();
                    if !r.handle.is_finished() {
                        intel.abandoned.push(r.handle);
                    }
                    let reason = status.ended;
                    intel.remember(stream, r.session, *status);
                    self.log(
                        Observation::log(Level::Info, "analysis.ended")
                            .field("stream", stream)
                            .field("reason", format!("{reason:?}")),
                    );
                }
            }
        }
    }

    /// Housekeeping: reap exited threads, detect stalls.
    pub(super) fn intel_housekeeping(&mut self) {
        let Some(intel) = self.intel.as_mut() else {
            return;
        };
        intel
            .abandoned
            .retain_mut(|h| h.join_if_finished().is_none());
        let stalled: Vec<AudioStreamId> = intel
            .running
            .iter()
            .filter(|(_, r)| r.handle.stalled())
            .map(|(s, _)| *s)
            .collect();
        for stream in stalled {
            let Some(intel) = self.intel.as_mut() else {
                return;
            };
            let Some(mut r) = intel.running.remove(&stream) else {
                continue;
            };
            let mut status = r.handle.status();
            r.handle.abandon();
            status.running = false;
            status.ended = Some(AnalysisEndReason::Failed);
            let counters = status.counters;
            let session = r.session;
            intel.remember(stream, session, status);
            let err = CeError::new(
                ErrorCategory::Timeout,
                "analysis.stalled",
                "the analysis worker stopped making progress",
            );
            let report = err.report(&self.config.runtime.redaction);
            Metrics::inc(&self.metrics.analysis_failures, 1);
            self.log(Observation::log(Level::Warn, "analysis.stalled").field("stream", stream));
            self.emit(
                EventPayload::Analysis(AnalysisEvent::Failed {
                    stream,
                    error: report,
                }),
                Some(session),
            );
            self.emit(
                EventPayload::Analysis(AnalysisEvent::Stopped {
                    stream,
                    reason: AnalysisEndReason::Failed,
                    counters,
                }),
                Some(session),
            );
        }
    }

    /// Wait until each stream in `ids` has reported its end (or the
    /// deadline passes), draining all signals meanwhile.
    async fn await_analysis_end(&mut self, ids: &[AudioStreamId], deadline: Instant) {
        loop {
            let pending = self
                .intel
                .as_ref()
                .is_some_and(|i| ids.iter().any(|s| i.running.contains_key(s)));
            if !pending {
                return;
            }
            tokio::select! {
                biased;
                () = tokio::time::sleep_until(deadline) => break,
                Some(sig) = super::recv_opt(&mut self.intel_rx) => self.handle_intel_signal(sig),
                Some(sig) = super::recv_opt(&mut self.speech_rx) => self.handle_speech_signal(sig),
                Some(ingress) = self.ingress_rx.recv() => self.handle_ingress(ingress),
                Some(signal) = super::recv_opt(&mut self.audio_rx) => self.handle_audio_signal(signal),
            }
        }
        // Whatever did not finish in time is abandoned and reported.
        let late: Vec<AudioStreamId> = self
            .intel
            .as_ref()
            .map(|i| {
                ids.iter()
                    .filter(|s| i.running.contains_key(s))
                    .copied()
                    .collect()
            })
            .unwrap_or_default();
        for stream in late {
            let Some(intel) = self.intel.as_mut() else {
                return;
            };
            let Some(mut r) = intel.running.remove(&stream) else {
                continue;
            };
            let mut status = r.handle.status();
            r.handle.abandon();
            status.running = false;
            status.ended.get_or_insert(AnalysisEndReason::Failed);
            intel.remember(stream, r.session, status);
            Metrics::inc(&self.metrics.analysis_failures, 1);
            self.log(
                Observation::log(Level::Warn, "analysis.stop_timed_out").field("stream", stream),
            );
        }
    }

    /// Stop every analysis of `session` (or all, for `None`) with `reason`
    /// and wait (bounded) for their ends.
    pub(super) async fn stop_analyses(
        &mut self,
        session: Option<SessionId>,
        reason: AnalysisEndReason,
        deadline: Instant,
    ) {
        let ids: Vec<AudioStreamId> = match self.intel.as_ref() {
            Some(i) => i
                .running
                .iter()
                .filter(|(_, r)| session.is_none_or(|s| s == r.session))
                .map(|(s, r)| {
                    r.handle.stop(reason);
                    *s
                })
                .collect(),
            None => return,
        };
        if !ids.is_empty() {
            self.await_analysis_end(&ids, deadline).await;
        }
    }

    /// Statuses for health: running first, then recently finished.
    pub(super) fn analysis_statuses(&self) -> Vec<AnalysisStatus> {
        let Some(i) = self.intel.as_ref() else {
            return Vec::new();
        };
        i.running
            .values()
            .map(|r| r.handle.status())
            .chain(
                i.finished_order
                    .iter()
                    .filter_map(|s| i.finished.get(s).map(|f| f.1.clone())),
            )
            .collect()
    }

    /// Wait (bounded) for abandoned analysis threads. Returns how many are
    /// still running.
    pub(super) async fn drain_intel(&mut self, deadline: Instant) -> usize {
        loop {
            let Some(i) = self.intel.as_mut() else {
                return 0;
            };
            i.abandoned.retain_mut(|h| h.join_if_finished().is_none());
            if i.abandoned.is_empty() || Instant::now() >= deadline {
                return i.abandoned.len();
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}
