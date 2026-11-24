//! Text language identification with explicit scope, uncertainty and
//! provenance (ADR-0022).
//!
//! * [`LanguageIdentifier`] — the replaceable identifier boundary.
//! * [`WhichlangIdentifier`] — the default (whichlang 0.1.1, MIT, 16
//!   languages, no score exposed → `score_semantics: None`).
//! * [`ScriptedIdentifier`] — deterministic, for tests.
//! * [`observe_utterance`] — one utterance's final text → an observation
//!   with scope `Utterance`, abstaining when evidence is short, ambiguous
//!   (the two halves disagree), low-scoring (only for identifiers that
//!   report scores) or outside the expected set.
//! * [`StreamLanguage`] — the stream-scope aggregate over *decided*
//!   utterances. It is a separate observation with scope `Stream`; session,
//!   utterance and span languages are never substituted for one another.
//!
//! Observations describe the **language of text**, never the speaker:
//! nothing here infers nationality, ethnicity, identity, location or any
//! other trait. Code-switching (scope `Span`) is not produced; see
//! `SpeechCapabilities::code_switching` and the Phase 4 report.

use std::collections::BTreeMap;
use std::sync::Arc;

use ce_core::intel::ScoreSemantics;
use ce_core::speech::{
    AbstainReason, LanguageCandidate, LanguageDecision, LanguageHint, LanguageIdConfig,
    LanguageObservation, LanguageScope, LanguageSource, LanguageTag,
};

use crate::fnv;

/// A text language identifier. Implementations must be deterministic for a
/// given version and cheap to construct.
pub trait LanguageIdentifier: Send + Sync + std::fmt::Debug {
    /// Stable id, e.g. `ce.lid.whichlang`.
    fn id(&self) -> &str;
    /// Version (of the identifier and its model).
    fn version(&self) -> &str;
    /// Languages it can name.
    fn languages(&self) -> Vec<LanguageTag>;
    /// Semantics of candidate scores (`None`: it reports no scores).
    fn score_semantics(&self) -> Option<ScoreSemantics>;
    /// Candidates for `text`, best first. Empty means "no answer".
    fn detect(&self, text: &str) -> Vec<LanguageCandidate>;
}

fn tag(s: &str) -> LanguageTag {
    // Only called with the constant tags below, which are valid.
    LanguageTag::parse(s).unwrap_or_else(|_| unreachable_tag())
}

#[cold]
fn unreachable_tag() -> LanguageTag {
    LanguageTag::parse("und").expect("'und' is a valid tag")
}

/// whichlang 0.1.1: a small logistic-regression classifier over character
/// n-grams. It returns one language and **no score**; it always answers
/// (English for text without features), so CE's evidence gate (letters)
/// decides when to abstain.
#[derive(Debug, Clone, Copy, Default)]
pub struct WhichlangIdentifier;

/// whichlang languages as BCP-47 tags (in whichlang's order).
pub const WHICHLANG_LANGUAGES: [&str; 16] = [
    "ar", "zh", "de", "en", "fr", "hi", "it", "ja", "ko", "nl", "pt", "ru", "es", "sv", "tr", "vi",
];

impl WhichlangIdentifier {
    /// Id.
    pub const ID: &'static str = "ce.lid.whichlang";
    /// Version (CE wrapper version + whichlang version).
    pub const VERSION: &'static str = "1.0.0+whichlang-0.1.1";

    fn map(l: whichlang::Lang) -> &'static str {
        use whichlang::Lang;
        match l {
            Lang::Ara => "ar",
            Lang::Cmn => "zh",
            Lang::Deu => "de",
            Lang::Eng => "en",
            Lang::Fra => "fr",
            Lang::Hin => "hi",
            Lang::Ita => "it",
            Lang::Jpn => "ja",
            Lang::Kor => "ko",
            Lang::Nld => "nl",
            Lang::Por => "pt",
            Lang::Rus => "ru",
            Lang::Spa => "es",
            Lang::Swe => "sv",
            Lang::Tur => "tr",
            Lang::Vie => "vi",
        }
    }
}

