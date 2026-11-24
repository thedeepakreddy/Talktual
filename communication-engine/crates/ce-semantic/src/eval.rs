//! Replay evaluation of semantic processing.
//!
//! A corpus is JSON Lines of [`Dialogue`]s. Each dialogue has utterances with
//! gold entities, references and an act label (or `null`: abstention
//! expected). The runner replays every utterance through a fresh
//! [`SemanticProcessor`] per dialogue:
//! * explicit partial revisions, or synthesized ones (word prefixes,
//!   committed text one word behind);
//! * then the final.
//!
//! Scoring uses each utterance's **final** observations as held at the end
//! of the dialogue, after any retraction cascades.
//!
//! **Span policy.**
//! * *Exact*: same category, same character span.
//! * *Overlap*: same category, overlapping span.
//! * *Normalized*: exact, and the normalized value equals the gold value
//!   (only gold entities with a normalized value).
//! * A gold entity with `mention: null` names a value the speaker said that
//!   the recognizer lost (real-ASR subset). It is always a false negative.
//!
//! Nothing here is calibrated or generalizable beyond the corpus scored.

use std::collections::BTreeMap;

use ce_core::ids::AudioStreamId;
use ce_core::semantic::{
    ActDecision, Certainty, DomainProfile, NormalizedValue, ObservationPayload, ObservationStatus,
    Resolution, SemanticConfig, SemanticEvent, SemanticInput, SemanticObservation, TextSpan,
};
use ce_core::speech::{Percentiles, RevisionStatus};
use ce_core::time::MonotonicTime;
use serde::{Deserialize, Serialize};

use crate::adapter::SemanticFactory;
use crate::metrics::Reservoir;
use crate::processor::{ProcessorSetup, SemanticProcessor};
use crate::rules::ACT_LABELS;

/// A gold entity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldEntity {
    /// Category.
    pub category: String,
    /// Mention as in the text; `None`: lost to recognition errors.
    pub mention: Option<String>,
    /// Which occurrence of `mention` (0-based).
    #[serde(default)]
    pub occurrence: usize,
    /// Expected normalized value, if annotated.
    #[serde(default)]
    pub normalized: Option<NormalizedValue>,
}

/// Expected resolution outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GoldOutcome {
    /// One antecedent (`target`).
    Resolved,
    /// Several equally plausible antecedents.
    Ambiguous,
    /// No annotated entity is the antecedent.
    Unresolved,
}

/// A gold referring expression.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldRef {
    /// Mention.
    pub mention: String,
    /// Occurrence (0-based).
    #[serde(default)]
    pub occurrence: usize,
    /// Expected outcome.
    pub outcome: GoldOutcome,
    /// Antecedent mention (an earlier gold entity) when resolved.
    #[serde(default)]
    pub target: Option<String>,
}

/// One gold utterance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldUtterance {
    /// Final text.
    pub text: String,
    /// Explicit partial revisions `(committed, provisional)`; `None`:
    /// synthesized from word prefixes.
    #[serde(default)]
    pub revisions: Option<Vec<(String, String)>>,
    /// Entities.
    #[serde(default)]
    pub entities: Vec<GoldEntity>,
    /// Referring expressions.
    #[serde(default)]
    pub references: Vec<GoldRef>,
    /// Act label, or `None` when abstention is expected.
    pub act: Option<String>,
    /// Free-text annotation note.
    #[serde(default)]
    pub note: Option<String>,
}

/// One dialogue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dialogue {
    /// Id.
    pub id: String,
    /// `dev`, `test` or `asr`.
    pub split: String,
    /// `authored` (written for CE) or `asr-real` (real recognizer output
    /// of synthetic speech).
    pub source: String,
    /// Profile id from the corpus profile set, if any.
    #[serde(default)]
    pub profile: Option<String>,
    /// Utterances.
    pub utterances: Vec<GoldUtterance>,
}

/// Parse JSON Lines (blank lines and `#` comments ignored).
pub fn parse_corpus(text: &str) -> Result<Vec<Dialogue>, String> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(|(i, l)| serde_json::from_str(l).map_err(|e| format!("line {}: {e}", i + 1)))
        .collect()
}

