//! Replay, manifests and metrics (Phase 4 evaluation).
//!
//! * [`load_manifest`] / [`load_split`] — the speech fixtures with their
//!   reference transcripts (`fixtures/speech/manifest.json`; the reference
//!   is the exact TTS input text).
//! * [`build`] — a deterministic stream from fixture utterances (the
//!   Phase 3 scenario builder: lead, pauses, tail, acoustic condition,
//!   device format) plus reference word spans for the oracle adapter.
//! * [`replay`] — the Phase 3 analyzer and the recognizer in lock-step over
//!   the scenario, exactly as the runtime wires them (hints forwarded after
//!   each block), with optional block drops and forced load.
//! * [`score`] — WER / CER (normalized), case- and punctuation-sensitive
//!   WER, revision counts / rate / corrections, first-partial,
//!   first-commit and final delays, boundary errors, language decisions
//!   and processing time.
//! * [`load_lid_corpus`] / [`evaluate_lid`] — text language identification
//!   accuracy, coverage, abstention, confusion and mixed-language items.
//!
//! **Evidence labels.** Scenarios are *synthetic* (TTS fixtures with
//! generated noise) and replayed (*replay*). With the reference adapter
//! the transcripts come from an *oracle*: its WER measures what the
//! pipeline (segmentation, finalization, gaps) loses, never recognition
//! accuracy. No real-ASR result exists in Phase 4.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use ce_audio_intel::StreamAnalyzer;
use ce_audio_intel::eval::{self as ieval, Condition, Format, Scenario};
use ce_audio_intel::synth::{self, SYNTH_STREAM};
use ce_core::intel::AnalysisConfig;
use ce_core::speech::{
    LanguageDecision, LanguageHint, LanguageIdConfig, Percentiles, RecognitionConfig,
    RecognitionEndReason, RevisionStatus, SpeechEvent, TranscriptRevision,
};
use ce_core::{CeError, ErrorCategory};
use serde::Deserialize;

use crate::adapter::AsrFactory;
use crate::fake::ReferenceUtterance;
use crate::language::{LanguageIdentifier, observe_utterance};
use crate::metrics::Reservoir;
use crate::recognizer::{RecognizerSetup, StreamRecognizer};

pub use ce_audio_intel::eval::Split;

fn bad(code: &'static str, msg: &'static str) -> CeError {
    CeError::new(ErrorCategory::InvalidInput, code, msg)
}

/// One manifest entry.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ManifestItem {
    /// Id (file stem).
    pub id: String,
    /// File name in `fixtures/speech`.
    pub file: String,
    /// `dev` or `test`.
    pub split: String,
    /// Synthesizer voice.
    pub voice: String,
    /// Tempo factor.
    pub tempo: f32,
    /// Language.
    pub language: String,
    /// Reference transcript (exact TTS input).
    pub reference: String,
}

#[derive(Deserialize)]
struct Manifest {
    schema: String,
    items: Vec<ManifestItem>,
}

/// Load `fixtures/speech/manifest.json`.
pub fn load_manifest() -> Result<Vec<ManifestItem>, CeError> {
    let path = ieval::fixture_dir().join("manifest.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| bad("eval.no_manifest", "speech manifest missing").with_source(e))?;
    let m: Manifest = serde_json::from_str(&text)
        .map_err(|e| bad("eval.bad_manifest", "speech manifest is malformed").with_source(e))?;
    if m.schema != "ce.speech-manifest.v1" {
        return Err(bad("eval.bad_manifest", "unknown manifest schema"));
    }
    Ok(m.items)
}

/// A split's utterances with their manifest entries (sorted by id).
pub fn load_split(split: Split) -> Result<Vec<(ManifestItem, Vec<f32>)>, CeError> {
    let want = match split {
        Split::Dev => "dev",
        Split::Test => "test",
    };
    let mut items: Vec<ManifestItem> = load_manifest()?
        .into_iter()
        .filter(|i| i.split == want)
        .collect();
    items.sort_by(|a, b| a.id.cmp(&b.id));
    items
        .into_iter()
        .map(|i| {
            let (rate, ch, s) = ieval::read_wav(&ieval::fixture_dir().join(&i.file))?;
            if rate != ieval::FIXTURE_RATE || ch != 1 {
                return Err(bad("eval.bad_fixture", "fixtures must be 16 kHz mono"));
            }
            Ok((i, s))
        })
        .collect()
}

// ------------------------------------------------------------------ text

/// Normalize for WER/CER: lowercase, punctuation removed (word-internal
/// apostrophes kept), whitespace collapsed.
pub fn normalize(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            let w: String = w
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '\'')
                .flat_map(char::to_lowercase)
                .collect();
            w.trim_matches('\'').to_owned()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Levenshtein distance over any sequence.
pub fn levenshtein<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, x) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            cur[j + 1] = (prev[j + 1] + 1)
                .min(cur[j] + 1)
                .min(prev[j] + usize::from(x != y));
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Edits over reference length.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ErrorRate {
    /// Edits (substitutions + deletions + insertions).
    pub edits: usize,
    /// Reference length.
    pub reference: usize,
}

