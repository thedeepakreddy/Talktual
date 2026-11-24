//! Recognition and language configuration (Phase 4): typed, validated,
//! bounded and fingerprinted.

use serde::{Deserialize, Serialize};

use crate::error::CeError;

use super::model::LanguageTag;

/// How utterances are delimited.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SegmentationMode {
    /// Phase 3 activity onsets open utterances and segment ends / endpoint
    /// candidates finalize them (hints, not rigid boundaries: the adapter's
    /// own endpointing and `max_utterance_ms` still apply). Requires an
    /// analysis running on the stream; falls back to `Continuous` while the
    /// analysis is degraded to `Shedding` or has ended.
    #[default]
    ActivityHints,
    /// Audio is fed continuously; utterances end on the adapter's own
    /// endpoint or `max_utterance_ms`.
    Continuous,
}

/// Stable-prefix (commit) policy. Model-independent: it looks only at
/// successive hypotheses of one utterance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StabilityPolicy {
    /// A word prefix unchanged across this many consecutive hypotheses is
    /// stable (0 disables the count rule). Default 3.
    pub stable_after_hypotheses: u32,
    /// A word prefix unchanged for this much audio time is stable (0
    /// disables the time rule). Default 800 ms.
    pub stable_after_ms: u32,
    /// The last N words of a partial hypothesis are never committed before
    /// finalization (they are the most volatile). Default 1.
    pub holdback_words: u32,
}

impl Default for StabilityPolicy {
    fn default() -> Self {
        Self {
            stable_after_hypotheses: 3,
            stable_after_ms: 800,
            holdback_words: 1,
        }
    }
}

/// When an utterance is finalized.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FinalizationPolicy {
    /// Finalize when the Phase 3 segment ends on its silence policy.
    /// Default true.
    pub on_segment_end: bool,
    /// Finalize earlier, at this Phase 3 endpoint-candidate tier (`None`:
    /// wait for the segment end). Default `None`.
    pub on_endpoint_tier: Option<u8>,
    /// Honour the adapter's own endpoint detection. Default true.
    pub on_adapter_endpoint: bool,
    /// Force a final after this much utterance audio. Default 20 000 ms.
    pub max_utterance_ms: u32,
    /// Audio after the end hint still fed before finalizing (captures word
    /// tails the activity detector cut). Default 200 ms.
    pub trailing_ms: u32,
}

impl Default for FinalizationPolicy {
    fn default() -> Self {
        Self {
            on_segment_end: true,
            on_endpoint_tier: None,
            on_adapter_endpoint: true,
            max_utterance_ms: 20_000,
            trailing_ms: 200,
        }
    }
}

/// Language constraint for recognition and identification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "mode", content = "languages")]
pub enum LanguageHint {
    /// No constraint.
    #[default]
    Auto,
    /// The speech is expected to be one of these (identification is
    /// restricted to them).
    Expected(Vec<LanguageTag>),
    /// The speech is declared to be this language: identification is not
    /// run; observations carry source `Declared`.
    Fixed(LanguageTag),
}

/// Language identification policy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LanguageIdConfig {
    /// Run text language identification on final utterance text. Default true.
    pub enabled: bool,
    /// Abstain below this many letters of evidence. Default 10 (set on the
    /// development split of the language corpus).
    pub min_letters: u32,
    /// Abstain when the identifier's score is below this (see its score
    /// semantics; ignored for identifiers without scores). Default 0.15.
    pub min_score: f32,
    /// Utterance decisions needed before a session-scope decision. Default 2.
    pub session_min_utterances: u32,
    /// Share of decided utterances the session language must hold. Default 0.6.
    pub session_min_share: f32,
    /// Abstain as `Ambiguous` when the two halves of the text (each with at
    /// least max(min_letters, 10) letters) are identified as different
    /// languages: mixed-language evidence. Default true (dev split: costs
    /// 1 of 42 single-language items in coverage, no accuracy change).
    pub split_check: bool,
}

impl Default for LanguageIdConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_letters: 10,
            min_score: 0.15,
            session_min_utterances: 2,
            session_min_share: 0.6,
            split_check: true,
        }
    }
}

/// Per-stream resource budget.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RecognitionBudget {
    /// Audio tap capacity in blocks. Default 200 (≈ 2 s of 10 ms blocks).
    pub queue_blocks: u32,
    /// Phase 3 hint queue capacity. Default 256.
    pub hint_queue: u32,
    /// Load (processing time / audio time) above which a block counts as an
    /// overrun. Default 0.8.
    pub max_load: f32,
    /// Consecutive overruns (or backlog above half the tap) before
    /// throttling. Default 10.
    pub degrade_after: u32,
    /// Consecutive healthy blocks before un-throttling. Default 100.
    pub recover_after: u32,
    /// No progress for this long → stalled. Default 5000 ms.
    pub stall_timeout_ms: u64,
    /// Model load deadline. Default 60 000 ms.
    pub load_timeout_ms: u64,
    /// Allow throttling. Default true.
    pub degrade: bool,
}

