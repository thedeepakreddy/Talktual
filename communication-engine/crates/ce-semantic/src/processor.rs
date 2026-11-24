//! Per-stream semantic processing: revision handling, contract enforcement,
//! stable identity, lifecycle and context.
//!
//! ## Revision semantics
//!
//! * Inputs of one utterance must have increasing revision numbers. An older
//!   revision is rejected as `Stale` and an equal one as `Duplicate`.
//!   Anything for an utterance that already finalized or was cancelled is
//!   rejected as `AfterFinal`. Stale results never become current.
//! * Revision numbers may skip: the worker coalesces queued partial
//!   revisions.
//! * Each processed revision is interpreted **as a whole snapshot**. The
//!   new set of observations is diffed against the current set:
//!   * `New`: first appearance;
//!   * `Updated`: payload, span, uncertainty, evidence or epistemic status
//!     changed;
//!   * `Stabilized`: the span entered committed text;
//!   * `Finalized`: from the final revision;
//!   * `Retracted`: no longer supported.
//!
//!   Unchanged observations produce no event.
//! * **Identity.** An observation keeps its id across revisions when the
//!   adapter produces the same key (kind, category, folded mention,
//!   occurrence). Failing that, it keeps the id when an unmatched draft of
//!   the same kind and category overlaps its span (e.g. "TWENTY" →
//!   "TWENTY FIVE" is an update).
//! * **Status** comes from the revision, not the adapter:
//!   * final revision → `Final`;
//!   * span inside committed text → `Stable`;
//!   * otherwise → `Provisional`.
//! * A cancelled utterance retracts all its observations and invalidates
//!   its context items.
//!
//! ## Contract enforcement
//!
//! Drafts are dropped (`invalid_drafts`, `Degraded(InvalidOutput)`) when:
//! * the span is outside the text;
//! * an entity or reference mention differs from the transcript text at
//!   its span;
//! * the evidence is empty or an evidence span is invalid;
//! * the draft claims `UserConfirmedFact`;
//! * a score is not finite, or does not have the adapter's declared score
//!   semantics, or claims calibration;
//! * a relation points forward or to a dropped draft.
//!
//! Only the host can confirm an observation ([`SemanticProcessor::confirm`]).
//!
//! ## Time
//!
//! `produced_at` and context expiry use the **stream timeline** (the end of
//! the revision's audio), which makes replay deterministic.

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use ce_core::ids::AudioStreamId;
use ce_core::intel::ScoreSemantics;
use ce_core::privacy::RedactionPolicy;
use ce_core::semantic::{
    DegradeReason, Epistemic, ObservationChange, ObservationId, ObservationPayload,
    ObservationStatus, ProfileRef, Provenance, RejectReason, Relation, Resolution,
    SEMANTIC_SCHEMA_VERSION, SemanticCapabilities, SemanticConfig, SemanticCounters,
    SemanticEndReason, SemanticEvent, SemanticInput, SemanticObservation, SemanticStatus,
    SourceRef, TextSpan,
};
use ce_core::speech::RevisionStatus;
use ce_core::time::MonotonicTime;
use ce_core::{CeError, ErrorCategory};

use crate::adapter::{Draft, DraftPayload, RelationTarget, SemanticAdapter, unsupported_features};
use crate::context::ContextManager;
use crate::metrics::Reservoir;

/// Closed utterance ids remembered for `AfterFinal` rejection.
const CLOSED_MEMORY: usize = 1_024;
/// Finalized utterances whose observations are kept for correction cascades.
const FINAL_MEMORY: usize = 16;

/// What a processor is built from.
#[derive(Clone, Debug)]
pub struct ProcessorSetup {
    /// Stream.
    pub stream: AudioStreamId,
    /// Validated configuration.
    pub config: SemanticConfig,
    /// The adapter's capabilities.
    pub capabilities: SemanticCapabilities,
}

