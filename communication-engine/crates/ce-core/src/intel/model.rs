//! Analysis results, metadata, events and status.
//!
//! Units: times are CE monotonic nanoseconds ([`MonotonicTime`]) on the
//! stream's sample clock (the Phase 2 block `timestamp` timeline); positions
//! are frames of that sample clock; levels are dBFS where 0 dBFS is the RMS
//! of a full-scale square wave (a full-scale sine is -3.01 dBFS).

use serde::{Deserialize, Serialize};

use crate::audio::DiscontinuityKind;
use crate::error::ErrorReport;
use crate::ids::AudioStreamId;
use crate::time::MonotonicTime;

/// What a detector's score means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreSemantics {
    /// A calibrated probability of activity.
    CalibratedProbability,
    /// A monotonic confidence in [0, 1]: higher means more likely active,
    /// but values are **not** probabilities and are not comparable across
    /// detectors.
    HeuristicConfidence,
    /// Only 0 or 1.
    Binary,
}

/// Static facts about a voice-activity detector.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VadDescriptor {
    /// Stable id, e.g. `ce.vad.spectral`.
    pub id: String,
    /// Implementation version.
    pub version: String,
    /// Score semantics.
    pub score_semantics: ScoreSemantics,
    /// Lowest supported sample rate (Hz).
    pub min_sample_rate: u32,
    /// Highest supported sample rate (Hz).
    pub max_sample_rate: u32,
    /// Whether identical input always yields identical scores.
    pub deterministic: bool,
    /// Look-ahead the detector needs beyond the frame it scores, ms.
    pub lookahead_ms: u32,
    /// Frames scored before its scores are considered settled (noise-floor
    /// adaptation), in hops.
    pub warmup_hops: u32,
}

/// What the analysis pipeline can and cannot report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisCapabilities {
    /// The detector in use.
    pub vad: VadDescriptor,
    /// Quality analysis available.
    pub quality: bool,
    /// Condition indicators available.
    pub conditions: Vec<ConditionKind>,
    /// Overlapping activity (several simultaneous sources) detection.
    /// **Not supported**: a single-channel energy/spectral analysis cannot
    /// separate sources, so CE does not claim it.
    pub overlap_detection: bool,
}

/// Algorithm identity in results.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlgorithmRef {
    /// Detector id.
    pub vad: String,
    /// Detector version.
    pub version: String,
    /// `AnalysisConfig::fingerprint`.
    pub config_fingerprint: String,
}

/// Metadata attached to every analysis result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResultMeta {
    /// Stream analysed (session/correlation ids are on the event envelope).
    pub stream: AudioStreamId,
    /// First and last source block sequence numbers that contributed.
    pub sequence_range: (u64, u64),
    /// Sample-clock frame range [start, end).
    pub frame_range: (u64, u64),
    /// Monotonic time range [start, end).
    pub time_range: (MonotonicTime, MonotonicTime),
    /// The most recent discontinuity before this result, if it shaped it.
    pub discontinuity: Option<DiscontinuityContext>,
    /// Degraded mode in force when the result was produced.
    pub mode: DegradedMode,
}

/// Why analysis state was reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "context")]
pub enum DiscontinuityContext {
    /// A gap flagged by Phase 2 (device overflow, delivery overflow, restart).
    Capture {
        /// Phase 2 kind.
        kind: DiscontinuityKind,
        /// Lost frames, when known.
        lost_frames: Option<u64>,
    },
    /// Blocks dropped from the analysis tap (analysis overload).
    AnalysisDrop {
        /// Blocks dropped.
        blocks: u64,
    },
    /// Sample rate or channel count changed.
    FormatChange,
}

/// Stabilized activity state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityState {
    /// No probable activity.
    Inactive,
    /// Probable acoustic activity (typically speech; not guaranteed).
    Active,
}

/// Why a segment closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentCloseReason {
    /// Silence reached `close_silence_ms`.
    SilencePolicy,
    /// The segment reached `max_segment_ms`; a continuation opens if
    /// activity continues.
    MaxDuration,
    /// Analysis was stopped.
    ExplicitStop,
    /// A discontinuity (gap, restart, format change); segments are never
    /// bridged across it.
    Discontinuity,
    /// The capture stream ended (closed, revoked, invalidated).
    StreamEnd,
    /// Blocks were dropped by the resource policy.
    ResourcePolicy,
    /// The analysis worker failed.
    Failure,
}

