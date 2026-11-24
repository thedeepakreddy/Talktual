//! Streaming orchestration inside the runtime core (Phase 7).
//!
//! * **Worker generations** (ADR-0031): every stage worker start gets a
//!   fresh engine-wide generation. The worker sends into its own bounded
//!   channel; a forwarding task tags each signal and passes it on to the
//!   stage's core channel (order and backpressure preserved). The core
//!   admits a signal only if the stream's running worker for that stage has
//!   the same generation; anything else is a stale output — counted,
//!   logged, reported once per generation, never published.
//! * **Pipelines** (ADR-0030): a validated graph of stages on one capture
//!   stream, started in dependency order with rollback, stopped by `drain`
//!   (upstream first; downstream workers finish what they have) or
//!   `cancel` (downstream first), reset in dependency order, restarted with
//!   a new epoch. Optional stage failures degrade the pipeline explicitly;
//!   required failures stop it.
//! * **Observer** (ADR-0032): each pipeline's admitted stage events pass
//!   through a `ce_orchestrate::Observer` (correlation, revision
//!   reconciliation, causal checks, timeline, latency, deadlines). Findings
//!   are published as operational orchestration events after the stage
//!   event; payloads are never changed.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;

use ce_core::CeResult;
use ce_core::audio::{AudioDirection, AudioStreamState};
use ce_core::command::CommandOutcome;
use ce_core::error::{CeError, ErrorCategory};
use ce_core::event::EventPayload;
use ce_core::ids::{AudioStreamId, SessionId};
use ce_core::intel::AnalysisEvent;
use ce_core::observability::{Level, Observation};
use ce_core::orchestration::{
    FaultClass, OrchestrationCounters, OrchestrationEvent, PipelineEndReason, PipelineSpec,
    PipelineState, PipelineStatus, Requirement, StageConfig, StageEnd, StageKind, StageState,
    StageStatus, StopMode, StopOutcome, TimingUncertainty,
};
use ce_core::semantic::SemanticEvent;
use ce_core::speech::SpeechEvent;
use ce_core::translation::TranslationEvent;
use ce_orchestrate::{
    Admission, Finding, GenerationCounter, Observer, ResolvedStage, ValidatedGraph, admit,
};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::Core;
use crate::runtime::metrics::Metrics;

/// Capacity of each worker's own signal channel (before tagging).
pub(crate) const WORKER_SIGNAL_CAPACITY: usize = 64;
const FINISHED_KEPT: usize = 16;
const STALE_REPORTED_KEPT: usize = 64;

/// A worker signal tagged with its worker's generation.
#[derive(Debug)]
pub(crate) struct Tagged<T> {
    pub(crate) generation: u64,
    pub(crate) signal: T,
}

/// A sender for one worker whose signals reach `out` tagged with
/// `generation`. The forwarding task ends when the worker drops its sender
/// (it exits) or the core's receiver is gone.
pub(crate) fn tagged_sender<T: Send + 'static>(
    generation: u64,
    out: mpsc::Sender<Tagged<T>>,
    metrics: Arc<Metrics>,
) -> mpsc::Sender<T> {
    let (tx, mut rx) = mpsc::channel::<T>(WORKER_SIGNAL_CAPACITY);
    metrics.worker_forwarders.fetch_add(1, Relaxed);
    tokio::spawn(async move {
        while let Some(signal) = rx.recv().await {
            if out.send(Tagged { generation, signal }).await.is_err() {
                break;
            }
        }
        metrics.worker_forwarders.fetch_sub(1, Relaxed);
    });
    tx
}

/// Deferred pipeline work (needs `await`; run by the control loop).
#[derive(Debug)]
enum Pending {
    Finish {
        stream: AudioStreamId,
        epoch: u64,
        reason: PipelineEndReason,
        failure: Option<(StageKind, FaultClass, String)>,
    },
}

struct Pipeline {
    session: SessionId,
    spec: PipelineSpec,
    graph: ValidatedGraph,
    state: PipelineState,
    epoch: u64,
    reset_epoch: u64,
    stages: Vec<StageStatus>,
    observer: Observer,
    /// Last end per stage: (generation, how, timed out).
    ends: BTreeMap<StageKind, (u64, StageEnd, bool)>,
    /// Stopping under the pipeline's control (stage ends are expected).
    stopping: bool,
    finish_queued: bool,
}

impl Pipeline {
    fn stage_mut(&mut self, kind: StageKind) -> Option<&mut StageStatus> {
        self.stages.iter_mut().find(|s| s.kind == kind)
    }

    fn status(&self, stream: AudioStreamId, ended: Option<PipelineEndReason>) -> PipelineStatus {
        PipelineStatus {
            stream,
            session: self.session,
            state: self.state,
            epoch: self.epoch,
            reset_epoch: self.reset_epoch,
            fingerprint: self.graph.fingerprint.clone(),
            stop_mode: self.graph.stop,
            stages: self.stages.clone(),
            notes: self.graph.notes.clone(),
            counters: self.observer.counters().clone(),
            latency: self.observer.latency(),
            timeline: self.observer.timeline(),
            ended,
        }
    }
}