#[derive(Debug, Default)]
struct UttState {
    last_revision: Option<u32>,
    current: BTreeMap<ObservationId, SemanticObservation>,
    keys: BTreeMap<ObservationId, String>,
    next_id: u32,
    first_done: bool,
}

/// The per-stream processor (single-threaded; owned by the worker).
pub struct SemanticProcessor {
    stream: AudioStreamId,
    config: SemanticConfig,
    caps: SemanticCapabilities,
    fingerprint: String,
    profile: Option<ProfileRef>,
    adapter: Box<dyn SemanticAdapter>,
    context: ContextManager,
    open: BTreeMap<u64, UttState>,
    finals: VecDeque<(u64, BTreeMap<ObservationId, SemanticObservation>)>,
    closed: VecDeque<u64>,
    counters: SemanticCounters,
    processing: Reservoir,
    queue_delay: Reservoir,
    first_obs: Reservoir,
    consecutive_errors: u32,
    running: bool,
    ended: Option<SemanticEndReason>,
}

impl std::fmt::Debug for SemanticProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SemanticProcessor")
            .field("stream", &self.stream)
            .field("adapter", &self.caps.adapter)
            .field("fingerprint", &self.fingerprint)
            .field("running", &self.running)
            .finish_non_exhaustive()
    }
}

fn key_with_occurrence(drafts: &[Draft]) -> Vec<String> {
    let mut seen: BTreeMap<&str, u32> = BTreeMap::new();
    drafts
        .iter()
        .map(|d| {
            let n = seen.entry(d.key.as_str()).or_insert(0);
            let k = format!("{}#{}", d.key, n);
            *n += 1;
            k
        })
        .collect()
}

fn overlaps(a: TextSpan, b: TextSpan) -> bool {
    a.start < b.end && b.start < a.end
}

fn same_class(d: &Draft, o: &SemanticObservation) -> bool {
    match (&d.payload, &o.payload) {
        (DraftPayload::Entity(a), ObservationPayload::Entity(b)) => a.category == b.category,
        (DraftPayload::Reference(_), ObservationPayload::Reference(_)) => true,
        (DraftPayload::Act(_), ObservationPayload::Act(_)) => true,
        (DraftPayload::Relation { kind, .. }, ObservationPayload::Relation(r)) => *kind == r.relation,
        _ => false,
    }
}

fn cut(text: &str, s: TextSpan) -> String {
    text.chars()
        .skip(s.start as usize)
        .take((s.end - s.start) as usize)
        .collect()
}

impl SemanticProcessor {
    /// Build a processor. Fails when the locality policy forbids the
    /// adapter or the configuration asks for unsupported features.
    pub fn new(setup: ProcessorSetup, adapter: Box<dyn SemanticAdapter>) -> Result<Self, CeError> {
        setup.config.validate()?;
        if !setup.config.policy.permits(setup.capabilities.locality) {
            return Err(CeError::new(
                ErrorCategory::Unauthorized,
                "semantic.locality_not_permitted",
                "the semantic adapter's locality is not permitted by the policy",
            )
            .with_context("adapter", setup.capabilities.adapter.clone()));
        }
        let missing = unsupported_features(&setup.capabilities, &setup.config);
        if !missing.is_empty() {
            return Err(CeError::new(
                ErrorCategory::CapabilityResolution,
                "semantic.feature_unsupported",
                "the semantic adapter does not support a configured feature",
            )
            .with_context("features", missing.join(","))
            .with_context("adapter", setup.capabilities.adapter.clone()));
        }
        Ok(Self {
            stream: setup.stream,
            fingerprint: setup.config.fingerprint(),
            profile: setup.config.profile.as_ref().map(|p| p.reference()),
            context: ContextManager::new(setup.stream, setup.config.context.clone()),
            config: setup.config,
            caps: setup.capabilities,
            adapter,
            open: BTreeMap::new(),
            finals: VecDeque::new(),
            closed: VecDeque::new(),
            counters: SemanticCounters::default(),
            processing: Reservoir::new(1_024),
            queue_delay: Reservoir::new(1_024),
            first_obs: Reservoir::new(1_024),
            consecutive_errors: 0,
            running: false,
            ended: None,
        })
    }

