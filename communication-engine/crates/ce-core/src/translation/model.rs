//! Translation requests, results, events and status (Phase 6).
//!
//! A translation result is a **derived** object: it links to the exact
//! source transcript revision it was produced from and never replaces or
//! edits that transcript, its language observation or any Phase 5
//! observation.

use serde::{Deserialize, Serialize};

use crate::error::ErrorReport;
use crate::ids::AudioStreamId;
use crate::intel::ScoreSemantics;
use crate::semantic::{Locality, ProfileRef};
use crate::speech::{LanguageSource, LanguageTag, ModelInfo, Percentiles, RevisionStatus};
use crate::time::MonotonicTime;

/// A directed language pair. Pairs match on the primary language subtag.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LanguagePair {
    /// Source.
    pub source: LanguageTag,
    /// Target.
    pub target: LanguageTag,
}

impl LanguagePair {
    /// Whether this pair covers `source → target` (primary subtags).
    pub fn covers(&self, source: &LanguageTag, target: &LanguageTag) -> bool {
        self.source.primary() == source.primary() && self.target.primary() == target.primary()
    }
}

/// What a translation adapter can and cannot do. Building this must not
/// load a model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranslationCapabilities {
    /// Stable adapter id.
    pub adapter: String,
    /// Version.
    pub version: String,
    /// Where text is processed. `remote` means text leaves the device.
    pub locality: Locality,
    /// Supported directed pairs (exhaustive).
    pub pairs: Vec<LanguagePair>,
    /// Can detect the source language itself.
    pub detects_source: bool,
    /// Produces incremental output for one request (token streaming).
    pub incremental_output: bool,
    /// Accepts partial (unfinished) source text meaningfully.
    pub partial_input: bool,
    /// Uses context hints.
    pub context_hints: bool,
    /// Applies terminology constraints itself.
    pub terminology: bool,
    /// Keeps `⟦n⟧` placeholders intact (needed for protected spans).
    pub placeholders: bool,
    /// Longest input accepted, characters.
    pub max_input_chars: u32,
    /// Same input and configuration always give the same output.
    pub deterministic: bool,
    /// Needs an (expensive) load before use.
    pub requires_load: bool,
    /// Semantics of its scores (`None`: no scores).
    pub score_semantics: Option<ScoreSemantics>,
    /// The model, if any.
    pub model: Option<ModelInfo>,
}

impl TranslationCapabilities {
    /// Whether the pair is supported.
    pub fn supports(&self, source: &LanguageTag, target: &LanguageTag) -> bool {
        self.pairs.iter().any(|p| p.covers(source, target))
    }
}

/// Where the source language came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "provenance")]
pub enum SourceLanguageProvenance {
    /// Declared by configuration.
    Explicit,
    /// Phase 4's utterance language decision.
    Recognition {
        /// Phase 4 source of that decision.
        source: LanguageSource,
    },
    /// Detected by the translation adapter.
    AdapterDetected,
}

/// The source language used for a result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLanguageRef {
    /// Language.
    pub language: LanguageTag,
    /// Provenance.
    pub provenance: SourceLanguageProvenance,
}

/// Exact source linkage (the source text itself is not repeated: it is the
/// transcript revision named here).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranslationSource {
    /// Stream.
    pub stream: AudioStreamId,
    /// Utterance.
    pub utterance: u64,
    /// Transcript revision.
    pub revision: u32,
    /// Status of that revision.
    pub revision_status: RevisionStatus,
    /// Characters of the source that were translated (committed text only
    /// for `stable` results).
    pub chars: u32,
    /// Source audio frames [start, end), from the transcript.
    pub frame_range: (u64, u64),
    /// Source time range, from the transcript.
    pub time_range: (MonotonicTime, MonotonicTime),
}

