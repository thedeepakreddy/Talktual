//! Translation orchestration inside the runtime core (Phase 6).
//!
//! * The core owns which streams are translated. Each runs on its own
//!   supervised thread (`ce-translate`), fed with Phase 4 transcript
//!   revisions that the core hands over with a non-blocking push (the
//!   worker coalesces superseded partials) and, optionally, final Phase 5
//!   entity observations as bounded hints. A slow, hung or failing adapter
//!   never delays capture, recognition, semantics or the control plane.
//! * Adapters are registered on the builder; selection is explicit (the
//!   configured adapter, else the first registered one that supports the
//!   pair and privacy policy) and fallback happens only to adapters listed
//!   in `TranslationConfig::fallback`. Remote adapters need
//!   `privacy.allow_remote`.
//! * Events are published as `EventPayload::Translation` in the stream's
//!   session, classified by `TranslationEvent::data_class` (results
//!   `Content`, everything else `Operational`).
//! * When the stream's recognition stops, the translation worker drains and
//!   ends (`SourceEnded`). Session close and shutdown stop translations
//!   **after** recognitions and semantics, so final transcripts (and their
//!   observations) are translated, and before streams close.
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
use ce_core::semantic::{Locality, SemanticObservation};
use ce_core::speech::TranscriptRevision;
use ce_core::translation::{
    SourceLanguagePolicy, TerminologyProfile, TranslationCapabilities, TranslationConfig,
    TranslationEndReason, TranslationEvent, TranslationStatus,
};
use ce_translate::{
    AdapterSlot, ProcessorSetup, PushOutcome, TranslationFactory, TranslationHandle,
    TranslationProcessor, TranslationSignal, spawn_translation,
};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::Core;
use crate::runtime::metrics::Metrics;

/// Capacity of the translation event channel.
pub(crate) const TRANSLATION_SIGNAL_CAPACITY: usize = 1024;
const FINISHED_KEPT: usize = 16;

struct Running {
    handle: TranslationHandle,
    session: SessionId,
}

/// Translation bookkeeping owned by the core.
pub(crate) struct TranslationStateMap {
    tx: mpsc::Sender<TranslationSignal>,
    running: BTreeMap<AudioStreamId, Running>,
    finished: BTreeMap<AudioStreamId, (SessionId, TranslationStatus)>,
    finished_order: Vec<AudioStreamId>,
    abandoned: Vec<TranslationHandle>,
}

impl std::fmt::Debug for TranslationStateMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranslationStateMap")
            .field("running", &self.running.len())
            .field("finished", &self.finished.len())
            .field("abandoned", &self.abandoned.len())
            .finish_non_exhaustive()
    }
}

impl TranslationStateMap {
    pub(crate) fn new(tx: mpsc::Sender<TranslationSignal>) -> Self {
        Self {
            tx,
            running: BTreeMap::new(),
            finished: BTreeMap::new(),
            finished_order: Vec::new(),
            abandoned: Vec::new(),
        }
    }

    fn remember(&mut self, stream: AudioStreamId, session: SessionId, status: TranslationStatus) {
        self.finished_order.retain(|s| *s != stream);
        self.finished_order.push(stream);
        self.finished.insert(stream, (session, status));
        while self.finished_order.len() > FINISHED_KEPT {
            let old = self.finished_order.remove(0);
            self.finished.remove(&old);
        }
    }
}

/// Translation adapters and terminology profiles registered on the builder.
#[derive(Clone, Default)]
pub(crate) struct TranslationProviders {
    pub(crate) adapters: Vec<Arc<dyn TranslationFactory>>,
    pub(crate) terminology: Vec<TerminologyProfile>,
}

impl std::fmt::Debug for TranslationProviders {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranslationProviders")
            .field("adapters", &self.adapters.len())
            .field("terminology", &self.terminology.len())
            .finish()
    }
}

fn not_found(id: AudioStreamId) -> CeError {
    CeError::invalid_input(
        "translation.not_found",
        "no translation exists for this stream",
    )
    .with_context("stream", id.to_string())
}

fn unavailable() -> CeError {
    CeError::new(
        ErrorCategory::CapabilityResolution,
        "translation.unavailable",
        "translation needs an audio backend",
    )
}

impl Core {
    fn translation_mut(&mut self) -> CeResult<&mut TranslationStateMap> {
        self.translation.as_mut().ok_or_else(unavailable)
    }