impl LanguageIdentifier for WhichlangIdentifier {
    fn id(&self) -> &str {
        Self::ID
    }
    fn version(&self) -> &str {
        Self::VERSION
    }
    fn languages(&self) -> Vec<LanguageTag> {
        WHICHLANG_LANGUAGES.iter().map(|s| tag(s)).collect()
    }
    fn score_semantics(&self) -> Option<ScoreSemantics> {
        None
    }
    fn detect(&self, text: &str) -> Vec<LanguageCandidate> {
        if text.trim().is_empty() {
            return Vec::new();
        }
        vec![LanguageCandidate {
            language: tag(Self::map(whichlang::detect_language(text))),
            score: None,
        }]
    }
}

/// A deterministic identifier for tests: a function from text to candidates.
pub struct ScriptedIdentifier {
    languages: Vec<LanguageTag>,
    semantics: Option<ScoreSemantics>,
    #[allow(clippy::type_complexity)]
    f: Arc<dyn Fn(&str) -> Vec<LanguageCandidate> + Send + Sync>,
}

impl std::fmt::Debug for ScriptedIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptedIdentifier")
            .field("languages", &self.languages)
            .finish_non_exhaustive()
    }
}

impl ScriptedIdentifier {
    /// From a function.
    pub fn new(
        languages: Vec<LanguageTag>,
        semantics: Option<ScoreSemantics>,
        f: impl Fn(&str) -> Vec<LanguageCandidate> + Send + Sync + 'static,
    ) -> Self {
        Self {
            languages,
            semantics,
            f: Arc::new(f),
        }
    }
}

impl LanguageIdentifier for ScriptedIdentifier {
    fn id(&self) -> &str {
        "ce.lid.scripted"
    }
    fn version(&self) -> &str {
        "1.0.0"
    }
    fn languages(&self) -> Vec<LanguageTag> {
        self.languages.clone()
    }
    fn score_semantics(&self) -> Option<ScoreSemantics> {
        self.semantics
    }
    fn detect(&self, text: &str) -> Vec<LanguageCandidate> {
        (self.f)(text)
    }
}

/// Letters of evidence (alphabetic Unicode scalar values).
pub fn letters(text: &str) -> u32 {
    u32::try_from(text.chars().filter(|c| c.is_alphabetic()).count()).unwrap_or(u32::MAX)
}

/// Fingerprint of an identifier + policy (16 hex digits).
pub fn fingerprint(ident: &dyn LanguageIdentifier, cfg: &LanguageIdConfig) -> String {
    fnv(&format!(
        "lid.v1|{}|{}|{:?}",
        ident.id(),
        ident.version(),
        cfg
    ))
}

/// Split `text` at the word boundary closest to the middle (by letters).
fn halves(text: &str) -> Option<(&str, &str)> {
    let total = letters(text);
    let mut seen = 0u32;
    let mut best: Option<(u32, usize)> = None;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            let d = seen.abs_diff(total - seen);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, i));
            }
        } else if c.is_alphabetic() {
            seen += 1;
        }
    }
    best.map(|(_, i)| (&text[..i], &text[i..]))
}

fn abstain(reason: AbstainReason) -> LanguageDecision {
    LanguageDecision::Abstained { reason }
}

