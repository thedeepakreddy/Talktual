//! Speech-recognition orchestration inside the runtime core (Phase 4).
//!
//! The core owns which streams are recognized. Each recognition runs on
//! its own supervised thread (`ce-speech`), fed by its own bounded,
//! non-blocking tap on the capture worker and by a bounded hint queue of
//! Phase 3 analysis events that the core forwards with `try_send`:
//! recognition can fall behind, stall or fail without touching capture,
//! playback, analysis or the control plane.
//!
//! * Events are published as `EventPayload::Speech` in the stream's
//!   session, classified by `SpeechEvent::data_class` (transcripts
//!   `Content`, language observations `Personal`).
//! * `ActivityHints` requires a running analysis on the stream.
//! * Concurrency is limited engine-wide (`max_concurrent_recognitions`)
//!   and per adapter (`max_concurrent_streams`).
//! * Housekeeping detects load timeouts and stalls, reports them and
//!   abandons the worker (it sends nothing more).
//! * Session close and shutdown stop recognitions before analyses and
//!   streams, so final transcripts are delivered.

use std::collections::BTreeMap;
use std::sync::Arc;

use ce_audio::analysis_tap;
use ce_core::CeResult;
use ce_core::audio::{AudioDirection, AudioStreamState};
use ce_core::command::CommandOutcome;
use ce_core::error::{CeError, ErrorCategory};
use ce_core::event::EventPayload;
use ce_core::ids::{AudioStreamId, SessionId};
use ce_core::intel::AnalysisEvent;
use ce_core::observability::{Level, Observation};
use ce_core::speech::{
    AsrCapabilities, RecognitionConfig, RecognitionEndReason, RecognitionStatus, SegmentationMode,
    SpeechEvent,
};
use ce_speech::recognizer::RecognizerSetup;
use ce_speech::worker::{RecognitionHandle, SpeechSignal, spawn_recognition};
use ce_speech::{AsrFactory, LanguageIdentifier};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::Core;
use super::pipeline::{Tagged, recognition_end, tagged_sender};
use crate::runtime::metrics::Metrics;
use ce_core::orchestration::{StageEnd, StageKind};

/// Capacity of the speech event channel.
pub(crate) const SPEECH_SIGNAL_CAPACITY: usize = 1024;
/// Ended recognitions kept for queries/health.
const FINISHED_KEPT: usize = 16;

struct Running {
    handle: RecognitionHandle,
    session: SessionId,
    adapter: String,
    generation: u64,
}

/// Recognition bookkeeping owned by the core.
pub(crate) struct SpeechState {
    tx: mpsc::Sender<Tagged<SpeechSignal>>,
    running: BTreeMap<AudioStreamId, Running>,
    finished: BTreeMap<AudioStreamId, (SessionId, RecognitionStatus)>,
    finished_order: Vec<AudioStreamId>,
    abandoned: Vec<RecognitionHandle>,
}

impl std::fmt::Debug for SpeechState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpeechState")
            .field("running", &self.running.len())
            .field("finished", &self.finished.len())
            .field("abandoned", &self.abandoned.len())
            .finish_non_exhaustive()
    }
}

impl SpeechState {
    pub(crate) fn new(tx: mpsc::Sender<Tagged<SpeechSignal>>) -> Self {
        Self {
            tx,
            running: BTreeMap::new(),
            finished: BTreeMap::new(),
            finished_order: Vec::new(),
            abandoned: Vec::new(),
        }
    }

    fn remember(&mut self, stream: AudioStreamId, session: SessionId, status: RecognitionStatus) {
        self.finished_order.retain(|s| *s != stream);
        self.finished_order.push(stream);
        self.finished.insert(stream, (session, status));
        while self.finished_order.len() > FINISHED_KEPT {
            let old = self.finished_order.remove(0);
            self.finished.remove(&old);
        }
    }
}

/// The ASR adapters and language identifier registered on the builder.
#[derive(Clone, Default)]
pub(crate) struct SpeechProviders {
    pub(crate) adapters: Vec<Arc<dyn AsrFactory>>,
    pub(crate) identifier: Option<Arc<dyn LanguageIdentifier>>,
}

impl std::fmt::Debug for SpeechProviders {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpeechProviders")
            .field("adapters", &self.adapters.len())
            .field("identifier", &self.identifier)
            .finish()
    }
}

