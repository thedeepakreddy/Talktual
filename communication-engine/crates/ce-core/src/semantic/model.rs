//! Semantic observations, evidence, provenance, context items, events and
//! status (Phase 5).
//!
//! **The transcript is immutable.** Every observation points at a specific
//! transcript revision and a character span of that revision's text; nothing
//! here edits or replaces transcript text.
//!
//! **Uncertainty is explicit.** Each observation states what kind of claim it
//! is ([`Epistemic`]), how certain the producer is ([`Uncertainty`]) and why
//! ([`Evidence`]). Rule tiers are not probabilities; scores carry their
//! semantics and are never calibrated unless calibration evidence exists
//! (none does in Phase 5).

use serde::{Deserialize, Serialize};

use crate::error::{CeError, ErrorReport};
use crate::ids::AudioStreamId;
use crate::intel::ScoreSemantics;
use crate::speech::{LanguageTag, Percentiles, RevisionStatus};
use crate::time::MonotonicTime;

/// Version of the semantic observation schema.
pub const SEMANTIC_SCHEMA_VERSION: u32 = 1;

/// A stable observation id: the same entity/reference/act in later
/// revisions of the same utterance keeps its id.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ObservationId(pub String);

/// A stable context item id.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContextItemId(pub String);

/// A `[start, end)` range of Unicode scalar values in the source text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TextSpan {
    /// Start (inclusive).
    pub start: u32,
    /// End (exclusive).
    pub end: u32,
}

impl TextSpan {
    /// Whether the span is non-empty and inside a text of `len` characters.
    pub fn valid_for(&self, len: u32) -> bool {
        self.start < self.end && self.end <= len
    }
}

/// Where an observation comes from: one revision of one utterance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    /// Stream.
    pub stream: AudioStreamId,
    /// Utterance.
    pub utterance: u64,
    /// Transcript revision analysed.
    pub revision: u32,
    /// Status of that revision.
    pub revision_status: RevisionStatus,
    /// Span in the revision's text (`committed` + " " + `provisional`).
    pub span: TextSpan,
    /// The span lies entirely in committed text.
    pub committed: bool,
}

/// What kind of claim an observation is. Never upgraded implicitly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Epistemic {
    /// Explicitly present in the text (a mention, a number).
    Observation,
    /// Inferred and uncertain (an interpretation not directly stated).
    Hypothesis,
    /// Resolved using scoped context (e.g. a pronoun linked to an entity).
    ResolvedInterpretation,
    /// Confirmed by the user through the host (never set by an adapter).
    UserConfirmedFact,
}

/// Rule tier. **Not a probability.**
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Certainty {
    /// Weak cue or many alternatives.
    Low,
    /// Clear cue with some room for error.
    Medium,
    /// Unambiguous surface form (e.g. digits, an exact profile term).
    High,
}

/// A score with its meaning.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticScore {
    /// Value.
    pub value: f32,
    /// Semantics (never a calibrated probability without calibration tests).
    pub semantics: ScoreSemantics,
}

/// Uncertainty of an observation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Uncertainty {
    /// Tier.
    pub level: Certainty,
    /// Adapter score, if the adapter declares score semantics.
    pub score: Option<SemanticScore>,
    /// Calibration status: always false unless calibration tests exist.
    pub calibrated: bool,
}

impl Uncertainty {
    /// A rule tier without a score.
    pub const fn tier(level: Certainty) -> Self {
        Self {
            level,
            score: None,
            calibrated: false,
        }
    }
}

/// How evidence supports an observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// The span states it.
    Explicit,
    /// Derived from the span by a rule (e.g. a cue word for an act).
    Inferred,
    /// Supported by a scoped context item.
    Context,
    /// Matched a domain-profile term (terminology, not certainty).
    Profile,
}

/// One piece of evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// Kind.
    pub kind: EvidenceKind,
    /// Transcript span, when the evidence is in the transcript.
    pub span: Option<TextSpan>,
    /// Context item, when the evidence is context.
    pub context_item: Option<ContextItemId>,
    /// Rule or cue id (operational text only, never user content).
    pub rule: String,
}