/// FNV-1a 64 of bytes (identity, not security), 16 hex digits.
pub fn fnv64(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Precision / recall / F1 counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Prf {
    /// True positives.
    pub tp: u32,
    /// False positives.
    pub fp: u32,
    /// False negatives.
    pub fn_: u32,
    /// Precision (0 when undefined).
    pub precision: f64,
    /// Recall (0 when undefined).
    pub recall: f64,
    /// F1 (0 when undefined).
    pub f1: f64,
}

impl Prf {
    fn finish(mut self) -> Self {
        let p = if self.tp + self.fp > 0 {
            f64::from(self.tp) / f64::from(self.tp + self.fp)
        } else {
            0.0
        };
        let r = if self.tp + self.fn_ > 0 {
            f64::from(self.tp) / f64::from(self.tp + self.fn_)
        } else {
            0.0
        };
        self.precision = p;
        self.recall = r;
        self.f1 = if p + r > 0.0 {
            2.0 * p * r / (p + r)
        } else {
            0.0
        };
        self
    }
}

/// Entity results at one certainty threshold.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct EntityScores {
    /// Minimum certainty of predictions counted.
    pub min_certainty: String,
    /// Exact span + category, micro over all categories.
    pub exact: Prf,
    /// Overlapping span + category.
    pub overlap: Prf,
    /// Per category (exact).
    pub per_category: BTreeMap<String, Prf>,
    /// Exact matches whose gold has a normalized value.
    pub normalized_checked: u32,
    /// … of which the normalized value is equal.
    pub normalized_correct: u32,
    /// Gold entities lost to recognition errors (always FN).
    pub lost_to_asr: u32,
}

/// Reference results.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ReferenceScores {
    /// Gold referring expressions.
    pub gold: u32,
    /// Detected (a predicted reference at the gold span).
    pub detected: u32,
    /// Outcome and (for resolved) target correct.
    pub correct: u32,
    /// `correct / gold`.
    pub accuracy: f64,
    /// Predicted references not in the gold set.
    pub spurious: u32,
    /// Predicted outcomes: resolved / ambiguous / unresolved (detected only).
    pub predicted_resolved: u32,
    /// Ambiguous.
    pub predicted_ambiguous: u32,
    /// Unresolved.
    pub predicted_unresolved: u32,
    /// Resolved with the correct target / predicted resolved.
    pub resolution_precision: f64,
    /// (predicted ambiguous + unresolved) / detected.
    pub abstention_rate: f64,
    /// Confusion gold outcome → predicted outcome (`missed` if undetected,
    /// `resolved_wrong_target` if resolved to another item).
    pub confusion: BTreeMap<String, BTreeMap<String, u32>>,
}

/// Act results.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ActScores {
    /// Utterances.
    pub utterances: u32,
    /// Per label.
    pub per_label: BTreeMap<String, Prf>,
    /// Macro F1 over labels with gold or predicted support.
    pub macro_f1: f64,
    /// Labeled predictions / utterances.
    pub coverage: f64,
    /// Correct labels / labeled predictions.
    pub selective_accuracy: f64,
    /// Gold abstentions predicted as abstentions / gold abstentions.
    pub abstention_recall: f64,
    /// Gold abstentions.
    pub gold_abstentions: u32,
    /// Confusion gold → predicted (`abstain` for none).
    pub confusion: BTreeMap<String, BTreeMap<String, u32>>,
}

/// Streaming and lifecycle results (synthetic timeline: one revision per
/// 300 ms of audio).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct StreamingScores {
    /// Inputs processed (partials + finals).
    pub inputs: u64,
    /// Partial inputs.
    pub partial_inputs: u64,
    /// Observation events by change kind.
    pub changes: BTreeMap<String, u64>,
    /// Observation events per input.
    pub revision_rate: f64,
    /// Observations retracted before their utterance finalized (they were
    /// only ever provisional or stable): speculative output withdrawn.
    pub retracted_before_final: u64,
    /// Final observations whose first appearance was in a partial.
    pub finals_seen_early: u64,
    /// Final observations.
    pub finals: u64,
    /// Synthetic audio time to an utterance's first non-act observation, ms.
    pub first_observation_ms: Percentiles,
    /// Processor time per input (adapter + contract checks + diff +
    /// context) on this machine, ms.
    pub processing_ms: Percentiles,
}