/// Confidence of an endpoint candidate, by silence tier. An endpoint
/// candidate means "activity has paused for this long", never "the speaker
/// has finished".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointConfidence {
    /// First tier.
    Low,
    /// Middle tiers.
    Medium,
    /// Last tier.
    High,
}

/// Overall usability of the input for downstream processing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Usability {
    /// No problems found.
    Good,
    /// Usable with caveats.
    Fair,
    /// Likely to degrade downstream results.
    Poor,
    /// No usable signal.
    Unusable,
}

/// Why quality is not `Good`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityReason {
    /// Window level below -80 dBFS (digital silence / dead input).
    NoSignal,
    /// Activity level below -45 dBFS.
    LowLevel,
    /// Estimated SNR below 15 dB (Fair) or 6 dB (Poor).
    LowSnr,
    /// Clipped samples above 0.05 % (Fair) or 0.5 % (Poor).
    Clipping,
    /// Frames lost to discontinuities in the window.
    Dropouts,
    /// Channels differ by more than 20 dB.
    ChannelImbalance,
    /// |mean| above 0.05 of full scale.
    DcOffset,
    /// Non-finite samples were received (replaced by 0).
    NonFinite,
}

/// Measured quality over one window. `None` = not measurable with the data
/// available (never a guess).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QualityMetrics {
    /// Window length, ms.
    pub window_ms: u32,
    /// RMS level, dBFS.
    pub level_dbfs: f32,
    /// Peak sample magnitude, dBFS (20·log10|x|).
    pub peak_dbfs: f32,
    /// Fraction of samples at or above the clip level.
    pub clipping_ratio: f32,
    /// 10th-percentile hop level over the last 3 s while inactive, dBFS;
    /// `None` until at least 30 inactive hops were seen.
    pub noise_floor_dbfs: Option<f32>,
    /// Active level minus noise floor, dB; `None` without both.
    pub snr_db: Option<f32>,
    /// Mean sample value (DC offset), full-scale units.
    pub dc_offset: f32,
    /// Largest per-channel RMS difference, dB; `None` for mono or when a
    /// channel is below -80 dBFS on both sides.
    pub channel_imbalance_db: Option<f32>,
    /// Non-finite samples received.
    pub non_finite_samples: u64,
    /// Frames lost to discontinuities in the window, when known.
    pub lost_frames: u64,
    /// Fraction of hops with stabilized activity.
    pub activity_ratio: f32,
    /// Usability verdict.
    pub usability: Usability,
    /// Reasons (sorted).
    pub reasons: Vec<QualityReason>,
}

/// Bounded set of signal-condition indicators. Each describes the signal,
/// never the person.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionKind {
    /// No activity for `sustained_silence_ms`.
    SustainedSilence,
    /// Stabilized activity present.
    ProbableActivity,
    /// Inactive input with a noise floor above the configured level.
    SustainedNoise,
    /// Clipped-sample ratio above the configured level.
    Clipping,
    /// RMS level above the configured overload level.
    Overload,
    /// Frames lost to discontinuities.
    Dropout,
}

/// Numbers behind a condition change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConditionEvidence {
    /// Measured value (unit depends on the kind: ms, dBFS, ratio, frames).
    pub value: f32,
    /// Threshold compared against.
    pub threshold: f32,
    /// Unit of `value`/`threshold`.
    pub unit: String,
}

/// A deterministic, reversible decision of the adaptive or resource policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecision {
    /// Noisy / low-SNR input: stricter onset and longer hangover.
    ConservativeSegmentation,
    /// Clipping or overload: noise floor and SNR are unreliable and are
    /// withheld from quality results.
    SuppressLevelEstimates,
    /// Unusable input: activity results carry no confidence.
    LowConfidenceActivity,
}

/// Resource-driven operating level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DegradedMode {
    /// Everything enabled.
    Full,
    /// Spectral features off: energy detector, basic quality, no conditions.
    Reduced,
    /// Only every other block is analysed; skipped blocks are gaps (segments
    /// close with `ResourcePolicy`).
    Shedding,
}

/// Why analysis ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisEndReason {
    /// `stop_analysis` was called.
    Stopped,
    /// The capture stream was closed.
    StreamClosed,
    /// Capture authorization was withdrawn; queued audio was discarded.
    Revoked,
    /// The capture stream was invalidated (device lost).
    StreamInvalidated,
    /// The capture stream failed.
    StreamFailed,
    /// The analysis worker panicked or stalled.
    Failed,
    /// The runtime shut down.
    Shutdown,
}