/// Where processing happened (drives policy and disclosure).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Locality {
    /// Inside the CE process.
    InProcess,
    /// Another process on this device.
    LocalService,
    /// A remote service (content leaves the device).
    Remote,
}

/// Domain profile identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileRef {
    /// Id.
    pub id: String,
    /// Version.
    pub version: String,
    /// Fingerprint.
    pub fingerprint: String,
}

/// Who produced an observation, with what configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// Adapter id.
    pub adapter: String,
    /// Adapter version.
    pub adapter_version: String,
    /// `SemanticConfig::fingerprint`.
    pub config_fingerprint: String,
    /// Domain profile, if one was used.
    pub profile: Option<ProfileRef>,
    /// Where it ran.
    pub locality: Locality,
}

/// A normalized value. Present only when the adapter supports the
/// normalization for this category; never filled in by guessing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum NormalizedValue {
    /// An integer ("twenty five" → 25).
    Integer(i64),
    /// A decimal ("two point five" → 2.5).
    Decimal(f64),
    /// An ordinal ("second" → 2).
    Ordinal(u32),
    /// A count of something ("two blocks").
    Quantity {
        /// Amount.
        amount: f64,
        /// Unit or counted noun, as spoken (lower case).
        unit: String,
    },
    /// An amount of money.
    Money {
        /// Amount.
        amount: f64,
        /// ISO 4217 code when the currency word maps unambiguously.
        currency: String,
    },
    /// A percentage.
    Percentage(f64),
    /// A clock time; `pm` is `None` when the speaker did not say.
    TimeOfDay {
        /// Hour as spoken (1–12 or 0–23).
        hour: u8,
        /// Minute.
        minute: u8,
        /// Some(true) pm, Some(false) am, None unknown.
        pm: Option<bool>,
    },
    /// A day relative to the utterance ("tomorrow" → +1).
    RelativeDay(i32),
    /// A weekday (0 = Monday).
    Weekday(u8),
    /// A calendar date; missing parts stay missing.
    Date {
        /// Year, if stated.
        year: Option<i32>,
        /// Month 1–12, if stated.
        month: Option<u8>,
        /// Day 1–31, if stated.
        day: Option<u8>,
    },
    /// A duration in seconds.
    Duration(u64),
    /// The canonical form of a domain-profile term.
    Canonical(String),
}

/// A named attribute of an entity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribute {
    /// Name.
    pub name: String,
    /// Value.
    pub value: String,
}

/// An entity mention.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    /// Category: a domain-neutral built-in (`number`, `time`, …) or a
    /// profile category (`profile:<name>`).
    pub category: String,
    /// The mention exactly as in the transcript span.
    pub mention: String,
    /// Normalized value, only when supported.
    pub normalized: Option<NormalizedValue>,
    /// Attributes.
    pub attributes: Vec<Attribute>,
}

/// A typed link between two observations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relation {
    /// Relation type (e.g. `quantity_of`, `refers_to`).
    pub relation: String,
    /// Source observation.
    pub from: ObservationId,
    /// Target observation or context item.
    pub to: String,
}

/// What sort of referring expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    /// it / they / them.
    Pronoun,
    /// this / that / these / those.
    Demonstrative,
    /// there / here.
    Locative,
    /// then.
    Temporal,
    /// he / she / him / her.
    Personal,
}

/// A candidate antecedent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReferenceCandidate {
    /// Context item.
    pub item: ContextItemId,
    /// Its category.
    pub category: String,
    /// Its mention (content).
    pub mention: String,
    /// Why it is a candidate (rule id).
    pub rule: String,
}

/// The outcome of resolution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum Resolution {
    /// Exactly one permitted, compatible candidate.
    Resolved {
        /// Target.
        target: ContextItemId,
    },
    /// Several compatible candidates: no forced choice.
    Ambiguous,
    /// No permitted compatible candidate.
    Unresolved {
        /// Why (operational).
        reason: String,
    },
}

/// A reference resolution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReferenceResolution {
    /// The referring mention.
    pub mention: String,
    /// Kind.
    pub kind: ReferenceKind,
    /// Candidates, most recent first.
    pub candidates: Vec<ReferenceCandidate>,
    /// Outcome.
    pub resolution: Resolution,
}

