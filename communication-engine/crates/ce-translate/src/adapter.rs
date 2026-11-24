//! The translation adapter boundary.
//!
//! Model, library and provider APIs stay behind [`TranslationAdapter`]; CE
//! code depends only on this trait and CE-owned types. An adapter sees one
//! request at a time: text (possibly with `⟦n⟧` placeholders, if it declares
//! `placeholders`), a source language (or `None` if it declares
//! `detects_source` and the source is unknown), the target, bounded context
//! hints (if it declares `context_hints`) and terminology constraints (if it
//! declares `terminology`). It cannot see or edit the transcript, other
//! utterances' results or other streams.
//!
//! **Lifecycle** (on the translation worker thread, never in an audio
//! callback or on the control plane): [`TranslationFactory::capabilities`]
//! (cheap, never loads a model) → [`TranslationFactory::create`] →
//! [`TranslationAdapter::load`] (once) → [`TranslationAdapter::translate`]*
//! → [`TranslationAdapter::reset`] / drop.

use ce_core::CeError;
use ce_core::speech::LanguageTag;
use ce_core::translation::{Term, TranslationCapabilities, TranslationConfig};

/// A bounded, typed context hint derived from a final Phase 5 entity
/// observation. Hints are evidence about what was said; they never replace
/// source text.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextHint {
    /// Entity category (e.g. `time_of_day`, `profile:room_type`).
    pub category: String,
    /// The mention, exactly as transcribed.
    pub mention: String,
    /// The normalized value as text, when Phase 5 normalized it.
    pub normalized: Option<String>,
    /// Utterance it came from.
    pub utterance: u64,
    /// It came from the utterance being translated.
    pub same_utterance: bool,
}

/// One request.
#[derive(Clone, Debug)]
pub struct TranslationRequest<'a> {
    /// Text to translate (with `⟦n⟧` placeholders for protected spans when
    /// the adapter declares `placeholders`).
    pub text: &'a str,
    /// Source language (`None`: unknown; only for adapters that detect).
    pub source: Option<&'a LanguageTag>,
    /// Target language.
    pub target: &'a LanguageTag,
    /// The source is a partial revision.
    pub partial: bool,
    /// Context hints (empty unless the adapter declares `context_hints`).
    pub hints: &'a [ContextHint],
    /// Terminology constraints (empty unless the adapter declares
    /// `terminology`).
    pub terms: &'a [Term],
    /// Longest acceptable output, characters.
    pub max_output_chars: u32,
}

/// An adapter's answer.
#[derive(Clone, Debug, PartialEq)]
pub enum AdapterOutcome {
    /// A translation.
    Translated {
        /// Text.
        text: String,
        /// Score (adapter semantics), if any.
        score: Option<f32>,
        /// Source language the adapter detected (when the request had none).
        detected_source: Option<LanguageTag>,
    },
    /// The adapter has no translation for this text (nothing is invented).
    NoCoverage,
    /// The adapter does not support this pair.
    UnsupportedPair,
}

/// A translation engine instance for one stream.
pub trait TranslationAdapter: Send {
    /// Load and warm up (expensive; called once on the worker thread).
    fn load(&mut self) -> Result<(), CeError> {
        Ok(())
    }
    /// Translate one request.
    fn translate(&mut self, request: &TranslationRequest<'_>) -> Result<AdapterOutcome, CeError>;
    /// Forget any internal state.
    fn reset(&mut self) {}
}

/// Creates adapters and describes them.
pub trait TranslationFactory: Send + Sync {
    /// Static capabilities (cheap; must not load a model).
    fn capabilities(&self) -> TranslationCapabilities;
    /// A new, unloaded adapter for one stream.
    fn create(&self, config: &TranslationConfig) -> Result<Box<dyn TranslationAdapter>, CeError>;
}