/// Orchestration bookkeeping owned by the core.
#[derive(Default)]
pub(crate) struct Pipelines {
    generations: GenerationCounter,
    epochs: GenerationCounter,
    running: BTreeMap<AudioStreamId, Pipeline>,
    finished: BTreeMap<AudioStreamId, (PipelineStatus, PipelineSpec)>,
    finished_order: Vec<AudioStreamId>,
    stale_reported: VecDeque<(StageKind, AudioStreamId, u64)>,
    pending: VecDeque<Pending>,
}

impl std::fmt::Debug for Pipelines {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pipelines")
            .field("running", &self.running.len())
            .field("finished", &self.finished.len())
            .field("pending", &self.pending.len())
            .finish_non_exhaustive()
    }
}

impl Pipelines {
    pub(crate) fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    fn remember(&mut self, stream: AudioStreamId, status: PipelineStatus, spec: PipelineSpec) {
        self.finished_order.retain(|s| *s != stream);
        self.finished_order.push(stream);
        self.finished.insert(stream, (status, spec));
        while self.finished_order.len() > FINISHED_KEPT {
            let old = self.finished_order.remove(0);
            self.finished.remove(&old);
        }
    }
}

fn not_found(id: AudioStreamId) -> CeError {
    CeError::invalid_input(
        "orchestration.not_found",
        "no pipeline exists for this stream",
    )
    .with_context("stream", id.to_string())
}

/// Stage end of an analysis worker.
pub(crate) fn analysis_end(r: Option<ce_core::intel::AnalysisEndReason>) -> StageEnd {
    use ce_core::intel::AnalysisEndReason as R;
    match r {
        Some(R::Failed) => StageEnd::Failed,
        Some(R::StreamClosed | R::Revoked | R::StreamInvalidated | R::StreamFailed) => {
            StageEnd::SourceEnded
        }
        Some(R::Stopped | R::Shutdown) | None => StageEnd::Stopped,
    }
}

/// Stage end of a recognition worker.
pub(crate) fn recognition_end(r: Option<ce_core::speech::RecognitionEndReason>) -> StageEnd {
    use ce_core::speech::RecognitionEndReason as R;
    match r {
        Some(R::Failed | R::LoadFailed) => StageEnd::Failed,
        Some(R::StreamClosed | R::Revoked | R::StreamInvalidated | R::StreamFailed) => {
            StageEnd::SourceEnded
        }
        Some(R::Stopped | R::Shutdown) | None => StageEnd::Stopped,
    }
}

/// Stage end of a semantic worker.
pub(crate) fn semantic_end(r: Option<ce_core::semantic::SemanticEndReason>) -> StageEnd {
    use ce_core::semantic::SemanticEndReason as R;
    match r {
        Some(R::Failed) => StageEnd::Failed,
        Some(R::SourceEnded) => StageEnd::SourceEnded,
        Some(R::Stopped | R::Shutdown) | None => StageEnd::Stopped,
    }
}

/// Stage end of a translation worker.
pub(crate) fn translation_end(r: Option<ce_core::translation::TranslationEndReason>) -> StageEnd {
    use ce_core::translation::TranslationEndReason as R;
    match r {
        Some(R::Failed) => StageEnd::Failed,
        Some(R::SourceEnded) => StageEnd::SourceEnded,
        Some(R::Stopped | R::Shutdown) | None => StageEnd::Stopped,
    }
}

impl Core {
    // ------------------------------------------------------------ generations

    /// A fresh worker generation.
    pub(super) fn next_generation(&mut self) -> u64 {
        self.pipelines.generations.issue()
    }

    /// Admit a tagged signal of `stage` for `stream`; report a stale one.
    pub(super) fn admit_signal(
        &mut self,
        stage: StageKind,
        stream: AudioStreamId,
        generation: u64,
        current: Option<u64>,
    ) -> bool {
        match admit(current, generation) {
            Admission::Current => true,
            Admission::Stale { current } => {
                Metrics::inc(&self.metrics.stale_outputs_rejected, 1);
                self.log(
                    Observation::log(Level::Debug, "orchestration.stale_output_rejected")
                        .field("stage", stage.as_str())
                        .field("stream", stream)
                        .field("generation", generation),
                );
                if let Some(p) = self.pipelines.running.get_mut(&stream) {
                    p.observer.on_stale_rejected();
                }
                let key = (stage, stream, generation);
                if !self.pipelines.stale_reported.contains(&key) {
                    self.pipelines.stale_reported.push_back(key);
                    while self.pipelines.stale_reported.len() > STALE_REPORTED_KEPT {
                        self.pipelines.stale_reported.pop_front();
                    }
                    let session = self.stream_session(stream);
                    self.emit(
                        EventPayload::Orchestration(OrchestrationEvent::StaleOutputRejected {
                            stream,
                            stage,
                            generation,
                            current,
                        }),
                        session,
                    );
                }
                false
            }
        }
    }