    /// Capabilities of every registered translation adapter.
    pub(super) fn list_translation_adapters(&self) -> CommandOutcome {
        CommandOutcome::TranslationAdapters {
            adapters: self
                .translation_providers
                .adapters
                .iter()
                .map(|f| f.capabilities())
                .collect(),
        }
    }

    fn select_adapters(
        &self,
        cfg: &TranslationConfig,
    ) -> CeResult<Vec<(Arc<dyn TranslationFactory>, TranslationCapabilities)>> {
        let all: Vec<(Arc<dyn TranslationFactory>, TranslationCapabilities)> = self
            .translation_providers
            .adapters
            .iter()
            .map(|f| (f.clone(), f.capabilities()))
            .collect();
        let find = |id: &str| all.iter().find(|(_, c)| c.adapter == id).cloned();
        let no_adapter = |id: Option<&str>| {
            let e = CeError::new(
                ErrorCategory::CapabilityResolution,
                "translation.no_adapter",
                "no matching translation adapter is registered",
            );
            match id {
                Some(i) => e.with_context("adapter", i.to_string()),
                None => e,
            }
        };
        let primary = match &cfg.adapter {
            Some(id) => find(id).ok_or_else(|| no_adapter(Some(id)))?,
            None => {
                // The first registered adapter that supports the pair (when
                // the source is explicit) and the privacy policy.
                let target = cfg.target.as_ref();
                all.iter()
                    .find(|(_, c)| {
                        (cfg.privacy.allow_remote || c.locality != Locality::Remote)
                            && match (&cfg.source, target) {
                                (SourceLanguagePolicy::Explicit { language }, Some(t)) => {
                                    c.supports(language, t)
                                }
                                (_, Some(t)) => {
                                    c.pairs.iter().any(|p| p.target.primary() == t.primary())
                                }
                                _ => true,
                            }
                    })
                    .cloned()
                    .ok_or_else(|| no_adapter(None))?
            }
        };
        let mut out = vec![primary];
        for id in &cfg.fallback {
            out.push(find(id).ok_or_else(|| no_adapter(Some(id)))?);
        }
        Ok(out)
    }

    /// Start translating a capture stream's transcripts.
    pub(super) fn start_translation(
        &mut self,
        id: AudioStreamId,
        config: Option<TranslationConfig>,
    ) -> CeResult<CommandOutcome> {
        let cfg = config.unwrap_or_else(|| self.config.translation.translation.clone());
        cfg.validate()?;
        let session = {
            let audio = self.audio.as_ref().ok_or_else(unavailable)?;
            let (info, state, ended) = audio.stream(id).ok_or_else(|| {
                CeError::invalid_input("audio.unknown_stream", "no such audio stream")
                    .with_context("stream", id.to_string())
            })?;
            if info.direction != AudioDirection::Capture {
                return Err(CeError::invalid_input(
                    "translation.not_capture",
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
                    "translation.stream_ended",
                    "the stream has ended",
                ));
            }
            info.session_id
        };
        let selected = self.select_adapters(&cfg)?;
        let terminology = match &cfg.terminology {
            Some(tid) => Some(
                self.translation_providers
                    .terminology
                    .iter()
                    .find(|p| &p.id == tid)
                    .cloned()
                    .ok_or_else(|| {
                        CeError::invalid_input(
                            "translation.unknown_terminology",
                            "no terminology profile with this id is registered",
                        )
                        .with_context("terminology", tid.clone())
                    })?,
            ),
            None => None,
        };
        let max = self.config.translation.max_concurrent as usize;
        let state = self.translation_mut()?;
        if state.running.contains_key(&id) {
            return Err(CeError::lifecycle(
                "translation.already_running",
                "this stream is already being translated",
            ));
        }
        if state.running.len() >= max {
            return Err(CeError::new(
                ErrorCategory::QueueSaturated,
                "translation.capacity_exhausted",
                "the maximum number of concurrent translation workers has been reached",
            )
            .with_context("max", max.to_string()));
        }
        let tx = state.tx.clone();
        let mut adapters = Vec::with_capacity(selected.len());
        for (f, caps) in &selected {
            adapters.push(AdapterSlot {
                capabilities: caps.clone(),
                adapter: f.create(&cfg)?,
            });
        }
        let processor = TranslationProcessor::new(ProcessorSetup {
            stream: id,
            config: cfg,
            adapters,
            terminology,
        })?;
        let handle = spawn_translation(processor, tx)?;
        let status = handle.status();
        self.translation_mut()?
            .running
            .insert(id, Running { handle, session });
        Metrics::inc(&self.metrics.translations_started, 1);
        self.log(
            Observation::log(Level::Info, "translation.started")
                .field("stream", id)
                .field("adapter", status.capabilities.adapter.as_str())
                .field("target", status.target.as_str())
                .field("locality", format!("{:?}", status.capabilities.locality))
                .field("config", status.config_fingerprint.as_str()),
        );
        Ok(CommandOutcome::Translation {
            status: Box::new(status),
        })
    }