/// Why no communicative act was labelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActAbstention {
    /// Cues for several labels.
    Ambiguous,
    /// No cue at all.
    NoCue,
    /// The utterance is classifiable but outside the taxonomy.
    OutOfTaxonomy,
}

/// The act decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "decision")]
pub enum ActDecision {
    /// A label from the taxonomy.
    Labeled {
        /// Label.
        label: String,
    },
    /// No label.
    Abstained {
        /// Why.
        reason: ActAbstention,
    },
}

/// A communicative act / intent observation (optional capability).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunicativeAct {
    /// Taxonomy id.
    pub taxonomy: String,
    /// Taxonomy version.
    pub taxonomy_version: String,
    /// Decision.
    pub decision: ActDecision,
    /// Other labels whose cues were present (ambiguity).
    pub alternatives: Vec<String>,
}

/// The payload of an observation (tagged `type`: payloads have their own
/// `kind` fields).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ObservationPayload {
    /// Entity.
    Entity(Entity),
    /// Relation.
    Relation(Relation),
    /// Reference resolution.
    Reference(ReferenceResolution),
    /// Communicative act.
    Act(CommunicativeAct),
}

impl ObservationPayload {
    /// Stable kind name.
    pub fn kind(&self) -> &'static str {
        match self {
            ObservationPayload::Entity(_) => "entity",
            ObservationPayload::Relation(_) => "relation",
            ObservationPayload::Reference(_) => "reference",
            ObservationPayload::Act(_) => "act",
        }
    }
}

/// Lifecycle status of an observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationStatus {
    /// From provisional text: may change or disappear.
    Provisional,
    /// Its span is in committed text: changes only if a final amends it.
    Stable,
    /// From the utterance's final text.
    Final,
    /// No longer supported by the latest revision (or the utterance was
    /// cancelled). Kept for traceability; not current.
    Retracted,
}

/// A semantic observation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticObservation {
    /// Stable id.
    pub id: ObservationId,
    /// Schema version.
    pub schema_version: u32,
    /// Version of this observation (increments on every change).
    pub version: u32,
    /// Payload.
    pub payload: ObservationPayload,
    /// Status.
    pub status: ObservationStatus,
    /// Kind of claim.
    pub epistemic: Epistemic,
    /// Uncertainty.
    pub uncertainty: Uncertainty,
    /// Evidence (at least one item).
    pub evidence: Vec<Evidence>,
    /// Source revision and span.
    pub source: SourceRef,
    /// Provenance.
    pub provenance: Provenance,
    /// When it was produced (monotonic).
    pub produced_at: MonotonicTime,
}

/// Scope of a context item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "scope")]
pub enum ContextScope {
    /// One utterance only.
    Utterance {
        /// Utterance.
        utterance: u64,
    },
    /// The semantic session of one stream (cleared at stop/reset).
    Session,
}

/// Status of a context item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextStatus {
    /// Usable for resolution.
    Active,
    /// Its source was corrected (e.g. the final amended it); not usable.
    Invalidated,
}

/// A bounded, scoped, expiring context item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextItem {
    /// Id.
    pub id: ContextItemId,
    /// Scope.
    pub scope: ContextScope,
    /// Category of the entity it records.
    pub category: String,
    /// The mention (content).
    pub mention: String,
    /// Normalized value, if any.
    pub normalized: Option<NormalizedValue>,
    /// Source observation.
    pub observation: ObservationId,
    /// Source revision and span.
    pub source: SourceRef,
    /// Certainty of the source observation.
    pub certainty: Certainty,
    /// Status.
    pub status: ContextStatus,
    /// Privacy class of the item (always `Content` for mentions).
    pub data_class: crate::privacy::DataClass,
    /// Created at.
    pub created_at: MonotonicTime,
    /// Expires at.
    pub expires_at: MonotonicTime,
    /// Approximate size in bytes (for limits).
    pub bytes: u32,
}

/// Current context size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContextSize {
    /// Items.
    pub items: u32,
    /// Bytes.
    pub bytes: u32,
}

/// What changed in the context.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextChange {
    /// Added.
    Added,
    /// Evicted (limits).
    Evicted,
    /// Expired (TTL).
    Expired,
    /// Invalidated (its source was corrected or retracted).
    Invalidated,
    /// Everything cleared (reset, stop, cancellation).
    Cleared,
}