/// Robustness checks.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct RobustnessScores {
    /// Dialogues replayed with duplicated and stale (out-of-order) partials.
    pub disorder_dialogues: u32,
    /// … whose final observations equal the clean replay.
    pub disorder_consistent: u32,
    /// Stale/duplicate inputs rejected in those replays.
    pub disorder_rejected: u64,
    /// Dialogues replayed with finals only (partials off).
    pub finals_only_dialogues: u32,
    /// … whose final observations equal the clean replay.
    pub finals_only_consistent: u32,
}

/// One split's report.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SplitReport {
    /// Split.
    pub split: String,
    /// Dialogues.
    pub dialogues: u32,
    /// Utterances.
    pub utterances: u32,
    /// Entities, all certainties.
    pub entities_all: EntityScores,
    /// Entities, Medium or better.
    pub entities_medium: EntityScores,
    /// References.
    pub references: ReferenceScores,
    /// Acts.
    pub acts: ActScores,
    /// Streaming.
    pub streaming: StreamingScores,
    /// Robustness.
    pub robustness: RobustnessScores,
    /// Errors for analysis: `(dialogue, utterance, kind, detail)`; detail
    /// contains corpus text (authored or synthetic), never user content.
    pub errors: Vec<(String, usize, String, String)>,
}

fn nth_span(text: &str, mention: &str, n: usize) -> Option<TextSpan> {
    let chars: Vec<char> = text.chars().collect();
    let m: Vec<char> = mention.chars().collect();
    if m.is_empty() || m.len() > chars.len() {
        return None;
    }
    let mut seen = 0;
    for i in 0..=chars.len() - m.len() {
        let boundary_l = i == 0 || chars[i - 1] == ' ';
        let boundary_r = i + m.len() == chars.len() || chars[i + m.len()] == ' ';
        if chars[i..i + m.len()] == m[..] && boundary_l && boundary_r {
            if seen == n {
                return Some(TextSpan {
                    start: i as u32,
                    end: (i + m.len()) as u32,
                });
            }
            seen += 1;
        }
    }
    None
}

fn input(
    stream: AudioStreamId,
    utt: u64,
    rev: u32,
    status: RevisionStatus,
    committed: &str,
    provisional: &str,
) -> SemanticInput {
    let text = match (committed.is_empty(), provisional.is_empty()) {
        (_, true) => committed.to_string(),
        (true, false) => provisional.to_string(),
        _ => format!("{committed} {provisional}"),
    };
    let end_ms = u64::from(rev + 1) * 300 + utt * 10_000;
    SemanticInput {
        stream,
        utterance: utt,
        revision: rev,
        status,
        committed_chars: if status == RevisionStatus::Partial {
            committed.chars().count() as u32
        } else {
            text.chars().count() as u32
        },
        text,
        language: None,
        recognition_score: None,
        time_range: (
            MonotonicTime::from_nanos(utt * 10_000 * 1_000_000),
            MonotonicTime::from_nanos(end_ms * 1_000_000),
        ),
        audio_ms: u64::from(rev + 1) * 300,
    }
}

/// The partial revisions replayed before an utterance's final.
pub fn partials(u: &GoldUtterance) -> Vec<(String, String)> {
    if let Some(r) = &u.revisions {
        return r.clone();
    }
    let words: Vec<&str> = u.text.split_whitespace().collect();
    (1..words.len())
        .map(|k| {
            let c = k.saturating_sub(1);
            (words[..c].join(" "), words[c..k].join(" "))
        })
        .collect()
}

/// How a dialogue is delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// Partials in order, then the final.
    Clean,
    /// Each partial twice, and the previous partial again after each one
    /// (stale), then the final twice.
    Disordered,
    /// Finals only.
    FinalsOnly,
}

/// Result of replaying one dialogue.
#[derive(Debug, Default)]
pub struct Replay {
    /// Every event.
    pub events: Vec<SemanticEvent>,
    /// Final observations per utterance index at the end of the dialogue.
    pub finals: Vec<Vec<SemanticObservation>>,
    /// Counters snapshot.
    pub counters: ce_core::semantic::SemanticCounters,
    /// Processor time per input (wall clock, this machine), ms.
    pub input_ms: Vec<f32>,
}

