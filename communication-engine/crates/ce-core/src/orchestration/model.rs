//! Pipeline states, status, counters, summaries, fault classes and events.
//!
//! Everything here is **operational**: identifiers, kinds, counts, frame
//! positions and durations. No transcript, translation, observation or
//! audio content.

use serde::{Deserialize, Serialize};

use super::config::{Requirement, StageKind, StopMode};
use crate::error::{CeError, ErrorReport};
use crate::ids::{AudioStreamId, SessionId};
use crate::privacy::DataClass;

/// Pipeline lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineState {
    /// Stages are being started.
    Starting,
    /// Every started stage is running.
    Running,
    /// Running without at least one optional stage (reasons in the status).
    Degraded,
    /// Stages are being stopped.
    Stopping,
    /// Ended (see [`PipelineEndReason`]).
    Stopped,
    /// A required stage failed (to start or while running).
    Failed,
}

impl PipelineState {
    /// Whether `self → to` is legal.
    pub fn can_transition_to(self, to: PipelineState) -> bool {
        use PipelineState::*;
        matches!(
            (self, to),
            (Starting, Running | Degraded | Failed | Stopping)
                | (Running, Degraded | Stopping | Failed | Stopped)
                | (Degraded, Stopping | Failed | Stopped)
                | (Stopping, Stopped | Failed)
        )
    }

    /// Validated transition.
    pub fn transition_to(self, to: PipelineState) -> Result<PipelineState, CeError> {
        if self.can_transition_to(to) {
            Ok(to)
        } else {
            Err(CeError::lifecycle(
                "orchestration.invalid_transition",
                "illegal pipeline state transition",
            )
            .with_context("from", format!("{self:?}"))
            .with_context("to", format!("{to:?}")))
        }
    }

    /// Ended (stopped or failed).
    pub fn is_terminal(self) -> bool {
        matches!(self, PipelineState::Stopped | PipelineState::Failed)
    }
}

/// Why a pipeline ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineEndReason {
    /// Stop requested.
    Stopped,
    /// Every stage ended because the source ended.
    SourceEnded,
    /// A required stage failed.
    Failed,
    /// Restart requested (a new epoch follows).
    Restart,
    /// Session close or engine shutdown.
    Shutdown,
}

/// A stage's state within a pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageState {
    /// Not started yet.
    Pending,
    /// Running (worker of `generation`).
    Running,
    /// Optional stage that could not start or failed; the pipeline runs
    /// without it.
    Degraded,
    /// Ended normally (source ended or stopped).
    Ended,
    /// Failed.
    Failed,
}

/// How a stage's worker ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageEnd {
    /// Stopped by the pipeline or directly.
    Stopped,
    /// Its input ended.
    SourceEnded,
    /// Failed (panic, repeated errors, load failure).
    Failed,
    /// Stopped making progress and was abandoned.
    Stalled,
}

/// How stopping one stage went.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopOutcome {
    /// Ended within the bound.
    Clean,
    /// Did not end within the bound; abandoned and reported.
    TimedOut,
    /// Was not running.
    NotRunning,
}

/// Failure classes (spec §12).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultClass {
    /// Configuration or pipeline invalid.
    InvalidConfig,
    /// A capability (adapter, backend, stream) is unavailable.
    UnavailableCapability,
    /// A bound elapsed.
    Timeout,
    /// Capacity exhausted.
    Overload,
    /// Cancelled.
    Cancellation,
    /// A worker failed.
    WorkerFailure,
    /// A worker broke its output contract.
    ProtocolViolation,
    /// An output of an earlier generation.
    StaleEvent,
    /// Timing metadata is inconsistent.
    TemporalIntegrity,
    /// Shutdown could not complete cleanly.
    ShutdownFailure,
}

impl FaultClass {
    /// Classify an error by its category.
    pub fn of(error: &CeError) -> FaultClass {
        use crate::error::ErrorCategory as C;
        match error.category() {
            C::InvalidConfig | C::InvalidInput => FaultClass::InvalidConfig,
            C::CapabilityResolution => FaultClass::UnavailableCapability,
            C::Timeout => FaultClass::Timeout,
            C::QueueSaturated => FaultClass::Overload,
            C::Cancelled => FaultClass::Cancellation,
            C::Shutdown => FaultClass::ShutdownFailure,
            _ => FaultClass::WorkerFailure,
        }
    }
}

