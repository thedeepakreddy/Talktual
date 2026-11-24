//! Evaluation: corpus, metrics and replay through the real processor.
//!
//! * **Corpus** (`fixtures/translation/corpus.json`): authored parallel
//!   items with `dev`/`test` splits, directed pairs, a category (general,
//!   negation, numbers, question, names, terminology, correction, register)
//!   and provenance. Tuning and memory construction use `dev` only.
//! * **BLEU**: corpus-level, 4-gram, uniform weights, brevity penalty, no
//!   smoothing, case-sensitive, 13a-style tokenization (punctuation split
//!   from words). **chrF**: chrF2 (character 1..6-grams, β = 2, whitespace
//!   removed), corpus-level statistics. Both follow the published
//!   definitions (Papineni et al. 2002; Popović 2015) and are cross-checked
//!   against sacreBLEU 2.4.3 in the Phase 6 evaluation report.
//! * **Fidelity checks** (automatic, conservative): digits preserved,
//!   negation present in the output iff in the source (small per-language
//!   cue lists), question mark preserved. They detect some errors; passing
//!   them is not evidence of a correct translation.
//! * **Replay** builds a final Phase 4 `TranscriptRevision` per item and
//!   runs it through [`TranslationProcessor`], so evaluation exercises the
//!   same code as production (language policy, protection, validation).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ce_core::ids::AudioStreamId;
use ce_core::speech::{
    HintSource, LanguageTag, RecognitionMode, RevisionStatus, SpeechMeta, TranscriptDelta,
    TranscriptRevision, TranscriptText,
};
use ce_core::time::MonotonicTime;
use ce_core::translation::{
    SourceLanguagePolicy, TerminologyProfile, TranslationConfig, TranslationEvent,
    UnavailableReason,
};
use ce_core::{CeError, ErrorCategory};
use serde::{Deserialize, Serialize};

use crate::adapter::TranslationFactory;
use crate::processor::{AdapterSlot, ProcessorSetup, TranslationProcessor};

/// One corpus item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// Id.
    pub id: String,
    /// `dev` or `test`.
    pub split: String,
    /// Source language.
    pub source_language: String,
    /// Target language.
    pub target_language: String,
    /// Category.
    pub category: String,
    /// `short` (≤ 3 words), `medium` (≤ 8), `long`.
    pub length: String,
    /// Source text.
    pub source: String,
    /// Reference translation.
    pub reference: String,
    /// Provenance.
    pub provenance: String,
}

#[derive(Deserialize)]
struct CorpusFile {
    items: Vec<Item>,
}

/// Fixture directory.
pub fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/translation")
}

fn read(path: &Path) -> Result<String, CeError> {
    std::fs::read_to_string(path).map_err(|e| {
        CeError::new(
            ErrorCategory::InvalidInput,
            "eval.missing_fixture",
            "fixture not found",
        )
        .with_context("path", path.display().to_string())
        .with_source(e)
    })
}

/// Load the corpus.
pub fn load_corpus() -> Result<Vec<Item>, CeError> {
    let f: CorpusFile =
        serde_json::from_str(&read(&fixture_dir().join("corpus.json"))?).map_err(|e| {
            CeError::new(
                ErrorCategory::InvalidInput,
                "eval.bad_corpus",
                "corpus is invalid",
            )
            .with_source(e)
        })?;
    Ok(f.items)
}

/// Load the dev-only translation memory.
pub fn load_memory() -> Result<crate::memory::TranslationMemory, CeError> {
    crate::memory::TranslationMemory::from_json(&read(&fixture_dir().join("memory.json"))?)
}

/// Load the hospitality terminology profile.
pub fn load_terminology() -> Result<TerminologyProfile, CeError> {
    let p: TerminologyProfile = serde_json::from_str(&read(
        &fixture_dir().join("terminology-hospitality-en-es.json"),
    )?)
    .map_err(|e| {
        CeError::new(
            ErrorCategory::InvalidInput,
            "eval.bad_terminology",
            "profile is invalid",
        )
        .with_source(e)
    })?;
    p.validate()?;
    Ok(p)
}