fn no_recognition(id: AudioStreamId) -> CeError {
    CeError::invalid_input("speech.not_found", "no recognition exists for this stream")
        .with_context("stream", id.to_string())
}

fn unavailable() -> CeError {
    CeError::new(
        ErrorCategory::CapabilityResolution,
        "speech.unavailable",
        "speech recognition needs an audio backend",
    )
}

/// Whether a Phase 3 event is a hint recognitions use.
fn is_hint(e: &AnalysisEvent) -> bool {
    matches!(
        e,
        AnalysisEvent::ActivityChanged { .. }
            | AnalysisEvent::SegmentEnded { .. }
            | AnalysisEvent::EndpointCandidate { .. }
            | AnalysisEvent::DegradedModeChanged { .. }
            | AnalysisEvent::Stopped { .. }
            | AnalysisEvent::Failed { .. }
    )
}

impl Core {
    /// Abandoned workers of this stage whose threads are still alive.
    pub(super) fn speech_abandoned_live(&self) -> usize {
        self.speech
            .as_ref()
            .map(|s| s.abandoned.iter().filter(|h| !h.is_finished()).count())
            .unwrap_or(0)
    }

    fn speech_mut(&mut self) -> CeResult<&mut SpeechState> {
        self.speech.as_mut().ok_or_else(unavailable)
    }

    /// Capabilities of every registered adapter (no model is loaded).
    pub(super) fn list_recognizers(&self) -> CommandOutcome {
        CommandOutcome::Recognizers {
            adapters: self
                .providers
                .adapters
                .iter()
                .map(|f| f.capabilities())
                .collect(),
        }
    }

    pub(super) fn resolve_adapter(
        &self,
        cfg: &RecognitionConfig,
    ) -> CeResult<(Arc<dyn AsrFactory>, AsrCapabilities)> {
        let found = match &cfg.adapter {
            Some(id) => self
                .providers
                .adapters
                .iter()
                .map(|f| (f.clone(), f.capabilities()))
                .find(|(_, c)| &c.adapter == id),
            None => self
                .providers
                .adapters
                .first()
                .map(|f| (f.clone(), f.capabilities())),
        };
        found.ok_or_else(|| {
            let e = CeError::new(
                ErrorCategory::CapabilityResolution,
                "speech.no_recognizer",
                "no matching speech recognizer is registered",
            );
            match &cfg.adapter {
                Some(id) => e.with_context("adapter", id.clone()),
                None => e,
            }
        })
    }

    /// Start recognizing a capture stream.
    pub(super) fn start_recognition(
        &mut self,
        id: AudioStreamId,
        config: Option<RecognitionConfig>,
    ) -> CeResult<CommandOutcome> {
        let cfg = config.unwrap_or_else(|| self.config.speech.recognition.clone());
        cfg.validate()?;
        self.admit_worker_start(StageKind::Recognition)?;
        let generation = self.next_generation();
        let metrics = self.metrics.clone();
        let session = {
            let audio = self.audio.as_ref().ok_or_else(unavailable)?;
            let (info, state, ended) = audio.stream(id).ok_or_else(|| {
                CeError::invalid_input("audio.unknown_stream", "no such audio stream")
                    .with_context("stream", id.to_string())
            })?;
            if info.direction != AudioDirection::Capture {
                return Err(CeError::invalid_input(
                    "speech.not_capture",
                    "only capture streams can be recognized",
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
                    "speech.stream_ended",
                    "the stream has ended",
                ));
            }
            info.session_id
        };
        let (factory, caps) = self.resolve_adapter(&cfg)?;
        let analysis_running = self.analysis_running(id);
        if cfg.segmentation == SegmentationMode::ActivityHints && !analysis_running {
            return Err(CeError::lifecycle(
                "speech.analysis_required",
                "activity-hint segmentation needs a running analysis on the stream",
            ));
        }
        let max = self.config.speech.max_concurrent_recognitions as usize;
        let speech = self.speech_mut()?;
        if speech.running.contains_key(&id) {
            return Err(CeError::lifecycle(
                "speech.already_running",
                "this stream is already being recognized",
            ));
        }
        let same_adapter = speech
            .running
            .values()
            .filter(|r| r.adapter == caps.adapter)
            .count();
        if speech.running.len() >= max
            || caps
                .max_concurrent_streams
                .is_some_and(|m| same_adapter >= m as usize)
        {
            return Err(CeError::new(
                ErrorCategory::QueueSaturated,
                "speech.capacity_exhausted",
                "the maximum number of concurrent recognitions has been reached",
            )
            .with_context("max", max.to_string()));
        }
        let tx = tagged_sender(generation, speech.tx.clone(), metrics);
        let adapter = factory.create()?;
        let (tap_tx, tap_rx) = analysis_tap(cfg.budget.queue_blocks as usize);
        let setup = RecognizerSetup {
            stream: id,
            config: cfg,
            capabilities: caps.clone(),
            identifier: self.providers.identifier.clone(),
            analysis_available: analysis_running,
        };
        let handle = spawn_recognition(setup, adapter, tap_rx, tx)?;
        let attached = self
            .audio
            .as_ref()
            .map(|a| a.attach_tap(id, tap_tx).map(|_| ()))
            .unwrap_or_else(|| Err(CeError::internal("speech.no_audio", "audio state vanished")));
        if let Err(e) = attached {
            let mut handle = handle;
            handle.abandon(RecognitionEndReason::StreamClosed);
            if let Some(s) = self.speech.as_mut() {
                s.abandoned.push(handle);
            }
            self.discard_generation(generation);
            return Err(e);
        }
        let status = handle.status();
        self.speech_mut()?.running.insert(
            id,
            Running {
                handle,
                session,
                adapter: caps.adapter.clone(),
                generation,
            },
        );
        Metrics::inc(&self.metrics.recognitions_started, 1);
        self.log(
            Observation::log(Level::Info, "speech.started")
                .field("stream", id)
                .field("adapter", caps.adapter.as_str())
                .field("config", status.recognizer.config_fingerprint.as_str()),
        );
        Ok(CommandOutcome::Recognition {
            status: Box::new(status),
        })
    }

