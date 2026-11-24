//! Translation contracts (Phase 6).
//!
//! Translation turns final (and optionally partial) Phase 4 transcript
//! revisions into target-language text. Results are **derived** objects
//! linked to the exact source revision; the transcript, its language
//! observation and Phase 5 observations are never modified. Translation does
//! not imply understanding: a translated request is not safe to execute.
//!
//! * [`config`] — validated, bounded, fingerprinted configuration;
//! * [`model`] — capabilities, results, provenance, uncertainty, events,
//!   status and terminology profiles.
//!
//! Adapters, the processor and the worker live in the `ce-translate` crate.

pub mod config;
pub mod model;

pub use config::{
    ContextPolicy, PartialPolicy, SourceLanguagePolicy, TranslationBudget, TranslationConfig,
    TranslationPrivacy, TranslationSettings,
};
pub use model::{
    LanguagePair, MAX_TERMS, SourceLanguageProvenance, SourceLanguageRef, Term, TermKind,
    TerminologyProfile, TranslationCapabilities, TranslationCounters, TranslationDegrade,
    TranslationEndReason, TranslationEvent, TranslationProvenance, TranslationRejectReason,
    TranslationResult, TranslationScore, TranslationSource, TranslationState, TranslationStatus,
    TranslationTiming, UnavailableReason, UncertaintyFlag, WithdrawReason,
};