// ------------------------------------------------------------------ metrics

/// sacreBLEU's `13a` tokenizer (the mteval-v13a rules): ASCII
/// punctuation in ``{|}~ [\]^_` !"#$%& ()*+ :;<=>?@ /`` is split off;
/// `.` and `,` are split unless both neighbours are digits; `-` is split after a
/// digit. Non-ASCII punctuation (e.g. `¿`) is **not** split, exactly as in
/// sacreBLEU.
pub fn tokenize(s: &str) -> Vec<String> {
    let is_punct =
        |c: char| matches!(c, '{'..='~' | '['..='`' | ' '..='&' | '('..='+' | ':'..='@' | '/');
    let chars: Vec<char> = s.replace('\n', " ").chars().collect();
    let mut out = String::with_capacity(s.len() * 2);
    for (i, &c) in chars.iter().enumerate() {
        let prev = if i > 0 { chars[i - 1] } else { ' ' };
        let next = chars.get(i + 1).copied().unwrap_or(' ');
        if c != ' ' && is_punct(c) {
            out.push(' ');
            out.push(c);
            out.push(' ');
        } else if (c == '.' || c == ',') && !(prev.is_ascii_digit() && next.is_ascii_digit()) {
            // 13a: split unless both neighbours are digits ("1,000", "3.50").
            out.push(' ');
            out.push(c);
            out.push(' ');
        } else if c == '-' && prev.is_ascii_digit() {
            out.push(' ');
            out.push(c);
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out.split_whitespace().map(str::to_owned).collect()
}

fn ngrams<T: Ord + Clone>(v: &[T], n: usize) -> BTreeMap<Vec<T>, usize> {
    let mut m = BTreeMap::new();
    if v.len() >= n {
        for w in v.windows(n) {
            *m.entry(w.to_vec()).or_insert(0) += 1;
        }
    }
    m
}

/// Corpus BLEU (0–100), as sacreBLEU's `corpus_bleu` defaults: `13a`
/// tokenization, case-sensitive, 4-grams, brevity penalty, `exp`
/// smoothing (an order with no match gets precision 1/(2^k·total), k
/// counting such orders).
pub fn bleu(hyps: &[String], refs: &[String]) -> f64 {
    let (mut matches, mut totals) = ([0usize; 4], [0usize; 4]);
    let (mut hl, mut rl) = (0usize, 0usize);
    for (h, r) in hyps.iter().zip(refs) {
        let (h, r) = (tokenize(h), tokenize(r));
        hl += h.len();
        rl += r.len();
        for n in 1..=4 {
            let (hn, rn) = (ngrams(&h, n), ngrams(&r, n));
            totals[n - 1] += h.len().saturating_sub(n - 1);
            matches[n - 1] += hn
                .iter()
                .map(|(g, c)| (*c).min(*rn.get(g).unwrap_or(&0)))
                .sum::<usize>();
        }
    }
    // As sacreBLEU: no match of any order (#141) or no hypothesis → 0.
    if hl == 0 || matches.iter().all(|m| *m == 0) {
        return 0.0;
    }
    let mut smooth = 1.0f64;
    let mut logp = 0.0f64;
    for i in 0..4 {
        if totals[i] == 0 {
            return 0.0;
        }
        let p = if matches[i] == 0 {
            smooth *= 2.0;
            1.0 / (smooth * totals[i] as f64)
        } else {
            matches[i] as f64 / totals[i] as f64
        };
        logp += p.ln() / 4.0;
    }
    let bp = if hl >= rl {
        1.0
    } else {
        (1.0 - rl as f64 / hl as f64).exp()
    };
    100.0 * bp * logp.exp()
}

/// Corpus chrF2 (0–100).
pub fn chrf(hyps: &[String], refs: &[String]) -> f64 {
    const N: usize = 6;
    const BETA: f64 = 2.0;
    let (mut m, mut hc, mut rc) = ([0usize; N], [0usize; N], [0usize; N]);
    for (h, r) in hyps.iter().zip(refs) {
        let h: Vec<char> = h.chars().filter(|c| !c.is_whitespace()).collect();
        let r: Vec<char> = r.chars().filter(|c| !c.is_whitespace()).collect();
        for n in 1..=N {
            let (hn, rn) = (ngrams(&h, n), ngrams(&r, n));
            hc[n - 1] += hn.values().sum::<usize>();
            rc[n - 1] += rn.values().sum::<usize>();
            m[n - 1] += hn
                .iter()
                .map(|(g, c)| (*c).min(*rn.get(g).unwrap_or(&0)))
                .sum::<usize>();
        }
    }
    let (mut p, mut r, mut k) = (0.0, 0.0, 0.0);
    for i in 0..N {
        if hc[i] > 0 && rc[i] > 0 {
            p += m[i] as f64 / hc[i] as f64;
            r += m[i] as f64 / rc[i] as f64;
            k += 1.0;
        }
    }
    if k == 0.0 {
        return 0.0;
    }
    let (p, r) = (p / k, r / k);
    if p + r == 0.0 {
        return 0.0;
    }
    100.0 * (1.0 + BETA * BETA) * p * r / (BETA * BETA * p + r)
}

fn negation_cues(lang: &str) -> &'static [&'static str] {
    match lang {
        "en" => &[
            "not", "no", "never", "don't", "doesn't", "didn't", "isn't", "aren't", "won't",
            "can't", "cannot", "nothing", "nobody",
        ],
        "es" => &["no", "nunca", "jamás", "nada", "nadie", "ningún", "ninguna"],
        "fr" => &["ne", "pas", "jamais", "rien", "personne", "aucun", "aucune"],
        "de" => &[
            "nicht", "kein", "keine", "keinen", "nie", "niemals", "nichts", "niemand",
        ],
        _ => &[],
    }
}

fn has_negation(lang: &str, s: &str) -> Option<bool> {
    let cues = negation_cues(lang);
    if cues.is_empty() {
        return None;
    }
    let lower = s.to_lowercase();
    let words: Vec<String> = tokenize(&lower);
    Some(
        words
            .iter()
            .any(|w| cues.contains(&w.as_str()) || w.starts_with("n'")),
    )
}

/// Whether negation agrees between source and output (`None`: no cue list
/// for one of the languages). Correction phrases ("no, three") count as
/// negation cues too; the check is a screen, not a judgement.
pub fn negation_agrees(src_lang: &str, src: &str, tgt_lang: &str, out: &str) -> Option<bool> {
    Some(has_negation(src_lang, src)? == has_negation(tgt_lang, out)?)
}

// ------------------------------------------------------------------ replay

/// A translation produced in replay.
#[derive(Clone, Debug, PartialEq)]
pub struct Produced {
    /// Item id.
    pub id: String,
    /// Output text, or the reason none was produced.
    pub output: Result<String, UnavailableReason>,
    /// Uncertainty flags as strings.
    pub flags: Vec<String>,
    /// Adapter time, ms.
    pub adapter_ms: f32,
}

/// Build the final transcript revision for an item (utterance `n`).
pub fn revision_for(stream: AudioStreamId, n: u64, text: &str) -> TranscriptRevision {
    let t = MonotonicTime::from_nanos(n * 1_000_000_000);
    TranscriptRevision {
        meta: SpeechMeta {
            stream,
            utterance: n,
            sequence_range: (0, 0),
            frame_range: (n * 16_000, n * 16_000 + 8_000),
            time_range: (t, t),
            input_end_frame: n * 16_000 + 8_000,
            discontinuity: None,
            mode: RecognitionMode::Full,
            hints: HintSource::Continuous,
        },
        revision: 0,
        status: RevisionStatus::Final,
        text: TranscriptText {
            committed: text.to_string(),
            provisional: String::new(),
        },
        delta: TranscriptDelta {
            keep_committed_chars: 0,
            append_committed: text.to_string(),
            provisional: String::new(),
        },
        commit_amended: false,
        words: None,
        score: None,
        language: None,
        final_reason: None,
    }
}

/// Translate items (all one directed pair) through a processor built on
/// `factory`, with an explicit source language.
pub fn replay(
    factory: &dyn TranslationFactory,
    items: &[&Item],
    terminology: Option<TerminologyProfile>,
) -> Result<Vec<Produced>, CeError> {
    let Some(first) = items.first() else {
        return Ok(Vec::new());
    };
    let stream = AudioStreamId::from_uuid(uuid::Uuid::from_u128(0x7e57));
    let config = TranslationConfig {
        target: Some(LanguageTag::parse(&first.target_language)?),
        source: SourceLanguagePolicy::Explicit {
            language: LanguageTag::parse(&first.source_language)?,
        },
        terminology: terminology.as_ref().map(|t| t.id.clone()),
        ..TranslationConfig::default()
    };
    let terminology = terminology.filter(|t| {
        t.pair.source.primary() == first.source_language
            && t.pair.target.primary() == first.target_language
    });
    let config = TranslationConfig {
        terminology: terminology.as_ref().map(|t| t.id.clone()),
        ..config
    };
    let mut p = TranslationProcessor::new(ProcessorSetup {
        stream,
        adapters: vec![AdapterSlot {
            capabilities: factory.capabilities(),
            adapter: factory.create(&config)?,
        }],
        config,
        terminology,
    })?;
    p.load()?;
    let mut out = Vec::new();
    let mut produced = Vec::new();
    for (n, it) in items.iter().enumerate() {
        out.clear();
        p.process(&revision_for(stream, n as u64, &it.source), 0.0, &mut out)?;
        let mut rec = Produced {
            id: it.id.clone(),
            output: Err(UnavailableReason::AdapterError),
            flags: Vec::new(),
            adapter_ms: 0.0,
        };
        for e in &out {
            match e {
                TranslationEvent::Result(r) => {
                    rec.output = Ok(r.text.clone());
                    rec.flags = r.flags.iter().map(|f| format!("{f:?}")).collect();
                    rec.adapter_ms = r.timing.adapter_ms;
                }
                TranslationEvent::Unavailable { reason, .. } => rec.output = Err(*reason),
                _ => {}
            }
        }
        produced.push(rec);
    }
    Ok(produced)
}

/// Pair-level report.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PairReport {
    /// `en-es` etc.
    pub pair: String,
    /// Items.
    pub items: usize,
    /// Items translated.
    pub covered: usize,
    /// BLEU over covered items only (`None` if none covered).
    pub bleu_covered: Option<f64>,
    /// chrF over covered items only.
    pub chrf_covered: Option<f64>,
    /// BLEU over all items, empty output where unavailable.
    pub bleu_all: f64,
    /// chrF over all items, empty output where unavailable.
    pub chrf_all: f64,
    /// Covered items whose digits were not preserved.
    pub number_errors: usize,
    /// Covered items whose negation disagrees with the source.
    pub negation_errors: usize,
    /// Covered items that lost a question mark.
    pub question_errors: usize,
    /// Results with uncertainty flags.
    pub flagged: usize,
    /// Unavailable reasons.
    pub unavailable: BTreeMap<String, usize>,
    /// Exact match with the reference (covered items).
    pub exact: usize,
}

