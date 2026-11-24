//! Semantic orchestration inside the runtime core (Phase 5).
//!
//! * The core owns which streams are interpreted. Each runs on its own
//!   supervised thread (`ce-semantic`), fed with Phase 4 transcript
//!   revisions that the core hands over with a non-blocking push; the
//!   worker coalesces superseded partials. A slow, hung or failing adapter
//!   never delays capture, recognition, analysis or the control plane.
//! * Events are published as `EventPayload::Semantic` in the stream's
//!   session, classified by `SemanticEvent::data_class` (observations
//!   `Content`, everything else `Operational`).
//! * Semantics may start before or after recognition: revisions are
//!   forwarded while both run. When the stream's recognition stops, the
//!   semantic worker drains its queue and ends (`SourceEnded`).
//! * Session close and shutdown stop semantics **after** recognitions, so
//!   final transcripts are interpreted, and before streams close.
//! * Housekeeping detects stalls, reports them and abandons the worker.

use std::collections::BTreeMap;
use std::sync::Arc;

use ce_core::CeResult;
use ce_core::audio::{AudioDirection, AudioStreamState};
use ce_core::command::CommandOutcome;
use ce_core::error::{CeError, ErrorCategory};
use ce_core::event::EventPayload;
use ce_core::ids::{AudioStreamId, SessionId};
use ce_core::observability::{Level, Observation};
use ce_core::semantic::{
    ObservationId, SemanticCapabilities, SemanticConfig, SemanticEndReason, SemanticEvent,
    SemanticStatus,
};
use ce_core::speech::TranscriptRevision;
use ce_semantic::{
    ProcessorSetup, PushOutcome, SemanticFactory, SemanticHandle, SemanticProcessor,
    SemanticSignal, input_from_revision, spawn_semantic,
};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::Core;
use super::pipeline::{Tagged, semantic_end, tagged_sender};
use crate::runtime::metrics::Metrics;
use ce_core::orchestration::{StageEnd, StageKind};

/// Capacity of the semantic event channel.
pub(crate) const SEMANTIC_SIGNAL_CAPACITY: usize = 1024;
const FINISHED_KEPT: usize = 16;

struct Running {
    handle: SemanticHandle,
    session: SessionId,
    generation: u64,
}

/// Semantic bookkeeping owned by the core.
pub(crate) struct SemanticState {
    tx: mpsc::Sender<Tagged<SemanticSignal>>,
    running: BTreeMap<AudioStreamId, Running>,
    finished: BTreeMap<AudioStreamId, (SessionId, SemanticStatus)>,
    finished_order: Vec<AudioStreamId>,
    abandoned: Vec<SemanticHandle>,
}

impl std::fmt::Debug for SemanticState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SemanticState")
            .field("running", &self.running.len())
            .field("finished", &self.finished.len())
            .field("abandoned", &self.abandoned.len())
            .finish_non_exhaustive()
    }
}

impl SemanticState {
    pub(crate) fn new(tx: mpsc::Sender<Tagged<SemanticSignal>>) -> Self {
        Self {
            tx,
            running: BTreeMap::new(),
            finished: BTreeMap::new(),
            finished_order: Vec::new(),
            abandoned: Vec::new(),
        }
    }

    fn remember(&mut self, stream: AudioStreamId, session: SessionId, status: SemanticStatus) {
        self.finished_order.retain(|s| *s != stream);
        self.finished_order.push(stream);
        self.finished.insert(stream, (session, status));
        while self.finished_order.len() > FINISHED_KEPT {
            let old = self.finished_order.remove(0);
            self.finished.remove(&old);
        }
    }
}

/// Semantic adapters registered on the builder.
#[derive(Clone, Default)]
pub(crate) struct SemanticProviders {
    pub(crate) adapters: Vec<Arc<dyn SemanticFactory>>,
}

impl std::fmt::Debug for SemanticProviders {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SemanticProviders")
            .field("adapters", &self.adapters.len())
            .finish()
    }
}

fn not_found(id: AudioStreamId) -> CeError {
    CeError::invalid_input(
        "semantic.not_found",
        "no semantic processing exists for this stream",
    )
    .with_context("stream", id.to_string())
}

fn unavailable() -> CeError {
    CeError::new(
        ErrorCategory::CapabilityResolution,
        "semantic.unavailable",
        "semantic processing needs an audio backend",
    )
}

impl Core {
    fn semantic_mut(&mut self) -> CeResult<&mut SemanticState> {
        self.semantic.as_mut().ok_or_else(unavailable)
    }