impl ErrorRate {
    /// Rate (0 for an empty reference with no edits).
    pub fn rate(&self) -> f32 {
        if self.reference == 0 {
            if self.edits == 0 { 0.0 } else { 1.0 }
        } else {
            self.edits as f32 / self.reference as f32
        }
    }

    fn add(&mut self, o: ErrorRate) {
        self.edits += o.edits;
        self.reference += o.reference;
    }
}

/// Word error rate after [`normalize`].
pub fn wer(reference: &str, hypothesis: &str) -> ErrorRate {
    let (r, h) = (normalize(reference), normalize(hypothesis));
    ErrorRate {
        edits: levenshtein(&r, &h),
        reference: r.len(),
    }
}

/// Character error rate after [`normalize`] (words joined by one space).
pub fn cer(reference: &str, hypothesis: &str) -> ErrorRate {
    let r: Vec<char> = normalize(reference).join(" ").chars().collect();
    let h: Vec<char> = normalize(hypothesis).join(" ").chars().collect();
    ErrorRate {
        edits: levenshtein(&r, &h),
        reference: r.len(),
    }
}

/// Case- and punctuation-sensitive WER over whitespace tokens.
pub fn wer_raw(reference: &str, hypothesis: &str) -> ErrorRate {
    let r: Vec<&str> = reference.split_whitespace().collect();
    let h: Vec<&str> = hypothesis.split_whitespace().collect();
    ErrorRate {
        edits: levenshtein(&r, &h),
        reference: r.len(),
    }
}

// -------------------------------------------------------------- scenarios

/// A speech scenario: the Phase 3 scenario plus references.
#[derive(Clone, Debug)]
pub struct SpeechScenario {
    /// Audio, format, activity labels.
    pub scenario: Scenario,
    /// Reference utterances on the 16 kHz adapter timeline (oracle input).
    pub references: Vec<ReferenceUtterance>,
    /// Raw reference texts, in order.
    pub texts: Vec<String>,
}

/// Build a scenario from manifest utterances.
pub fn build(
    name: &str,
    items: &[(ManifestItem, Vec<f32>)],
    condition: Condition,
    format: Format,
) -> SpeechScenario {
    let pairs: Vec<(String, Vec<f32>)> = items
        .iter()
        .map(|(m, s)| (m.id.clone(), s.clone()))
        .collect();
    let scenario = ieval::build(name, &pairs, condition, format);
    let grid = u64::from(ieval::FIXTURE_RATE * ieval::GRID_MS / 1_000);
    let mut references = Vec::new();
    let mut texts = Vec::new();
    for ((m, _), span) in items.iter().zip(&scenario.utterance_spans) {
        if let Some((a, b)) = span {
            references.push(ReferenceUtterance {
                start: u64::from(*a) * grid,
                end: u64::from(*b) * grid,
                words: normalize(&m.reference),
            });
            texts.push(m.reference.clone());
        }
    }
    SpeechScenario {
        scenario,
        references,
        texts,
    }
}