/// Observe the language of one utterance's final text.
///
/// Order of rules: `Fixed` hint → `Declared` (identification not run); too
/// few letters → `InsufficientEvidence`; no answer → `InsufficientEvidence`;
/// a score below `min_score` (only when the identifier reports scores) →
/// `LowConfidence`; with `split_check`, the two halves disagree (both with
/// ≥ max(min_letters, 10) letters) → `Ambiguous`; outside an `Expected` set → `Unsupported`;
/// otherwise `Identified`.
pub fn observe_utterance(
    ident: &dyn LanguageIdentifier,
    cfg: &LanguageIdConfig,
    hint: &LanguageHint,
    utterance: u64,
    text: &str,
) -> LanguageObservation {
    let evidence = letters(text);
    let scope = LanguageScope::Utterance { utterance };
    let fp = fingerprint(ident, cfg);
    if let LanguageHint::Fixed(l) = hint {
        return LanguageObservation {
            scope,
            decision: LanguageDecision::Identified {
                language: l.clone(),
            },
            candidates: vec![LanguageCandidate {
                language: l.clone(),
                score: None,
            }],
            score_semantics: None,
            source: LanguageSource::Declared,
            evidence,
            fingerprint: fp,
        };
    }
    let source = LanguageSource::TextIdentifier {
        id: ident.id().to_owned(),
        version: ident.version().to_owned(),
    };
    let semantics = ident.score_semantics();
    let mk = |decision, candidates: Vec<LanguageCandidate>| LanguageObservation {
        scope: scope.clone(),
        decision,
        candidates,
        score_semantics: semantics,
        source: source.clone(),
        evidence,
        fingerprint: fp.clone(),
    };
    if evidence < cfg.min_letters {
        return mk(abstain(AbstainReason::InsufficientEvidence), Vec::new());
    }
    let mut candidates = ident.detect(text);
    // Never trust a score the identifier does not declare.
    if semantics.is_none() {
        for c in &mut candidates {
            c.score = None;
        }
    } else {
        for c in &mut candidates {
            c.score = c.score.filter(|s| s.is_finite());
        }
    }
    let Some(top) = candidates.first().cloned() else {
        return mk(abstain(AbstainReason::InsufficientEvidence), candidates);
    };
    if semantics.is_some() && top.score.is_some_and(|s| s < cfg.min_score) {
        return mk(abstain(AbstainReason::LowConfidence), candidates);
    }
    let need = cfg.min_letters.max(10);
    if let Some((a, b)) = halves(text).filter(|_| cfg.split_check) {
        if letters(a) >= need && letters(b) >= need {
            let la = ident.detect(a).into_iter().next();
            let lb = ident.detect(b).into_iter().next();
            if let (Some(la), Some(lb)) = (la, lb) {
                if la.language.primary() != lb.language.primary() {
                    let other = if la.language.primary() == top.language.primary() {
                        lb.language
                    } else {
                        la.language
                    };
                    if !candidates.iter().any(|c| c.language == other) {
                        candidates.push(LanguageCandidate {
                            language: other,
                            score: None,
                        });
                    }
                    return mk(abstain(AbstainReason::Ambiguous), candidates);
                }
            }
        }
    }
    if let LanguageHint::Expected(set) = hint {
        if !set.iter().any(|l| l.primary() == top.language.primary()) {
            return mk(abstain(AbstainReason::Unsupported), candidates);
        }
    }
    mk(
        LanguageDecision::Identified {
            language: top.language,
        },
        candidates,
    )
}

/// Stream-scope aggregate over utterance observations.
///
/// Only `Identified` utterance decisions count. With fewer than
/// `session_min_utterances` of them the stream abstains
/// (`InsufficientEvidence`); when the most frequent language holds less
/// than `session_min_share` of them it abstains (`Ambiguous`). A `Fixed`
/// hint makes the stream language `Declared` from the start. A new
/// observation is produced only when the decision changes.
#[derive(Debug, Clone)]
pub struct StreamLanguage {
    cfg: LanguageIdConfig,
    declared: Option<LanguageTag>,
    counts: BTreeMap<LanguageTag, u32>,
    decided: u32,
    fingerprint: String,
    last: Option<LanguageObservation>,
}

impl StreamLanguage {
    /// New aggregate.
    pub fn new(cfg: LanguageIdConfig, hint: &LanguageHint, fingerprint: String) -> Self {
        Self {
            cfg,
            declared: match hint {
                LanguageHint::Fixed(l) => Some(l.clone()),
                _ => None,
            },
            counts: BTreeMap::new(),
            decided: 0,
            fingerprint,
            last: None,
        }
    }

    /// Latest stream-scope observation.
    pub fn current(&self) -> Option<&LanguageObservation> {
        self.last.as_ref()
    }

    /// Forget everything (e.g. after a requested reset nothing is kept:
    /// the stream language describes the utterances since).
    pub fn clear(&mut self) {
        self.counts.clear();
        self.decided = 0;
        self.last = None;
    }

