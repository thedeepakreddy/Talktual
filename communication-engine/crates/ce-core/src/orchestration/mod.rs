//! Streaming orchestration and timing contracts (Phase 7).
//!
//! A **pipeline** coordinates the existing stage workers of one capture
//! stream — analysis, recognition, semantics, translation — as an
//! explicit, validated graph with required and optional stages, lifecycle
//! epochs, stale-output rejection, causal and timeline validation, latency
//! measurement and deadline outcomes. Orchestration coordinates; it never
//! changes what a stage computes, and its events and status carry
//! identifiers, counts, kinds and durations only (no content).
//!
//! * [`config`] — pipeline specification, policies and engine settings;
//! * [`model`] — states, status, counters, latency and timeline summaries,
//!   fault classes and events.
//!
//! Graph validation and the observer live in `ce-orchestrate`; the
//! runtime applies them (`architecture/orchestration.md`).

pub mod config;
pub mod model;

pub use config::{
    DeadlinePolicy, OrchestrationSettings, PipelineSpec, Requirement, StageConfig, StageKind,
    StageSpec, StopMode,
};
pub use model::{
    CausalKind, FaultClass, LatencyMetric, LatencySummary, OrchestrationCounters,
    OrchestrationEvent, PipelineEndReason, PipelineState, PipelineStatus, StageEnd, StageState,
    StageStatus, StopOutcome, TimelineKind, TimelineStatus, TimingUncertainty,
};