/// How an observation changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationChange {
    /// First appearance.
    New,
    /// Payload, span or uncertainty changed.
    Updated,
    /// Moved into committed text.
    Stabilized,
    /// Final.
    Finalized,
    /// Retracted.
    Retracted,
    /// Confirmed by the user.
    Confirmed,
}

/// Why an input was not processed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    /// Its revision is older than one already processed.
    Stale,
    /// Same revision again.
    Duplicate,
    /// The utterance already finalized or was cancelled.
    AfterFinal,
    /// Text longer than the configured limit.
    Oversized,
    /// Not well formed (e.g. invalid span, control characters only).
    Malformed,
    /// A newer revision of the same utterance replaced it in the queue.
    Coalesced,
}

/// Why processing degraded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DegradeReason {
    /// The input queue is full; provisional inputs are being coalesced or dropped.
    Overload,
    /// The adapter exceeded the per-input deadline.
    DeadlineExceeded,
    /// Output exceeded the configured limits and was truncated.
    OutputLimited,
    /// A remote/local-service adapter is not permitted by policy.
    PolicyRestricted,
    /// The adapter returned an error for one input (non-fatal; repeated
    /// errors fail the worker).
    AdapterError,
    /// The adapter returned drafts that violate the contract (invalid span,
    /// mention not matching the transcript, missing evidence, claimed
    /// user confirmation); they were dropped.
    InvalidOutput,
}

/// Why semantic processing ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticEndReason {
    /// Stopped by request.
    Stopped,
    /// The recognition (or its stream) ended.
    SourceEnded,
    /// The worker failed (panic, stall, repeated adapter errors).
    Failed,
    /// Runtime shutdown.
    Shutdown,
}

/// Counters and latency summaries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct SemanticCounters {
    /// Inputs received.
    pub inputs: u64,
    /// Inputs processed by the adapter.
    pub processed: u64,
    /// Inputs replaced by a newer revision before processing.
    pub coalesced: u64,
    /// Stale or duplicate inputs rejected.
    pub stale_rejected: u64,
    /// Oversized or malformed inputs rejected.
    pub invalid_rejected: u64,
    /// Observations first emitted.
    pub observations_new: u64,
    /// Updates.
    pub updated: u64,
    /// Stabilizations.
    pub stabilized: u64,
    /// Finalizations.
    pub finalized: u64,
    /// Retractions.
    pub retracted: u64,
    /// Context items evicted by limits.
    pub context_evicted: u64,
    /// Context items expired.
    pub context_expired: u64,
    /// Adapter errors.
    pub adapter_errors: u64,
    /// Adapter drafts dropped for violating the contract.
    pub invalid_drafts: u64,
    /// Drafts below `min_certainty` (not emitted).
    pub below_min_certainty: u64,
    /// Observations confirmed by the user.
    pub confirmed: u64,
    /// Inputs over the deadline.
    pub deadline_exceeded: u64,
    /// Utterances cancelled.
    pub cancelled: u64,
    /// Outputs truncated by limits.
    pub output_truncated: u64,
    /// Adapter time per processed input, ms.
    pub processing_ms: Percentiles,
    /// Time an input waited in the queue, ms.
    pub queue_delay_ms: Percentiles,
    /// Audio time from utterance start to the input that produced the
    /// utterance's first observation, ms.
    pub first_observation_ms: Percentiles,
}

/// What a semantic adapter can do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticCapabilities {
    /// Adapter id.
    pub adapter: String,
    /// Version.
    pub version: String,
    /// Where it runs.
    pub locality: Locality,
    /// Same input, configuration and context → same output.
    pub deterministic: bool,
    /// Entity extraction.
    pub entities: bool,
    /// Relations.
    pub relations: bool,
    /// Reference resolution.
    pub references: bool,
    /// Communicative acts.
    pub acts: bool,
    /// Built-in entity categories.
    pub categories: Vec<String>,
    /// Categories with normalization.
    pub normalizes: Vec<String>,
    /// Act taxonomies supported (`id@version`).
    pub act_taxonomies: Vec<String>,
    /// Languages (primary subtags); empty = unknown.
    pub languages: Vec<String>,
    /// Score semantics (`None`: rule tiers only).
    pub score_semantics: Option<ScoreSemantics>,
    /// Needs an expensive load.
    pub requires_load: bool,
}