    /// Stream.
    pub fn stream(&self) -> AudioStreamId {
        self.stream
    }

    /// Configuration.
    pub fn config(&self) -> &SemanticConfig {
        &self.config
    }

    /// Context (read-only).
    pub fn context(&self) -> &ContextManager {
        &self.context
    }

    /// Emit `Started`.
    pub fn start(&mut self, out: &mut Vec<SemanticEvent>) {
        self.running = true;
        out.push(SemanticEvent::Started {
            stream: self.stream,
            capabilities: Box::new(self.caps.clone()),
            config_fingerprint: self.fingerprint.clone(),
            profile: self.profile.clone(),
        });
    }

    fn reject(
        &mut self,
        input: &SemanticInput,
        reason: RejectReason,
        out: &mut Vec<SemanticEvent>,
    ) {
        match reason {
            RejectReason::Stale | RejectReason::Duplicate | RejectReason::AfterFinal => {
                self.counters.stale_rejected += 1
            }
            RejectReason::Oversized | RejectReason::Malformed => {
                self.counters.invalid_rejected += 1
            }
            RejectReason::Coalesced => self.counters.coalesced += 1,
        }
        out.push(SemanticEvent::InputRejected {
            stream: self.stream,
            utterance: input.utterance,
            revision: input.revision,
            reason,
        });
    }

    /// Record inputs the worker coalesced away (no event per input).
    pub fn note_coalesced(&mut self, n: u64) {
        self.counters.coalesced += n;
    }

    /// Record an overload (queue full).
    pub fn note_overload(&mut self, out: &mut Vec<SemanticEvent>) {
        out.push(SemanticEvent::Degraded {
            stream: self.stream,
            reason: DegradeReason::Overload,
        });
    }

    fn close(&mut self, utterance: u64) {
        self.closed.push_back(utterance);
        while self.closed.len() > CLOSED_MEMORY {
            self.closed.pop_front();
        }
    }

    fn retract(&mut self, mut o: SemanticObservation, out: &mut Vec<SemanticEvent>) {
        o.version += 1;
        o.status = ObservationStatus::Retracted;
        self.counters.retracted += 1;
        out.push(SemanticEvent::Observation {
            change: ObservationChange::Retracted,
            observation: Box::new(o),
        });
    }

    /// Retract every current observation that depends on invalidated
    /// context items (resolved references and their relations), in open
    /// and recently finalized utterances.
    fn cascade(
        &mut self,
        items: &[ce_core::semantic::ContextItemId],
        out: &mut Vec<SemanticEvent>,
    ) {
        if items.is_empty() {
            return;
        }
        let depends = |o: &SemanticObservation| match &o.payload {
            ObservationPayload::Reference(r) => {
                matches!(&r.resolution, Resolution::Resolved { target } if items.contains(target))
            }
            ObservationPayload::Relation(r) => {
                r.relation == "refers_to" && items.iter().any(|i| i.0 == r.to)
            }
            _ => false,
        };
        let mut gone = Vec::new();
        for st in self.open.values_mut() {
            let ids: Vec<ObservationId> = st
                .current
                .values()
                .filter(|o| depends(o))
                .map(|o| o.id.clone())
                .collect();
            for id in ids {
                if let Some(o) = st.current.remove(&id) {
                    gone.push(o);
                }
            }
        }
        for (_, m) in self.finals.iter_mut() {
            let ids: Vec<ObservationId> = m
                .values()
                .filter(|o| depends(o))
                .map(|o| o.id.clone())
                .collect();
            for id in ids {
                if let Some(o) = m.remove(&id) {
                    gone.push(o);
                }
            }
        }
        for o in gone {
            self.retract(o, out);
        }
    }