/// Lifecycle state of a translation result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationState {
    /// Translation of a whole partial snapshot: may change; never final.
    Provisional,
    /// Translation of a partial revision's committed text. Superseded by a
    /// later stable result or the final; the final re-translates the whole
    /// utterance and **may differ** from it (a translated prefix is not a
    /// prefix of the translation).
    Stable,
    /// Translation of the final source revision.
    Final,
}

/// Why a result is in doubt. Flags are evidence of uncertainty, never
/// silently corrected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UncertaintyFlag {
    /// Source is a partial revision.
    ProvisionalSource,
    /// The source language came from adapter detection, not Phase 4 or config.
    DetectedSourceLanguage,
    /// The explicit source language disagrees with Phase 4's decision.
    SourceLanguageDisagreement,
    /// A digit sequence of the source is missing from the output.
    NumberMismatch,
    /// A protected span (placeholder) was not restored.
    ProtectedSpanLost,
    /// A terminology term present in the source has no target term in the
    /// output.
    TermMissing,
    /// A context hint disagreed with the source text and was not used.
    ContextConflict,
    /// The adapter's score is below its own documented threshold.
    LowScore,
}

/// How a result was produced.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranslationProvenance {
    /// Adapter id.
    pub adapter: String,
    /// Adapter version.
    pub version: String,
    /// Model fingerprint, when known.
    pub model_fingerprint: Option<String>,
    /// `TranslationConfig::fingerprint`.
    pub config_fingerprint: String,
    /// Locality of the adapter.
    pub locality: Locality,
    /// Terminology profile, if one was in force.
    pub terminology: Option<ProfileRef>,
    /// Terms applied (constraints or protected spans).
    pub terms_applied: u32,
    /// Context hints passed to the adapter.
    pub context_hints: u32,
    /// Spans protected by placeholders.
    pub protected_spans: u32,
    /// Set when a fallback adapter produced the result: the adapter that
    /// could not.
    pub fallback_from: Option<String>,
}

/// A score with its meaning.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranslationScore {
    /// Value.
    pub value: f32,
    /// Semantics (never a probability without calibration).
    pub semantics: ScoreSemantics,
}

/// Wall-clock timing of a result, ms.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct TranslationTiming {
    /// Time in the queue.
    pub queued_ms: f32,
    /// Adapter time.
    pub adapter_ms: f32,
}

/// One translation result. Results of an utterance are numbered 0, 1, 2,
/// … (`sequence`, gapless); a consumer keeps the highest sequence it has
/// seen and ignores lower ones.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranslationResult {
    /// Sequence within the utterance.
    pub sequence: u32,
    /// State.
    pub state: TranslationState,
    /// Source linkage.
    pub source: TranslationSource,
    /// Source language used.
    pub source_language: SourceLanguageRef,
    /// Target language.
    pub target: LanguageTag,
    /// Translated text (derived; never the transcript).
    pub text: String,
    /// Uncertainty flags (sorted).
    pub flags: Vec<UncertaintyFlag>,
    /// Adapter score, if it reports one.
    pub score: Option<TranslationScore>,
    /// Provenance.
    pub provenance: TranslationProvenance,
    /// Timing.
    pub timing: TranslationTiming,
}

/// Why no translation was produced for a source revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    /// The pair is not supported by the selected (and fallback) adapters.
    UnsupportedPair,
    /// The source language is uncertain and cannot be decided safely.
    UncertainSource,
    /// The source and target languages are the same.
    SameLanguage,
    /// The adapter has no translation for this text (e.g. a translation
    /// memory without a match). Nothing is invented.
    NoCoverage,
    /// Source longer than `max_input_chars` (or the adapter's limit).
    InputTooLong,
    /// Output longer than `max_output_chars`.
    OutputTooLong,
    /// The adapter output violated the contract.
    MalformedOutput,
    /// The adapter returned an error.
    AdapterError,
    /// The result was later than `deadline_ms`.
    Timeout,
    /// The source text is empty.
    EmptySource,
}

