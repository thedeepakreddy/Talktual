//! Audio intelligence contracts (Phase 3).
//!
//! Acoustic analysis of captured audio — activity, segments, endpoint
//! candidates, silence, signal quality and a small set of signal-condition
//! indicators — plus the adaptive and resource policies that govern it.
//!
//! **Acoustic, not linguistic, and never about the person.** Nothing here
//! recognizes words or language, identifies speakers, or infers emotion,
//! health, intent or any other personal trait. Every observation is a
//! property of the *signal*. See `docs/architecture/audio-intelligence.md`.
//!
//! * [`config`] — typed, validated, fingerprinted analysis configuration;
//! * [`model`] — detector descriptors, results, result metadata, events and
//!   status.
//!
//! The algorithms live in the `ce-audio-intel` crate; the runtime exposes
//! them through the facade (`start_analysis`, `stop_analysis`,
//! `analysis_status`).

pub mod config;
pub mod model;

pub use config::{
    ActivityConfig, ActivityProfile, AnalysisBudget, AnalysisConfig, ConditionConfig,
    QualityConfig, SegmentConfig, VadAlgorithm,
};
pub use model::{
    ActivityState, AlgorithmRef, AnalysisCapabilities, AnalysisCounters, AnalysisEndReason,
    AnalysisEvent, AnalysisStatus, ConditionEvidence, ConditionKind, DegradedMode,
    EndpointConfidence, PolicyDecision, QualityMetrics, QualityReason, ResultMeta, ScoreSemantics,
    SegmentCloseReason, Usability, VadDescriptor,
};