/// A causal or revision-contract finding about a stage output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CausalKind {
    /// A transcript revision not newer than one already seen.
    StaleRevision,
    /// The same revision twice.
    DuplicateRevision,
    /// A revision after the utterance's final or cancellation.
    AfterFinal,
    /// A derived output refers to a revision never seen.
    UnknownSource,
    /// A translation sequence did not increase within a worker run.
    SequenceRegression,
    /// A translation sequence skipped numbers within a worker run.
    SequenceGap,
}

/// A timeline finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineKind {
    /// End before start.
    MalformedRange,
    /// Time span inconsistent with the frame span by more than a frame.
    DurationMismatch,
    /// A new unit started before the previous one on the same stage.
    OutOfOrder,
    /// The sample-clock → monotonic mapping changed (new segment).
    ReAnchor,
    /// Timing unknown (no anchor): media time only.
    UnknownTiming,
    /// Frames were lost upstream (from an audio discontinuity).
    Gap,
}

/// Uncertainty of the sample-clock → monotonic mapping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TimingUncertainty {
    /// No bound is known (CE never assumes device/host synchronization).
    #[default]
    Unknown,
    /// Absolute error bounded by `bound_ns`.
    Bounded {
        /// Bound.
        bound_ns: u64,
    },
}

/// Timeline summary of a pipeline.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineStatus {
    /// Sample rate of the stream's sample clock.
    pub sample_rate: u32,
    /// Mapping segments (1 + re-anchors; 0 before any timed output).
    pub segments: u32,
    /// Media time (ns, `frames / rate`, exact) of the latest output end.
    pub media_end_ns: Option<u64>,
    /// Reported monotonic time (ns) of the latest output end.
    pub monotonic_end_ns: Option<u64>,
    /// Latest observed offset `monotonic − media` (ns), per segment.
    pub offset_ns: Option<i64>,
    /// Frames lost upstream (from audio discontinuities).
    pub lost_frames: u64,
    /// Mapping uncertainty.
    pub uncertainty: TimingUncertainty,
}

/// Measured latency or duration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyMetric {
    /// Utterance audio end → final transcript at the core.
    TranscriptFinal,
    /// Utterance audio end → semantic interpretation finalized.
    SemanticFinal,
    /// Utterance audio end → final translation at the core.
    TranslationFinal,
    /// Translation queue wait (worker-reported).
    TranslationQueue,
    /// Translation adapter time (worker-reported).
    TranslationAdapter,
    /// Observer time per event (orchestration overhead), in ms.
    Overhead,
}

/// Summary of one metric over the retained window.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LatencySummary {
    /// Metric.
    pub metric: LatencyMetric,
    /// Samples recorded (all time).
    pub count: u64,
    /// Median over the retained window (ms).
    pub p50_ms: f32,
    /// 95th percentile over the retained window (ms).
    pub p95_ms: f32,
    /// Maximum over the retained window (ms).
    pub max_ms: f32,
}

/// Counters of one pipeline.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrchestrationCounters {
    /// Transcript revisions observed.
    pub transcript_revisions: u64,
    /// Semantic observations and translation results observed.
    pub derived_outputs: u64,
    /// Derived outputs whose source revision was already superseded.
    pub superseded_outputs: u64,
    /// Derived outputs referring to utterances outside the bounded window.
    pub evicted_references: u64,
    /// Causal and revision-contract violations.
    pub causal_violations: u64,
    /// Timeline findings other than re-anchors and gaps.
    pub timeline_anomalies: u64,
    /// Mapping re-anchors.
    pub re_anchors: u64,
    /// Outputs with unknown timing.
    pub unknown_timing: u64,
    /// Upstream gaps.
    pub gaps: u64,
    /// Deadline misses.
    pub deadline_misses: u64,
    /// Outputs of earlier generations rejected for this stream.
    pub stale_outputs_rejected: u64,
    /// Translation worker runs observed (`Started`).
    pub translation_runs: u64,
}