impl Default for RecognitionBudget {
    fn default() -> Self {
        Self {
            queue_blocks: 200,
            hint_queue: 256,
            max_load: 0.8,
            degrade_after: 10,
            recover_after: 100,
            stall_timeout_ms: 5_000,
            load_timeout_ms: 60_000,
            degrade: true,
        }
    }
}

/// Complete recognition configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RecognitionConfig {
    /// Adapter id (`None`: the engine's default adapter).
    pub adapter: Option<String>,
    /// Utterance delimitation.
    pub segmentation: SegmentationMode,
    /// Audio per adapter call, ms (10..=1000). Default 100.
    pub chunk_ms: u32,
    /// Audio kept before an activity onset so the utterance can start at
    /// the backdated boundary minus `lead_in_ms` even when the Phase 3 hint
    /// arrives late, ms (0..=2000). Default 1000 (≈ 64 KB per stream at
    /// 16 kHz).
    pub preroll_ms: u32,
    /// Audio fed before a Phase 3 activity onset, ms (0..=preroll_ms,
    /// at most 1000). Energy-based onsets fire after soft word beginnings
    /// (unvoiced /t/, /h/, …); a real recognizer needs that audio. The
    /// utterance's frame range includes it (nothing is shifted). Default
    /// 500, chosen on the dev split with the real sherpa-onnx baseline
    /// (pipeline WER 0.271 at 0 ms; 0.115–0.125 from 500 to 800 ms; the
    /// smallest best value is used; ADR-0023).
    pub lead_in_ms: u32,
    /// Emit partial hypotheses (if the adapter supports them). Default true.
    pub partials: bool,
    /// Minimum audio time between partial revisions, ms. Default 200.
    pub partial_interval_ms: u32,
    /// Commit policy.
    pub stability: StabilityPolicy,
    /// Finalization policy.
    pub finalization: FinalizationPolicy,
    /// Language constraint.
    pub language: LanguageHint,
    /// Language identification.
    pub language_id: LanguageIdConfig,
    /// Include word timings when the adapter provides them. Default true.
    pub word_timings: bool,
    /// Budget.
    pub budget: RecognitionBudget,
}

impl Default for RecognitionConfig {
    fn default() -> Self {
        Self {
            adapter: None,
            segmentation: SegmentationMode::ActivityHints,
            chunk_ms: 100,
            preroll_ms: 1_000,
            lead_in_ms: 500,
            partials: true,
            partial_interval_ms: 200,
            stability: StabilityPolicy::default(),
            finalization: FinalizationPolicy::default(),
            language: LanguageHint::Auto,
            language_id: LanguageIdConfig::default(),
            word_timings: true,
            budget: RecognitionBudget::default(),
        }
    }
}

/// Engine-wide speech settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpeechConfig {
    /// Default per-stream recognition configuration.
    pub recognition: RecognitionConfig,
    /// Concurrent recognitions across the engine (1..=64). Default 4.
    pub max_concurrent_recognitions: u32,
}

impl Default for SpeechConfig {
    fn default() -> Self {
        Self {
            recognition: RecognitionConfig::default(),
            max_concurrent_recognitions: 4,
        }
    }
}

fn bad(field: &'static str, allowed: &'static str) -> CeError {
    CeError::invalid_config(
        "config.recognition_invalid",
        "recognition configuration value is invalid",
    )
    .with_context("field", field)
    .with_context("allowed", allowed)
}