/// Replay options.
#[derive(Clone, Debug)]
pub struct ReplayOptions {
    /// Recognition configuration.
    pub recognition: RecognitionConfig,
    /// Run the Phase 3 analyzer and forward its hints (`None`: no analysis).
    pub analysis: Option<AnalysisConfig>,
    /// Blocks withheld from the recognizer (the tap dropped them):
    /// `(first block, count)`.
    pub drops: Vec<(usize, usize)>,
    /// Load fed to the governor after every block (deterministic).
    pub load: f32,
}

impl Default for ReplayOptions {
    fn default() -> Self {
        let mut recognition = RecognitionConfig::default();
        recognition.budget.degrade = false;
        Self {
            recognition,
            analysis: Some(AnalysisConfig::default()),
            drops: Vec::new(),
            load: 0.0,
        }
    }
}

/// A replay result.
#[derive(Clone, Debug)]
pub struct SpeechReplay {
    /// Every speech event, in order.
    pub events: Vec<SpeechEvent>,
    /// Recognizer processing time (blocks + hints), seconds — CE overhead
    /// plus the adapter's own cost.
    pub processing_s: f64,
    /// Analyzer processing time, seconds.
    pub analysis_s: f64,
    /// Audio duration, seconds.
    pub audio_s: f64,
    /// Device rate.
    pub rate: u32,
}

/// Replay `sc` through (optionally) the Phase 3 analyzer and a recognizer
/// built from `factory`.
pub fn replay(
    sc: &SpeechScenario,
    factory: &dyn AsrFactory,
    identifier: Option<Arc<dyn LanguageIdentifier>>,
    opts: &ReplayOptions,
) -> Result<SpeechReplay, CeError> {
    let f = sc.scenario.format;
    let mut analyzer = match &opts.analysis {
        Some(c) => Some(StreamAnalyzer::new(SYNTH_STREAM, c.clone())?),
        None => None,
    };
    let setup = RecognizerSetup {
        stream: SYNTH_STREAM,
        config: opts.recognition.clone(),
        capabilities: factory.capabilities(),
        identifier,
        analysis_available: analyzer.is_some(),
    };
    let mut rec = StreamRecognizer::new(setup, factory.create()?)?;
    let mut events = Vec::new();
    let mut aev = Vec::new();
    rec.start(&mut events);
    rec.load(&mut events)?;
    if let Some(a) = analyzer.as_mut() {
        a.start(&mut aev);
        aev.clear();
    }
    let blocks = synth::blocks(&sc.scenario.samples, f.rate, f.channels, f.block_frames);
    let dropped = |i: usize| opts.drops.iter().any(|(s, n)| i >= *s && i < s + n);
    let (mut proc_s, mut ana_s) = (0.0f64, 0.0f64);
    let mut drops = 0u64;
    for (i, b) in blocks.iter().enumerate() {
        if dropped(i) {
            drops += 1;
        } else {
            let t0 = Instant::now();
            rec.push_block(b, &mut events)?;
            rec.note_load(opts.load, 0, drops, &mut events);
            proc_s += t0.elapsed().as_secs_f64();
        }
        if let Some(a) = analyzer.as_mut() {
            let t0 = Instant::now();
            a.push_block(b, &mut aev)?;
            a.note_load(0.0, 0, &mut aev);
            ana_s += t0.elapsed().as_secs_f64();
            let t0 = Instant::now();
            for e in aev.drain(..) {
                rec.push_hint(&e, &mut events)?;
            }
            proc_s += t0.elapsed().as_secs_f64();
        }
    }
    rec.finish(RecognitionEndReason::StreamClosed, &mut events);
    let frames = sc.scenario.samples.len() / usize::from(f.channels.max(1));
    Ok(SpeechReplay {
        events,
        processing_s: proc_s,
        analysis_s: ana_s,
        audio_s: frames as f64 / f64::from(f.rate),
        rate: f.rate,
    })
}

