//! Communication Engine semantic and contextual intelligence (Phase 5).
//!
//! Consumes Phase 4 transcript revisions and produces typed, versioned,
//! evidence-linked semantic observations (contracts in
//! [`ce_core::semantic`]).
//!
//! * [`adapter`] — the provider-agnostic adapter boundary;
//! * [`rules`] — `ce.semantic.rules`, the deterministic English baseline;
//! * [`scripted`] — a scripted adapter for lifecycle and failure tests;
//! * [`context`] — scoped, bounded, expiring, ephemeral context;
//! * [`processor`] — revision handling, contract enforcement, identity,
//!   lifecycle;
//! * [`worker`] — the supervised, bounded, coalescing worker thread;
//! * [`eval`] — replay evaluation (entities, references, acts, streaming).
//!
//! No model, network or credential is needed by anything in this crate.

pub mod adapter;
pub mod context;
pub mod eval;
pub mod metrics;
pub mod processor;
pub mod rules;
pub mod scripted;
pub mod text;
pub mod worker;

pub use adapter::{
    ContextView, Draft, DraftPayload, RelationTarget, SemanticAdapter, SemanticFactory,
};
pub use context::ContextManager;
pub use processor::{ProcessorSetup, SemanticProcessor};
pub use rules::{RulesAdapter, RulesFactory};
pub use worker::{PushOutcome, SemanticHandle, SemanticSignal, spawn_semantic};

use ce_core::semantic::SemanticInput;
use ce_core::speech::TranscriptRevision;

/// Build the semantic input for a Phase 4 transcript revision. The text is
/// `committed` + (a space +) `provisional`; nothing is changed.
pub fn input_from_revision(r: &TranscriptRevision) -> SemanticInput {
    let committed = r.text.committed.as_str();
    let provisional = r.text.provisional.as_str();
    let text = match (committed.is_empty(), provisional.is_empty()) {
        (_, true) => committed.to_string(),
        (true, false) => provisional.to_string(),
        (false, false) => format!("{committed} {provisional}"),
    };
    let audio_ms = r
        .meta
        .time_range
        .1
        .as_nanos()
        .saturating_sub(r.meta.time_range.0.as_nanos())
        / 1_000_000;
    SemanticInput {
        stream: r.meta.stream,
        utterance: r.meta.utterance,
        revision: r.revision,
        status: r.status,
        committed_chars: committed.chars().count() as u32,
        text,
        language: r.language.as_ref().and_then(|l| match &l.decision {
            ce_core::speech::LanguageDecision::Identified { language } => Some(language.clone()),
            ce_core::speech::LanguageDecision::Abstained { .. } => None,
        }),
        recognition_score: r.score.map(|s| ce_core::semantic::SemanticScore {
            value: s.value,
            semantics: s.semantics,
        }),
        time_range: r.meta.time_range,
        audio_ms,
    }
}