/// Replay one dialogue with a fresh processor.
pub fn replay(
    factory: &dyn SemanticFactory,
    base: &SemanticConfig,
    profiles: &BTreeMap<String, DomainProfile>,
    d: &Dialogue,
    delivery: Delivery,
) -> Result<Replay, String> {
    let mut config = base.clone();
    if let Some(p) = &d.profile {
        config.profile = Some(
            profiles
                .get(p)
                .cloned()
                .ok_or_else(|| format!("{}: unknown profile {p}", d.id))?,
        );
    }
    let stream = AudioStreamId::from_uuid(uuid::Uuid::from_u128(u128::from(
        u64::from_str_radix(&fnv64(d.id.as_bytes()), 16).unwrap_or(1),
    )));
    let adapter = factory.create(&config).map_err(|e| e.to_string())?;
    let mut p = SemanticProcessor::new(
        ProcessorSetup {
            stream,
            config,
            capabilities: factory.capabilities(),
        },
        adapter,
    )
    .map_err(|e| e.to_string())?;
    let mut events = Vec::new();
    let mut input_ms = Vec::new();
    p.start(&mut events);
    for (ui, u) in d.utterances.iter().enumerate() {
        let utt = ui as u64 + 1;
        let parts = partials(u);
        let mut seq: Vec<SemanticInput> = Vec::new();
        if delivery != Delivery::FinalsOnly {
            for (r, (c, pv)) in parts.iter().enumerate() {
                let i = input(stream, utt, r as u32, RevisionStatus::Partial, c, pv);
                seq.push(i.clone());
                if delivery == Delivery::Disordered {
                    seq.push(i);
                    if r > 0 {
                        let (c0, p0) = &parts[r - 1];
                        seq.push(input(
                            stream,
                            utt,
                            r as u32 - 1,
                            RevisionStatus::Partial,
                            c0,
                            p0,
                        ));
                    }
                }
            }
        }
        let fin = input(
            stream,
            utt,
            parts.len() as u32,
            RevisionStatus::Final,
            &u.text,
            "",
        );
        seq.push(fin.clone());
        if delivery == Delivery::Disordered {
            seq.push(fin);
        }
        for i in seq {
            let t0 = std::time::Instant::now();
            p.process(i, 0.0, &mut events).map_err(|e| e.to_string())?;
            input_ms.push(t0.elapsed().as_secs_f32() * 1_000.0);
        }
    }
    let mut finals = vec![Vec::new(); d.utterances.len()];
    for o in p.current() {
        if o.status == ObservationStatus::Final {
            let ui = o.source.utterance as usize - 1;
            if ui < finals.len() {
                finals[ui].push(o.clone());
            }
        }
    }
    for f in &mut finals {
        f.sort_by(|a, b| {
            a.source
                .span
                .start
                .cmp(&b.source.span.start)
                .then(a.id.cmp(&b.id))
        });
    }
    let counters = p.counters();
    Ok(Replay {
        events,
        finals,
        counters,
        input_ms,
    })
}

fn fingerprint_finals(f: &[Vec<SemanticObservation>]) -> String {
    // Compare meaning, not internal ids (context item and relation ids
    // depend on the delivery path).
    let v: Vec<Vec<String>> = f
        .iter()
        .map(|u| {
            u.iter()
                .map(|o| {
                    let what = match &o.payload {
                        ObservationPayload::Entity(e) => format!(
                            "{}|{}|{:?}|{:?}",
                            e.category, e.mention, e.normalized, e.attributes
                        ),
                        ObservationPayload::Reference(r) => {
                            let target = match &r.resolution {
                                Resolution::Resolved { target } => r
                                    .candidates
                                    .iter()
                                    .find(|c| &c.item == target)
                                    .map(|c| c.mention.clone())
                                    .unwrap_or_default(),
                                other => format!("{other:?}"),
                            };
                            format!("{}|{:?}|{}", r.mention, r.kind, target)
                        }
                        ObservationPayload::Act(a) => {
                            format!("{:?}|{:?}", a.decision, a.alternatives)
                        }
                        ObservationPayload::Relation(r) => r.kind.clone(),
                    };
                    format!(
                        "{}:{}-{}:{}:{:?}",
                        o.payload.kind(),
                        o.source.span.start,
                        o.source.span.end,
                        what,
                        o.uncertainty.level
                    )
                })
                .collect()
        })
        .collect();
    serde_json::to_string(&v).unwrap_or_default()
}

