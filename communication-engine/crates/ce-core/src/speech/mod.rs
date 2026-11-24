//! Speech recognition and language contracts (Phase 4).
//!
//! Streaming transcripts — provisional hypotheses, revisions, committed
//! (stable) text and finals — correlated with the capture stream's frames
//! and with Phase 3 analysis, plus text language observations with explicit
//! scope, uncertainty and provenance.
//!
//! **Content, not the person.** Transcripts are `Content`; language
//! observations describe the language of text, never nationality,
//! ethnicity, identity, location or any other trait.
//!
//! * [`config`] — validated, bounded, fingerprinted configuration;
//! * [`model`] — capabilities, transcript revisions, language observations,
//!   events and status.
//!
//! Adapters and the orchestration live in the `ce-speech` crate.

pub mod config;
pub mod model;

pub use config::{
    FinalizationPolicy, LanguageHint, LanguageIdConfig, RecognitionBudget, RecognitionConfig,
    SegmentationMode, SpeechConfig, StabilityPolicy,
};
pub use model::{
    AbstainReason, AsrCapabilities, FinalReason, HintSource, LanguageCandidate, LanguageDecision,
    LanguageObservation, LanguageScope, LanguageSource, LanguageSupport, LanguageTag, ModelInfo,
    Percentiles, RecognitionCounters, RecognitionEndReason, RecognitionMode, RecognitionStatus,
    RecognizerRef, ResetReason, RevisionStatus, Score, SpeechCapabilities, SpeechEvent, SpeechMeta,
    TranscriptDelta, TranscriptRevision, TranscriptText, WordTiming,
};