/// Score a replay.
pub fn score(pair: &str, items: &[&Item], produced: &[Produced]) -> PairReport {
    let mut r = PairReport {
        pair: pair.into(),
        items: items.len(),
        ..PairReport::default()
    };
    let (mut ch, mut cr, mut ah, mut ar) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (it, p) in items.iter().zip(produced) {
        ar.push(it.reference.clone());
        match &p.output {
            Ok(o) => {
                r.covered += 1;
                ch.push(o.clone());
                cr.push(it.reference.clone());
                ah.push(o.clone());
                if crate::fidelity::numbers_missing(&it.source, o) {
                    r.number_errors += 1;
                }
                if negation_agrees(&it.source_language, &it.source, &it.target_language, o)
                    == Some(false)
                {
                    r.negation_errors += 1;
                }
                if it.source.contains('?') && !o.contains('?') {
                    r.question_errors += 1;
                }
                if !p.flags.is_empty() {
                    r.flagged += 1;
                }
                if o.trim() == it.reference.trim() {
                    r.exact += 1;
                }
            }
            Err(reason) => {
                ah.push(String::new());
                *r.unavailable.entry(format!("{reason:?}")).or_insert(0) += 1;
            }
        }
    }
    if !ch.is_empty() {
        r.bleu_covered = Some(bleu(&ch, &cr));
        r.chrf_covered = Some(chrf(&ch, &cr));
    }
    r.bleu_all = bleu(&ah, &ar);
    r.chrf_all = chrf(&ah, &ar);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn metric_properties() {
        let r = s(&["the cat sat on the mat today"]);
        assert!((bleu(&r, &r) - 100.0).abs() < 1e-9);
        assert!((chrf(&r, &r) - 100.0).abs() < 1e-9);
        // No match of any order: 0, exactly as sacreBLEU 2.4.3.
        assert_eq!(bleu(&s(&["zzz yyy xxx www"]), &r), 0.0);
        // One unigram match: exp smoothing keeps it above zero; this is
        // the value sacreBLEU 2.4.3 reports for this pair (7.5454).
        let z = bleu(&s(&["the yyy xxx www"]), &r);
        assert!((z - 7.5454).abs() < 1e-3, "{z}");
        assert!(chrf(&s(&["zzz"]), &r) < 5.0);
        let near = s(&["the cat sat on a mat today"]);
        let b = bleu(&near, &r);
        assert!(b > 0.0 && b < 100.0);
        assert_eq!(
            tokenize("¿Dónde está?"),
            s(&["¿Dónde", "está", "?"]),
            "13a splits ASCII punctuation only"
        );
        assert_eq!(
            tokenize("It costs 3.50, ok."),
            s(&["It", "costs", "3.50", ",", "ok", "."])
        );
        assert_eq!(
            tokenize("room 4-5 l'hôtel"),
            s(&["room", "4", "-", "5", "l'hôtel"])
        );
        assert_eq!(bleu(&s(&[""]), &r), 0.0);
    }

    #[test]
    fn fidelity_screens() {
        assert_eq!(
            negation_agrees("en", "I do not want it", "es", "No lo quiero"),
            Some(true)
        );
        assert_eq!(
            negation_agrees("en", "I do not want it", "es", "Lo quiero"),
            Some(false)
        );
        assert_eq!(
            negation_agrees("en", "I want it", "fr", "Je ne le veux pas"),
            Some(false)
        );
        assert_eq!(negation_agrees("en", "x", "ja", "x"), None);
    }

    #[test]
    fn corpus_and_fixtures_load_and_splits_do_not_leak() {
        let c = load_corpus().unwrap();
        assert!(c.iter().filter(|i| i.split == "test").count() == 62);
        let mem = load_memory().unwrap();
        let test_sources: Vec<_> = c.iter().filter(|i| i.split == "test").collect();
        for it in test_sources {
            let (a, b) = (
                LanguageTag::parse(&it.source_language).unwrap(),
                LanguageTag::parse(&it.target_language).unwrap(),
            );
            assert!(
                mem.lookup(&a, &b, &it.source).is_none(),
                "test item {} leaked into the memory",
                it.id
            );
        }
        load_terminology().unwrap();
    }
}