    /// Stop translating; queued inputs are discarded and results of
    /// unfinished utterances withdrawn. Waits (bounded).
    pub(super) async fn stop_translation(&mut self, id: AudioStreamId) -> CeResult<CommandOutcome> {
        {
            let state = self.translation_mut()?;
            match state.running.get(&id) {
                Some(r) => r.handle.stop(TranslationEndReason::Stopped),
                None => {
                    return state
                        .finished
                        .get(&id)
                        .map(|(_, s)| CommandOutcome::Translation {
                            status: Box::new(s.clone()),
                        })
                        .ok_or_else(|| not_found(id));
                }
            }
        }
        let deadline = Instant::now() + self.config.audio.operation_timeout();
        self.await_translation_end(&[id], deadline).await;
        self.query_translation(id)
    }

    /// Current or final status.
    pub(super) fn query_translation(&mut self, id: AudioStreamId) -> CeResult<CommandOutcome> {
        let state = self.translation_mut()?;
        if let Some(r) = state.running.get(&id) {
            return Ok(CommandOutcome::Translation {
                status: Box::new(r.handle.status()),
            });
        }
        state
            .finished
            .get(&id)
            .map(|(_, s)| CommandOutcome::Translation {
                status: Box::new(s.clone()),
            })
            .ok_or_else(|| not_found(id))
    }

    /// Discard pending work and withdraw open results; continue.
    pub(super) fn reset_translation(&mut self, id: AudioStreamId) -> CeResult<CommandOutcome> {
        let state = self.translation_mut()?;
        let r = state.running.get(&id).ok_or_else(|| not_found(id))?;
        r.handle.request_reset();
        Ok(CommandOutcome::Translation {
            status: Box::new(r.handle.status()),
        })
    }

    /// Hand a transcript revision to the stream's translation worker, if
    /// any (never blocks; the revision is cloned, never modified).
    pub(super) fn forward_transcript_to_translation(&self, revision: &TranscriptRevision) {
        let Some(r) = self
            .translation
            .as_ref()
            .and_then(|s| s.running.get(&revision.meta.stream))
        else {
            return;
        };
        if r.handle.push(revision.clone()) == PushOutcome::Dropped {
            Metrics::inc(&self.metrics.translation_inputs_dropped, 1);
        }
    }

    /// Hand a Phase 5 observation to the stream's translation worker as a
    /// potential hint (never blocks; read-only copy).
    pub(super) fn forward_observation_to_translation(&self, observation: &SemanticObservation) {
        if let Some(r) = self
            .translation
            .as_ref()
            .and_then(|s| s.running.get(&observation.source.stream))
        {
            r.handle.observe(observation.clone());
        }
    }

    /// The stream's recognition ended: its translation worker drains and ends.
    pub(super) fn transcripts_ended_for_translation(&self, stream: AudioStreamId) {
        if let Some(r) = self
            .translation
            .as_ref()
            .and_then(|s| s.running.get(&stream))
        {
            r.handle.stop(TranslationEndReason::SourceEnded);
        }
    }

    /// Process one translation signal.
    pub(super) fn handle_translation_signal(&mut self, signal: TranslationSignal) {
        match signal {
            TranslationSignal::Event(event) => {
                let stream = event.stream();
                let session = self.translation.as_ref().and_then(|s| {
                    s.running
                        .get(&stream)
                        .map(|r| r.session)
                        .or_else(|| s.finished.get(&stream).map(|f| f.0))
                });
                if matches!(event, TranslationEvent::Failed { .. }) {
                    Metrics::inc(&self.metrics.translation_failures, 1);
                    self.log(
                        Observation::log(Level::Warn, "translation.failed").field("stream", stream),
                    );
                }
                let class = event.data_class();
                self.emit_classified(EventPayload::Translation(event), class, session);
            }
            TranslationSignal::Ended { stream, status } => {
                let Some(state) = self.translation.as_mut() else {
                    return;
                };
                if let Some(mut r) = state.running.remove(&stream) {
                    let _ = r.handle.join_if_finished();
                    if !r.handle.is_finished() {
                        state.abandoned.push(r.handle);
                    }
                    let reason = status.ended;
                    state.remember(stream, r.session, *status);
                    self.log(
                        Observation::log(Level::Info, "translation.ended")
                            .field("stream", stream)
                            .field("reason", format!("{reason:?}")),
                    );
                }
            }
        }
    }

