//! Per-stream translation processing: revision handling, language policy,
//! adapter selection, context, terminology, output validation, fidelity
//! checks and lifecycle. Single-threaded and deterministic apart from
//! measured times (replay relies on this).
//!
//! **Revision rules.** Inputs of an utterance must increase in revision
//! number: older → `stale`, equal → `duplicate`, after the final or a
//! cancellation → `after_final` (all reported, none translated). Revision
//! gaps are normal (the worker coalesces partials).
//!
//! **What is translated** (`PartialPolicy`): finals always; partials only
//! under `committed` (their committed text, results `stable`, emitted only
//! when the committed text grew) or `full` (the whole snapshot, results
//! `provisional`). Every result carries the exact source revision; results
//! of an utterance get gapless sequence numbers; a newer result supersedes
//! all earlier ones of that utterance.
//!
//! **Withdrawal.** If the source utterance is cancelled, translation is
//! reset or stopped before the utterance's final, or the final cannot be
//! translated, earlier results of that utterance are withdrawn
//! (`withdrawn{through_sequence}`) — stale output is never left current.
//!
//! **Precedence** (deterministic): explicit configuration (source/target
//! language) > source text > terminology constraints > context hints >
//! adapter defaults. Context hints never rewrite text: a hint from the
//! utterance being translated whose mention is not in the source is dropped
//! and flagged `context_conflict`.

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use ce_core::error::ErrorReport;
use ce_core::ids::AudioStreamId;
use ce_core::privacy::RedactionPolicy;
use ce_core::semantic::{
    Epistemic, ObservationId, ObservationPayload, ObservationStatus, ProfileRef,
    SemanticObservation,
};
use ce_core::speech::{LanguageDecision, LanguageTag, RevisionStatus, TranscriptRevision};
use ce_core::translation::{
    PartialPolicy, SourceLanguagePolicy, SourceLanguageProvenance, SourceLanguageRef, Term,
    TerminologyProfile, TranslationCapabilities, TranslationConfig, TranslationCounters,
    TranslationDegrade, TranslationEndReason, TranslationEvent, TranslationProvenance,
    TranslationRejectReason, TranslationResult, TranslationScore, TranslationSource,
    TranslationState, TranslationStatus, TranslationTiming, UnavailableReason, UncertaintyFlag,
    WithdrawReason,
};
use ce_core::{CeError, ErrorCategory};

use crate::adapter::{AdapterOutcome, ContextHint, TranslationAdapter, TranslationRequest};
use crate::fidelity;
use crate::metrics::Reservoir;

/// Utterance states kept (older finished ones are forgotten; a revision
/// for a forgotten utterance is treated as new, which is safe because the
/// recognizer never reuses utterance numbers).
const MAX_UTTERANCES: usize = 256;
/// Hints kept per stream.
const MAX_STORED_HINTS: usize = 64;

/// One adapter with its capabilities.
pub struct AdapterSlot {
    /// Capabilities.
    pub capabilities: TranslationCapabilities,
    /// Instance.
    pub adapter: Box<dyn TranslationAdapter>,
}

impl std::fmt::Debug for AdapterSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdapterSlot")
            .field("adapter", &self.capabilities.adapter)
            .finish_non_exhaustive()
    }
}

/// Everything a processor needs.
#[derive(Debug)]
pub struct ProcessorSetup {
    /// Stream.
    pub stream: AudioStreamId,
    /// Validated configuration with a target.
    pub config: TranslationConfig,
    /// Primary adapter, then explicit fallbacks in order.
    pub adapters: Vec<AdapterSlot>,
    /// Terminology profile, if configured.
    pub terminology: Option<TerminologyProfile>,
}

#[derive(Clone, Debug, Default)]
struct Utt {
    last_revision: Option<u32>,
    finished: bool,
    next_sequence: u32,
    /// `next_sequence` at the last withdrawal (nothing newer to withdraw
    /// while equal).
    withdrawn_at: u32,
    stable_chars: u32,
    first_input: Option<Instant>,
}