    fn compute(&self) -> LanguageObservation {
        if let Some(l) = &self.declared {
            return LanguageObservation {
                scope: LanguageScope::Stream,
                decision: LanguageDecision::Identified {
                    language: l.clone(),
                },
                candidates: vec![LanguageCandidate {
                    language: l.clone(),
                    score: None,
                }],
                score_semantics: None,
                source: LanguageSource::Declared,
                evidence: self.decided,
                fingerprint: self.fingerprint.clone(),
            };
        }
        let mut ranked: Vec<(&LanguageTag, u32)> =
            self.counts.iter().map(|(l, n)| (l, *n)).collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        let candidates = ranked
            .iter()
            .map(|(l, _)| LanguageCandidate {
                language: (*l).clone(),
                score: None,
            })
            .collect();
        let decision = if self.decided < self.cfg.session_min_utterances {
            abstain(AbstainReason::InsufficientEvidence)
        } else {
            let (top, n) = ranked[0];
            let tie = ranked.get(1).is_some_and(|(_, m)| *m == n);
            if tie || (n as f32) / (self.decided as f32) < self.cfg.session_min_share {
                abstain(AbstainReason::Ambiguous)
            } else {
                LanguageDecision::Identified {
                    language: top.clone(),
                }
            }
        };
        LanguageObservation {
            scope: LanguageScope::Stream,
            decision,
            candidates,
            score_semantics: None,
            source: LanguageSource::Aggregate,
            evidence: self.decided,
            fingerprint: self.fingerprint.clone(),
        }
    }