/// Deterministic digest of a replay's content events (wall-clock fields
/// such as load and processing times are excluded).
pub fn digest(events: &[SpeechEvent]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for e in events {
        if matches!(
            e,
            SpeechEvent::ModelReady { .. }
                | SpeechEvent::Stopped { .. }
                | SpeechEvent::Started { .. }
        ) {
            continue;
        }
        for b in format!("{e:?}").bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

// ---------------------------------------------------------------- scoring

/// Pipeline metrics of one replay.
#[derive(Clone, Debug, Default)]
pub struct SpeechMetrics {
    /// Reference utterances.
    pub references: usize,
    /// Finals.
    pub finals: usize,
    /// Cancelled utterances.
    pub cancelled: usize,
    /// References with no overlapping final.
    pub missed_references: usize,
    /// References covered by more than one final.
    pub split_references: usize,
    /// Finals covering more than one reference.
    pub merged_finals: usize,
    /// Stream-level WER (normalized).
    pub wer: ErrorRate,
    /// Stream-level CER (normalized).
    pub cer: ErrorRate,
    /// Case/punctuation-sensitive WER.
    pub wer_raw: ErrorRate,
    /// Partial revisions.
    pub partials: usize,
    /// Partial revisions per second of utterance audio.
    pub partial_rate_per_s: f32,
    /// Mean revisions (all statuses) per utterance.
    pub revisions_per_utterance: f32,
    /// Mean non-append word edits between consecutive revisions, per
    /// utterance (corrections a reader sees).
    pub corrections_per_utterance: f32,
    /// Finals that amended committed text.
    pub commit_amendments: usize,
    /// Reference speech start → first partial (input position), ms.
    pub first_partial_ms: Percentiles,
    /// Reference speech start → first committed text, ms.
    pub first_commit_ms: Percentiles,
    /// Reference speech end → final (input position), ms.
    pub final_ms: Percentiles,
    /// |utterance start − reference start|, ms.
    pub start_error_ms: Percentiles,
    /// Utterance-scope language decisions (`en`, `abstain:…`).
    pub languages: BTreeMap<String, usize>,
    /// Recognizer processing time / audio time.
    pub rtf: f32,
    /// Analyzer processing time / audio time.
    pub analysis_rtf: f32,
}

fn summarize(v: &[f32]) -> Percentiles {
    let mut r = Reservoir::new(v.len().max(1));
    for x in v {
        r.record(*x);
    }
    r.summary()
}

fn overlap(a: (u64, u64), b: (u64, u64)) -> u64 {
    a.1.min(b.1).saturating_sub(a.0.max(b.0))
}

/// Score a replay against its scenario.
pub fn score(sc: &SpeechScenario, r: &SpeechReplay) -> SpeechMetrics {
    let rate = u64::from(r.rate);
    let to_ms = |frames: i64| frames as f32 * 1_000.0 / rate as f32;
    // References in device frames.
    let refs: Vec<(u64, u64)> = sc
        .references
        .iter()
        .map(|u| (u.start * rate / 16_000, u.end * rate / 16_000))
        .collect();
    let mut by_utt: BTreeMap<u64, Vec<&TranscriptRevision>> = BTreeMap::new();
    for e in &r.events {
        if let SpeechEvent::Transcript(t) = e {
            by_utt.entry(t.meta.utterance).or_default().push(t);
        }
    }
    let mut m = SpeechMetrics {
        references: refs.len(),
        rtf: (r.processing_s / r.audio_s.max(1e-9)) as f32,
        analysis_rtf: (r.analysis_s / r.audio_s.max(1e-9)) as f32,
        ..Default::default()
    };
    let mut hyp_all = Vec::new();
    let mut covered = vec![0usize; refs.len()];
    let (mut fp, mut fc, mut fin, mut serr) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut revs, mut corr, mut utt_audio_ms) = (0usize, 0usize, 0f32);
    for revs_u in by_utt.values() {
        let Some(last) = revs_u.last() else { continue };
        revs += revs_u.len();
        let range = last.meta.frame_range;
        utt_audio_ms += to_ms((range.1 - range.0) as i64);
        let mut prev: Vec<String> = Vec::new();
        for t in revs_u {
            let next = normalize(&t.text.full());
            corr += levenshtein(&prev, &next).saturating_sub(next.len().saturating_sub(prev.len()));
            prev = next;
            if t.status == RevisionStatus::Partial {
                m.partials += 1;
            }
        }
        match last.status {
            RevisionStatus::Final => m.finals += 1,
            RevisionStatus::Cancelled => m.cancelled += 1,
            RevisionStatus::Partial => {}
        }
        if last.commit_amended {
            m.commit_amendments += 1;
        }
        if last.status == RevisionStatus::Final {
            hyp_all.push(last.text.committed.clone());
            if let Some(l) = &last.language {
                let k = match &l.decision {
                    LanguageDecision::Identified { language } => language.to_string(),
                    LanguageDecision::Abstained { reason } => format!("abstain:{reason:?}"),
                };
                *m.languages.entry(k).or_default() += 1;
            }
        }
        // The reference this utterance mostly covers.
        let hits: Vec<usize> = (0..refs.len())
            .filter(|k| overlap(range, refs[*k]) > 0)
            .collect();
        if hits.len() > 1 {
            m.merged_finals += 1;
        }
        // Delays and boundary errors only for utterances of one reference.
        let single = hits.len() == 1;
        let Some(k) = hits
            .iter()
            .copied()
            .max_by_key(|k| overlap(range, refs[*k]))
        else {
            continue;
        };
        covered[k] += 1;
        if covered[k] > 1 || !single {
            continue; // delays are measured on the first utterance of one reference
        }
        let (rs, re) = refs[k];
        serr.push(to_ms(range.0 as i64 - rs as i64).abs());
        if let Some(p) = revs_u.iter().find(|t| t.status == RevisionStatus::Partial) {
            fp.push(to_ms(p.meta.input_end_frame as i64 - rs as i64));
        }
        if let Some(c) = revs_u.iter().find(|t| !t.text.committed.is_empty()) {
            fc.push(to_ms(c.meta.input_end_frame as i64 - rs as i64));
        }
        if last.status == RevisionStatus::Final {
            fin.push(to_ms(last.meta.input_end_frame as i64 - re as i64));
        }
    }
    m.missed_references = covered.iter().filter(|c| **c == 0).count();
    m.split_references = covered.iter().filter(|c| **c > 1).count();
    let n_utt = by_utt.len().max(1) as f32;
    m.revisions_per_utterance = revs as f32 / n_utt;
    m.corrections_per_utterance = corr as f32 / n_utt;
    m.partial_rate_per_s = m.partials as f32 / (utt_audio_ms / 1_000.0).max(1e-3);
    let reference = sc.texts.join(" ");
    let hypothesis = hyp_all.join(" ");
    m.wer = wer(&reference, &hypothesis);
    m.cer = cer(&reference, &hypothesis);
    m.wer_raw = wer_raw(&reference, &hypothesis);
    m.first_partial_ms = summarize(&fp);
    m.first_commit_ms = summarize(&fc);
    m.final_ms = summarize(&fin);
    m.start_error_ms = summarize(&serr);
    m
}

/// Sum error rates.
pub fn total(rates: impl IntoIterator<Item = ErrorRate>) -> ErrorRate {
    let mut t = ErrorRate::default();
    for r in rates {
        t.add(r);
    }
    t
}

// ------------------------------------------------------------------- LID

/// One corpus item.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct LidItem {
    /// Id.
    pub id: String,
    /// `dev` or `test`.
    pub split: String,
    /// Language, or `mixed`.
    pub language: String,
    /// `short`, `medium` or `long`.
    pub length: String,
    /// Text.
    pub text: String,
    /// Character spans with languages (mixed items).
    #[serde(default)]
    pub spans: Option<Vec<(u32, u32, String)>>,
}