/// The processor.
#[derive(Debug)]
pub struct TranslationProcessor {
    stream: AudioStreamId,
    cfg: TranslationConfig,
    target: LanguageTag,
    fingerprint: String,
    adapters: Vec<AdapterSlot>,
    terminology: Option<TerminologyProfile>,
    term_ref: Option<ProfileRef>,
    utterances: BTreeMap<u64, Utt>,
    hints: VecDeque<(ObservationId, ContextHint)>,
    counters: TranslationCounters,
    queue_ms: Reservoir,
    adapter_ms: Reservoir,
    first_ms: Reservoir,
    consecutive_errors: u32,
    backlog: u32,
    ended: Option<TranslationEndReason>,
}

fn invalid(code: &'static str, msg: &'static str) -> CeError {
    CeError::new(ErrorCategory::CapabilityResolution, code, msg)
}

/// Check a configuration against the adapters before starting. Errors are
/// typed: `translation.no_target`, `translation.remote_not_allowed`,
/// `translation.partials_unsupported`, `translation.terminology_mismatch`.
pub fn check_setup(setup: &ProcessorSetup) -> Result<(), CeError> {
    let cfg = &setup.config;
    cfg.validate()?;
    let target = cfg.target.as_ref().ok_or_else(|| {
        CeError::invalid_input("translation.no_target", "a target language is required")
    })?;
    let primary = setup
        .adapters
        .first()
        .ok_or_else(|| invalid("translation.no_adapter", "no translation adapter"))?;
    for a in &setup.adapters {
        if a.capabilities.locality == ce_core::semantic::Locality::Remote
            && !cfg.privacy.allow_remote
        {
            return Err(invalid(
                "translation.remote_not_allowed",
                "a remote translation adapter needs privacy.allow_remote",
            )
            .with_context("adapter", a.capabilities.adapter.clone()));
        }
    }
    if cfg.partials != PartialPolicy::Off && !primary.capabilities.partial_input {
        return Err(invalid(
            "translation.partials_unsupported",
            "the adapter does not translate partial input; use partials = off",
        )
        .with_context("adapter", primary.capabilities.adapter.clone()));
    }
    if let Some(p) = &setup.terminology {
        p.validate()?;
        let source_ok = match &cfg.source {
            SourceLanguagePolicy::Explicit { language } => {
                language.primary() == p.pair.source.primary()
            }
            SourceLanguagePolicy::Detected => true,
        };
        if !source_ok || p.pair.target.primary() != target.primary() {
            return Err(invalid(
                "translation.terminology_mismatch",
                "the terminology profile is for another language pair",
            ));
        }
    }
    Ok(())
}

fn hint_from(o: &SemanticObservation) -> Option<ContextHint> {
    if o.status != ObservationStatus::Final {
        return None;
    }
    // Only stated or user-confirmed claims become context; hypotheses and
    // resolved interpretations are inferences (spoken vs inferred, CTX-4).
    if !matches!(
        o.epistemic,
        Epistemic::Observation | Epistemic::UserConfirmedFact
    ) {
        return None;
    }
    let ObservationPayload::Entity(e) = &o.payload else {
        return None;
    };
    Some(ContextHint {
        category: e.category.clone(),
        mention: e.mention.clone(),
        normalized: e
            .normalized
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok()),
        utterance: o.source.utterance,
        same_utterance: false,
    })
}

impl TranslationProcessor {
    /// A processor (call [`check_setup`] first; this checks again).
    pub fn new(setup: ProcessorSetup) -> Result<Self, CeError> {
        check_setup(&setup)?;
        let target = setup.config.target.clone().expect("checked");
        let term_ref = setup
            .terminology
            .as_ref()
            .map(TerminologyProfile::reference);
        Ok(Self {
            stream: setup.stream,
            fingerprint: setup.config.fingerprint(),
            cfg: setup.config,
            target,
            adapters: setup.adapters,
            terminology: setup.terminology,
            term_ref,
            utterances: BTreeMap::new(),
            hints: VecDeque::new(),
            counters: TranslationCounters::default(),
            queue_ms: Reservoir::new(1_024),
            adapter_ms: Reservoir::new(1_024),
            first_ms: Reservoir::new(1_024),
            consecutive_errors: 0,
            backlog: 0,
            ended: None,
        })
    }