    fn stream_session(&self, stream: AudioStreamId) -> Option<SessionId> {
        self.audio
            .as_ref()
            .and_then(|a| a.stream(stream))
            .map(|(info, _, _)| info.session_id)
    }

    fn stage_generation(&self, stage: StageKind, stream: AudioStreamId) -> Option<u64> {
        match stage {
            StageKind::Analysis => self.analysis_generation(stream),
            StageKind::Recognition => self.recognition_generation(stream),
            StageKind::Semantics => self.semantic_generation(stream),
            StageKind::Translation => self.translation_generation(stream),
        }
    }

    /// A stage worker ended (normally, failed, stalled or abandoned after a
    /// stop timeout). Updates the stream's pipeline, if any.
    pub(super) fn note_stage_end(
        &mut self,
        stage: StageKind,
        stream: AudioStreamId,
        generation: u64,
        end: StageEnd,
        timed_out: bool,
    ) {
        let Some(p) = self.pipelines.running.get_mut(&stream) else {
            return;
        };
        let Some(s) = p.stage_mut(stage) else {
            return;
        };
        if s.generation != Some(generation) {
            return;
        }
        let requirement = s.requirement;
        let failed = matches!(end, StageEnd::Failed | StageEnd::Stalled) || timed_out;
        s.state = if failed {
            StageState::Failed
        } else {
            StageState::Ended
        };
        if failed && s.reason.is_none() {
            s.reason = Some(
                if timed_out {
                    "orchestration.stop_timed_out"
                } else if end == StageEnd::Stalled {
                    "orchestration.stage_stalled"
                } else {
                    "orchestration.stage_failed"
                }
                .to_string(),
            );
        }
        p.ends.insert(stage, (generation, end, timed_out));
        let (epoch, session, stopping) = (p.epoch, p.session, p.stopping);
        self.emit(
            EventPayload::Orchestration(OrchestrationEvent::StageEnded {
                stream,
                epoch,
                stage,
                generation,
                end,
            }),
            Some(session),
        );
        if stopping {
            return;
        }
        let Some(p) = self.pipelines.running.get_mut(&stream) else {
            return;
        };
        if failed && requirement == Requirement::Optional {
            if let Ok(to) = p.state.transition_to(PipelineState::Degraded) {
                p.state = to;
            }
            let fault = if end == StageEnd::Stalled {
                FaultClass::Timeout
            } else {
                FaultClass::WorkerFailure
            };
            let code = p
                .stage_mut(stage)
                .and_then(|s| s.reason.clone())
                .unwrap_or_default();
            self.emit(
                EventPayload::Orchestration(OrchestrationEvent::StageDegraded {
                    stream,
                    epoch,
                    stage,
                    fault,
                    code,
                }),
                Some(session),
            );
            return;
        }
        if p.finish_queued {
            return;
        }
        if failed {
            p.finish_queued = true;
            let fault = if end == StageEnd::Stalled {
                FaultClass::Timeout
            } else {
                FaultClass::WorkerFailure
            };
            self.pipelines.pending.push_back(Pending::Finish {
                stream,
                epoch,
                reason: PipelineEndReason::Failed,
                failure: Some((stage, fault, "orchestration.required_stage_failed".into())),
            });
            return;
        }
        // A required stage ended normally (its source ended, or it was
        // stopped directly), or every stage has ended: the pipeline ends.
        let all_ended = p.stages.iter().all(|s| s.state != StageState::Running);
        if requirement == Requirement::Required || all_ended {
            p.finish_queued = true;
            let reason = if end == StageEnd::SourceEnded {
                PipelineEndReason::SourceEnded
            } else {
                PipelineEndReason::Stopped
            };
            self.pipelines.pending.push_back(Pending::Finish {
                stream,
                epoch,
                reason,
                failure: None,
            });
        }
    }

    // --------------------------------------------------------------- observer

    fn observe<F>(&mut self, stream: AudioStreamId, f: F) -> Vec<Finding>
    where
        F: FnOnce(&mut Observer, ce_core::time::MonotonicTime) -> Vec<Finding>,
    {
        let now = self.clock.monotonic();
        let Some(p) = self.pipelines.running.get_mut(&stream) else {
            return Vec::new();
        };
        let t0 = std::time::Instant::now();
        let out = f(&mut p.observer, now);
        p.observer
            .record_overhead(t0.elapsed().as_secs_f32() * 1_000.0);
        out
    }