#[derive(Deserialize)]
struct LidCorpus {
    schema: String,
    items: Vec<LidItem>,
}

/// Load `fixtures/language/lid_corpus.json`.
pub fn load_lid_corpus() -> Result<Vec<LidItem>, CeError> {
    let path = ieval::fixture_dir().join("../language/lid_corpus.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| bad("eval.no_corpus", "language corpus missing").with_source(e))?;
    let c: LidCorpus = serde_json::from_str(&text)
        .map_err(|e| bad("eval.bad_corpus", "language corpus is malformed").with_source(e))?;
    if c.schema != "ce.lid-corpus.v1" {
        return Err(bad("eval.bad_corpus", "unknown corpus schema"));
    }
    Ok(c.items)
}

/// Counts for one group.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LidCounts {
    /// Items.
    pub n: usize,
    /// Items with a decision.
    pub decided: usize,
    /// Correct decisions.
    pub correct: usize,
}

impl LidCounts {
    /// Accuracy over decided items.
    pub fn accuracy(&self) -> f32 {
        self.correct as f32 / self.decided.max(1) as f32
    }
    /// Share of items decided.
    pub fn coverage(&self) -> f32 {
        self.decided as f32 / self.n.max(1) as f32
    }
}

/// Language-identification results.
#[derive(Clone, Debug, Default)]
pub struct LidResult {
    /// Single-language items by `(split, length)`; `length` `all` too.
    pub groups: BTreeMap<(String, String), LidCounts>,
    /// `(split, truth, outcome)` → count; outcome is a language or
    /// `abstain:<reason>`.
    pub confusion: BTreeMap<(String, String, String), usize>,
    /// Mixed items: `(id, outcome)`.
    pub mixed: Vec<(String, String)>,
}