/// Running counters for one analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AnalysisCounters {
    /// Blocks analysed.
    pub blocks: u64,
    /// Hops (analysis frames) analysed.
    pub hops: u64,
    /// Blocks dropped by the tap because the analysis queue was full.
    pub tap_dropped_blocks: u64,
    /// Blocks skipped by the `Shedding` mode.
    pub shed_blocks: u64,
    /// Segments closed.
    pub segments: u64,
    /// Discontinuities handled.
    pub discontinuities: u64,
    /// Non-finite samples replaced.
    pub non_finite_samples: u64,
    /// Highest observed processing load (time / block duration), permille.
    pub max_load_permille: u32,
    /// Highest observed tap backlog, blocks.
    pub max_backlog: u32,
}

/// Snapshot of one analysis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnalysisStatus {
    /// Stream.
    pub stream: AudioStreamId,
    /// Algorithm identity.
    pub algorithm: AlgorithmRef,
    /// Capabilities.
    pub capabilities: AnalysisCapabilities,
    /// Whether the analysis is still running.
    pub running: bool,
    /// Current activity.
    pub activity: ActivityState,
    /// Current degraded mode.
    pub mode: DegradedMode,
    /// Active policy decisions.
    pub policies: Vec<PolicyDecision>,
    /// Active conditions.
    pub conditions: Vec<ConditionKind>,
    /// Last quality window, if any.
    pub quality: Option<QualityMetrics>,
    /// Counters.
    pub counters: AnalysisCounters,
    /// Why it ended, if it has.
    pub ended: Option<AnalysisEndReason>,
}

/// Analysis events (`EventPayload::Analysis`). Sparse by design: transitions,
/// segments, candidates, changes — never per-frame data or audio.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "analysis_event")]
#[non_exhaustive]
pub enum AnalysisEvent {
    /// Analysis began.
    Started {
        /// Stream.
        stream: AudioStreamId,
        /// Algorithm identity.
        algorithm: AlgorithmRef,
        /// Capabilities.
        capabilities: AnalysisCapabilities,
    },
    /// Stabilized activity changed. `meta.time_range.0` is the boundary
    /// (backdated to the first qualifying hop); `decision_latency_ms` is how
    /// much later the change was decided.
    ActivityChanged {
        /// Metadata.
        meta: ResultMeta,
        /// New state.
        state: ActivityState,
        /// Mean detector score over the qualifying hops (see the detector's
        /// score semantics); `None` under `LowConfidenceActivity`.
        score: Option<f32>,
        /// Decision delay after the boundary, ms.
        decision_latency_ms: u32,
    },
    /// A candidate segment opened.
    SegmentStarted {
        /// Metadata (start at the activity onset).
        meta: ResultMeta,
        /// Per-stream segment number.
        segment: u64,
        /// Whether it continues a segment closed by `MaxDuration`.
        continuation: bool,
    },
    /// A candidate segment closed. `meta.time_range` is [first activity,
    /// last activity).
    SegmentEnded {
        /// Metadata.
        meta: ResultMeta,
        /// Segment number.
        segment: u64,
        /// Duration, ms.
        duration_ms: u32,
        /// Why it closed.
        reason: SegmentCloseReason,
        /// Mean detector score over active hops.
        mean_score: Option<f32>,
    },
    /// Activity has paused for a policy tier. Not a judgement that anyone
    /// finished speaking.
    EndpointCandidate {
        /// Metadata (range = the pause so far).
        meta: ResultMeta,
        /// Segment number.
        segment: u64,
        /// Silence so far, ms.
        silence_ms: u32,
        /// Tier (index into `endpoint_tiers_ms`).
        tier: u8,
        /// Confidence by tier.
        confidence: EndpointConfidence,
    },
    /// A silence window ended (≥ `min_silence_report_ms`).
    SilenceWindow {
        /// Metadata (range = the silence).
        meta: ResultMeta,
        /// Duration, ms.
        duration_ms: u32,
        /// Mean level during the silence, dBFS.
        mean_level_dbfs: f32,
    },
    /// Quality usability or reasons changed (after two consistent windows).
    QualityChanged {
        /// Metadata (range = the window).
        meta: ResultMeta,
        /// Metrics.
        metrics: QualityMetrics,
    },
    /// A signal condition became active or cleared.
    ConditionChanged {
        /// Metadata.
        meta: ResultMeta,
        /// Condition.
        condition: ConditionKind,
        /// Active now.
        active: bool,
        /// Evidence.
        evidence: ConditionEvidence,
    },
    /// An adaptive-policy decision was applied or reverted.
    PolicyChanged {
        /// Metadata.
        meta: ResultMeta,
        /// Decision.
        decision: PolicyDecision,
        /// Applied (true) or reverted (false).
        active: bool,
    },
    /// The resource policy changed the operating level.
    DegradedModeChanged {
        /// Stream.
        stream: AudioStreamId,
        /// Previous level.
        from: DegradedMode,
        /// New level.
        to: DegradedMode,
        /// Load (permille) and backlog (blocks) that triggered it.
        load_permille: u32,
        /// Backlog in blocks.
        backlog: u32,
    },
    /// Analysis state was reset by a discontinuity.
    Discontinuity {
        /// Metadata.
        meta: ResultMeta,
        /// What happened.
        context: DiscontinuityContext,
    },
    /// The analysis worker failed.
    Failed {
        /// Stream.
        stream: AudioStreamId,
        /// Redacted error.
        error: ErrorReport,
    },
    /// Analysis ended.
    Stopped {
        /// Stream.
        stream: AudioStreamId,
        /// Why.
        reason: AnalysisEndReason,
        /// Final counters.
        counters: AnalysisCounters,
    },
}