fn entity_scores(
    dialogues: &[Dialogue],
    replays: &[Replay],
    min: Certainty,
    errors: &mut Vec<(String, usize, String, String)>,
    record: bool,
) -> EntityScores {
    let mut s = EntityScores {
        min_certainty: format!("{min:?}").to_lowercase(),
        ..EntityScores::default()
    };
    let mut exact = Prf::default();
    let mut overlap = Prf::default();
    let mut per: BTreeMap<String, Prf> = BTreeMap::new();
    for (d, r) in dialogues.iter().zip(replays) {
        for (ui, u) in d.utterances.iter().enumerate() {
            let preds: Vec<(String, TextSpan, Option<NormalizedValue>, String)> = r.finals[ui]
                .iter()
                .filter(|o| o.uncertainty.level >= min)
                .filter_map(|o| match &o.payload {
                    ObservationPayload::Entity(e) => Some((
                        e.category.clone(),
                        o.source.span,
                        e.normalized.clone(),
                        e.mention.clone(),
                    )),
                    _ => None,
                })
                .collect();
            let mut used = vec![false; preds.len()];
            let mut used_overlap = vec![false; preds.len()];
            for g in &u.entities {
                let Some(m) = &g.mention else {
                    s.lost_to_asr += 1;
                    exact.fn_ += 1;
                    overlap.fn_ += 1;
                    per.entry(g.category.clone()).or_default().fn_ += 1;
                    continue;
                };
                let Some(span) = nth_span(&u.text, m, g.occurrence) else {
                    if record {
                        errors.push((
                            d.id.clone(),
                            ui,
                            "gold_error".into(),
                            format!("mention {m:?} not in text"),
                        ));
                    }
                    continue;
                };
                let hit = preds
                    .iter()
                    .enumerate()
                    .position(|(i, p)| !used[i] && p.0 == g.category && p.1 == span);
                match hit {
                    Some(i) => {
                        used[i] = true;
                        exact.tp += 1;
                        per.entry(g.category.clone()).or_default().tp += 1;
                        if let Some(n) = &g.normalized {
                            s.normalized_checked += 1;
                            if preds[i].2.as_ref() == Some(n) {
                                s.normalized_correct += 1;
                            } else if record {
                                errors.push((
                                    d.id.clone(),
                                    ui,
                                    "entity_normalized".into(),
                                    format!(
                                        "{} {m:?}: expected {n:?}, got {:?}",
                                        g.category, preds[i].2
                                    ),
                                ));
                            }
                        }
                    }
                    None => {
                        exact.fn_ += 1;
                        per.entry(g.category.clone()).or_default().fn_ += 1;
                        if record {
                            errors.push((
                                d.id.clone(),
                                ui,
                                "entity_missed".into(),
                                format!("{} {m:?}", g.category),
                            ));
                        }
                    }
                }
                let ov = preds.iter().enumerate().position(|(i, p)| {
                    !used_overlap[i]
                        && p.0 == g.category
                        && p.1.start < span.end
                        && span.start < p.1.end
                });
                match ov {
                    Some(i) => {
                        used_overlap[i] = true;
                        overlap.tp += 1;
                    }
                    None => overlap.fn_ += 1,
                }
            }
            for (i, p) in preds.iter().enumerate() {
                if !used[i] {
                    exact.fp += 1;
                    per.entry(p.0.clone()).or_default().fp += 1;
                    if record {
                        errors.push((
                            d.id.clone(),
                            ui,
                            "entity_spurious".into(),
                            format!("{} {:?}", p.0, p.3),
                        ));
                    }
                }
                if !used_overlap[i] {
                    overlap.fp += 1;
                }
            }
        }
    }
    s.exact = exact.finish();
    s.overlap = overlap.finish();
    s.per_category = per.into_iter().map(|(k, v)| (k, v.finish())).collect();
    s
}