    /// Add an utterance observation; returns the stream observation when
    /// its decision changed (the first call always returns one).
    pub fn observe(&mut self, utterance: &LanguageObservation) -> Option<LanguageObservation> {
        if !matches!(utterance.scope, LanguageScope::Utterance { .. }) {
            return None;
        }
        if let LanguageDecision::Identified { language } = &utterance.decision {
            if self.declared.is_none() {
                *self.counts.entry(language.clone()).or_default() += 1;
            }
            self.decided += 1;
        }
        let next = self.compute();
        // Evidence growth alone is not a change worth an event.
        let changed = self
            .last
            .as_ref()
            .is_none_or(|l| l.decision != next.decision);
        self.last = Some(next.clone());
        changed.then_some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> LanguageTag {
        LanguageTag::parse(s).unwrap()
    }

    fn cfg() -> LanguageIdConfig {
        LanguageIdConfig::default()
    }

    fn decision(o: &LanguageObservation) -> String {
        match &o.decision {
            LanguageDecision::Identified { language } => language.to_string(),
            LanguageDecision::Abstained { reason } => format!("{reason:?}"),
        }
    }

    #[test]
    fn whichlang_identifies_supported_languages_without_scores() {
        let w = WhichlangIdentifier;
        assert_eq!(w.languages().len(), 16);
        assert!(w.score_semantics().is_none());
        assert!(w.detect("   ").is_empty());
        let h = LanguageHint::Auto;
        for (text, lang) in [
            ("Could you tell me where the train station is", "en"),
            ("Dónde está la estación de tren, por favor", "es"),
            ("Je voudrais un café et un verre d'eau", "fr"),
            ("Wir treffen uns morgen um neun Uhr am Bahnhof", "de"),
        ] {
            let o = observe_utterance(&w, &cfg(), &h, 1, text);
            assert_eq!(decision(&o), lang, "{text}");
            assert!(o.candidates.iter().all(|c| c.score.is_none()));
            assert_eq!(o.score_semantics, None);
            assert_eq!(o.scope, LanguageScope::Utterance { utterance: 1 });
            assert!(matches!(o.source, LanguageSource::TextIdentifier { .. }));
        }
    }

    #[test]
    fn short_text_abstains_and_fixed_hints_are_declared_not_inferred() {
        let w = WhichlangIdentifier;
        let o = observe_utterance(&w, &cfg(), &LanguageHint::Auto, 0, "Hello");
        assert_eq!(decision(&o), "InsufficientEvidence");
        assert!(o.candidates.is_empty());
        assert_eq!(o.evidence, 5);
        let o = observe_utterance(&w, &cfg(), &LanguageHint::Auto, 0, "");
        assert_eq!(decision(&o), "InsufficientEvidence");
        let o = observe_utterance(&w, &cfg(), &LanguageHint::Fixed(t("pt-BR")), 0, "Hi");
        assert_eq!(decision(&o), "pt-BR");
        assert_eq!(o.source, LanguageSource::Declared);
    }

    #[test]
    fn expected_sets_low_scores_and_mixed_halves_abstain() {
        let w = WhichlangIdentifier;
        let en = "Could you tell me where the train station is";
        let o = observe_utterance(
            &w,
            &cfg(),
            &LanguageHint::Expected(vec![t("es"), t("fr")]),
            0,
            en,
        );
        assert_eq!(decision(&o), "Unsupported");
        assert_eq!(
            o.candidates[0].language,
            t("en"),
            "the answer is still shown"
        );
        let o = observe_utterance(&w, &cfg(), &LanguageHint::Expected(vec![t("en-GB")]), 0, en);
        assert_eq!(decision(&o), "en", "primary subtags are compared");
        // Scores are honoured only when declared.
        let scored = ScriptedIdentifier::new(
            vec![t("en")],
            Some(ScoreSemantics::HeuristicConfidence),
            |_| {
                vec![LanguageCandidate {
                    language: LanguageTag::parse("en").unwrap(),
                    score: Some(0.1),
                }]
            },
        );
        let o = observe_utterance(&scored, &cfg(), &LanguageHint::Auto, 0, en);
        assert_eq!(decision(&o), "LowConfidence");
        let unscored = ScriptedIdentifier::new(vec![t("en")], None, |_| {
            vec![LanguageCandidate {
                language: LanguageTag::parse("en").unwrap(),
                score: Some(0.1),
            }]
        });
        let o = observe_utterance(&unscored, &cfg(), &LanguageHint::Auto, 0, en);
        assert_eq!(decision(&o), "en");
        assert_eq!(o.candidates[0].score, None, "undeclared scores are dropped");
        // Halves in different languages: ambiguous, both candidates listed.
        let mixed = ScriptedIdentifier::new(vec![t("en"), t("es")], None, |s| {
            let l = if s.contains("playa") { "es" } else { "en" };
            vec![LanguageCandidate {
                language: LanguageTag::parse(l).unwrap(),
                score: None,
            }]
        });
        let o = observe_utterance(
            &mixed,
            &cfg(),
            &LanguageHint::Auto,
            0,
            "vamos todos a la playa mañana and then we can get ice cream",
        );
        assert_eq!(decision(&o), "Ambiguous");
        assert_eq!(o.candidates.len(), 2);
        // The same check can misfire on short single-language text: here
        // one half of a German sentence is misidentified. It can be
        // switched off (then the whole-text answer is used).
        let de = "Ich möchte bitte einen Kaffee bestellen";
        let o = observe_utterance(&w, &cfg(), &LanguageHint::Auto, 0, de);
        assert_eq!(decision(&o), "Ambiguous");
        let off = LanguageIdConfig {
            split_check: false,
            ..cfg()
        };
        let o = observe_utterance(&w, &off, &LanguageHint::Auto, 0, de);
        assert_eq!(decision(&o), "de");
    }

    #[test]
    fn stream_language_aggregates_decided_utterances_only_and_reports_changes() {
        let w = WhichlangIdentifier;
        let c = cfg();
        let fp = fingerprint(&w, &c);
        let mut s = StreamLanguage::new(c.clone(), &LanguageHint::Auto, fp);
        let obs = |u, text: &str| observe_utterance(&w, &c, &LanguageHint::Auto, u, text);
        let first = s.observe(&obs(0, "Hi")).unwrap();
        assert_eq!(first.scope, LanguageScope::Stream);
        assert_eq!(decision(&first), "InsufficientEvidence");
        assert!(s.observe(&obs(1, "Okay")).is_none(), "no change, no event");
        assert!(
            s.observe(&obs(2, "Could you tell me where the train station is"))
                .is_none()
        );
        let en = s
            .observe(&obs(3, "I would like a cup of coffee, please"))
            .unwrap();
        assert_eq!(decision(&en), "en");
        assert_eq!(en.source, LanguageSource::Aggregate);
        assert_eq!(en.evidence, 2);
        // 2 of 3 decided utterances (0.67 >= 0.6): still English, no event.
        assert!(
            s.observe(&obs(4, "Dónde está la estación de tren, por favor"))
                .is_none()
        );
        // A tie (2 vs 2) is ambiguous.
        let amb = s
            .observe(&obs(5, "Quisiera un café con leche, por favor"))
            .unwrap();
        assert_eq!(decision(&amb), "Ambiguous");
        assert_eq!(amb.candidates.len(), 2);
        // A span-scope observation is never folded into the stream.
        let span = LanguageObservation {
            scope: LanguageScope::Span {
                utterance: 9,
                chars: (0, 4),
            },
            ..obs(9, "Could you tell me where the train station is")
        };
        assert!(s.observe(&span).is_none());
        let mut d = StreamLanguage::new(cfg(), &LanguageHint::Fixed(t("fr")), "x".into());
        assert_eq!(decision(&d.observe(&obs(0, "Hi")).unwrap()), "fr");
    }
}