    /// Stop recognizing a stream (the open utterance is finalized); waits
    /// (bounded) for the final status.
    pub(super) async fn stop_recognition(&mut self, id: AudioStreamId) -> CeResult<CommandOutcome> {
        {
            let speech = self.speech_mut()?;
            match speech.running.get(&id) {
                Some(r) => r.handle.stop(RecognitionEndReason::Stopped),
                None => {
                    return match speech.finished.get(&id) {
                        Some((_, s)) => Ok(CommandOutcome::Recognition {
                            status: Box::new(s.clone()),
                        }),
                        None => Err(no_recognition(id)),
                    };
                }
            }
        }
        let deadline = Instant::now() + self.config.audio.operation_timeout();
        self.await_recognition_end(&[id], deadline).await;
        self.query_recognition(id)
    }

    /// Cancel the open utterance and reset the adapter; recognition goes on.
    pub(super) fn reset_recognition(&mut self, id: AudioStreamId) -> CeResult<CommandOutcome> {
        let speech = self.speech_mut()?;
        let r = speech.running.get(&id).ok_or_else(|| no_recognition(id))?;
        r.handle.request_reset();
        Ok(CommandOutcome::Recognition {
            status: Box::new(r.handle.status()),
        })
    }

    /// Current or final status.
    /// Generation of the recognition worker running on `id`, if any.
    pub(super) fn recognition_generation(&self, id: AudioStreamId) -> Option<u64> {
        self.speech
            .as_ref()
            .and_then(|s| s.running.get(&id))
            .map(|r| r.generation)
    }

    pub(super) fn query_recognition(&mut self, id: AudioStreamId) -> CeResult<CommandOutcome> {
        let speech = self.speech_mut()?;
        if let Some(r) = speech.running.get(&id) {
            return Ok(CommandOutcome::Recognition {
                status: Box::new(r.handle.status()),
            });
        }
        speech
            .finished
            .get(&id)
            .map(|(_, s)| CommandOutcome::Recognition {
                status: Box::new(s.clone()),
            })
            .ok_or_else(|| no_recognition(id))
    }

    /// Forward a Phase 3 event to the stream's recognition (non-blocking;
    /// a full hint queue drops it and the drop is counted).
    pub(super) fn forward_hint(&self, event: &AnalysisEvent) {
        if !is_hint(event) {
            return;
        }
        if let Some(r) = self
            .speech
            .as_ref()
            .and_then(|s| s.running.get(&event.stream()))
        {
            r.handle.send_hint(event);
        }
    }

