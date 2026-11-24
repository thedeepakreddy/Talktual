//! Semantic and contextual intelligence contracts (Phase 5).
//!
//! Semantic processing consumes Phase 4 transcript revisions and produces
//! typed, versioned **observations**: entities, relations, reference
//! resolutions and (optionally) communicative acts. Each observation links
//! to the exact transcript revision and span it came from, states what kind
//! of claim it is (observation, hypothesis, resolved interpretation,
//! user-confirmed fact), its uncertainty and its provenance.
//!
//! **The transcript is immutable; observations are not ground truth.**
//! Context is scoped, bounded, expiring and ephemeral by default.
//!
//! * [`config`] — validated, bounded, fingerprinted configuration;
//! * [`model`] — inputs, observations, context items, events and status;
//! * [`profile`] — domain profiles as validated data.
//!
//! Adapters, the context manager and the per-stream processor live in the
//! `ce-semantic` crate.

pub mod config;
pub mod model;
pub mod profile;

pub use config::{
    CivilDate, ContextConfig, ContextScopeMode, LocalityPolicy, SemanticBudget, SemanticConfig,
    SemanticSettings,
};
pub use model::{
    ActAbstention, ActDecision, Attribute, Certainty, CommunicativeAct, ContextChange, ContextItem,
    ContextItemId, ContextScope, ContextSize, ContextStatus, DegradeReason, Entity, Epistemic,
    Evidence, EvidenceKind, Locality, NormalizedValue, ObservationChange, ObservationId,
    ObservationPayload, ObservationStatus, ProfileRef, Provenance, ReferenceCandidate,
    ReferenceKind, ReferenceResolution, RejectReason, Relation, Resolution,
    SEMANTIC_SCHEMA_VERSION, SemanticCapabilities, SemanticCounters, SemanticEndReason,
    SemanticEvent, SemanticInput, SemanticObservation, SemanticScore, SemanticStatus, SourceRef,
    TextSpan, Uncertainty,
};
pub use profile::{ContextHint, DomainProfile, PROFILE_SCHEMA_VERSION, ProfileTerm};