fn bump(m: &mut BTreeMap<String, BTreeMap<String, u32>>, a: &str, b: &str) {
    *m.entry(a.to_string())
        .or_default()
        .entry(b.to_string())
        .or_default() += 1;
}

fn reference_scores(
    dialogues: &[Dialogue],
    replays: &[Replay],
    errors: &mut Vec<(String, usize, String, String)>,
) -> ReferenceScores {
    let mut s = ReferenceScores::default();
    let mut resolved_right = 0u32;
    for (d, r) in dialogues.iter().zip(replays) {
        for (ui, u) in d.utterances.iter().enumerate() {
            let preds: Vec<(TextSpan, &ce_core::semantic::ReferenceResolution)> = r.finals[ui]
                .iter()
                .filter_map(|o| match &o.payload {
                    ObservationPayload::Reference(x) => Some((o.source.span, x)),
                    _ => None,
                })
                .collect();
            let mut used = vec![false; preds.len()];
            for g in &u.references {
                s.gold += 1;
                let gname = format!("{:?}", g.outcome).to_lowercase();
                let Some(span) = nth_span(&u.text, &g.mention, g.occurrence) else {
                    errors.push((
                        d.id.clone(),
                        ui,
                        "gold_error".into(),
                        format!("reference {:?} not in text", g.mention),
                    ));
                    continue;
                };
                let hit = preds.iter().position(|(sp, _)| sp.start == span.start);
                let Some(i) = hit else {
                    bump(&mut s.confusion, &gname, "missed");
                    errors.push((
                        d.id.clone(),
                        ui,
                        "reference_missed".into(),
                        g.mention.clone(),
                    ));
                    continue;
                };
                used[i] = true;
                s.detected += 1;
                let pr = preds[i].1;
                let (pname, ok) = match &pr.resolution {
                    Resolution::Resolved { target } => {
                        s.predicted_resolved += 1;
                        let tm = pr
                            .candidates
                            .iter()
                            .find(|c| &c.item == target)
                            .map(|c| c.mention.clone());
                        let right =
                            g.outcome == GoldOutcome::Resolved && tm.is_some() && tm == g.target;
                        if right {
                            resolved_right += 1;
                        }
                        let name = if g.outcome == GoldOutcome::Resolved && !right {
                            "resolved_wrong_target"
                        } else {
                            "resolved"
                        };
                        if !right {
                            errors.push((
                                d.id.clone(),
                                ui,
                                "reference_wrong".into(),
                                format!(
                                    "{} → {:?} (gold {:?} {:?})",
                                    g.mention, tm, g.outcome, g.target
                                ),
                            ));
                        }
                        (name, right)
                    }
                    Resolution::Ambiguous => {
                        s.predicted_ambiguous += 1;
                        ("ambiguous", g.outcome == GoldOutcome::Ambiguous)
                    }
                    Resolution::Unresolved { .. } => {
                        s.predicted_unresolved += 1;
                        ("unresolved", g.outcome == GoldOutcome::Unresolved)
                    }
                };
                if ok {
                    s.correct += 1;
                } else if pname != "resolved" && pname != "resolved_wrong_target" {
                    errors.push((
                        d.id.clone(),
                        ui,
                        "reference_outcome".into(),
                        format!("{}: {pname} (gold {gname} {:?})", g.mention, g.target),
                    ));
                }
                bump(&mut s.confusion, &gname, pname);
            }
            s.spurious += used.iter().filter(|u| !**u).count() as u32;
            for (i, (_, p)) in preds.iter().enumerate() {
                if !used[i] {
                    errors.push((
                        d.id.clone(),
                        ui,
                        "reference_spurious".into(),
                        p.mention.clone(),
                    ));
                }
            }
        }
    }
    s.accuracy = if s.gold > 0 {
        f64::from(s.correct) / f64::from(s.gold)
    } else {
        0.0
    };
    s.resolution_precision = if s.predicted_resolved > 0 {
        f64::from(resolved_right) / f64::from(s.predicted_resolved)
    } else {
        0.0
    };
    s.abstention_rate = if s.detected > 0 {
        f64::from(s.predicted_ambiguous + s.predicted_unresolved) / f64::from(s.detected)
    } else {
        0.0
    };
    s
}