/// Why results were withdrawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WithdrawReason {
    /// The source utterance was cancelled.
    SourceCancelled,
    /// Translation was reset.
    Reset,
    /// Translation stopped before the utterance's final was translated.
    Stopped,
    /// The final translation failed: earlier provisional/stable results are
    /// not valid for the final source.
    FinalUnavailable,
}

/// Why an input was rejected (not translated).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationRejectReason {
    /// Older revision than one already handled.
    Stale,
    /// Same revision again.
    Duplicate,
    /// After the utterance's final or cancellation.
    AfterFinal,
}

/// Why translation was degraded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationDegrade {
    /// Inputs were dropped (queue full).
    Overload,
    /// An adapter error occurred.
    AdapterError,
    /// A result was later than the deadline.
    DeadlineExceeded,
}

/// Why translation ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationEndReason {
    /// Stopped by request (queued inputs discarded).
    Stopped,
    /// The stream's recognition ended (queued inputs drained).
    SourceEnded,
    /// The worker failed.
    Failed,
    /// The runtime shut down (queued inputs drained).
    Shutdown,
}

/// Counters for one translation worker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct TranslationCounters {
    /// Inputs accepted for processing.
    pub inputs: u64,
    /// Results emitted (all states).
    pub results: u64,
    /// Final results.
    pub finals: u64,
    /// Stable results.
    pub stable: u64,
    /// Provisional results.
    pub provisional: u64,
    /// Withdrawal events.
    pub withdrawals: u64,
    /// Unavailable outcomes.
    pub unavailable: u64,
    /// Inputs rejected (stale/duplicate/after final).
    pub rejected: u64,
    /// Partials replaced in the queue before translation.
    pub coalesced: u64,
    /// Inputs dropped (overload).
    pub dropped: u64,
    /// Adapter errors.
    pub adapter_errors: u64,
    /// Timeouts.
    pub timeouts: u64,
    /// Malformed outputs.
    pub malformed: u64,
    /// Fallback adapter uses.
    pub fallbacks: u64,
    /// Results with at least one uncertainty flag.
    pub flagged: u64,
    /// Queue delay, ms.
    pub queue_ms: Percentiles,
    /// Adapter time, ms.
    pub adapter_ms: Percentiles,
    /// Time from an utterance's first input to its first result, ms.
    pub first_result_ms: Percentiles,
}

/// Status of one translation worker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranslationStatus {
    /// Stream.
    pub stream: AudioStreamId,
    /// Primary adapter capabilities.
    pub capabilities: TranslationCapabilities,
    /// Fallback adapters, in order.
    pub fallback: Vec<String>,
    /// Target.
    pub target: LanguageTag,
    /// Configuration fingerprint.
    pub config_fingerprint: String,
    /// Terminology profile, if any.
    pub terminology: Option<ProfileRef>,
    /// Running.
    pub running: bool,
    /// Queued inputs.
    pub backlog: u32,
    /// Counters.
    pub counters: TranslationCounters,
    /// Why it ended.
    pub ended: Option<TranslationEndReason>,
}