    /// Process one speech signal.
    pub(super) fn handle_speech_signal(&mut self, tagged: Tagged<SpeechSignal>) {
        let Tagged { generation, signal } = tagged;
        let stream = match &signal {
            SpeechSignal::Event(e) => e.stream(),
            SpeechSignal::Ended { stream, .. } => *stream,
        };
        let current = self.recognition_generation(stream);
        if !self.admit_signal(StageKind::Recognition, stream, generation, current) {
            return;
        }
        match signal {
            SpeechSignal::Event(event) => {
                let stream = event.stream();
                let session = self.speech.as_ref().and_then(|s| {
                    s.running
                        .get(&stream)
                        .map(|r| r.session)
                        .or_else(|| s.finished.get(&stream).map(|f| f.0))
                });
                if matches!(event, SpeechEvent::Failed { .. }) {
                    Metrics::inc(&self.metrics.recognition_failures, 1);
                    self.log(
                        Observation::log(Level::Warn, "speech.failed").field("stream", stream),
                    );
                }
                // Phase 5: transcripts are handed to the stream's semantic
                // worker (non-blocking) before publication; when the
                // recognition stops, the worker drains and ends.
                // Phase 6: and to its translation worker (read-only copy).
                match &event {
                    SpeechEvent::Transcript(rev) => {
                        self.forward_transcript(rev);
                        self.forward_transcript_to_translation(rev);
                    }
                    SpeechEvent::Stopped { .. } => {
                        self.transcripts_ended(stream);
                        self.transcripts_ended_for_translation(stream);
                    }
                    _ => {}
                }
                let findings = self.observe_speech(&event);
                let class = event.data_class();
                self.emit_classified(EventPayload::Speech(event), class, session);
                self.report_findings(stream, findings);
            }
            SpeechSignal::Ended { stream, status } => {
                let Some(speech) = self.speech.as_mut() else {
                    return;
                };
                if let Some(mut r) = speech.running.remove(&stream) {
                    let _ = r.handle.join_if_finished();
                    if !r.handle.is_finished() {
                        speech.abandoned.push(r.handle);
                    }
                    let reason = status.ended;
                    speech.remember(stream, r.session, *status);
                    self.log(
                        Observation::log(Level::Info, "speech.ended")
                            .field("stream", stream)
                            .field("reason", format!("{reason:?}")),
                    );
                    self.note_stage_end(
                        StageKind::Recognition,
                        stream,
                        generation,
                        recognition_end(reason),
                        false,
                    );
                }
            }
        }
    }

    fn give_up(
        &mut self,
        stream: AudioStreamId,
        code: &'static str,
        message: &'static str,
        reason: RecognitionEndReason,
    ) {
        let Some(speech) = self.speech.as_mut() else {
            return;
        };
        let Some(mut r) = speech.running.remove(&stream) else {
            return;
        };
        let generation = r.generation;
        let mut status = r.handle.status();
        r.handle.abandon(reason);
        status.running = false;
        status.ended = Some(reason);
        let counters = status.counters.clone();
        let session = r.session;
        speech.remember(stream, session, status);
        speech.abandoned.push(r.handle);
        let err = CeError::new(ErrorCategory::Timeout, code, message);
        let report = err.report(&self.config.runtime.redaction);
        Metrics::inc(&self.metrics.recognition_failures, 1);
        self.log(Observation::log(Level::Warn, code).field("stream", stream));
        self.emit(
            EventPayload::Speech(SpeechEvent::Failed {
                stream,
                error: report,
            }),
            Some(session),
        );
        self.emit(
            EventPayload::Speech(SpeechEvent::Stopped {
                stream,
                reason,
                counters: Box::new(counters),
            }),
            Some(session),
        );
        self.note_stage_end(
            StageKind::Recognition,
            stream,
            generation,
            StageEnd::Stalled,
            false,
        );
    }

    /// Housekeeping: reap exited threads, detect load timeouts and stalls.
    pub(super) fn speech_housekeeping(&mut self) {
        let Some(speech) = self.speech.as_mut() else {
            return;
        };
        speech
            .abandoned
            .retain_mut(|h| h.join_if_finished().is_none());
        let late: Vec<AudioStreamId> = speech
            .running
            .iter()
            .filter(|(_, r)| r.handle.load_timed_out())
            .map(|(s, _)| *s)
            .collect();
        for s in late {
            self.give_up(
                s,
                "speech.load_timeout",
                "the speech model did not load in time",
                RecognitionEndReason::LoadFailed,
            );
        }
        let stalled: Vec<AudioStreamId> = self
            .speech
            .as_ref()
            .map(|sp| {
                sp.running
                    .iter()
                    .filter(|(_, r)| r.handle.stalled())
                    .map(|(s, _)| *s)
                    .collect()
            })
            .unwrap_or_default();
        for s in stalled {
            self.give_up(
                s,
                "speech.stalled",
                "the recognition worker stopped making progress",
                RecognitionEndReason::Failed,
            );
        }
    }