    fn invalidate(&mut self, id: &ObservationId, out: &mut Vec<SemanticEvent>) {
        let items = self.context.invalidate_observation(id, out);
        self.cascade(&items, out);
    }

    fn validate_drafts(
        &mut self,
        input: &SemanticInput,
        drafts: Vec<Draft>,
        out: &mut Vec<SemanticEvent>,
    ) -> Vec<Option<Draft>> {
        let len = input.text.chars().count() as u32;
        let mut kept: Vec<Option<Draft>> = Vec::with_capacity(drafts.len());
        let mut invalid = 0u64;
        for d in drafts {
            let idx = kept.len();
            let mention_ok = match &d.payload {
                DraftPayload::Entity(e) => {
                    d.span.valid_for(len) && cut(&input.text, d.span) == e.mention
                }
                DraftPayload::Reference(r) => {
                    d.span.valid_for(len) && cut(&input.text, d.span) == r.mention
                }
                _ => true,
            };
            let score_ok = d.uncertainty.score.is_none_or(|s| {
                s.value.is_finite()
                    && Some(s.semantics) == self.caps.score_semantics
                    && (!d.uncertainty.calibrated
                        || s.semantics == ScoreSemantics::CalibratedProbability)
            }) && (!d.uncertainty.calibrated || d.uncertainty.score.is_some());
            let relation_ok = match &d.payload {
                DraftPayload::Relation { from, to, .. } => {
                    *from < idx
                        && kept[*from].is_some()
                        && match to {
                            RelationTarget::Draft(t) => *t < idx && kept[*t].is_some(),
                            RelationTarget::Context(_) => true,
                        }
                }
                _ => true,
            };
            let ok = d.span.valid_for(len)
                && mention_ok
                && score_ok
                && relation_ok
                && d.epistemic != Epistemic::UserConfirmedFact
                && !d.evidence.is_empty()
                && d.evidence
                    .iter()
                    .all(|e| e.span.is_none_or(|s| s.valid_for(len)) && e.rule.len() <= 128);
            if ok {
                kept.push(Some(d));
            } else {
                invalid += 1;
                kept.push(None);
            }
        }
        if invalid > 0 {
            self.counters.invalid_drafts += invalid;
            out.push(SemanticEvent::Degraded {
                stream: self.stream,
                reason: DegradeReason::InvalidOutput,
            });
        }
        // Below the emission threshold (and relations that depend on them).
        for i in 0..kept.len() {
            let drop = match &kept[i] {
                Some(d) => {
                    d.certainty() < self.config.min_certainty
                        || match &d.payload {
                            DraftPayload::Relation { from, to, .. } => {
                                kept[*from].is_none()
                                    || matches!(to, RelationTarget::Draft(t) if kept[*t].is_none())
                            }
                            _ => false,
                        }
                }
                None => false,
            };
            if drop {
                kept[i] = None;
                self.counters.below_min_certainty += 1;
            }
        }
        // Output limit.
        let max = self.config.budget.max_observations as usize;
        let mut n = 0;
        let mut truncated = false;
        for slot in kept.iter_mut() {
            if slot.is_some() {
                n += 1;
                if n > max {
                    *slot = None;
                    truncated = true;
                }
            }
        }
        if truncated {
            self.counters.output_truncated += 1;
            out.push(SemanticEvent::Degraded {
                stream: self.stream,
                reason: DegradeReason::OutputLimited,
            });
            // Relations whose endpoints were truncated.
            for i in 0..kept.len() {
                if let Some(Draft {
                    payload: DraftPayload::Relation { from, to, .. },
                    ..
                }) = &kept[i]
                {
                    if kept[*from].is_none()
                        || matches!(to, RelationTarget::Draft(t) if kept[*t].is_none())
                    {
                        kept[i] = None;
                    }
                }
            }
        }
        kept
    }