/// A stage of a pipeline, as it is now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageStatus {
    /// Kind.
    pub kind: StageKind,
    /// Requirement.
    pub requirement: Requirement,
    /// State.
    pub state: StageState,
    /// Generation of its current or last worker.
    pub generation: Option<u64>,
    /// Configuration fingerprint.
    pub config_fingerprint: String,
    /// Why it is degraded or failed (stable error code), if it is.
    pub reason: Option<String>,
    /// How its last stop went, if it was stopped.
    pub stop: Option<StopOutcome>,
}

/// Status of a pipeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PipelineStatus {
    /// Stream.
    pub stream: AudioStreamId,
    /// Owning session.
    pub session: SessionId,
    /// State.
    pub state: PipelineState,
    /// Start/restart epoch (engine-wide monotonic; never reused).
    pub epoch: u64,
    /// Resets within the epoch.
    pub reset_epoch: u64,
    /// Graph fingerprint.
    pub fingerprint: String,
    /// Stop mode.
    pub stop_mode: StopMode,
    /// Stages in execution (topological) order.
    pub stages: Vec<StageStatus>,
    /// Validation notes (e.g. `translation.hints_unavailable`).
    pub notes: Vec<String>,
    /// Counters.
    pub counters: OrchestrationCounters,
    /// Latency summaries.
    pub latency: Vec<LatencySummary>,
    /// Timeline.
    pub timeline: TimelineStatus,
    /// Why it ended, if it did.
    pub ended: Option<PipelineEndReason>,
}

/// Orchestration events. All [`DataClass::Operational`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OrchestrationEvent {
    /// A pipeline started (some optional stages may be degraded).
    PipelineStarted {
        /// Stream.
        stream: AudioStreamId,
        /// Epoch.
        epoch: u64,
        /// Graph fingerprint.
        fingerprint: String,
        /// Stages in execution order.
        stages: Vec<StageKind>,
        /// Validation notes.
        notes: Vec<String>,
    },
    /// A stage's worker started.
    StageStarted {
        /// Stream.
        stream: AudioStreamId,
        /// Epoch.
        epoch: u64,
        /// Stage.
        stage: StageKind,
        /// Worker generation.
        generation: u64,
    },
    /// An optional stage is unavailable; the pipeline continues without it.
    StageDegraded {
        /// Stream.
        stream: AudioStreamId,
        /// Epoch.
        epoch: u64,
        /// Stage.
        stage: StageKind,
        /// Class.
        fault: FaultClass,
        /// Stable reason code.
        code: String,
    },
    /// A stage's worker ended.
    StageEnded {
        /// Stream.
        stream: AudioStreamId,
        /// Epoch.
        epoch: u64,
        /// Stage.
        stage: StageKind,
        /// Generation.
        generation: u64,
        /// How.
        end: StageEnd,
    },
    /// The pipeline was reset.
    PipelineReset {
        /// Stream.
        stream: AudioStreamId,
        /// Epoch.
        epoch: u64,
        /// New reset epoch.
        reset_epoch: u64,
    },
    /// The pipeline ended.
    PipelineStopped {
        /// Stream.
        stream: AudioStreamId,
        /// Epoch.
        epoch: u64,
        /// Why.
        reason: PipelineEndReason,
        /// Per-stage stop outcomes, in stop order.
        outcomes: Vec<(StageKind, StopOutcome)>,
        /// Final counters.
        counters: Box<OrchestrationCounters>,
    },
    /// A required stage failed; the pipeline is stopping or did not start.
    PipelineFailed {
        /// Stream.
        stream: AudioStreamId,
        /// Epoch.
        epoch: u64,
        /// The stage, if one is to blame.
        stage: Option<StageKind>,
        /// Class.
        fault: FaultClass,
        /// Redacted error.
        error: ErrorReport,
    },
    /// An output of an earlier worker generation was rejected (never
    /// published).
    StaleOutputRejected {
        /// Stream.
        stream: AudioStreamId,
        /// Stage.
        stage: StageKind,
        /// The output's generation.
        generation: u64,
        /// The stream's current generation for the stage, if any.
        current: Option<u64>,
    },
    /// A causal or revision-contract finding (the output was published:
    /// within a generation the worker is authoritative; ADR-0032).
    CausalViolation {
        /// Stream.
        stream: AudioStreamId,
        /// Stage of the output.
        stage: StageKind,
        /// Utterance.
        utterance: u64,
        /// Source revision referred to.
        revision: u32,
        /// Kind.
        kind: CausalKind,
    },
    /// A timeline finding.
    Timeline {
        /// Stream.
        stream: AudioStreamId,
        /// Stage of the output (recognition for audio gaps).
        stage: StageKind,
        /// Kind.
        kind: TimelineKind,
        /// Sample-clock frame where it was observed, if known.
        frame: Option<u64>,
    },
    /// A final output exceeded its audio-to-output budget.
    DeadlineMissed {
        /// Stream.
        stream: AudioStreamId,
        /// Stage.
        stage: StageKind,
        /// Utterance.
        utterance: u64,
        /// Measured latency (ms, monotonic).
        latency_ms: u32,
        /// Budget (ms).
        budget_ms: u32,
    },
}