/// Translation events (`EventPayload::Translation`). Results are
/// `Content`; everything else is `Operational` (ids, sizes, reasons).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "translation_event")]
#[non_exhaustive]
pub enum TranslationEvent {
    /// Translation began.
    Started {
        /// Stream.
        stream: AudioStreamId,
        /// Status at start.
        status: Box<TranslationStatus>,
    },
    /// A result.
    Result(Box<TranslationResult>),
    /// Results of an utterance up to `through_sequence` are no longer valid.
    Withdrawn {
        /// Stream.
        stream: AudioStreamId,
        /// Utterance.
        utterance: u64,
        /// Highest withdrawn sequence (`None`: nothing had been emitted).
        through_sequence: Option<u32>,
        /// Why.
        reason: WithdrawReason,
    },
    /// No translation for a source revision.
    Unavailable {
        /// Stream.
        stream: AudioStreamId,
        /// Utterance.
        utterance: u64,
        /// Source revision.
        revision: u32,
        /// Status of that revision.
        revision_status: RevisionStatus,
        /// Why.
        reason: UnavailableReason,
        /// Adapters tried, in order.
        adapters: Vec<String>,
    },
    /// An input was rejected.
    InputRejected {
        /// Stream.
        stream: AudioStreamId,
        /// Utterance.
        utterance: u64,
        /// Revision.
        revision: u32,
        /// Why.
        reason: TranslationRejectReason,
    },
    /// Degraded operation.
    Degraded {
        /// Stream.
        stream: AudioStreamId,
        /// Why.
        reason: TranslationDegrade,
    },
    /// The worker failed.
    Failed {
        /// Stream.
        stream: AudioStreamId,
        /// Redacted error.
        error: ErrorReport,
    },
    /// Translation ended.
    Stopped {
        /// Stream.
        stream: AudioStreamId,
        /// Why.
        reason: TranslationEndReason,
        /// Final counters.
        counters: Box<TranslationCounters>,
    },
}

impl TranslationEvent {
    /// Stable kind name.
    pub fn kind(&self) -> &'static str {
        match self {
            TranslationEvent::Started { .. } => "started",
            TranslationEvent::Result(_) => "result",
            TranslationEvent::Withdrawn { .. } => "withdrawn",
            TranslationEvent::Unavailable { .. } => "unavailable",
            TranslationEvent::InputRejected { .. } => "input_rejected",
            TranslationEvent::Degraded { .. } => "degraded",
            TranslationEvent::Failed { .. } => "failed",
            TranslationEvent::Stopped { .. } => "stopped",
        }
    }

    /// The stream.
    pub fn stream(&self) -> AudioStreamId {
        match self {
            TranslationEvent::Result(r) => r.source.stream,
            TranslationEvent::Started { stream, .. }
            | TranslationEvent::Withdrawn { stream, .. }
            | TranslationEvent::Unavailable { stream, .. }
            | TranslationEvent::InputRejected { stream, .. }
            | TranslationEvent::Degraded { stream, .. }
            | TranslationEvent::Failed { stream, .. }
            | TranslationEvent::Stopped { stream, .. } => *stream,
        }
    }

    /// Privacy class: results are `Content`, the rest `Operational`.
    pub fn data_class(&self) -> crate::privacy::DataClass {
        match self {
            TranslationEvent::Result(_) => crate::privacy::DataClass::Content,
            _ => crate::privacy::DataClass::Operational,
        }
    }
}

/// What a term does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TermKind {
    /// Translate the source term as `target`.
    #[default]
    Translate,
    /// Keep the source term unchanged (names, codes, brand terms).
    DoNotTranslate,
}

/// One terminology entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Term {
    /// Source term (one or more words).
    pub source: String,
    /// Target term (ignored for `do_not_translate`).
    #[serde(default)]
    pub target: String,
    /// Kind.
    #[serde(default)]
    pub kind: TermKind,
    /// Match case exactly (default: case-insensitive).
    #[serde(default)]
    pub case_sensitive: bool,
}

/// A terminology profile: scoped to one directed pair and a domain,
/// versioned, with provenance. Matching is whole-word only; the longest
/// matching term wins; one source term has one target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyProfile {
    /// Id.
    pub id: String,
    /// Version.
    pub version: String,
    /// Pair.
    pub pair: LanguagePair,
    /// Domain (e.g. `hospitality`).
    pub domain: String,
    /// Who wrote it and from what.
    pub provenance: String,
    /// Terms.
    pub terms: Vec<Term>,
}

/// Most terms in one profile.
pub const MAX_TERMS: usize = 1_000;