impl AnalysisEvent {
    /// Stable kind name.
    pub fn kind(&self) -> &'static str {
        match self {
            AnalysisEvent::Started { .. } => "started",
            AnalysisEvent::ActivityChanged { .. } => "activity_changed",
            AnalysisEvent::SegmentStarted { .. } => "segment_started",
            AnalysisEvent::SegmentEnded { .. } => "segment_ended",
            AnalysisEvent::EndpointCandidate { .. } => "endpoint_candidate",
            AnalysisEvent::SilenceWindow { .. } => "silence_window",
            AnalysisEvent::QualityChanged { .. } => "quality_changed",
            AnalysisEvent::ConditionChanged { .. } => "condition_changed",
            AnalysisEvent::PolicyChanged { .. } => "policy_changed",
            AnalysisEvent::DegradedModeChanged { .. } => "degraded_mode_changed",
            AnalysisEvent::Discontinuity { .. } => "discontinuity",
            AnalysisEvent::Failed { .. } => "failed",
            AnalysisEvent::Stopped { .. } => "stopped",
        }
    }

    /// The stream the event concerns.
    pub fn stream(&self) -> AudioStreamId {
        match self {
            AnalysisEvent::Started { stream, .. }
            | AnalysisEvent::DegradedModeChanged { stream, .. }
            | AnalysisEvent::Failed { stream, .. }
            | AnalysisEvent::Stopped { stream, .. } => *stream,
            AnalysisEvent::ActivityChanged { meta, .. }
            | AnalysisEvent::SegmentStarted { meta, .. }
            | AnalysisEvent::SegmentEnded { meta, .. }
            | AnalysisEvent::EndpointCandidate { meta, .. }
            | AnalysisEvent::SilenceWindow { meta, .. }
            | AnalysisEvent::QualityChanged { meta, .. }
            | AnalysisEvent::ConditionChanged { meta, .. }
            | AnalysisEvent::PolicyChanged { meta, .. }
            | AnalysisEvent::Discontinuity { meta, .. } => meta.stream,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_serialize_with_explicit_tags_and_round_trip() {
        let stream = AudioStreamId::from_uuid(uuid::Uuid::from_u128(5));
        let meta = ResultMeta {
            stream,
            sequence_range: (3, 9),
            frame_range: (480, 1440),
            time_range: (MonotonicTime::from_nanos(10), MonotonicTime::from_nanos(20)),
            discontinuity: Some(DiscontinuityContext::AnalysisDrop { blocks: 2 }),
            mode: DegradedMode::Full,
        };
        let e = AnalysisEvent::SegmentEnded {
            meta,
            segment: 4,
            duration_ms: 1200,
            reason: SegmentCloseReason::SilencePolicy,
            mean_score: Some(0.8),
        };
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["analysis_event"], "segment_ended");
        assert_eq!(json["reason"], "silence_policy");
        assert_eq!(json["meta"]["discontinuity"]["context"], "analysis_drop");
        let back: AnalysisEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, e);
        assert_eq!(back.stream(), stream);
        assert_eq!(back.kind(), "segment_ended");
        assert!(Usability::Good < Usability::Unusable);
        assert!(DegradedMode::Full < DegradedMode::Shedding);
    }
}