fn act_scores(
    dialogues: &[Dialogue],
    replays: &[Replay],
    errors: &mut Vec<(String, usize, String, String)>,
) -> ActScores {
    let mut s = ActScores::default();
    let mut per: BTreeMap<String, Prf> = ACT_LABELS
        .iter()
        .map(|l| ((*l).to_string(), Prf::default()))
        .collect();
    let (mut labeled, mut correct, mut abst_ok) = (0u32, 0u32, 0u32);
    for (d, r) in dialogues.iter().zip(replays) {
        for (ui, u) in d.utterances.iter().enumerate() {
            s.utterances += 1;
            let pred = r.finals[ui].iter().find_map(|o| match &o.payload {
                ObservationPayload::Act(a) => Some(match &a.decision {
                    ActDecision::Labeled { label } => Some(label.clone()),
                    ActDecision::Abstained { .. } => None,
                }),
                _ => None,
            });
            let pred = pred.flatten();
            let g = u.act.clone();
            bump(
                &mut s.confusion,
                g.as_deref().unwrap_or("abstain"),
                pred.as_deref().unwrap_or("abstain"),
            );
            if g.is_none() {
                s.gold_abstentions += 1;
                if pred.is_none() {
                    abst_ok += 1;
                }
            }
            if let Some(p) = &pred {
                labeled += 1;
                if Some(p) == g.as_ref() {
                    correct += 1;
                }
            }
            match (&g, &pred) {
                (Some(a), Some(b)) if a == b => per.entry(a.clone()).or_default().tp += 1,
                (a, b) => {
                    if let Some(a) = a {
                        per.entry(a.clone()).or_default().fn_ += 1;
                    }
                    if let Some(b) = b {
                        per.entry(b.clone()).or_default().fp += 1;
                    }
                    if a != b {
                        errors.push((
                            d.id.clone(),
                            ui,
                            "act".into(),
                            format!("{:?}: gold {a:?}, got {b:?}", u.text),
                        ));
                    }
                }
            }
        }
    }
    let per: BTreeMap<String, Prf> = per.into_iter().map(|(k, v)| (k, v.finish())).collect();
    let supported: Vec<&Prf> = per.values().filter(|p| p.tp + p.fp + p.fn_ > 0).collect();
    s.macro_f1 = if supported.is_empty() {
        0.0
    } else {
        supported.iter().map(|p| p.f1).sum::<f64>() / supported.len() as f64
    };
    s.per_label = per;
    s.coverage = if s.utterances > 0 {
        f64::from(labeled) / f64::from(s.utterances)
    } else {
        0.0
    };
    s.selective_accuracy = if labeled > 0 {
        f64::from(correct) / f64::from(labeled)
    } else {
        0.0
    };
    s.abstention_recall = if s.gold_abstentions > 0 {
        f64::from(abst_ok) / f64::from(s.gold_abstentions)
    } else {
        0.0
    };
    s
}

fn streaming_scores(dialogues: &[Dialogue], replays: &[Replay]) -> StreamingScores {
    let mut s = StreamingScores::default();
    let mut first = Reservoir::new(100_000);
    let mut proc = Reservoir::new(100_000);
    for (d, r) in dialogues.iter().zip(replays) {
        s.inputs += r.counters.processed;
        s.partial_inputs += d
            .utterances
            .iter()
            .map(|u| partials(u).len() as u64)
            .sum::<u64>();
        let mut first_seen: BTreeMap<String, ObservationStatus> = BTreeMap::new();
        let mut final_ids: Vec<String> = Vec::new();
        for e in &r.events {
            if let SemanticEvent::Observation {
                change,
                observation,
            } = e
            {
                *s.changes
                    .entry(format!("{change:?}").to_lowercase())
                    .or_default() += 1;
                first_seen
                    .entry(observation.id.0.clone())
                    .or_insert(observation.status);
                match observation.status {
                    ObservationStatus::Final => final_ids.push(observation.id.0.clone()),
                    ObservationStatus::Retracted if !final_ids.contains(&observation.id.0) => {
                        s.retracted_before_final += 1
                    }
                    _ => {}
                }
            }
        }
        s.finals += r.finals.iter().map(|f| f.len() as u64).sum::<u64>();
        for f in r.finals.iter().flatten() {
            if first_seen
                .get(&f.id.0)
                .is_some_and(|st| *st != ObservationStatus::Final)
            {
                s.finals_seen_early += 1;
            }
        }
        let mut seen_utt: Vec<u64> = Vec::new();
        for e in &r.events {
            if let SemanticEvent::Observation { observation: o, .. } = e {
                if !matches!(o.payload, ObservationPayload::Act(_))
                    && !seen_utt.contains(&o.source.utterance)
                {
                    seen_utt.push(o.source.utterance);
                    first.record(((o.source.revision + 1) * 300) as f32);
                }
            }
        }
        for v in &r.input_ms {
            proc.record(*v);
        }
    }
    let total: u64 = s.changes.values().sum();
    s.revision_rate = if s.inputs > 0 {
        total as f64 / s.inputs as f64
    } else {
        0.0
    };
    s.first_observation_ms = first.summary();
    s.processing_ms = proc.summary();
    s
}