    /// Stream.
    pub fn stream(&self) -> AudioStreamId {
        self.stream
    }

    /// Configuration.
    pub fn config(&self) -> &TranslationConfig {
        &self.cfg
    }

    /// Load every adapter (worker thread).
    pub fn load(&mut self) -> Result<(), CeError> {
        for a in &mut self.adapters {
            a.adapter.load()?;
        }
        Ok(())
    }

    /// Record the worker's queue backlog (for status).
    pub fn set_backlog(&mut self, n: u32) {
        self.backlog = n;
    }

    /// Status.
    pub fn status(&self) -> TranslationStatus {
        let mut counters = self.counters.clone();
        counters.queue_ms = self.queue_ms.summary();
        counters.adapter_ms = self.adapter_ms.summary();
        counters.first_result_ms = self.first_ms.summary();
        TranslationStatus {
            stream: self.stream,
            capabilities: self.adapters[0].capabilities.clone(),
            fallback: self.adapters[1..]
                .iter()
                .map(|a| a.capabilities.adapter.clone())
                .collect(),
            target: self.target.clone(),
            config_fingerprint: self.fingerprint.clone(),
            terminology: self.term_ref.clone(),
            running: self.ended.is_none(),
            backlog: self.backlog,
            counters,
            ended: self.ended,
        }
    }

    /// Emit `Started`.
    pub fn start(&mut self, out: &mut Vec<TranslationEvent>) {
        out.push(TranslationEvent::Started {
            stream: self.stream,
            status: Box::new(self.status()),
        });
    }

    /// Count coalesced (superseded, never translated) partials.
    pub fn note_coalesced(&mut self, n: u64) {
        self.counters.coalesced += n;
    }

    /// Inputs dropped by the queue.
    pub fn note_overload(&mut self, dropped: u64, out: &mut Vec<TranslationEvent>) {
        self.counters.dropped += dropped;
        out.push(TranslationEvent::Degraded {
            stream: self.stream,
            reason: TranslationDegrade::Overload,
        });
    }

    /// Admit a Phase 5 observation as a context hint (final, stated
    /// entities only; bounded; most recent kept). A newer version of an
    /// observation replaces the older one; a retraction removes it.
    pub fn observe(&mut self, o: &SemanticObservation) {
        if !self.cfg.context.use_semantic || o.source.stream != self.stream {
            return;
        }
        self.hints.retain(|(id, _)| *id != o.id);
        if let Some(h) = hint_from(o) {
            if self.hints.len() == MAX_STORED_HINTS {
                self.hints.pop_front();
            }
            self.hints.push_back((o.id.clone(), h));
        }
    }

    fn utt(&mut self, u: u64) -> &mut Utt {
        if !self.utterances.contains_key(&u) && self.utterances.len() >= MAX_UTTERANCES {
            if let Some(old) = self
                .utterances
                .iter()
                .find(|(_, s)| s.finished)
                .map(|(k, _)| *k)
            {
                self.utterances.remove(&old);
            } else if let Some(first) = self.utterances.keys().next().copied() {
                self.utterances.remove(&first);
            }
        }
        self.utterances.entry(u).or_default()
    }

    fn withdraw(&mut self, u: u64, reason: WithdrawReason, out: &mut Vec<TranslationEvent>) {
        let (seq, done) = self
            .utterances
            .get(&u)
            .map(|s| (s.next_sequence, s.withdrawn_at))
            .unwrap_or((0, 0));
        if seq == done && reason != WithdrawReason::SourceCancelled {
            return;
        }
        if let Some(s) = self.utterances.get_mut(&u) {
            s.withdrawn_at = seq;
        }
        self.counters.withdrawals += 1;
        out.push(TranslationEvent::Withdrawn {
            stream: self.stream,
            utterance: u,
            through_sequence: seq.checked_sub(1),
            reason,
        });
    }