    fn give_up_translation(&mut self, stream: AudioStreamId) {
        let Some(state) = self.translation.as_mut() else {
            return;
        };
        let Some(mut r) = state.running.remove(&stream) else {
            return;
        };
        let mut status = r.handle.status();
        r.handle.abandon(TranslationEndReason::Failed);
        status.running = false;
        status.ended = Some(TranslationEndReason::Failed);
        let counters = status.counters.clone();
        let session = r.session;
        state.remember(stream, session, status);
        state.abandoned.push(r.handle);
        let err = CeError::new(
            ErrorCategory::Timeout,
            "translation.stalled",
            "the translation worker stopped making progress",
        );
        let report = err.report(&self.config.runtime.redaction);
        Metrics::inc(&self.metrics.translation_failures, 1);
        self.log(Observation::log(Level::Warn, "translation.stalled").field("stream", stream));
        self.emit(
            EventPayload::Translation(TranslationEvent::Failed {
                stream,
                error: report,
            }),
            Some(session),
        );
        self.emit(
            EventPayload::Translation(TranslationEvent::Stopped {
                stream,
                reason: TranslationEndReason::Failed,
                counters: Box::new(counters),
            }),
            Some(session),
        );
    }

    /// Housekeeping: reap exited threads, detect stalls.
    pub(super) fn translation_housekeeping(&mut self) {
        let Some(state) = self.translation.as_mut() else {
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
            self.give_up_translation(s);
        }
    }

    async fn await_translation_end(&mut self, ids: &[AudioStreamId], deadline: Instant) {
        loop {
            let pending = self
                .translation
                .as_ref()
                .is_some_and(|s| ids.iter().any(|i| s.running.contains_key(i)));
            if !pending {
                return;
            }
            tokio::select! {
                biased;
                () = tokio::time::sleep_until(deadline) => break,
                Some(sig) = super::recv_opt(&mut self.translation_rx) => self.handle_translation_signal(sig),
                Some(sig) = super::recv_opt(&mut self.semantic_rx) => self.handle_semantic_signal(sig),
                Some(sig) = super::recv_opt(&mut self.speech_rx) => self.handle_speech_signal(sig),
                Some(sig) = super::recv_opt(&mut self.intel_rx) => self.handle_intel_signal(sig),
                Some(ingress) = self.ingress_rx.recv() => self.handle_ingress(ingress),
                Some(signal) = super::recv_opt(&mut self.audio_rx) => self.handle_audio_signal(signal),
            }
        }
        let late: Vec<AudioStreamId> = self
            .translation
            .as_ref()
            .map(|s| {
                ids.iter()
                    .filter(|i| s.running.contains_key(i))
                    .copied()
                    .collect()
            })
            .unwrap_or_default();
        for stream in late {
            let Some(state) = self.translation.as_mut() else {
                return;
            };
            let Some(mut r) = state.running.remove(&stream) else {
                continue;
            };
            let mut status = r.handle.status();
            r.handle.abandon(TranslationEndReason::Failed);
            status.running = false;
            status.ended.get_or_insert(TranslationEndReason::Failed);
            state.remember(stream, r.session, status);
            state.abandoned.push(r.handle);
            Metrics::inc(&self.metrics.translation_failures, 1);
            self.log(
                Observation::log(Level::Warn, "translation.stop_timed_out").field("stream", stream),
            );
        }
    }

    /// Stop every translation worker of `session` (or all) and wait (bounded).
    /// `SourceEnded`/`Shutdown` drain queued revisions first.
    pub(super) async fn stop_all_translations(
        &mut self,
        session: Option<SessionId>,
        reason: TranslationEndReason,
        deadline: Instant,
    ) {
        let ids: Vec<AudioStreamId> = match self.translation.as_ref() {
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
            self.await_translation_end(&ids, deadline).await;
        }
    }

    /// Statuses for health: running first, then recently finished.
    pub(super) fn translation_statuses(&self) -> Vec<TranslationStatus> {
        let Some(s) = self.translation.as_ref() else {
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

    /// Wait (bounded) for abandoned translation threads; returns how many are
    /// still running.
    pub(super) async fn drain_translation(&mut self, deadline: Instant) -> usize {
        loop {
            let Some(s) = self.translation.as_mut() else {
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