/// Evaluate `ident` with `cfg` on the corpus.
pub fn evaluate_lid(
    items: &[LidItem],
    ident: &dyn LanguageIdentifier,
    cfg: &LanguageIdConfig,
) -> LidResult {
    let mut out = LidResult::default();
    for it in items {
        let o = observe_utterance(ident, cfg, &LanguageHint::Auto, 0, &it.text);
        let outcome = match &o.decision {
            LanguageDecision::Identified { language } => language.to_string(),
            LanguageDecision::Abstained { reason } => format!("abstain:{reason:?}"),
        };
        if it.language == "mixed" {
            out.mixed.push((it.id.clone(), outcome));
            continue;
        }
        let decided = !outcome.starts_with("abstain");
        let correct = decided && outcome == it.language;
        for len in [it.length.as_str(), "all"] {
            let g = out
                .groups
                .entry((it.split.clone(), len.to_owned()))
                .or_default();
            g.n += 1;
            g.decided += usize::from(decided);
            g.correct += usize::from(correct);
        }
        *out.confusion
            .entry((it.split.clone(), it.language.clone(), outcome))
            .or_default() += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_and_error_rates() {
        assert_eq!(
            normalize("Good morning, could you tell me? It's \"fine\"."),
            vec![
                "good", "morning", "could", "you", "tell", "me", "it's", "fine"
            ]
        );
        assert_eq!(levenshtein(&[1, 2, 3], &[1, 3]), 1);
        assert_eq!(levenshtein::<u8>(&[], &[1, 2]), 2);
        let w = wer("the cat sat", "the hat sat down");
        assert_eq!((w.edits, w.reference), (2, 3));
        assert_eq!(wer("Hello, World", "hello world").edits, 0);
        assert_eq!(wer_raw("Hello, World", "hello world").edits, 2);
        assert_eq!(cer("abc", "abd").edits, 1);
        assert_eq!(ErrorRate::default().rate(), 0.0);
        assert_eq!(
            total([
                ErrorRate {
                    edits: 1,
                    reference: 2
                },
                ErrorRate {
                    edits: 1,
                    reference: 2
                }
            ])
            .rate(),
            0.5
        );
    }

    #[test]
    fn manifest_matches_the_fixtures() {
        let m = load_manifest().unwrap();
        assert_eq!(m.len(), 10);
        assert!(
            m.iter()
                .all(|i| ieval::fixture_dir().join(&i.file).exists())
        );
        assert_eq!(load_split(Split::Dev).unwrap().len(), 4);
        let c = load_lid_corpus().unwrap();
        assert_eq!(c.len(), 134);
        assert_eq!(c.iter().filter(|i| i.language == "mixed").count(), 8);
    }
}