impl TerminologyProfile {
    /// Validate: ids, sizes, non-empty terms, no conflicting duplicates.
    pub fn validate(&self) -> Result<(), crate::CeError> {
        let bad = |what: &str| {
            crate::CeError::invalid_config(
                "translation.bad_terminology",
                "terminology profile is invalid",
            )
            .with_context("problem", what.to_string())
            .with_context("profile", self.id.chars().take(64).collect::<String>())
        };
        if self.id.is_empty()
            || self.id.len() > 64
            || self.version.is_empty()
            || self.version.len() > 32
        {
            return Err(bad("id 1..=64 chars, version 1..=32 chars"));
        }
        if self.terms.is_empty() || self.terms.len() > MAX_TERMS {
            return Err(bad("1..=1000 terms"));
        }
        let mut seen = std::collections::BTreeMap::new();
        for t in &self.terms {
            let s = t.source.trim();
            if s.is_empty() || s.chars().count() > 128 || t.target.chars().count() > 128 {
                return Err(bad("term source 1..=128 chars, target <= 128 chars"));
            }
            if t.kind == TermKind::Translate && t.target.trim().is_empty() {
                return Err(bad("a translate term needs a target"));
            }
            let key = s.to_lowercase();
            let value = (t.kind, t.target.trim().to_string());
            if let Some(prev) = seen.insert(key, value.clone()) {
                if prev != value {
                    return Err(bad("conflicting entries for the same source term"));
                }
            }
        }
        Ok(())
    }

    /// Identity for provenance.
    pub fn reference(&self) -> ProfileRef {
        ProfileRef {
            id: self.id.clone(),
            version: self.version.clone(),
            fingerprint: crate::speech::config::fnv(&format!("terminology.v1|{self:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(s: &str) -> LanguageTag {
        LanguageTag::parse(s).unwrap()
    }

    fn profile(terms: Vec<Term>) -> TerminologyProfile {
        TerminologyProfile {
            id: "hotel".into(),
            version: "1".into(),
            pair: LanguagePair {
                source: tag("en"),
                target: tag("es"),
            },
            domain: "hospitality".into(),
            provenance: "test".into(),
            terms,
        }
    }

    fn term(s: &str, t: &str) -> Term {
        Term {
            source: s.into(),
            target: t.into(),
            kind: TermKind::Translate,
            case_sensitive: false,
        }
    }

    #[test]
    fn pairs_capabilities_and_terminology_validation() {
        let p = LanguagePair {
            source: tag("en"),
            target: tag("es"),
        };
        assert!(p.covers(&tag("en-GB"), &tag("es-419")));
        assert!(!p.covers(&tag("es"), &tag("en")), "pairs are directed");
        profile(vec![
            term("check-in", "registro"),
            term("Check-in", "registro"),
        ])
        .validate()
        .unwrap();
        assert!(
            profile(vec![
                term("check-in", "registro"),
                term("check-in", "entrada")
            ])
            .validate()
            .is_err()
        );
        assert!(profile(vec![]).validate().is_err());
        assert!(profile(vec![term("room", "")]).validate().is_err());
        let dnt = Term {
            source: "Talktual".into(),
            target: String::new(),
            kind: TermKind::DoNotTranslate,
            case_sensitive: true,
        };
        profile(vec![dnt]).validate().unwrap();
        let r = profile(vec![term("a", "b")]).reference();
        assert_eq!((r.id.as_str(), r.fingerprint.len()), ("hotel", 16));
    }

    #[test]
    fn events_serialize_with_tags_and_privacy_classes() {
        let stream = AudioStreamId::from_uuid(uuid::Uuid::from_u128(3));
        let e = TranslationEvent::Unavailable {
            stream,
            utterance: 1,
            revision: 4,
            revision_status: RevisionStatus::Final,
            reason: UnavailableReason::UnsupportedPair,
            adapters: vec!["ce.translate.memory".into()],
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["translation_event"], "unavailable");
        assert_eq!(v["reason"], "unsupported_pair");
        assert_eq!(serde_json::from_value::<TranslationEvent>(v).unwrap(), e);
        assert_eq!(e.data_class(), crate::privacy::DataClass::Operational);
        assert_eq!(e.stream(), stream);
    }
}