    fn unavailable(
        &mut self,
        r: &TranscriptRevision,
        reason: UnavailableReason,
        adapters: Vec<String>,
        out: &mut Vec<TranslationEvent>,
    ) {
        self.counters.unavailable += 1;
        match reason {
            UnavailableReason::MalformedOutput => self.counters.malformed += 1,
            UnavailableReason::Timeout => self.counters.timeouts += 1,
            _ => {}
        }
        out.push(TranslationEvent::Unavailable {
            stream: self.stream,
            utterance: r.meta.utterance,
            revision: r.revision,
            revision_status: r.status,
            reason,
            adapters,
        });
        if r.status == RevisionStatus::Final {
            self.withdraw(r.meta.utterance, WithdrawReason::FinalUnavailable, out);
        }
    }

    fn source_language(
        &self,
        r: &TranscriptRevision,
        flags: &mut Vec<UncertaintyFlag>,
    ) -> Option<SourceLanguageRef> {
        let detected = r.language.as_ref().and_then(|l| match &l.decision {
            LanguageDecision::Identified { language } => Some((language.clone(), l.source.clone())),
            LanguageDecision::Abstained { .. } => None,
        });
        match &self.cfg.source {
            SourceLanguagePolicy::Explicit { language } => {
                if detected
                    .as_ref()
                    .is_some_and(|(d, _)| d.primary() != language.primary())
                {
                    flags.push(UncertaintyFlag::SourceLanguageDisagreement);
                }
                Some(SourceLanguageRef {
                    language: language.clone(),
                    provenance: SourceLanguageProvenance::Explicit,
                })
            }
            SourceLanguagePolicy::Detected => {
                detected.map(|(language, source)| SourceLanguageRef {
                    language,
                    provenance: SourceLanguageProvenance::Recognition { source },
                })
            }
        }
    }

    fn hints_for(
        &self,
        utterance: u64,
        text: &str,
        flags: &mut Vec<UncertaintyFlag>,
    ) -> Vec<ContextHint> {
        let c = &self.cfg.context;
        if !c.use_semantic || c.max_hints == 0 {
            return Vec::new();
        }
        let lower = text.to_lowercase();
        let mut out = Vec::new();
        let mut chars = 0u32;
        for (_, h) in self.hints.iter().rev() {
            let distance = utterance.saturating_sub(h.utterance);
            if h.utterance > utterance || distance > u64::from(c.max_utterance_distance) {
                continue;
            }
            let mut h = h.clone();
            h.same_utterance = h.utterance == utterance;
            if h.same_utterance && !lower.contains(&h.mention.to_lowercase()) {
                // The hint disagrees with the source text: never let it
                // override what was said.
                flags.push(UncertaintyFlag::ContextConflict);
                continue;
            }
            let n = h.mention.chars().count() as u32;
            if out.len() as u32 >= c.max_hints || chars + n > c.max_hint_chars {
                break;
            }
            chars += n;
            out.push(h);
        }
        out
    }