    /// Wait until each stream in `ids` has reported its end (or the
    /// deadline passes), draining all signals meanwhile.
    pub(super) async fn await_recognition_end(&mut self, ids: &[AudioStreamId], deadline: Instant) {
        loop {
            let pending = self
                .speech
                .as_ref()
                .is_some_and(|s| ids.iter().any(|i| s.running.contains_key(i)));
            if !pending {
                return;
            }
            tokio::select! {
                biased;
                () = tokio::time::sleep_until(deadline) => break,
                Some(sig) = super::recv_opt(&mut self.speech_rx) => self.handle_speech_signal(sig),
                Some(sig) = super::recv_opt(&mut self.semantic_rx) => self.handle_semantic_signal(sig),
                Some(sig) = super::recv_opt(&mut self.translation_rx) => self.handle_translation_signal(sig),
                Some(sig) = super::recv_opt(&mut self.intel_rx) => self.handle_intel_signal(sig),
                Some(ingress) = self.ingress_rx.recv() => self.handle_ingress(ingress),
                Some(signal) = super::recv_opt(&mut self.audio_rx) => self.handle_audio_signal(signal),
            }
        }
        let late: Vec<AudioStreamId> = self
            .speech
            .as_ref()
            .map(|s| {
                ids.iter()
                    .filter(|i| s.running.contains_key(i))
                    .copied()
                    .collect()
            })
            .unwrap_or_default();
        for stream in late {
            let Some(speech) = self.speech.as_mut() else {
                return;
            };
            let Some(mut r) = speech.running.remove(&stream) else {
                continue;
            };
            let generation = r.generation;
            let mut status = r.handle.status();
            r.handle.abandon(RecognitionEndReason::Failed);
            status.running = false;
            status.ended.get_or_insert(RecognitionEndReason::Failed);
            speech.remember(stream, r.session, status);
            speech.abandoned.push(r.handle);
            Metrics::inc(&self.metrics.recognition_failures, 1);
            self.log(
                Observation::log(Level::Warn, "speech.stop_timed_out").field("stream", stream),
            );
            self.note_stage_end(
                StageKind::Recognition,
                stream,
                generation,
                StageEnd::Stalled,
                true,
            );
        }
    }

    /// Stop every recognition of `session` (or all, for `None`) with
    /// `reason` and wait (bounded) for their ends.
    pub(super) async fn stop_recognitions(
        &mut self,
        session: Option<SessionId>,
        reason: RecognitionEndReason,
        deadline: Instant,
    ) {
        let ids: Vec<AudioStreamId> = match self.speech.as_ref() {
            Some(s) => s
                .running
                .iter()
                .filter(|(_, r)| session.is_none_or(|x| x == r.session))
                .map(|(i, r)| {
                    r.handle.stop(reason);
                    *i
                })
                .collect(),
            None => return,
        };
        if !ids.is_empty() {
            self.await_recognition_end(&ids, deadline).await;
        }
    }

    /// Statuses for health: running first, then recently finished.
    pub(super) fn recognition_statuses(&self) -> Vec<RecognitionStatus> {
        let Some(s) = self.speech.as_ref() else {
            return Vec::new();
        };
        s.running
            .values()
            .map(|r| r.handle.status())
            .chain(
                s.finished_order
                    .iter()
                    .filter_map(|i| s.finished.get(i).map(|f| f.1.clone())),
            )
            .collect()
    }

    /// Wait (bounded) for abandoned recognition threads. Returns how many
    /// are still running.
    pub(super) async fn drain_speech(&mut self, deadline: Instant) -> usize {
        loop {
            let Some(s) = self.speech.as_mut() else {
                return 0;
            };
            s.abandoned.retain_mut(|h| h.join_if_finished().is_none());
            if s.abandoned.is_empty() || Instant::now() >= deadline {
                return s.abandoned.len();
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}