    /// Process one transcript revision. `queued_ms` is how long it waited
    /// in the worker queue. An `Err` is fatal (repeated adapter errors).
    pub fn process(
        &mut self,
        input: SemanticInput,
        queued_ms: f32,
        out: &mut Vec<SemanticEvent>,
    ) -> Result<(), CeError> {
        self.counters.inputs += 1;
        self.queue_delay.record(queued_ms);
        if self.closed.contains(&input.utterance) {
            self.reject(&input, RejectReason::AfterFinal, out);
            return Ok(());
        }
        if let Some(last) = self
            .open
            .get(&input.utterance)
            .and_then(|s| s.last_revision)
        {
            if input.revision <= last {
                let r = if input.revision == last {
                    RejectReason::Duplicate
                } else {
                    RejectReason::Stale
                };
                self.reject(&input, r, out);
                return Ok(());
            }
        }
        if let Err(e) = input.validate(self.config.budget.max_input_chars) {
            let r = if e.code() == "semantic.input_oversized" {
                RejectReason::Oversized
            } else {
                RejectReason::Malformed
            };
            self.reject(&input, r, out);
            // A final or cancellation that cannot be interpreted still ends
            // the utterance: its provisional observations are retracted.
            if input.status != RevisionStatus::Partial {
                self.end_without_output(&input, out);
            }
            return Ok(());
        }
        let now = input.time_range.1;
        self.context.expire(now, out);
        self.context.note_utterance(input.utterance);
        self.open.entry(input.utterance).or_default().last_revision = Some(input.revision);

        if input.status == RevisionStatus::Cancelled {
            self.cancel_utterance(input.utterance, out);
            return Ok(());
        }
        if input.status == RevisionStatus::Partial && !self.config.budget.partials {
            return Ok(());
        }

        let view = self.context.view(input.utterance);
        let t0 = Instant::now();
        let result = self.adapter.analyze(&input, &view);
        let elapsed = t0.elapsed();
        let ms = elapsed.as_secs_f32() * 1_000.0;
        let drafts = match result {
            Ok(d) => {
                self.consecutive_errors = 0;
                d
            }
            Err(e) => {
                self.counters.adapter_errors += 1;
                self.consecutive_errors += 1;
                out.push(SemanticEvent::Degraded {
                    stream: self.stream,
                    reason: DegradeReason::AdapterError,
                });
                if self.consecutive_errors >= self.config.budget.max_consecutive_errors {
                    return Err(e);
                }
                if input.status == RevisionStatus::Final {
                    self.end_without_output(&input, out);
                }
                return Ok(());
            }
        };
        self.processing.record(ms);
        self.counters.processed += 1;
        if elapsed.as_millis() as u64 > self.config.budget.deadline_ms {
            self.counters.deadline_exceeded += 1;
            out.push(SemanticEvent::Degraded {
                stream: self.stream,
                reason: DegradeReason::DeadlineExceeded,
            });
            if input.status == RevisionStatus::Final {
                self.end_without_output(&input, out);
            }
            return Ok(());
        }
        let kept = self.validate_drafts(&input, drafts, out);
        self.apply(&input, kept, now, out);
        Ok(())
    }

    /// A final that produced no usable output: retract and finalize.
    fn end_without_output(&mut self, input: &SemanticInput, out: &mut Vec<SemanticEvent>) {
        let st = self.open.remove(&input.utterance).unwrap_or_default();
        for (_, o) in st.current {
            if matches!(o.payload, ObservationPayload::Entity(_)) {
                self.invalidate(&o.id, out);
            }
            self.retract(o, out);
        }
        self.close(input.utterance);
        self.context.end_utterance(input.utterance, out);
        if input.status == RevisionStatus::Cancelled {
            self.counters.cancelled += 1;
            out.push(SemanticEvent::UtteranceCancelled {
                stream: self.stream,
                utterance: input.utterance,
            });
        } else {
            out.push(SemanticEvent::UtteranceFinalized {
                stream: self.stream,
                utterance: input.utterance,
                revision: input.revision,
                observations: 0,
            });
        }
    }