impl RecognitionConfig {
    /// Validate ranges and relationships.
    pub fn validate(&self) -> Result<(), CeError> {
        if let Some(a) = &self.adapter {
            if a.is_empty()
                || a.len() > 64
                || !a
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
            {
                return Err(bad("adapter", "1..=64 chars of [A-Za-z0-9._-]"));
            }
        }
        if !(10..=1_000).contains(&self.chunk_ms) {
            return Err(bad("chunk_ms", "10..=1000"));
        }
        if self.preroll_ms > 2_000 {
            return Err(bad("preroll_ms", "0..=2000"));
        }
        if self.lead_in_ms > self.preroll_ms.min(1_000) {
            return Err(bad("lead_in_ms", "0..=min(preroll_ms, 1000)"));
        }
        if !(50..=5_000).contains(&self.partial_interval_ms) {
            return Err(bad("partial_interval_ms", "50..=5000"));
        }
        let s = &self.stability;
        if s.stable_after_hypotheses > 100 || s.stable_after_ms > 10_000 || s.holdback_words > 10 {
            return Err(bad(
                "stability",
                "hypotheses <= 100, ms <= 10000, holdback <= 10",
            ));
        }
        if s.stable_after_hypotheses == 0 && s.stable_after_ms == 0 {
            return Err(bad(
                "stability",
                "at least one of stable_after_hypotheses / stable_after_ms must be > 0",
            ));
        }
        let f = &self.finalization;
        if !(1_000..=120_000).contains(&f.max_utterance_ms) || f.trailing_ms > 1_000 {
            return Err(bad(
                "finalization",
                "max_utterance_ms 1000..=120000, trailing_ms <= 1000",
            ));
        }
        if f.on_endpoint_tier.is_some_and(|t| t > 7) {
            return Err(bad("finalization.on_endpoint_tier", "0..=7"));
        }
        match &self.language {
            LanguageHint::Expected(v) if v.is_empty() || v.len() > 32 => {
                return Err(bad("language", "1..=32 expected languages"));
            }
            _ => {}
        }
        let l = &self.language_id;
        if l.min_letters > 10_000
            || !(0.0..=1.0).contains(&l.min_score)
            || l.session_min_utterances == 0
            || !(0.0..=1.0).contains(&l.session_min_share)
        {
            return Err(bad(
                "language_id",
                "min_letters <= 10000, min_score and session_min_share in [0, 1], session_min_utterances >= 1",
            ));
        }
        let b = &self.budget;
        if !(4..=10_000).contains(&b.queue_blocks)
            || !(4..=10_000).contains(&b.hint_queue)
            || !(b.max_load > 0.0 && b.max_load <= 10.0)
            || b.degrade_after == 0
            || b.recover_after == 0
            || !(100..=600_000).contains(&b.stall_timeout_ms)
            || !(100..=600_000).contains(&b.load_timeout_ms)
        {
            return Err(bad(
                "budget",
                "queues 4..=10000, max_load (0,10], counts >= 1, timeouts 100..=600000 ms",
            ));
        }
        Ok(())
    }

    /// A stable 64-bit fingerprint of the configuration (FNV-1a over its
    /// canonical debug form; stable within a CE release), 16 hex digits.
    pub fn fingerprint(&self) -> String {
        fnv(&format!("speech.v1|{self:?}"))
    }
}

impl SpeechConfig {
    /// Validate.
    pub fn validate(&self) -> Result<(), CeError> {
        if !(1..=64).contains(&self.max_concurrent_recognitions) {
            return Err(bad("max_concurrent_recognitions", "1..=64"));
        }
        self.recognition.validate()
    }
}

pub(crate) fn fnv(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate_and_round_trip() {
        let c = RecognitionConfig::default();
        c.validate().unwrap();
        SpeechConfig::default().validate().unwrap();
        let json = serde_json::to_string(&c).unwrap();
        let back: RecognitionConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.fingerprint(), c.fingerprint());
        let partial: RecognitionConfig = serde_json::from_str(
            r#"{"chunk_ms": 50, "language": {"mode": "fixed", "languages": "es"}}"#,
        )
        .unwrap();
        assert_eq!(partial.chunk_ms, 50);
        assert_eq!(
            partial.language,
            LanguageHint::Fixed(LanguageTag::parse("es").unwrap())
        );
        assert!(serde_json::from_str::<RecognitionConfig>(r#"{"bogus": 1}"#).is_err());
    }

    #[test]
    fn invalid_values_are_rejected_with_one_code() {
        type Mutation = Box<dyn Fn(&mut RecognitionConfig)>;
        let cases: Vec<Mutation> = vec![
            Box::new(|c| c.chunk_ms = 5),
            Box::new(|c| c.preroll_ms = 5_000),
            Box::new(|c| c.lead_in_ms = 1_200),
            Box::new(|c| c.partial_interval_ms = 10),
            Box::new(|c| {
                c.stability.stable_after_hypotheses = 0;
                c.stability.stable_after_ms = 0;
            }),
            Box::new(|c| c.finalization.max_utterance_ms = 10),
            Box::new(|c| c.finalization.on_endpoint_tier = Some(9)),
            Box::new(|c| c.language = LanguageHint::Expected(vec![])),
            Box::new(|c| c.language_id.min_score = 1.5),
            Box::new(|c| c.budget.queue_blocks = 1),
            Box::new(|c| c.adapter = Some("bad id!".into())),
        ];
        for (i, m) in cases.iter().enumerate() {
            let mut c = RecognitionConfig::default();
            m(&mut c);
            let e = c.validate().unwrap_err();
            assert_eq!(e.code(), "config.recognition_invalid", "case {i}");
        }
        let s = SpeechConfig {
            max_concurrent_recognitions: 0,
            ..SpeechConfig::default()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn fingerprints_change_with_configuration() {
        let a = RecognitionConfig::default();
        let mut b = a.clone();
        b.stability.holdback_words = 2;
        assert_ne!(a.fingerprint(), b.fingerprint());
        assert_eq!(a.fingerprint().len(), 16);
    }
}