impl OrchestrationEvent {
    /// Stream.
    pub fn stream(&self) -> AudioStreamId {
        match self {
            OrchestrationEvent::PipelineStarted { stream, .. }
            | OrchestrationEvent::StageStarted { stream, .. }
            | OrchestrationEvent::StageDegraded { stream, .. }
            | OrchestrationEvent::StageEnded { stream, .. }
            | OrchestrationEvent::PipelineReset { stream, .. }
            | OrchestrationEvent::PipelineStopped { stream, .. }
            | OrchestrationEvent::PipelineFailed { stream, .. }
            | OrchestrationEvent::StaleOutputRejected { stream, .. }
            | OrchestrationEvent::CausalViolation { stream, .. }
            | OrchestrationEvent::Timeline { stream, .. }
            | OrchestrationEvent::DeadlineMissed { stream, .. } => *stream,
        }
    }

    /// Stable name.
    pub fn kind(&self) -> &'static str {
        match self {
            OrchestrationEvent::PipelineStarted { .. } => "pipeline_started",
            OrchestrationEvent::StageStarted { .. } => "stage_started",
            OrchestrationEvent::StageDegraded { .. } => "stage_degraded",
            OrchestrationEvent::StageEnded { .. } => "stage_ended",
            OrchestrationEvent::PipelineReset { .. } => "pipeline_reset",
            OrchestrationEvent::PipelineStopped { .. } => "pipeline_stopped",
            OrchestrationEvent::PipelineFailed { .. } => "pipeline_failed",
            OrchestrationEvent::StaleOutputRejected { .. } => "stale_output_rejected",
            OrchestrationEvent::CausalViolation { .. } => "causal_violation",
            OrchestrationEvent::Timeline { .. } => "timeline",
            OrchestrationEvent::DeadlineMissed { .. } => "deadline_missed",
        }
    }

    /// Privacy class: always operational.
    pub fn data_class(&self) -> DataClass {
        DataClass::Operational
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_are_validated() {
        use PipelineState::*;
        assert!(Starting.can_transition_to(Running));
        assert!(Running.can_transition_to(Degraded));
        assert!(Degraded.can_transition_to(Stopping));
        assert!(Stopping.can_transition_to(Stopped));
        assert!(
            !Stopped.can_transition_to(Running),
            "restart is a new epoch, not a transition"
        );
        assert!(!Failed.can_transition_to(Running));
        assert!(
            !Degraded.can_transition_to(Running),
            "degradation is not silently undone"
        );
        assert_eq!(
            Stopped.transition_to(Running).unwrap_err().code(),
            "orchestration.invalid_transition"
        );
        assert!(Failed.is_terminal() && Stopped.is_terminal() && !Running.is_terminal());
    }

    #[test]
    fn events_serialize_without_content() {
        let e = OrchestrationEvent::DeadlineMissed {
            stream: AudioStreamId::from_uuid(uuid::Uuid::nil()),
            stage: StageKind::Translation,
            utterance: 3,
            latency_ms: 900,
            budget_ms: 500,
        };
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["type"], "deadline_missed");
        assert_eq!(json["stage"], "translation");
        assert_eq!(e.kind(), "deadline_missed");
        assert_eq!(e.data_class(), DataClass::Operational);
        let back: OrchestrationEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, e);
    }
}