    /// Process one transcript revision.
    pub fn process(
        &mut self,
        r: &TranscriptRevision,
        queued_ms: f32,
        out: &mut Vec<TranslationEvent>,
    ) -> Result<(), CeError> {
        if self.ended.is_some() {
            return Ok(());
        }
        let u = r.meta.utterance;
        let state = self.utt(u).clone();
        let reject = if state.finished {
            Some(TranslationRejectReason::AfterFinal)
        } else {
            match state.last_revision {
                Some(l) if r.revision < l => Some(TranslationRejectReason::Stale),
                Some(l) if r.revision == l => Some(TranslationRejectReason::Duplicate),
                _ => None,
            }
        };
        if let Some(reason) = reject {
            self.counters.rejected += 1;
            out.push(TranslationEvent::InputRejected {
                stream: self.stream,
                utterance: u,
                revision: r.revision,
                reason,
            });
            return Ok(());
        }
        {
            let s = self.utt(u);
            s.last_revision = Some(r.revision);
            s.first_input.get_or_insert_with(Instant::now);
        }
        self.counters.inputs += 1;
        self.queue_ms.record(queued_ms);
        let (text, state_out) = match r.status {
            RevisionStatus::Cancelled => {
                self.utt(u).finished = true;
                self.withdraw(u, WithdrawReason::SourceCancelled, out);
                return Ok(());
            }
            RevisionStatus::Final => (r.text.committed.clone(), TranslationState::Final),
            RevisionStatus::Partial => match self.cfg.partials {
                PartialPolicy::Off => return Ok(()),
                PartialPolicy::Committed => {
                    let n = r.text.committed.chars().count() as u32;
                    if n == 0 || n <= self.utt(u).stable_chars {
                        return Ok(());
                    }
                    self.utt(u).stable_chars = n;
                    (r.text.committed.clone(), TranslationState::Stable)
                }
                PartialPolicy::Full => (r.text.full(), TranslationState::Provisional),
            },
        };
        // Mark the utterance finished only once its final was handled
        // (translated or reported unavailable): if translation fails the
        // processor (error limit, panic), `fail` must still withdraw the
        // utterance's earlier results.
        self.translate(r, &text, state_out, queued_ms, out)?;
        if r.status == RevisionStatus::Final {
            self.utt(u).finished = true;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn translate(
        &mut self,
        r: &TranscriptRevision,
        text: &str,
        state: TranslationState,
        queued_ms: f32,
        out: &mut Vec<TranslationEvent>,
    ) -> Result<(), CeError> {
        let u = r.meta.utterance;
        if text.trim().is_empty() {
            self.unavailable(r, UnavailableReason::EmptySource, vec![], out);
            return Ok(());
        }
        if text.chars().count() as u32 > self.cfg.budget.max_input_chars {
            self.unavailable(r, UnavailableReason::InputTooLong, vec![], out);
            return Ok(());
        }
        let mut flags = Vec::new();
        if state != TranslationState::Final {
            flags.push(UncertaintyFlag::ProvisionalSource);
        }
        let source = self.source_language(r, &mut flags);
        if let Some(s) = &source {
            if s.language.primary() == self.target.primary() {
                self.unavailable(r, UnavailableReason::SameLanguage, vec![], out);
                return Ok(());
            }
        }
        let hints_all = self.hints_for(u, text, &mut flags);
        let terms: Vec<Term> = self
            .terminology
            .as_ref()
            .map(|p| p.terms.clone())
            .unwrap_or_default();
        let target = self.target.clone();
        let deadline = std::time::Duration::from_millis(self.cfg.budget.deadline_ms);
        let mut tried = Vec::new();
        let mut last_reason = UnavailableReason::UnsupportedPair;
        for idx in 0..self.adapters.len() {
            let caps = self.adapters[idx].capabilities.clone();
            tried.push(caps.adapter.clone());
            // Pair support (when the source is known), else detection.
            match &source {
                Some(s) if !caps.supports(&s.language, &target) => {
                    last_reason = UnavailableReason::UnsupportedPair;
                    continue;
                }
                None if !caps.detects_source => {
                    last_reason = UnavailableReason::UncertainSource;
                    continue;
                }
                _ => {}
            }
            if text.chars().count() as u32 > caps.max_input_chars {
                last_reason = UnavailableReason::InputTooLong;
                continue;
            }
            let protected = if caps.placeholders {
                fidelity::protect(text, &terms)
            } else {
                fidelity::Protected {
                    text: text.to_string(),
                    spans: Vec::new(),
                    terms: 0,
                }
            };
            let hints: &[ContextHint] = if caps.context_hints { &hints_all } else { &[] };
            let req_terms: &[Term] = if caps.terminology { &terms } else { &[] };
            let request = TranslationRequest {
                text: &protected.text,
                source: source.as_ref().map(|s| &s.language),
                target: &target,
                partial: state != TranslationState::Final,
                hints,
                terms: req_terms,
                max_output_chars: self.cfg.budget.max_output_chars,
            };
            let mut attempt = 0;
            let (outcome, elapsed) = loop {
                let t0 = Instant::now();
                let res = self.adapters[idx].adapter.translate(&request);
                let el = t0.elapsed();
                match res {
                    Err(e) if attempt < self.cfg.budget.max_retries => {
                        attempt += 1;
                        let _ = e;
                        continue;
                    }
                    other => break (other, el),
                }
            };
            let ms = elapsed.as_secs_f32() * 1_000.0;
            self.adapter_ms.record(ms);
            let outcome = match outcome {
                Err(_) => {
                    self.counters.adapter_errors += 1;
                    self.consecutive_errors += 1;
                    out.push(TranslationEvent::Degraded {
                        stream: self.stream,
                        reason: TranslationDegrade::AdapterError,
                    });
                    if self.consecutive_errors >= self.cfg.budget.max_consecutive_errors {
                        return Err(CeError::new(
                            ErrorCategory::Provider,
                            "translation.adapter_failing",
                            "the translation adapter failed repeatedly",
                        ));
                    }
                    self.unavailable(r, UnavailableReason::AdapterError, tried, out);
                    return Ok(());
                }
                Ok(o) => o,
            };
            if elapsed > deadline {
                // A late result is never presented: it may already be stale.
                out.push(TranslationEvent::Degraded {
                    stream: self.stream,
                    reason: TranslationDegrade::DeadlineExceeded,
                });
                self.unavailable(r, UnavailableReason::Timeout, tried, out);
                return Ok(());
            }
            let (raw, score, detected) = match outcome {
                AdapterOutcome::Translated {
                    text,
                    score,
                    detected_source,
                } => (text, score, detected_source),
                AdapterOutcome::NoCoverage => {
                    last_reason = UnavailableReason::NoCoverage;
                    continue;
                }
                AdapterOutcome::UnsupportedPair => {
                    last_reason = UnavailableReason::UnsupportedPair;
                    continue;
                }
            };
            self.consecutive_errors = 0;
            // Validate the output.
            if raw.chars().count() as u32 > self.cfg.budget.max_output_chars {
                self.unavailable(r, UnavailableReason::OutputTooLong, tried, out);
                return Ok(());
            }
            if raw.trim().is_empty()
                || raw
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t')
            {
                self.unavailable(r, UnavailableReason::MalformedOutput, tried, out);
                return Ok(());
            }
            let restored = fidelity::restore(&raw, &protected.spans);
            if restored.foreign {
                self.unavailable(r, UnavailableReason::MalformedOutput, tried, out);
                return Ok(());
            }
            if restored.lost {
                flags.push(UncertaintyFlag::ProtectedSpanLost);
            }
            let source_ref = match (source.clone(), detected) {
                (Some(s), _) => s,
                (None, Some(d)) => {
                    flags.push(UncertaintyFlag::DetectedSourceLanguage);
                    if d.primary() == target.primary() {
                        self.unavailable(r, UnavailableReason::SameLanguage, tried, out);
                        return Ok(());
                    }
                    SourceLanguageRef {
                        language: d,
                        provenance: SourceLanguageProvenance::AdapterDetected,
                    }
                }
                (None, None) => {
                    // The adapter claims detection but did not say what it
                    // detected: provenance would be fabricated.
                    self.unavailable(r, UnavailableReason::MalformedOutput, tried, out);
                    return Ok(());
                }
            };
            if fidelity::numbers_missing(text, &restored.text) {
                flags.push(UncertaintyFlag::NumberMismatch);
            }
            let (applied, missing) = fidelity::terms_missing(text, &restored.text, &terms);
            if missing {
                flags.push(UncertaintyFlag::TermMissing);
            }
            let score = match (score, caps.score_semantics) {
                (Some(v), Some(semantics)) if v.is_finite() => Some(TranslationScore {
                    value: v,
                    semantics,
                }),
                _ => None,
            };
            flags.sort();
            flags.dedup();
            let s = self.utt(u);
            let sequence = s.next_sequence;
            s.next_sequence += 1;
            let first = sequence == 0;
            let since_first = s.first_input.map(|t| t.elapsed().as_secs_f32() * 1_000.0);
            if first {
                if let Some(v) = since_first {
                    self.first_ms.record(v);
                }
            }
            self.counters.results += 1;
            match state {
                TranslationState::Final => self.counters.finals += 1,
                TranslationState::Stable => self.counters.stable += 1,
                TranslationState::Provisional => self.counters.provisional += 1,
            }
            if idx > 0 {
                self.counters.fallbacks += 1;
            }
            if !flags.is_empty() {
                self.counters.flagged += 1;
            }
            out.push(TranslationEvent::Result(Box::new(TranslationResult {
                sequence,
                state,
                source: TranslationSource {
                    stream: self.stream,
                    utterance: u,
                    revision: r.revision,
                    revision_status: r.status,
                    chars: text.chars().count() as u32,
                    frame_range: r.meta.frame_range,
                    time_range: r.meta.time_range,
                },
                source_language: source_ref,
                target: target.clone(),
                text: restored.text,
                flags,
                score,
                provenance: TranslationProvenance {
                    adapter: caps.adapter.clone(),
                    version: caps.version.clone(),
                    model_fingerprint: caps.model.as_ref().and_then(|m| m.fingerprint.clone()),
                    config_fingerprint: self.fingerprint.clone(),
                    locality: caps.locality,
                    terminology: self.term_ref.clone(),
                    terms_applied: (if caps.terminology { applied } else { 0 }) + protected.terms,
                    context_hints: hints.len() as u32,
                    protected_spans: protected.spans.len() as u32,
                    fallback_from: (idx > 0).then(|| self.adapters[0].capabilities.adapter.clone()),
                },
                timing: TranslationTiming {
                    queued_ms,
                    adapter_ms: ms,
                },
            })));
            return Ok(());
        }
        self.unavailable(r, last_reason, tried, out);
        Ok(())
    }

    /// Reset: withdraw current results of unfinished utterances, forget
    /// hints and revision state, reset adapters.
    pub fn reset(&mut self, out: &mut Vec<TranslationEvent>) {
        let open: Vec<u64> = self
            .utterances
            .iter()
            .filter(|(_, s)| !s.finished)
            .map(|(k, _)| *k)
            .collect();
        for u in open {
            self.withdraw(u, WithdrawReason::Reset, out);
        }
        // Forget revision state but keep each utterance's sequence counter,
        // so later results never reuse a number a consumer has already seen
        // or had withdrawn.
        for s in self.utterances.values_mut() {
            s.last_revision = None;
            s.stable_chars = 0;
            s.first_input = None;
        }
        self.hints.clear();
        for a in &mut self.adapters {
            a.adapter.reset();
        }
    }

    /// End: withdraw results of utterances whose final was never
    /// translated, then `Stopped`.
    pub fn finish(&mut self, reason: TranslationEndReason, out: &mut Vec<TranslationEvent>) {
        if self.ended.is_some() {
            return;
        }
        let open: Vec<u64> = self
            .utterances
            .iter()
            .filter(|(_, s)| !s.finished)
            .map(|(k, _)| *k)
            .collect();
        for u in open {
            self.withdraw(u, WithdrawReason::Stopped, out);
        }
        self.ended = Some(reason);
        out.push(TranslationEvent::Stopped {
            stream: self.stream,
            reason,
            counters: Box::new(self.status().counters),
        });
    }

    /// Fail: `Failed`, then finish with `Failed`.
    pub fn fail(&mut self, error: &CeError, out: &mut Vec<TranslationEvent>) {
        if self.ended.is_some() {
            return;
        }
        out.push(TranslationEvent::Failed {
            stream: self.stream,
            error: report(error),
        });
        self.finish(TranslationEndReason::Failed, out);
    }
}

fn report(e: &CeError) -> ErrorReport {
    e.report(&RedactionPolicy::DEFAULT)
}
