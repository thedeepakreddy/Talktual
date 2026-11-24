//! The ASR adapter boundary.
//!
//! Vendor and model APIs stay behind [`AsrAdapter`]; CE code depends only
//! on this trait and on CE-owned types.
//!
//! **Lifecycle** (all on the recognition worker thread, never in an audio
//! callback):
//! 1. *Discovery*: [`AsrFactory::capabilities`] — cheap, never loads a model.
//! 2. *Creation*: [`AsrFactory::create`] — cheap; returns an unloaded adapter.
//! 3. *Load + warm-up*: [`AsrAdapter::load`] — may be expensive; a factory may
//!    share a loaded model between adapters (`LoadOutcome::reused`). The
//!    runtime enforces `load_timeout_ms` and reports load failure.
//! 4. *Utterances*: `begin` → `accept`* → `finalize`, or `reset` to abandon.
//! 5. *Unload*: dropping the adapter; a shared model is released when its
//!    last user drops it.
//!
//! Input is mono f32 at [`AsrCapabilities::sample_rate`]; positions are in
//! samples **from the start of the utterance**. CE maps them to stream
//! frames exactly; adapters never see or produce stream timestamps.

use ce_core::CeError;
use ce_core::speech::{AsrCapabilities, LanguageHint, LanguageTag};

/// Result of a load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadOutcome {
    /// An already-loaded model was reused.
    pub reused: bool,
}

/// A word with its position in utterance samples [start, end).
#[derive(Clone, Debug, PartialEq)]
pub struct AdapterWord {
    /// Text.
    pub text: String,
    /// Start sample.
    pub start: u64,
    /// End sample.
    pub end: u64,
    /// Score (adapter semantics).
    pub score: Option<f32>,
}

/// A hypothesis for the whole utterance so far (adapters always report
/// full hypotheses; CE derives stability, commits and deltas).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct AdapterHypothesis {
    /// Text.
    pub text: String,
    /// Word timings, if supported.
    pub words: Option<Vec<AdapterWord>>,
    /// Utterance score, if supported.
    pub score: Option<f32>,
    /// The adapter detected the end of speech.
    pub endpoint: bool,
    /// Recognized language, if reported.
    pub language: Option<LanguageTag>,
}

/// A recognizer instance for one stream.
pub trait AsrAdapter: Send {
    /// Load and warm up (expensive; called once before the first utterance).
    fn load(&mut self) -> Result<LoadOutcome, CeError>;
    /// Start an utterance.
    fn begin(&mut self, utterance: u64, language: &LanguageHint) -> Result<(), CeError>;
    /// Feed audio; returns a new hypothesis when the adapter has one.
    fn accept(&mut self, samples: &[f32]) -> Result<Option<AdapterHypothesis>, CeError>;
    /// End the utterance and return its final hypothesis.
    fn finalize(&mut self) -> Result<AdapterHypothesis, CeError>;
    /// Abandon the utterance (no final) and clear state.
    fn reset(&mut self);
    /// Where the next utterance starts: its stream frame converted to the
    /// adapter rate (called just before `begin`).
    /// Real adapters ignore it; the reference (oracle) adapter uses it to
    /// align with reference labels. Default: ignored.
    fn position_hint(&mut self, _absolute_start_sample: u64) {}
}

/// Creates adapters of one kind. Registered with the engine.
pub trait AsrFactory: Send + Sync {
    /// Static capabilities (cheap; must not load a model).
    fn capabilities(&self) -> AsrCapabilities;
    /// A new, unloaded adapter.
    fn create(&self) -> Result<Box<dyn AsrAdapter>, CeError>;
}

/// Problems found in an adapter output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct OutputIssues {
    /// Non-finite or out-of-range scores dropped.
    pub bad_scores: u32,
    /// Word timings dropped (unordered, out of range or reversed).
    pub bad_words: bool,
    /// Text was truncated or had control characters removed.
    pub bad_text: bool,
}

impl OutputIssues {
    /// Whether anything was wrong.
    pub fn any(&self) -> bool {
        self.bad_scores > 0 || self.bad_words || self.bad_text
    }
}

/// Longest text accepted from an adapter, in characters.
pub const MAX_TEXT_CHARS: usize = 10_000;

/// Sanitize an adapter hypothesis in place: never invent data, only drop
/// what is invalid. `consumed` is the number of utterance samples fed.
pub fn sanitize(h: &mut AdapterHypothesis, consumed: u64, word_timestamps: bool) -> OutputIssues {
    let mut issues = OutputIssues::default();
    let cleaned: String = h
        .text
        .chars()
        .filter(|c| !c.is_control() || *c == ' ')
        .take(MAX_TEXT_CHARS)
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned != h.text {
        // Whitespace normalization alone is not an issue.
        let only_ws = h.text.split_whitespace().collect::<Vec<_>>().join(" ") == cleaned;
        issues.bad_text = !only_ws;
        h.text = cleaned;
    }
    if let Some(s) = h.score {
        if !s.is_finite() {
            h.score = None;
            issues.bad_scores += 1;
        }
    }
    if let Some(words) = &mut h.words {
        let mut ok = word_timestamps;
        let mut prev_start = 0;
        for w in words.iter_mut() {
            if w.score.is_some_and(|s| !s.is_finite()) {
                w.score = None;
                issues.bad_scores += 1;
            }
            // Words must be non-empty, non-reversed, inside the audio fed
            // and in start order.
            if w.start > w.end
                || w.end > consumed
                || w.start < prev_start
                || w.text.trim().is_empty()
            {
                ok = false;
            }
            prev_start = w.start;
        }
        if !ok {
            issues.bad_words = word_timestamps;
            h.words = None;
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(t: &str, s: u64, e: u64) -> AdapterWord {
        AdapterWord {
            text: t.into(),
            start: s,
            end: e,
            score: None,
        }
    }

    #[test]
    fn sanitize_drops_invalid_data_and_never_invents() {
        let mut h = AdapterHypothesis {
            text: "  hello \t world ".into(),
            words: Some(vec![word("hello", 0, 100), word("world", 120, 300)]),
            score: Some(f32::NAN),
            ..Default::default()
        };
        let i = sanitize(&mut h, 400, true);
        assert_eq!(h.text, "hello world");
        assert_eq!((i.bad_scores, i.bad_words, i.bad_text), (1, false, false));
        assert!(h.score.is_none() && h.words.is_some());
        // Words beyond the audio consumed or reversed are dropped as a whole.
        let mut h = AdapterHypothesis {
            text: "a\u{7}b".into(),
            words: Some(vec![word("a", 50, 10)]),
            ..Default::default()
        };
        let i = sanitize(&mut h, 400, true);
        assert!(i.bad_words && i.bad_text && h.words.is_none());
        assert_eq!(h.text, "ab");
        let mut h = AdapterHypothesis {
            text: "x".repeat(MAX_TEXT_CHARS + 5),
            words: Some(vec![word("x", 0, 10_000)]),
            ..Default::default()
        };
        let i = sanitize(&mut h, 100, true);
        assert_eq!(h.text.chars().count(), MAX_TEXT_CHARS);
        assert!(i.any() && h.words.is_none());
        // Timings from an adapter that does not declare them are not used.
        let mut h = AdapterHypothesis {
            text: "a".into(),
            words: Some(vec![word("a", 0, 10)]),
            ..Default::default()
        };
        sanitize(&mut h, 100, false);
        assert!(h.words.is_none());
    }
}