    fn cancel_utterance(&mut self, utterance: u64, out: &mut Vec<SemanticEvent>) {
        let st = self.open.remove(&utterance).unwrap_or_default();
        for (_, o) in st.current {
            self.retract(o, out);
        }
        let items = self.context.invalidate_utterance(utterance, out);
        self.cascade(&items, out);
        self.close(utterance);
        self.counters.cancelled += 1;
        out.push(SemanticEvent::UtteranceCancelled {
            stream: self.stream,
            utterance,
        });
    }

    fn apply(
        &mut self,
        input: &SemanticInput,
        kept: Vec<Option<Draft>>,
        now: MonotonicTime,
        out: &mut Vec<SemanticEvent>,
    ) {
        let utt = input.utterance;
        let mut st = self.open.remove(&utt).unwrap_or_default();
        let idx: Vec<usize> = (0..kept.len()).filter(|i| kept[*i].is_some()).collect();
        let drafts: Vec<Draft> = idx.iter().filter_map(|i| kept[*i].clone()).collect();
        // Map original draft index → position in `drafts`.
        let pos: BTreeMap<usize, usize> = idx.iter().enumerate().map(|(p, i)| (*i, p)).collect();
        let keys = key_with_occurrence(&drafts);

        // Identity: exact key, then overlapping span of the same class.
        let mut ids: Vec<Option<ObservationId>> = vec![None; drafts.len()];
        let mut taken: Vec<ObservationId> = Vec::new();
        for (i, k) in keys.iter().enumerate() {
            if let Some((id, _)) = st
                .keys
                .iter()
                .find(|(id, key)| *key == k && st.current.contains_key(*id) && !taken.contains(id))
            {
                ids[i] = Some(id.clone());
                taken.push(id.clone());
            }
        }
        for (i, d) in drafts.iter().enumerate() {
            if ids[i].is_some() {
                continue;
            }
            if let Some(o) = st.current.values().find(|o| {
                !taken.contains(&o.id) && same_class(d, o) && overlaps(d.span, o.source.span)
            }) {
                ids[i] = Some(o.id.clone());
                taken.push(o.id.clone());
            }
        }
        for (i, d) in drafts.iter().enumerate() {
            if ids[i].is_none() {
                let id = ObservationId(format!("u{utt}.{}.{}", d.payload.kind(), st.next_id));
                st.next_id += 1;
                ids[i] = Some(id);
            }
        }
        let ids: Vec<ObservationId> = ids
            .into_iter()
            .map(|i| i.unwrap_or_else(|| ObservationId(String::new())))
            .collect();

        let provenance = Provenance {
            adapter: self.caps.adapter.clone(),
            adapter_version: self.caps.version.clone(),
            config_fingerprint: self.fingerprint.clone(),
            profile: self.profile.clone(),
            locality: self.caps.locality,
        };
        let mut next: BTreeMap<ObservationId, SemanticObservation> = BTreeMap::new();
        let mut next_keys: BTreeMap<ObservationId, String> = BTreeMap::new();
        let mut changes: Vec<(ObservationChange, ObservationId, bool)> = Vec::new();
        for (i, d) in drafts.into_iter().enumerate() {
            let id = ids[i].clone();
            let payload = match d.payload {
                DraftPayload::Entity(e) => ObservationPayload::Entity(e),
                DraftPayload::Reference(r) => ObservationPayload::Reference(r),
                DraftPayload::Act(a) => ObservationPayload::Act(a),
                DraftPayload::Relation { kind, from, to } => {
                    ObservationPayload::Relation(Relation {
                        relation: kind,
                        from: ids[pos[&from]].clone(),
                        to: match to {
                            RelationTarget::Draft(t) => ids[pos[&t]].0.clone(),
                            RelationTarget::Context(c) => c.0,
                        },
                    })
                }
            };
            let status = if input.status == RevisionStatus::Final {
                ObservationStatus::Final
            } else if d.span.end <= input.committed_chars {
                ObservationStatus::Stable
            } else {
                ObservationStatus::Provisional
            };
            let source = SourceRef {
                stream: self.stream,
                utterance: utt,
                revision: input.revision,
                revision_status: input.status,
                span: d.span,
                committed: d.span.end <= input.committed_chars
                    || input.status == RevisionStatus::Final,
            };
            let mut obs = SemanticObservation {
                id: id.clone(),
                schema_version: SEMANTIC_SCHEMA_VERSION,
                version: 1,
                payload,
                status,
                epistemic: d.epistemic,
                uncertainty: d.uncertainty,
                evidence: d.evidence,
                source,
                provenance: provenance.clone(),
                produced_at: now,
            };
            let mut content_changed = false;
            let change = match st.current.get(&id) {
                None => Some(ObservationChange::New),
                Some(prev) => {
                    let same_content =
                        prev.payload == obs.payload && prev.source.span == obs.source.span;
                    content_changed = !same_content;
                    if same_content && prev.epistemic == Epistemic::UserConfirmedFact {
                        obs.epistemic = Epistemic::UserConfirmedFact;
                    }
                    let same = same_content
                        && prev.uncertainty == obs.uncertainty
                        && prev.evidence == obs.evidence
                        && prev.epistemic == obs.epistemic;
                    obs.version = prev.version + 1;
                    if same && prev.status == obs.status {
                        obs = prev.clone();
                        None
                    } else if obs.status == ObservationStatus::Final {
                        Some(ObservationChange::Finalized)
                    } else if same
                        && prev.status == ObservationStatus::Provisional
                        && obs.status == ObservationStatus::Stable
                    {
                        Some(ObservationChange::Stabilized)
                    } else {
                        Some(ObservationChange::Updated)
                    }
                }
            };
            if let Some(c) = change {
                changes.push((c, id.clone(), content_changed));
            }
            next_keys.insert(id.clone(), keys[i].clone());
            next.insert(id, obs);
        }
        // Retractions: current observations not supported any more.
        let gone: Vec<ObservationId> = st
            .current
            .keys()
            .filter(|id| !next.contains_key(*id))
            .cloned()
            .collect();
        for id in gone {
            if let Some(o) = st.current.remove(&id) {
                if matches!(o.payload, ObservationPayload::Entity(_)) {
                    self.invalidate(&o.id, out);
                }
                self.retract(o, out);
            }
        }
        // Emit changes in draft order; updated entities leave the context
        // (their item is re-admitted below with the new content).
        for (change, id, content_changed) in &changes {
            let o = &next[id];
            if *content_changed && matches!(o.payload, ObservationPayload::Entity(_)) {
                self.invalidate(id, out);
            }
            match change {
                ObservationChange::New => self.counters.observations_new += 1,
                ObservationChange::Updated => self.counters.updated += 1,
                ObservationChange::Stabilized => self.counters.stabilized += 1,
                ObservationChange::Finalized => self.counters.finalized += 1,
                ObservationChange::Retracted => self.counters.retracted += 1,
                ObservationChange::Confirmed => self.counters.confirmed += 1,
            }
            out.push(SemanticEvent::Observation {
                change: *change,
                observation: Box::new(o.clone()),
            });
        }
        // Context admission.
        for o in next.values() {
            self.context.admit(o, now, out);
        }
        if !st.first_done
            && next
                .values()
                .any(|o| !matches!(o.payload, ObservationPayload::Act(_)))
        {
            st.first_done = true;
            self.first_obs.record(input.audio_ms as f32);
        }
        st.current = next;
        st.keys = next_keys;
        if input.status == RevisionStatus::Final {
            let n = st.current.len() as u32;
            self.finals.push_back((utt, st.current));
            while self.finals.len() > FINAL_MEMORY {
                self.finals.pop_front();
            }
            self.close(utt);
            self.context.end_utterance(utt, out);
            out.push(SemanticEvent::UtteranceFinalized {
                stream: self.stream,
                utterance: utt,
                revision: input.revision,
                observations: n,
            });
        } else {
            self.open.insert(utt, st);
        }
    }