    /// Capabilities of every registered semantic adapter.
    pub(super) fn list_semantic_adapters(&self) -> CommandOutcome {
        CommandOutcome::SemanticAdapters {
            adapters: self
                .semantic_providers
                .adapters
                .iter()
                .map(|f| f.capabilities())
                .collect(),
        }
    }

    pub(super) fn resolve_semantic(
        &self,
        cfg: &SemanticConfig,
    ) -> CeResult<(Arc<dyn SemanticFactory>, SemanticCapabilities)> {
        let all = self
            .semantic_providers
            .adapters
            .iter()
            .map(|f| (f.clone(), f.capabilities()));
        let found = match &cfg.adapter {
            Some(id) => all.clone().find(|(_, c)| &c.adapter == id),
            None => all.clone().next(),
        };
        found.ok_or_else(|| {
            let e = CeError::new(
                ErrorCategory::CapabilityResolution,
                "semantic.no_adapter",
                "no matching semantic adapter is registered",
            );
            match &cfg.adapter {
                Some(id) => e.with_context("adapter", id.clone()),
                None => e,
            }
        })
    }

    /// Start interpreting a capture stream's transcripts.
    pub(super) fn start_semantics(
        &mut self,
        id: AudioStreamId,
        config: Option<SemanticConfig>,
    ) -> CeResult<CommandOutcome> {
        let cfg = config.unwrap_or_else(|| self.config.semantic.semantic.clone());
        cfg.validate()?;
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
                    "semantic.not_capture",
                    "only capture streams have transcripts",
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
                    "semantic.stream_ended",
                    "the stream has ended",
                ));
            }
            info.session_id
        };
        let (factory, caps) = self.resolve_semantic(&cfg)?;
        let max = self.config.semantic.max_concurrent as usize;
        let state = self.semantic_mut()?;
        if state.running.contains_key(&id) {
            return Err(CeError::lifecycle(
                "semantic.already_running",
                "this stream is already being interpreted",
            ));
        }
        if state.running.len() >= max {
            return Err(CeError::new(
                ErrorCategory::QueueSaturated,
                "semantic.capacity_exhausted",
                "the maximum number of concurrent semantic workers has been reached",
            )
            .with_context("max", max.to_string()));
        }
        let tx = tagged_sender(generation, state.tx.clone(), metrics);
        let adapter = factory.create(&cfg)?;
        let processor = SemanticProcessor::new(
            ProcessorSetup {
                stream: id,
                config: cfg,
                capabilities: caps.clone(),
            },
            adapter,
        )?;
        let handle = spawn_semantic(processor, tx)?;
        let status = handle.status();
        self.semantic_mut()?.running.insert(
            id,
            Running {
                handle,
                session,
                generation,
            },
        );
        Metrics::inc(&self.metrics.semantics_started, 1);
        self.log(
            Observation::log(Level::Info, "semantic.started")
                .field("stream", id)
                .field("adapter", caps.adapter.as_str())
                .field("locality", format!("{:?}", caps.locality))
                .field("config", status.config_fingerprint.as_str()),
        );
        Ok(CommandOutcome::Semantics {
            status: Box::new(status),
        })
    }

    /// Stop interpreting; queued revisions are discarded. Waits (bounded).
    pub(super) async fn stop_semantics(&mut self, id: AudioStreamId) -> CeResult<CommandOutcome> {
        {
            let state = self.semantic_mut()?;
            match state.running.get(&id) {
                Some(r) => r.handle.stop(SemanticEndReason::Stopped),
                None => {
                    return state
                        .finished
                        .get(&id)
                        .map(|(_, s)| CommandOutcome::Semantics {
                            status: Box::new(s.clone()),
                        })
                        .ok_or_else(|| not_found(id));
                }
            }
        }
        let deadline = Instant::now() + self.config.audio.operation_timeout();
        self.await_semantic_end(&[id], deadline).await;
        self.query_semantics(id)
    }

    /// Clear the stream's context.
    pub(super) fn reset_semantic_context(&mut self, id: AudioStreamId) -> CeResult<CommandOutcome> {
        let state = self.semantic_mut()?;
        let r = state.running.get(&id).ok_or_else(|| not_found(id))?;
        r.handle.request_reset();
        Ok(CommandOutcome::Semantics {
            status: Box::new(r.handle.status()),
        })
    }

    /// The host (user) confirmed an observation.
    pub(super) fn confirm_observation(
        &mut self,
        id: AudioStreamId,
        observation: ObservationId,
    ) -> CeResult<CommandOutcome> {
        let state = self.semantic_mut()?;
        let r = state.running.get(&id).ok_or_else(|| not_found(id))?;
        r.handle.confirm(observation);
        Ok(CommandOutcome::Semantics {
            status: Box::new(r.handle.status()),
        })
    }

    /// Current or final status.
    /// Generation of the worker running on `id`, if any.
    pub(super) fn semantic_generation(&self, id: AudioStreamId) -> Option<u64> {
        self.semantic
            .as_ref()
            .and_then(|s| s.running.get(&id))
            .map(|r| r.generation)
    }

    pub(super) fn query_semantics(&mut self, id: AudioStreamId) -> CeResult<CommandOutcome> {
        let state = self.semantic_mut()?;
        if let Some(r) = state.running.get(&id) {
            return Ok(CommandOutcome::Semantics {
                status: Box::new(r.handle.status()),
            });
        }
        state
            .finished
            .get(&id)
            .map(|(_, s)| CommandOutcome::Semantics {
                status: Box::new(s.clone()),
            })
            .ok_or_else(|| not_found(id))
    }

    /// Hand a transcript revision to the stream's semantic worker, if any
    /// (never blocks).
    pub(super) fn forward_transcript(&self, revision: &TranscriptRevision) {
        let Some(r) = self
            .semantic
            .as_ref()
            .and_then(|s| s.running.get(&revision.meta.stream))
        else {
            return;
        };
        if r.handle.push(input_from_revision(revision)) == PushOutcome::Dropped {
            Metrics::inc(&self.metrics.semantic_inputs_dropped, 1);
        }
    }

    /// The stream's recognition ended: its semantic worker drains and ends.
    pub(super) fn transcripts_ended(&self, stream: AudioStreamId) {
        if let Some(r) = self.semantic.as_ref().and_then(|s| s.running.get(&stream)) {
            r.handle.stop(SemanticEndReason::SourceEnded);
        }
    }

    /// Process one semantic signal.
    pub(super) fn handle_semantic_signal(&mut self, tagged: Tagged<SemanticSignal>) {
        let Tagged { generation, signal } = tagged;
        let stream = match &signal {
            SemanticSignal::Event(e) => e.stream(),
            SemanticSignal::Ended { stream, .. } => *stream,
        };
        let current = self.semantic_generation(stream);
        if !self.admit_signal(StageKind::Semantics, stream, generation, current) {
            return;
        }
        match signal {
            SemanticSignal::Event(event) => {
                let stream = event.stream();
                let session = self.semantic.as_ref().and_then(|s| {
                    s.running
                        .get(&stream)
                        .map(|r| r.session)
                        .or_else(|| s.finished.get(&stream).map(|f| f.0))
                });
                if matches!(event, SemanticEvent::Failed { .. }) {
                    Metrics::inc(&self.metrics.semantic_failures, 1);
                    self.log(
                        Observation::log(Level::Warn, "semantic.failed").field("stream", stream),
                    );
                }
                // Phase 6: final observations are offered to the stream's
                // translation worker as bounded hints (read-only copies).
                if let SemanticEvent::Observation { observation, .. } = &event {
                    self.forward_observation_to_translation(observation);
                }
                let findings = self.observe_semantic(&event);
                let class = event.data_class();
                self.emit_classified(EventPayload::Semantic(event), class, session);
                self.report_findings(stream, findings);
            }
            SemanticSignal::Ended { stream, status } => {
                let Some(state) = self.semantic.as_mut() else {
                    return;
                };
                if let Some(mut r) = state.running.remove(&stream) {
                    let _ = r.handle.join_if_finished();
                    if !r.handle.is_finished() {
                        state.abandoned.push(r.handle);
                    }
                    let reason = status.ended;
                    state.remember(stream, r.session, *status);
                    let end = semantic_end(reason);
                    self.log(
                        Observation::log(Level::Info, "semantic.ended")
                            .field("stream", stream)
                            .field("reason", format!("{reason:?}")),
                    );
                    self.note_stage_end(StageKind::Semantics, stream, generation, end, false);
                }
            }
        }
    }

    fn give_up_semantic(&mut self, stream: AudioStreamId) {
        let Some(state) = self.semantic.as_mut() else {
            return;
        };
        let Some(mut r) = state.running.remove(&stream) else {
            return;
        };
        let generation = r.generation;
        let mut status = r.handle.status();
        r.handle.abandon(SemanticEndReason::Failed);
        status.running = false;
        status.ended = Some(SemanticEndReason::Failed);
        let counters = status.counters.clone();
        let session = r.session;
        state.remember(stream, session, status);
        state.abandoned.push(r.handle);
        let err = CeError::new(
            ErrorCategory::Timeout,
            "semantic.stalled",
            "the semantic worker stopped making progress",
        );
        let report = err.report(&self.config.runtime.redaction);
        Metrics::inc(&self.metrics.semantic_failures, 1);
        self.log(Observation::log(Level::Warn, "semantic.stalled").field("stream", stream));
        self.emit(
            EventPayload::Semantic(SemanticEvent::Failed {
                stream,
                error: report,
            }),
            Some(session),
        );
        self.emit(
            EventPayload::Semantic(SemanticEvent::Stopped {
                stream,
                reason: SemanticEndReason::Failed,
                counters: Box::new(counters),
            }),
            Some(session),
        );
        self.note_stage_end(
            StageKind::Semantics,
            stream,
            generation,
            StageEnd::Stalled,
            false,
        );
    }

    /// Housekeeping: reap exited threads, detect stalls.
    pub(super) fn semantic_housekeeping(&mut self) {
        let Some(state) = self.semantic.as_mut() else {
            return;
        };
        state
            .abandoned
            .retain_mut(|h| h.join_if_finished().is_none());
        let stalled: Vec<AudioStreamId> = state
            .running
            .iter()
            .filter(|(_, r)| r.handle.stalled())
            .map(|(s, _)| *s)
            .collect();
        for s in stalled {
            self.give_up_semantic(s);
        }
    }

    pub(super) async fn await_semantic_end(&mut self, ids: &[AudioStreamId], deadline: Instant) {
        loop {
            let pending = self
                .semantic
                .as_ref()
                .is_some_and(|s| ids.iter().any(|i| s.running.contains_key(i)));
            if !pending {
                return;
            }
            tokio::select! {
                biased;
                () = tokio::time::sleep_until(deadline) => break,
                Some(sig) = super::recv_opt(&mut self.semantic_rx) => self.handle_semantic_signal(sig),
                Some(sig) = super::recv_opt(&mut self.translation_rx) => self.handle_translation_signal(sig),
                Some(sig) = super::recv_opt(&mut self.speech_rx) => self.handle_speech_signal(sig),
                Some(sig) = super::recv_opt(&mut self.intel_rx) => self.handle_intel_signal(sig),
                Some(ingress) = self.ingress_rx.recv() => self.handle_ingress(ingress),
                Some(signal) = super::recv_opt(&mut self.audio_rx) => self.handle_audio_signal(signal),
            }
        }
        let late: Vec<AudioStreamId> = self
            .semantic
            .as_ref()
            .map(|s| {
                ids.iter()
                    .filter(|i| s.running.contains_key(i))
                    .copied()
                    .collect()
            })
            .unwrap_or_default();
        for stream in late {
            let Some(state) = self.semantic.as_mut() else {
                return;
            };
            let Some(mut r) = state.running.remove(&stream) else {
                continue;
            };
            let generation = r.generation;
            let mut status = r.handle.status();
            r.handle.abandon(SemanticEndReason::Failed);
            status.running = false;
            status.ended.get_or_insert(SemanticEndReason::Failed);
            state.remember(stream, r.session, status);
            state.abandoned.push(r.handle);
            Metrics::inc(&self.metrics.semantic_failures, 1);
            self.log(
                Observation::log(Level::Warn, "semantic.stop_timed_out").field("stream", stream),
            );
            self.note_stage_end(
                StageKind::Semantics,
                stream,
                generation,
                StageEnd::Stalled,
                true,
            );
        }
    }

    /// Stop every semantic worker of `session` (or all) and wait (bounded).
    /// `SourceEnded`/`Shutdown` drain queued revisions first.
    pub(super) async fn stop_all_semantics(
        &mut self,
        session: Option<SessionId>,
        reason: SemanticEndReason,
        deadline: Instant,
    ) {
        let ids: Vec<AudioStreamId> = match self.semantic.as_ref() {
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
            self.await_semantic_end(&ids, deadline).await;
        }
    }

    /// Statuses for health: running first, then recently finished.
    pub(super) fn semantic_statuses(&self) -> Vec<SemanticStatus> {
        let Some(s) = self.semantic.as_ref() else {
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

    /// Wait (bounded) for abandoned semantic threads; returns how many are
    /// still running.
    pub(super) async fn drain_semantic(&mut self, deadline: Instant) -> usize {
        loop {
            let Some(s) = self.semantic.as_mut() else {
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