/// Evaluate one split.
pub fn evaluate(
    factory: &dyn SemanticFactory,
    base: &SemanticConfig,
    profiles: &BTreeMap<String, DomainProfile>,
    dialogues: &[Dialogue],
    split: &str,
) -> Result<SplitReport, String> {
    let ds: Vec<Dialogue> = dialogues
        .iter()
        .filter(|d| d.split == split)
        .cloned()
        .collect();
    let mut replays = Vec::new();
    let mut rob = RobustnessScores::default();
    for d in &ds {
        let clean = replay(factory, base, profiles, d, Delivery::Clean)?;
        let fp = fingerprint_finals(&clean.finals);
        let dis = replay(factory, base, profiles, d, Delivery::Disordered)?;
        rob.disorder_dialogues += 1;
        rob.disorder_rejected += dis.counters.stale_rejected;
        if fingerprint_finals(&dis.finals) == fp {
            rob.disorder_consistent += 1;
        }
        let fo = replay(factory, base, profiles, d, Delivery::FinalsOnly)?;
        rob.finals_only_dialogues += 1;
        if fingerprint_finals(&fo.finals) == fp {
            rob.finals_only_consistent += 1;
        }
        replays.push(clean);
    }
    let mut errors = Vec::new();
    let entities_all = entity_scores(&ds, &replays, Certainty::Low, &mut errors, true);
    let entities_medium = entity_scores(&ds, &replays, Certainty::Medium, &mut errors, false);
    let references = reference_scores(&ds, &replays, &mut errors);
    let acts = act_scores(&ds, &replays, &mut errors);
    let streaming = streaming_scores(&ds, &replays);
    Ok(SplitReport {
        split: split.to_string(),
        dialogues: ds.len() as u32,
        utterances: ds.iter().map(|d| d.utterances.len() as u32).sum(),
        entities_all,
        entities_medium,
        references,
        acts,
        streaming,
        robustness: rob,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_respect_word_boundaries_and_occurrences() {
        assert_eq!(
            nth_span("ONE ONE TWO", "ONE", 1),
            Some(TextSpan { start: 4, end: 7 })
        );
        assert_eq!(
            nth_span("ONEX ONE", "ONE", 0),
            Some(TextSpan { start: 5, end: 8 })
        );
        assert_eq!(nth_span("A", "B", 0), None);
    }

    #[test]
    fn prf_handles_empty_support() {
        let p = Prf {
            tp: 0,
            fp: 0,
            fn_: 0,
            ..Prf::default()
        }
        .finish();
        assert_eq!((p.precision, p.recall, p.f1), (0.0, 0.0, 0.0));
        let p = Prf {
            tp: 1,
            fp: 1,
            fn_: 0,
            ..Prf::default()
        }
        .finish();
        assert!((p.f1 - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn synthesized_partials_lag_one_word() {
        let u = GoldUtterance {
            text: "A B C".into(),
            revisions: None,
            entities: vec![],
            references: vec![],
            act: None,
            note: None,
        };
        assert_eq!(
            partials(&u),
            vec![(String::new(), "A".to_string()), ("A".into(), "B".into())]
        );
    }
}