/// Snapshot of one stream's semantic processing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticStatus {
    /// Stream.
    pub stream: AudioStreamId,
    /// Adapter capabilities.
    pub capabilities: SemanticCapabilities,
    /// `SemanticConfig::fingerprint`.
    pub config_fingerprint: String,
    /// Profile in use.
    pub profile: Option<ProfileRef>,
    /// Running.
    pub running: bool,
    /// Context size.
    pub context: ContextSize,
    /// Current (non-retracted) observations.
    pub current_observations: u32,
    /// Counters.
    pub counters: SemanticCounters,
    /// Why it ended.
    pub ended: Option<SemanticEndReason>,
}

/// Semantic events (`EventPayload::Semantic`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "semantic_event")]
#[non_exhaustive]
pub enum SemanticEvent {
    /// Processing began.
    Started {
        /// Stream.
        stream: AudioStreamId,
        /// Capabilities.
        capabilities: Box<SemanticCapabilities>,
        /// Config fingerprint.
        config_fingerprint: String,
        /// Profile.
        profile: Option<ProfileRef>,
    },
    /// An observation appeared, changed, stabilized, finalized, was
    /// retracted or confirmed.
    Observation {
        /// Change.
        change: ObservationChange,
        /// The observation as it is now.
        observation: Box<SemanticObservation>,
    },
    /// An utterance's interpretation is final.
    UtteranceFinalized {
        /// Stream.
        stream: AudioStreamId,
        /// Utterance.
        utterance: u64,
        /// Final revision.
        revision: u32,
        /// Current observations of the utterance.
        observations: u32,
    },
    /// An utterance was cancelled: its observations were retracted.
    UtteranceCancelled {
        /// Stream.
        stream: AudioStreamId,
        /// Utterance.
        utterance: u64,
    },
    /// The context changed (ids and sizes only, no content).
    Context {
        /// Stream.
        stream: AudioStreamId,
        /// Change.
        change: ContextChange,
        /// Item, if one.
        item: Option<ContextItemId>,
        /// Size after the change.
        size: ContextSize,
    },
    /// An input was not processed.
    InputRejected {
        /// Stream.
        stream: AudioStreamId,
        /// Utterance.
        utterance: u64,
        /// Revision.
        revision: u32,
        /// Why.
        reason: RejectReason,
    },
    /// Processing degraded.
    Degraded {
        /// Stream.
        stream: AudioStreamId,
        /// Why.
        reason: DegradeReason,
    },
    /// The worker failed.
    Failed {
        /// Stream.
        stream: AudioStreamId,
        /// Redacted error.
        error: ErrorReport,
    },
    /// Processing ended.
    Stopped {
        /// Stream.
        stream: AudioStreamId,
        /// Why.
        reason: SemanticEndReason,
        /// Final counters.
        counters: Box<SemanticCounters>,
    },
}

impl SemanticEvent {
    /// Stable kind name.
    pub fn kind(&self) -> &'static str {
        match self {
            SemanticEvent::Started { .. } => "started",
            SemanticEvent::Observation { .. } => "observation",
            SemanticEvent::UtteranceFinalized { .. } => "utterance_finalized",
            SemanticEvent::UtteranceCancelled { .. } => "utterance_cancelled",
            SemanticEvent::Context { .. } => "context",
            SemanticEvent::InputRejected { .. } => "input_rejected",
            SemanticEvent::Degraded { .. } => "degraded",
            SemanticEvent::Failed { .. } => "failed",
            SemanticEvent::Stopped { .. } => "stopped",
        }
    }

    /// The stream.
    pub fn stream(&self) -> AudioStreamId {
        match self {
            SemanticEvent::Observation { observation, .. } => observation.source.stream,
            SemanticEvent::Started { stream, .. }
            | SemanticEvent::UtteranceFinalized { stream, .. }
            | SemanticEvent::UtteranceCancelled { stream, .. }
            | SemanticEvent::Context { stream, .. }
            | SemanticEvent::InputRejected { stream, .. }
            | SemanticEvent::Degraded { stream, .. }
            | SemanticEvent::Failed { stream, .. }
            | SemanticEvent::Stopped { stream, .. } => *stream,
        }
    }

    /// Privacy class: observations carry mentions (`Content`); everything
    /// else is ids, sizes and reasons (`Operational`).
    pub fn data_class(&self) -> crate::privacy::DataClass {
        match self {
            SemanticEvent::Observation { .. } => crate::privacy::DataClass::Content,
            _ => crate::privacy::DataClass::Operational,
        }
    }
}