    /// The host confirms an observation: it becomes a `UserConfirmedFact`.
    /// Only current (not retracted) observations can be confirmed.
    pub fn confirm(
        &mut self,
        id: &ObservationId,
        out: &mut Vec<SemanticEvent>,
    ) -> Result<(), CeError> {
        let found = self
            .open
            .values_mut()
            .find_map(|s| s.current.get_mut(id))
            .or_else(|| self.finals.iter_mut().find_map(|(_, m)| m.get_mut(id)));
        let Some(o) = found else {
            return Err(CeError::invalid_input(
                "semantic.observation_unknown",
                "no current observation has this id",
            ));
        };
        if o.epistemic == Epistemic::UserConfirmedFact {
            return Ok(());
        }
        o.epistemic = Epistemic::UserConfirmedFact;
        o.version += 1;
        self.counters.confirmed += 1;
        out.push(SemanticEvent::Observation {
            change: ObservationChange::Confirmed,
            observation: Box::new(o.clone()),
        });
        Ok(())
    }

    /// Clear the context (host request or stream reset). Observations stay.
    pub fn reset_context(&mut self, out: &mut Vec<SemanticEvent>) {
        self.context.clear(out);
        self.adapter.reset();
    }

    /// Finish: open utterances are retracted (they never finalized), the
    /// context is cleared, `Stopped` is emitted.
    pub fn finish(&mut self, reason: SemanticEndReason, out: &mut Vec<SemanticEvent>) {
        if !self.running && self.ended.is_some() {
            return;
        }
        let open: Vec<u64> = self.open.keys().copied().collect();
        for u in open {
            self.cancel_utterance(u, out);
        }
        self.context.clear(out);
        self.finals.clear();
        self.running = false;
        self.ended = Some(reason);
        out.push(SemanticEvent::Stopped {
            stream: self.stream,
            reason,
            counters: Box::new(self.counters()),
        });
    }

