//! Streaming orchestration and timing logic (Phase 7). Pure: no async
//! runtime, threads, clocks or I/O — the runtime feeds it and applies its
//! decisions, so every rule here is deterministic and unit-testable.
//!
//! * [`graph`] — pipeline dependency rules, validation, deterministic order
//!   and fingerprint;
//! * [`epoch`] — worker generations and stale-output admission;
//! * [`reconcile`] — bounded transcript-revision reconciliation and
//!   source classification for derived outputs;
//! * [`timeline`] — sample-clock normalization, re-anchors, gaps, ordering
//!   and integrity checks;
//! * [`latency`] — bounded reservoirs and deadline outcomes;
//! * [`observer`] — the per-pipeline observer combining the above.
//!
//! See `docs/architecture/orchestration.md` and ADR-0030 to ADR-0032.

pub mod epoch;
pub mod graph;
pub mod latency;
pub mod observer;
pub mod reconcile;
pub mod timeline;

pub use epoch::{Admission, GenerationCounter, admit};
pub use graph::{ResolvedStage, ValidatedGraph, toposort, validate};
pub use latency::{DeadlineOutcome, Reservoir};
pub use observer::{Finding, Observer};
pub use reconcile::{RevisionClass, SourceClass, TranscriptTable};
pub use timeline::{NormalizedSpan, Timeline};