/// Input to an adapter: one transcript revision of one utterance, plus
/// metadata. The text is the revision's snapshot; adapters never modify it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticInput {
    /// Stream.
    pub stream: AudioStreamId,
    /// Utterance.
    pub utterance: u64,
    /// Revision.
    pub revision: u32,
    /// Revision status.
    pub status: RevisionStatus,
    /// `committed` + " " + `provisional` (or just `committed`).
    pub text: String,
    /// Characters of `text` that are committed.
    pub committed_chars: u32,
    /// Utterance language, when identified.
    pub language: Option<LanguageTag>,
    /// Recognition confidence, when the recognizer reports one.
    pub recognition_score: Option<SemanticScore>,
    /// Audio of the utterance on the stream timeline.
    pub time_range: (MonotonicTime, MonotonicTime),
    /// Audio time from utterance start to this revision, ms.
    pub audio_ms: u64,
}

impl SemanticInput {
    /// Validate shape: committed prefix inside the text, no control
    /// characters other than spaces.
    pub fn validate(&self, max_chars: u32) -> Result<(), CeError> {
        let len = self.text.chars().count();
        if len > max_chars as usize {
            return Err(CeError::invalid_input(
                "semantic.input_oversized",
                "transcript text exceeds the configured limit",
            )
            .with_context("chars", len.to_string()));
        }
        if self.committed_chars as usize > len
            || self.text.chars().any(|c| c.is_control() && c != ' ')
        {
            return Err(CeError::invalid_input(
                "semantic.input_malformed",
                "transcript input is malformed",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::DataClass;

    fn stream() -> AudioStreamId {
        AudioStreamId::from_uuid(uuid::Uuid::from_u128(7))
    }

    fn observation() -> SemanticObservation {
        SemanticObservation {
            id: ObservationId("u1.entity.0-3".into()),
            schema_version: SEMANTIC_SCHEMA_VERSION,
            version: 1,
            payload: ObservationPayload::Entity(Entity {
                category: "number".into(),
                mention: "TWO".into(),
                normalized: Some(NormalizedValue::Integer(2)),
                attributes: vec![],
            }),
            status: ObservationStatus::Provisional,
            epistemic: Epistemic::Observation,
            uncertainty: Uncertainty::tier(Certainty::High),
            evidence: vec![Evidence {
                kind: EvidenceKind::Explicit,
                span: Some(TextSpan { start: 0, end: 3 }),
                context_item: None,
                rule: "number.word".into(),
            }],
            source: SourceRef {
                stream: stream(),
                utterance: 1,
                revision: 2,
                revision_status: RevisionStatus::Partial,
                span: TextSpan { start: 0, end: 3 },
                committed: false,
            },
            provenance: Provenance {
                adapter: "ce.semantic.rules".into(),
                adapter_version: "1.0.0".into(),
                config_fingerprint: "0123456789abcdef".into(),
                profile: None,
                locality: Locality::InProcess,
            },
            produced_at: MonotonicTime::from_nanos(5),
        }
    }

    #[test]
    fn observations_round_trip_with_explicit_tags() {
        let o = observation();
        let v = serde_json::to_value(&o).unwrap();
        assert_eq!(v["payload"]["type"], "entity");
        assert_eq!(v["payload"]["normalized"]["type"], "integer");
        assert_eq!(v["epistemic"], "observation");
        assert_eq!(v["uncertainty"]["level"], "high");
        assert_eq!(v["uncertainty"]["calibrated"], false);
        assert_eq!(v["source"]["revision_status"], "partial");
        let back: SemanticObservation = serde_json::from_value(v).unwrap();
        assert_eq!(back, o);

        let ev = SemanticEvent::Observation {
            change: ObservationChange::New,
            observation: Box::new(o),
        };
        assert_eq!(ev.kind(), "observation");
        assert_eq!(ev.stream(), stream());
        assert_eq!(ev.data_class(), DataClass::Content);
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["semantic_event"], "observation");
        assert_eq!(serde_json::from_value::<SemanticEvent>(v).unwrap(), ev);

        let ctx = SemanticEvent::Context {
            stream: stream(),
            change: ContextChange::Cleared,
            item: None,
            size: ContextSize::default(),
        };
        assert_eq!(ctx.data_class(), DataClass::Operational);
        let text = serde_json::to_string(&ctx).unwrap();
        assert!(!text.contains("TWO"), "context events carry no content");
    }

    #[test]
    fn every_payload_kind_round_trips_without_tag_clashes() {
        let rel = ObservationPayload::Relation(Relation {
            relation: "on_day".into(),
            from: ObservationId("u1.entity.0".into()),
            to: "u1.entity.1".into(),
        });
        let v = serde_json::to_value(&rel).unwrap();
        assert_eq!(v["type"], "relation");
        assert_eq!(v["relation"], "on_day");
        assert_eq!(
            serde_json::from_value::<ObservationPayload>(v).unwrap(),
            rel
        );
        let refr = ObservationPayload::Reference(ReferenceResolution {
            mention: "IT".into(),
            kind: ReferenceKind::Pronoun,
            candidates: vec![],
            resolution: Resolution::Unresolved { reason: "x".into() },
        });
        let text = serde_json::to_string(&refr).unwrap();
        let back: ObservationPayload = serde_json::from_str(&text).unwrap();
        assert_eq!(back, refr, "{text}");
        let act = ObservationPayload::Act(CommunicativeAct {
            taxonomy: "ce.acts".into(),
            taxonomy_version: "1.0.0".into(),
            decision: ActDecision::Labeled {
                label: "request".into(),
            },
            alternatives: vec![],
        });
        let text = serde_json::to_string(&act).unwrap();
        assert_eq!(
            serde_json::from_str::<ObservationPayload>(&text).unwrap(),
            act
        );
    }

    #[test]
    fn resolution_and_act_shapes() {
        let r = Resolution::Resolved {
            target: ContextItemId("c1".into()),
        };
        assert_eq!(serde_json::to_value(&r).unwrap()["outcome"], "resolved");
        let a = ActDecision::Abstained {
            reason: ActAbstention::Ambiguous,
        };
        let v = serde_json::to_value(&a).unwrap();
        assert_eq!(v["decision"], "abstained");
        assert_eq!(v["reason"], "ambiguous");
        let d = NormalizedValue::TimeOfDay {
            hour: 9,
            minute: 30,
            pm: None,
        };
        let v = serde_json::to_value(&d).unwrap();
        assert!(v["value"]["pm"].is_null(), "unknown am/pm stays unknown");
    }

    #[test]
    fn inputs_are_validated() {
        let mut i = SemanticInput {
            stream: stream(),
            utterance: 1,
            revision: 0,
            status: RevisionStatus::Partial,
            text: "TURN IT ON".into(),
            committed_chars: 4,
            language: None,
            recognition_score: None,
            time_range: (MonotonicTime::from_nanos(0), MonotonicTime::from_nanos(1)),
            audio_ms: 100,
        };
        i.validate(100).unwrap();
        assert_eq!(
            i.validate(5).unwrap_err().code(),
            "semantic.input_oversized"
        );
        i.committed_chars = 50;
        assert_eq!(
            i.validate(100).unwrap_err().code(),
            "semantic.input_malformed"
        );
        i.committed_chars = 0;
        i.text = "A\u{0}B".into();
        assert_eq!(
            i.validate(100).unwrap_err().code(),
            "semantic.input_malformed"
        );
        assert!(TextSpan { start: 0, end: 3 }.valid_for(3));
        assert!(!TextSpan { start: 2, end: 2 }.valid_for(3));
        assert!(!TextSpan { start: 0, end: 4 }.valid_for(3));
    }
}