    pub(super) fn observe_speech(&mut self, e: &SpeechEvent) -> Vec<Finding> {
        match e {
            SpeechEvent::Transcript(r) => {
                self.observe(e.stream(), |o, now| o.on_transcript(r, now))
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn observe_semantic(&mut self, e: &SemanticEvent) -> Vec<Finding> {
        self.observe(e.stream(), |o, now| o.on_semantic(e, now))
    }

    pub(super) fn observe_translation(&mut self, e: &TranslationEvent) -> Vec<Finding> {
        self.observe(e.stream(), |o, now| o.on_translation(e, now))
    }

    pub(super) fn observe_analysis(&mut self, e: &AnalysisEvent) -> Vec<Finding> {
        self.observe(e.stream(), |o, _| o.on_analysis(e))
    }

    pub(super) fn observe_audio_gap(&mut self, stream: AudioStreamId, lost: Option<u64>) {
        let f = self.observe(stream, |o, _| o.on_audio_gap(lost));
        self.report_findings(stream, f);
    }

    /// Publish findings (after the stage event they concern).
    pub(super) fn report_findings(&mut self, stream: AudioStreamId, findings: Vec<Finding>) {
        if findings.is_empty() {
            return;
        }
        let session = self.pipelines.running.get(&stream).map(|p| p.session);
        for f in findings {
            let event = match f {
                Finding::Causal {
                    stage,
                    utterance,
                    revision,
                    kind,
                } => {
                    Metrics::inc(&self.metrics.causal_violations, 1);
                    OrchestrationEvent::CausalViolation {
                        stream,
                        stage,
                        utterance,
                        revision,
                        kind,
                    }
                }
                Finding::Timeline { stage, kind, frame } => {
                    if !matches!(
                        kind,
                        ce_core::orchestration::TimelineKind::ReAnchor
                            | ce_core::orchestration::TimelineKind::Gap
                            | ce_core::orchestration::TimelineKind::UnknownTiming
                    ) {
                        Metrics::inc(&self.metrics.timeline_anomalies, 1);
                    }
                    OrchestrationEvent::Timeline {
                        stream,
                        stage,
                        kind,
                        frame,
                    }
                }
                Finding::Deadline {
                    stage,
                    utterance,
                    latency_ms,
                    budget_ms,
                } => {
                    Metrics::inc(&self.metrics.deadline_misses, 1);
                    OrchestrationEvent::DeadlineMissed {
                        stream,
                        stage,
                        utterance,
                        latency_ms,
                        budget_ms,
                    }
                }
            };
            self.emit(EventPayload::Orchestration(event), session);
        }
    }

    // ---------------------------------------------------------------- control

    fn default_stage_config(&self, kind: StageKind) -> StageConfig {
        match kind {
            StageKind::Analysis => StageConfig::Analysis(Box::new(self.config.analysis.clone())),
            StageKind::Recognition => {
                StageConfig::Recognition(Box::new(self.config.speech.recognition.clone()))
            }
            StageKind::Semantics => {
                StageConfig::Semantics(Box::new(self.config.semantic.semantic.clone()))
            }
            StageKind::Translation => {
                StageConfig::Translation(Box::new(self.config.translation.translation.clone()))
            }
        }
    }

    /// Check that a stage can be started on `stream` (adapters, no worker
    /// already running) without starting anything.
    fn preflight(&self, stream: AudioStreamId, s: &ResolvedStage) -> CeResult<()> {
        if self.stage_generation(s.kind, stream).is_some() {
            return Err(CeError::lifecycle(
                "orchestration.stage_already_running",
                "a worker of this stage already runs on the stream outside the pipeline",
            )
            .with_context("stage", s.kind.as_str()));
        }
        match &s.config {
            StageConfig::Analysis(_) => Ok(()),
            StageConfig::Recognition(c) => self.resolve_adapter(c).map(|_| ()),
            StageConfig::Semantics(c) => self.resolve_semantic(c).map(|_| ()),
            StageConfig::Translation(c) => {
                if c.target.is_none() {
                    return Err(CeError::invalid_input(
                        "translation.no_target",
                        "a target language is required",
                    )
                    .with_context("stage", s.kind.as_str()));
                }
                self.select_adapters(c).map(|_| ())
            }
        }
        .map_err(|e| e.with_context("stage", s.kind.as_str()))
    }

    fn start_stage(&mut self, stream: AudioStreamId, s: &ResolvedStage) -> CeResult<()> {
        match s.config.clone() {
            StageConfig::Analysis(c) => self.start_analysis(stream, Some(*c), None).map(|_| ()),
            StageConfig::Recognition(c) => self.start_recognition(stream, Some(*c)).map(|_| ()),
            StageConfig::Semantics(c) => self.start_semantics(stream, Some(*c)).map(|_| ()),
            StageConfig::Translation(c) => self.start_translation(stream, Some(*c)).map(|_| ()),
        }
    }

    async fn stop_stage(&mut self, kind: StageKind, stream: AudioStreamId) {
        let _ = match kind {
            StageKind::Analysis => self.stop_analysis(stream).await,
            StageKind::Recognition => self.stop_recognition(stream).await,
            StageKind::Semantics => self.stop_semantics(stream).await,
            StageKind::Translation => self.stop_translation(stream).await,
        };
    }

    async fn await_stage(&mut self, kind: StageKind, stream: AudioStreamId, deadline: Instant) {
        match kind {
            StageKind::Analysis => self.await_analysis_end(&[stream], deadline).await,
            StageKind::Recognition => self.await_recognition_end(&[stream], deadline).await,
            StageKind::Semantics => self.await_semantic_end(&[stream], deadline).await,
            StageKind::Translation => self.await_translation_end(&[stream], deadline).await,
        }
    }

    /// Validate and start a pipeline.
    pub(super) async fn start_pipeline(
        &mut self,
        stream: AudioStreamId,
        spec: PipelineSpec,
    ) -> CeResult<CommandOutcome> {
        spec.validate()?;
        let (session, rate) = {
            let audio = self.audio.as_ref().ok_or_else(|| {
                CeError::new(
                    ErrorCategory::CapabilityResolution,
                    "orchestration.unavailable",
                    "pipelines need an audio backend",
                )
            })?;
            let (info, state, ended) = audio.stream(stream).ok_or_else(|| {
                CeError::invalid_input("audio.unknown_stream", "no such audio stream")
                    .with_context("stream", stream.to_string())
            })?;
            if info.direction != AudioDirection::Capture {
                return Err(CeError::invalid_input(
                    "orchestration.not_capture",
                    "pipelines run on capture streams",
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
                    "orchestration.stream_ended",
                    "the stream has ended",
                ));
            }
            (info.session_id, info.format.delivered.sample_rate)
        };
        if self.pipelines.running.contains_key(&stream) {
            return Err(CeError::lifecycle(
                "orchestration.already_running",
                "this stream already has a pipeline",
            ));
        }
        let max = self.config.orchestration.max_pipelines as usize;
        if self.pipelines.running.len() >= max {
            return Err(CeError::new(
                ErrorCategory::QueueSaturated,
                "orchestration.capacity_exhausted",
                "the maximum number of concurrent pipelines has been reached",
            )
            .with_context("max", max.to_string()));
        }
        let resolved: Vec<ResolvedStage> = spec
            .stages
            .iter()
            .map(|s| ResolvedStage::resolve(s, |k| self.default_stage_config(k)))
            .collect();
        let graph = ce_orchestrate::validate(resolved, spec.stop, spec.deadlines)?;
        // Pre-flight before anything starts: required problems fail the
        // pipeline now; optional ones degrade the stage (and its optional
        // dependents) explicitly.
        let mut degraded: BTreeMap<StageKind, (FaultClass, String)> = BTreeMap::new();
        for s in &graph.stages {
            if let Err(e) = self.preflight(stream, s) {
                if s.requirement == Requirement::Required {
                    return Err(e);
                }
                degraded.insert(s.kind, (FaultClass::of(&e), e.code().to_string()));
            }
        }
        let epoch = self.pipelines.epochs.issue();
        let stages: Vec<StageStatus> = graph
            .stages
            .iter()
            .map(|s| StageStatus {
                kind: s.kind,
                requirement: s.requirement,
                state: StageState::Pending,
                generation: None,
                config_fingerprint: s.config.fingerprint(),
                reason: None,
                stop: None,
            })
            .collect();
        let observer = Observer::new(
            rate,
            self.config.orchestration.observer_utterances as usize,
            graph.deadlines,
            TimingUncertainty::Unknown,
        );
        self.pipelines.running.insert(
            stream,
            Pipeline {
                session,
                spec: spec.clone(),
                graph: graph.clone(),
                state: PipelineState::Starting,
                epoch,
                reset_epoch: 0,
                stages,
                observer,
                ends: BTreeMap::new(),
                stopping: false,
                finish_queued: false,
            },
        );
        Metrics::inc(&self.metrics.pipelines_started, 1);
        let mut started: Vec<StageKind> = Vec::new();
        for s in &graph.stages {
            // An optional stage whose input stage is unavailable cannot run.
            let dep_missing = graph
                .dependencies(s.kind)
                .iter()
                .any(|d| degraded.contains_key(d));
            if dep_missing && !degraded.contains_key(&s.kind) {
                degraded.insert(
                    s.kind,
                    (
                        FaultClass::UnavailableCapability,
                        "orchestration.dependency_unavailable".into(),
                    ),
                );
            }
            if let Some((fault, code)) = degraded.get(&s.kind).cloned() {
                self.degrade_stage(stream, s.kind, fault, code);
                continue;
            }
            match self.start_stage(stream, s) {
                Ok(()) => {
                    let generation = self.stage_generation(s.kind, stream);
                    if let Some(p) = self.pipelines.running.get_mut(&stream) {
                        if let Some(st) = p.stage_mut(s.kind) {
                            st.state = StageState::Running;
                            st.generation = generation;
                        }
                    }
                    started.push(s.kind);
                    if let Some(generation) = generation {
                        self.emit(
                            EventPayload::Orchestration(OrchestrationEvent::StageStarted {
                                stream,
                                epoch,
                                stage: s.kind,
                                generation,
                            }),
                            Some(session),
                        );
                    }
                }
                Err(e) if s.requirement == Requirement::Optional => {
                    degraded.insert(s.kind, (FaultClass::of(&e), e.code().to_string()));
                    self.degrade_stage(stream, s.kind, FaultClass::of(&e), e.code().to_string());
                }
                Err(e) => {
                    // Roll back what started, downstream first.
                    if let Some(p) = self.pipelines.running.get_mut(&stream) {
                        p.stopping = true;
                        if let Some(st) = p.stage_mut(s.kind) {
                            st.state = StageState::Failed;
                            st.reason = Some(e.code().to_string());
                        }
                    }
                    for k in started.iter().rev() {
                        self.stop_stage(*k, stream).await;
                    }
                    self.fail_pipeline(stream, Some(s.kind), FaultClass::of(&e), &e);
                    return Err(e.with_context("stage", s.kind.as_str()));
                }
            }
        }
        let Some(p) = self.pipelines.running.get_mut(&stream) else {
            return Err(CeError::internal(
                "orchestration.vanished",
                "pipeline disappeared while starting",
            ));
        };
        let to = if degraded.is_empty() {
            PipelineState::Running
        } else {
            PipelineState::Degraded
        };
        p.state = p.state.transition_to(to)?;
        let kinds = p.graph.stages.iter().map(|s| s.kind).collect();
        let notes = p.graph.notes.clone();
        let fingerprint = p.graph.fingerprint.clone();
        let status = p.status(stream, None);
        self.log(
            Observation::log(Level::Info, "orchestration.pipeline_started")
                .field("stream", stream)
                .field("epoch", epoch)
                .field("fingerprint", fingerprint.as_str()),
        );
        self.emit(
            EventPayload::Orchestration(OrchestrationEvent::PipelineStarted {
                stream,
                epoch,
                fingerprint,
                stages: kinds,
                notes,
            }),
            Some(session),
        );
        Ok(CommandOutcome::Pipeline {
            status: Box::new(status),
        })
    }

    fn degrade_stage(
        &mut self,
        stream: AudioStreamId,
        stage: StageKind,
        fault: FaultClass,
        code: String,
    ) {
        let Some(p) = self.pipelines.running.get_mut(&stream) else {
            return;
        };
        if let Some(st) = p.stage_mut(stage) {
            st.state = StageState::Degraded;
            st.reason = Some(code.clone());
        }
        let (epoch, session) = (p.epoch, p.session);
        self.log(
            Observation::log(Level::Warn, "orchestration.stage_degraded")
                .field("stream", stream)
                .field("stage", stage.as_str())
                .field("code", code.as_str()),
        );
        self.emit(
            EventPayload::Orchestration(OrchestrationEvent::StageDegraded {
                stream,
                epoch,
                stage,
                fault,
                code,
            }),
            Some(session),
        );
    }

    /// Mark a pipeline failed (stages already stopped) and retire it.
    fn fail_pipeline(
        &mut self,
        stream: AudioStreamId,
        stage: Option<StageKind>,
        fault: FaultClass,
        error: &CeError,
    ) {
        let Some(mut p) = self.pipelines.running.remove(&stream) else {
            return;
        };
        p.state = PipelineState::Failed;
        Metrics::inc(&self.metrics.pipelines_failed, 1);
        let report = error.report(&self.config.runtime.redaction);
        self.log(
            Observation::log(Level::Warn, "orchestration.pipeline_failed")
                .field("stream", stream)
                .field("code", error.code()),
        );
        self.emit(
            EventPayload::Orchestration(OrchestrationEvent::PipelineFailed {
                stream,
                epoch: p.epoch,
                stage,
                fault,
                error: report,
            }),
            Some(p.session),
        );
        let status = p.status(stream, Some(PipelineEndReason::Failed));
        self.pipelines.remember(stream, status, p.spec);
    }

    /// Stop a running pipeline's stages by its stop mode and retire it.
    async fn stop_procedure(
        &mut self,
        stream: AudioStreamId,
        reason: PipelineEndReason,
        mode: Option<StopMode>,
        final_state: PipelineState,
    ) -> Option<PipelineStatus> {
        let (order, mode, running) = {
            let p = self.pipelines.running.get_mut(&stream)?;
            p.stopping = true;
            // Stages running when the stop began (a downstream stage may end
            // on its own while upstream ones are stopped: that is a clean,
            // drained end, not "not running").
            let running: BTreeMap<StageKind, u64> = p
                .stages
                .iter()
                .filter(|s| s.state == StageState::Running)
                .filter_map(|s| s.generation.map(|g| (s.kind, g)))
                .collect();
            if let Ok(to) = p.state.transition_to(PipelineState::Stopping) {
                p.state = to;
            }
            let mut graph = p.graph.clone();
            if let Some(m) = mode {
                graph.stop = m;
            }
            (graph.stop_order(), graph.stop, running)
        };
        let has_recognition = order.contains(&StageKind::Recognition);
        let deadline = Instant::now() + self.config.audio.operation_timeout();
        let mut outcomes = Vec::new();
        for kind in order {
            let outcome = match running.get(&kind).copied() {
                None => StopOutcome::NotRunning,
                Some(g) => {
                    let drained = mode == StopMode::Drain
                        && has_recognition
                        && matches!(kind, StageKind::Semantics | StageKind::Translation);
                    if self.stage_generation(kind, stream) != Some(g) {
                        // Ended on its own before we got to it.
                    } else if drained {
                        // Its input ended with recognition: let it finish
                        // what it has, within the bound.
                        self.await_stage(kind, stream, deadline).await;
                    } else {
                        self.stop_stage(kind, stream).await;
                    }
                    match self
                        .pipelines
                        .running
                        .get(&stream)
                        .and_then(|p| p.ends.get(&kind))
                    {
                        Some((eg, _, true)) if *eg == g => StopOutcome::TimedOut,
                        Some((eg, _, false)) if *eg == g => StopOutcome::Clean,
                        _ => StopOutcome::TimedOut,
                    }
                }
            };
            if let Some(p) = self.pipelines.running.get_mut(&stream) {
                if let Some(st) = p.stage_mut(kind) {
                    st.stop = Some(outcome);
                    if st.state == StageState::Running {
                        st.state = StageState::Ended;
                    }
                }
            }
            outcomes.push((kind, outcome));
        }
        let mut p = self.pipelines.running.remove(&stream)?;
        p.state = final_state;
        let counters: OrchestrationCounters = p.observer.counters().clone();
        self.log(
            Observation::log(Level::Info, "orchestration.pipeline_stopped")
                .field("stream", stream)
                .field("epoch", p.epoch)
                .field("reason", format!("{reason:?}")),
        );
        self.emit(
            EventPayload::Orchestration(OrchestrationEvent::PipelineStopped {
                stream,
                epoch: p.epoch,
                reason,
                outcomes,
                counters: Box::new(counters),
            }),
            Some(p.session),
        );
        let status = p.status(stream, Some(reason));
        self.pipelines
            .remember(stream, status.clone(), p.spec.clone());
        Some(status)
    }

    /// Run deferred pipeline work (required failures, source ends).
    pub(super) async fn process_pipeline_actions(&mut self) {
        while let Some(action) = self.pipelines.pending.pop_front() {
            match action {
                Pending::Finish {
                    stream,
                    epoch,
                    reason,
                    failure,
                } => {
                    let current = self.pipelines.running.get(&stream).map(|p| p.epoch);
                    if current != Some(epoch) {
                        continue; // already stopped, restarted or retired
                    }
                    match failure {
                        Some((stage, fault, code)) => {
                            let session = self.pipelines.running.get(&stream).map(|p| p.session);
                            let err = CeError::new(
                                ErrorCategory::Provider,
                                "orchestration.required_stage_failed",
                                "a required pipeline stage failed",
                            )
                            .with_context("stage", stage.as_str())
                            .with_context("cause", code);
                            Metrics::inc(&self.metrics.pipelines_failed, 1);
                            let report = err.report(&self.config.runtime.redaction);
                            self.emit(
                                EventPayload::Orchestration(OrchestrationEvent::PipelineFailed {
                                    stream,
                                    epoch,
                                    stage: Some(stage),
                                    fault,
                                    error: report,
                                }),
                                session,
                            );
                            self.stop_procedure(
                                stream,
                                PipelineEndReason::Failed,
                                Some(StopMode::Cancel),
                                PipelineState::Failed,
                            )
                            .await;
                        }
                        None => {
                            self.stop_procedure(stream, reason, None, PipelineState::Stopped)
                                .await;
                        }
                    }
                }
            }
        }
    }

    /// Stop a pipeline (its stop mode).
    pub(super) async fn stop_pipeline(
        &mut self,
        stream: AudioStreamId,
    ) -> CeResult<CommandOutcome> {
        if !self.pipelines.running.contains_key(&stream) {
            return self.query_pipeline(stream);
        }
        let status = self
            .stop_procedure(
                stream,
                PipelineEndReason::Stopped,
                None,
                PipelineState::Stopped,
            )
            .await
            .ok_or_else(|| not_found(stream))?;
        Ok(CommandOutcome::Pipeline {
            status: Box::new(status),
        })
    }

    /// Reset running stages in dependency order; identifiers continue.
    pub(super) fn reset_pipeline(&mut self, stream: AudioStreamId) -> CeResult<CommandOutcome> {
        let kinds: Vec<StageKind> = {
            let p = self
                .pipelines
                .running
                .get(&stream)
                .ok_or_else(|| not_found(stream))?;
            p.stages
                .iter()
                .filter(|s| s.state == StageState::Running)
                .map(|s| s.kind)
                .collect()
        };
        for kind in kinds {
            let r = match kind {
                StageKind::Analysis => Ok(()),
                StageKind::Recognition => self.reset_recognition(stream).map(|_| ()),
                StageKind::Semantics => self.reset_semantic_context(stream).map(|_| ()),
                StageKind::Translation => self.reset_translation(stream).map(|_| ()),
            };
            if let Err(e) = r {
                self.log(
                    Observation::log(Level::Warn, "orchestration.reset_stage_failed")
                        .field("stream", stream)
                        .field("stage", kind.as_str())
                        .field("code", e.code()),
                );
            }
        }
        let p = self
            .pipelines
            .running
            .get_mut(&stream)
            .ok_or_else(|| not_found(stream))?;
        p.reset_epoch += 1;
        let (epoch, reset_epoch, session) = (p.epoch, p.reset_epoch, p.session);
        let status = p.status(stream, None);
        self.emit(
            EventPayload::Orchestration(OrchestrationEvent::PipelineReset {
                stream,
                epoch,
                reset_epoch,
            }),
            Some(session),
        );
        Ok(CommandOutcome::Pipeline {
            status: Box::new(status),
        })
    }

    /// Stop (if running) and start again with a new epoch.
    pub(super) async fn restart_pipeline(
        &mut self,
        stream: AudioStreamId,
        spec: Option<PipelineSpec>,
    ) -> CeResult<CommandOutcome> {
        let stored = match self.pipelines.running.get(&stream) {
            Some(p) => Some(p.spec.clone()),
            None => self.pipelines.finished.get(&stream).map(|(_, s)| s.clone()),
        };
        let spec = spec.or(stored).ok_or_else(|| not_found(stream))?;
        spec.validate()?;
        if self.pipelines.running.contains_key(&stream) {
            self.stop_procedure(
                stream,
                PipelineEndReason::Restart,
                None,
                PipelineState::Stopped,
            )
            .await;
        }
        self.start_pipeline(stream, spec).await
    }

    /// Current or final status.
    pub(super) fn query_pipeline(&self, stream: AudioStreamId) -> CeResult<CommandOutcome> {
        if let Some(p) = self.pipelines.running.get(&stream) {
            return Ok(CommandOutcome::Pipeline {
                status: Box::new(p.status(stream, None)),
            });
        }
        self.pipelines
            .finished
            .get(&stream)
            .map(|(s, _)| CommandOutcome::Pipeline {
                status: Box::new(s.clone()),
            })
            .ok_or_else(|| not_found(stream))
    }

    /// Running and recently ended pipelines.
    pub(super) fn pipeline_statuses(&self) -> Vec<PipelineStatus> {
        let mut out: Vec<PipelineStatus> = self
            .pipelines
            .running
            .iter()
            .map(|(s, p)| p.status(*s, None))
            .collect();
        out.extend(self.pipelines.finished.values().map(|(s, _)| s.clone()));
        out
    }

    /// Session close / shutdown is about to stop every stage of the
    /// session's (or every) pipeline: stage ends are expected from now on.
    pub(super) fn pipelines_teardown_begin(&mut self, session: Option<SessionId>) {
        for p in self.pipelines.running.values_mut() {
            if session.is_none_or(|s| s == p.session) {
                p.stopping = true;
                if let Ok(to) = p.state.transition_to(PipelineState::Stopping) {
                    p.state = to;
                }
            }
        }
    }

    /// The teardown stopped every stage: retire the pipelines.
    pub(super) fn pipelines_teardown_end(&mut self, session: Option<SessionId>) {
        let streams: Vec<AudioStreamId> = self
            .pipelines
            .running
            .iter()
            .filter(|(_, p)| session.is_none_or(|s| s == p.session))
            .map(|(s, _)| *s)
            .collect();
        for stream in streams {
            let Some(mut p) = self.pipelines.running.remove(&stream) else {
                continue;
            };
            let mut outcomes = Vec::new();
            for st in &mut p.stages {
                let outcome = match (st.generation, p.ends.get(&st.kind)) {
                    (Some(g), Some((eg, _, timed_out))) if *eg == g => {
                        if *timed_out {
                            StopOutcome::TimedOut
                        } else {
                            StopOutcome::Clean
                        }
                    }
                    (Some(_), _) if st.state == StageState::Running => StopOutcome::TimedOut,
                    _ => StopOutcome::NotRunning,
                };
                st.stop = Some(outcome);
                if st.state == StageState::Running {
                    st.state = StageState::Ended;
                }
                outcomes.push((st.kind, outcome));
            }
            p.state = PipelineState::Stopped;
            let counters = p.observer.counters().clone();
            self.emit(
                EventPayload::Orchestration(OrchestrationEvent::PipelineStopped {
                    stream,
                    epoch: p.epoch,
                    reason: PipelineEndReason::Shutdown,
                    outcomes,
                    counters: Box::new(counters),
                }),
                Some(p.session),
            );
            let status = p.status(stream, Some(PipelineEndReason::Shutdown));
            self.pipelines.remember(stream, status, p.spec);
        }
    }
}