    /// Fail with `error`, then finish.
    pub fn fail(&mut self, error: &CeError, out: &mut Vec<SemanticEvent>) {
        out.push(SemanticEvent::Failed {
            stream: self.stream,
            error: error.report(&RedactionPolicy::DEFAULT),
        });
        self.finish(SemanticEndReason::Failed, out);
    }

    /// Counters with latency summaries.
    pub fn counters(&self) -> SemanticCounters {
        let mut c = self.counters.clone();
        c.context_evicted = self.context.evicted();
        c.context_expired = self.context.expired();
        c.processing_ms = self.processing.summary();
        c.queue_delay_ms = self.queue_delay.summary();
        c.first_observation_ms = self.first_obs.summary();
        c
    }

    /// Current observations of open and recently finalized utterances.
    pub fn current(&self) -> Vec<&SemanticObservation> {
        self.open
            .values()
            .flat_map(|s| s.current.values())
            .chain(self.finals.iter().flat_map(|(_, m)| m.values()))
            .collect()
    }

    /// Status snapshot.
    pub fn status(&self) -> SemanticStatus {
        SemanticStatus {
            stream: self.stream,
            capabilities: self.caps.clone(),
            config_fingerprint: self.fingerprint.clone(),
            profile: self.profile.clone(),
            running: self.running,
            context: self.context.size(),
            current_observations: self.current().len() as u32,
            counters: self.counters(),
            ended: self.ended,
        }
    }
}
