//! The semantic adapter boundary.
//!
//! Provider, model and rule engines stay behind [`SemanticAdapter`]; CE
//! code depends only on this trait and CE-owned types.
//!
//! An adapter turns one transcript snapshot into [`Draft`]s: observations
//! without ids, versions, lifecycle status or provenance. The
//! [`crate::SemanticProcessor`] validates drafts (spans, mentions, evidence,
//! epistemic limits), assigns stable ids across revisions, derives status
//! from the revision, attaches provenance and diffs against the previous
//! revision. An adapter therefore cannot edit the transcript, cannot mark
//! anything as user-confirmed and cannot see other streams' context.
//!
//! **Lifecycle** (on the semantic worker thread, never in an audio callback
//! or on the control plane): [`SemanticFactory::capabilities`] (cheap) →
//! [`SemanticFactory::create`] → [`SemanticAdapter::analyze`]* →
//! [`SemanticAdapter::reset`] / drop.

use ce_core::CeError;
use ce_core::semantic::{
    Certainty, CommunicativeAct, ContextItem, ContextItemId, Entity, Epistemic, Evidence,
    ReferenceResolution, SemanticCapabilities, SemanticConfig, SemanticInput, TextSpan,
    Uncertainty,
};

/// Target of a drafted relation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelationTarget {
    /// Another draft of the same output (by index).
    Draft(usize),
    /// A context item.
    Context(ContextItemId),
}

/// The payload of a draft.
#[derive(Clone, Debug, PartialEq)]
pub enum DraftPayload {
    /// An entity.
    Entity(Entity),
    /// A typed link from draft `from` to `to`.
    Relation {
        /// Relation type.
        kind: String,
        /// Source draft index (must precede this draft).
        from: usize,
        /// Target.
        to: RelationTarget,
    },
    /// A reference resolution.
    Reference(ReferenceResolution),
    /// A communicative act.
    Act(CommunicativeAct),
}

impl DraftPayload {
    /// Kind name.
    pub fn kind(&self) -> &'static str {
        match self {
            DraftPayload::Entity(_) => "entity",
            DraftPayload::Relation { .. } => "relation",
            DraftPayload::Reference(_) => "reference",
            DraftPayload::Act(_) => "act",
        }
    }
}

/// An adapter's proposed observation.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    /// Identity key: drafts with the same key in successive revisions of an
    /// utterance are the same observation (e.g. `entity:number:twenty`).
    /// Operational; it may contain folded mention text but never leaves the
    /// processor.
    pub key: String,
    /// Payload.
    pub payload: DraftPayload,
    /// Span in the input text.
    pub span: TextSpan,
    /// Kind of claim (never `UserConfirmedFact`).
    pub epistemic: Epistemic,
    /// Uncertainty.
    pub uncertainty: Uncertainty,
    /// Evidence (at least one).
    pub evidence: Vec<Evidence>,
}

impl Draft {
    /// Certainty tier.
    pub fn certainty(&self) -> Certainty {
        self.uncertainty.level
    }
}

/// What the adapter may see of the context: only items the processor
/// permits (same stream, active, in scope, within the utterance distance),
/// most recent first.
#[derive(Clone, Debug, Default)]
pub struct ContextView {
    /// Permitted items.
    pub items: Vec<ContextItem>,
    /// Context is disabled by configuration (references stay unresolved).
    pub disabled: bool,
}

/// One semantic analyzer instance (one per stream).
pub trait SemanticAdapter: Send {
    /// Interpret one transcript snapshot.
    fn analyze(
        &mut self,
        input: &SemanticInput,
        context: &ContextView,
    ) -> Result<Vec<Draft>, CeError>;

    /// Forget any internal state (context reset, stream reset).
    fn reset(&mut self) {}
}

/// Creates adapters and describes them.
pub trait SemanticFactory: Send + Sync {
    /// Static capabilities (cheap, no loading).
    fn capabilities(&self) -> SemanticCapabilities;
    /// A new adapter for one stream with this (validated) configuration.
    fn create(&self, config: &SemanticConfig) -> Result<Box<dyn SemanticAdapter>, CeError>;
}

/// Configured features the adapter cannot provide.
pub fn unsupported_features(
    caps: &SemanticCapabilities,
    config: &SemanticConfig,
) -> Vec<&'static str> {
    let mut v = Vec::new();
    if config.entities && !caps.entities {
        v.push("entities");
    }
    if config.relations && !caps.relations {
        v.push("relations");
    }
    if config.references && !caps.references {
        v.push("references");
    }
    if config.acts && !caps.acts {
        v.push("acts");
    }
    v
}
