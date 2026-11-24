//! Deterministic adapters for tests, replay and pipeline evaluation.
//!
//! * [`ScriptedFactory`] / scripted adapter (`ce.asr.scripted`): plays a
//!   script of hypotheses keyed by utterance audio time, with optional
//!   adapter endpoints, malformed output, errors, panics, stalls and load
//!   delay / failure / panic. A shared-model counter shows load reuse.
//! * [`ReferenceFactory`] / reference adapter (`ce.asr.reference`): an
//!   **oracle**. It knows the reference transcript and its time spans and
//!   reveals reference words in proportion to the audio consumed, with
//!   optional deterministic revision noise and substitutions. It measures
//!   the *pipeline* (segmentation, timing, revisions, commit policy,
//!   finalization), never recognition accuracy: its WER is not an ASR
//!   result.
//!
//! Neither performs speech recognition, and neither claims capabilities it
//! does not have (no punctuation, casing, language reports or scores; word
//! timings only when explicitly enabled on the scripted adapter, and then
//! they are synthetic).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering::SeqCst;
use std::time::Duration;

use ce_core::CeError;
use ce_core::ErrorCategory;
use ce_core::speech::{AsrCapabilities, LanguageHint, LanguageSupport, LanguageTag};

use crate::adapter::{AdapterHypothesis, AdapterWord, AsrAdapter, AsrFactory, LoadOutcome};

/// Adapter rate of both fakes.
pub const FAKE_RATE: u32 = 16_000;

fn base_caps(adapter: &str) -> AsrCapabilities {
    AsrCapabilities {
        adapter: adapter.into(),
        version: "1.0.0".into(),
        model: None,
        streaming: true,
        partial_results: true,
        revises_partials: true,
        word_timestamps: false,
        punctuation: false,
        casing: false,
        languages: LanguageSupport::Unknown,
        reports_language: false,
        endpointing: true,
        sample_rate: FAKE_RATE,
        max_utterance_ms: None,
        score_semantics: None,
        deterministic: true,
        max_concurrent_streams: None,
        requires_load: true,
    }
}

/// One scripted hypothesis.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScriptedHypothesis {
    /// Utterance audio time (ms) from which it is available.
    pub at_ms: u32,
    /// Text (may be malformed on purpose).
    pub text: String,
    /// Utterance score.
    pub score: Option<f32>,
    /// Explicit word timings (utterance samples); `None` → synthetic,
    /// uniformly spread timings when the factory declares word timestamps.
    pub words: Option<Vec<AdapterWord>>,
    /// Reported language.
    pub language: Option<LanguageTag>,
}

/// Script of one utterance.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UtteranceScript {
    /// Hypotheses in time order.
    pub hypotheses: Vec<ScriptedHypothesis>,
    /// The adapter reports an endpoint once this much audio was fed.
    pub endpoint_at_ms: Option<u32>,
    /// Final text (`None`: the latest available hypothesis).
    pub final_text: Option<String>,
}

impl UtteranceScript {
    /// Hypotheses at the given times.
    pub fn timed(steps: &[(u32, &str)]) -> Self {
        Self {
            hypotheses: steps
                .iter()
                .map(|(at_ms, text)| ScriptedHypothesis {
                    at_ms: *at_ms,
                    text: (*text).into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    /// Reveal `text` one word every `step_ms`.
    pub fn progressive(text: &str, step_ms: u32) -> Self {
        let words: Vec<&str> = text.split_whitespace().collect();
        let steps: Vec<(u32, String)> = (1..=words.len())
            .map(|k| (k as u32 * step_ms, words[..k].join(" ")))
            .collect();
        let refs: Vec<(u32, &str)> = steps.iter().map(|(t, s)| (*t, s.as_str())).collect();
        Self::timed(&refs)
    }

    /// With an adapter endpoint.
    pub fn endpoint_at(mut self, ms: u32) -> Self {
        self.endpoint_at_ms = Some(ms);
        self
    }

    /// With an explicit final text.
    pub fn final_text(mut self, text: &str) -> Self {
        self.final_text = Some(text.into());
        self
    }
}

/// An injected adapter fault (each fires once per adapter instance).
#[derive(Clone, Debug, PartialEq)]
pub enum ScriptedFault {
    /// `accept` returns an error at this utterance time.
    Error {
        /// Utterance (`None`: any).
        utterance: Option<u64>,
        /// Audio time, ms.
        at_ms: u32,
    },
    /// Every `accept` from this utterance on fails (for "repeated errors").
    AlwaysError {
        /// First failing utterance.
        from_utterance: u64,
    },
    /// `accept` panics.
    Panic {
        /// Utterance.
        utterance: u64,
        /// Audio time, ms.
        at_ms: u32,
    },
    /// `accept` blocks for `ms`.
    Stall {
        /// Utterance.
        utterance: u64,
        /// Audio time, ms.
        at_ms: u32,
        /// Duration, ms.
        ms: u64,
    },
    /// `finalize` fails.
    FinalizeError {
        /// Utterance.
        utterance: u64,
    },
}

/// Load behaviour.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadBehavior {
    /// Sleep this long in `load`.
    pub delay_ms: u64,
    /// `load` returns an error.
    pub fail: bool,
    /// `load` panics.
    pub panic: bool,
}

/// Shared counters of a scripted factory.
#[derive(Debug, Default)]
pub struct ScriptedStats {
    /// Adapters created.
    pub created: AtomicU32,
    /// Loads performed (not reused).
    pub loads: AtomicU32,
    /// Loads that reused a live model.
    pub reuses: AtomicU32,
    /// Adapters currently holding the model.
    pub live: AtomicU32,
}

/// Factory of scripted adapters.
#[derive(Debug, Clone)]
pub struct ScriptedFactory {
    caps: AsrCapabilities,
    script: Arc<Vec<UtteranceScript>>,
    faults: Arc<Vec<ScriptedFault>>,
    load: LoadBehavior,
    stats: Arc<ScriptedStats>,
}

impl ScriptedFactory {
    /// Utterance `u` plays `script[u % len]` (an empty script yields no
    /// hypotheses).
    pub fn new(script: Vec<UtteranceScript>) -> Self {
        Self {
            caps: base_caps("ce.asr.scripted"),
            script: Arc::new(script),
            faults: Arc::new(Vec::new()),
            load: LoadBehavior::default(),
            stats: Arc::new(ScriptedStats::default()),
        }
    }

    /// Modify the declared capabilities (e.g. turn word timings on, or
    /// change the adapter id to register several).
    pub fn with_capabilities(mut self, f: impl FnOnce(&mut AsrCapabilities)) -> Self {
        f(&mut self.caps);
        self
    }

    /// Inject a fault.
    pub fn with_fault(mut self, fault: ScriptedFault) -> Self {
        Arc::make_mut(&mut self.faults).push(fault);
        self
    }

    /// Load behaviour.
    pub fn with_load(mut self, load: LoadBehavior) -> Self {
        self.load = load;
        self
    }

    /// Shared counters.
    pub fn stats(&self) -> Arc<ScriptedStats> {
        self.stats.clone()
    }
}

impl AsrFactory for ScriptedFactory {
    fn capabilities(&self) -> AsrCapabilities {
        self.caps.clone()
    }

    fn create(&self) -> Result<Box<dyn AsrAdapter>, CeError> {
        self.stats.created.fetch_add(1, SeqCst);
        Ok(Box::new(ScriptedAdapter {
            caps: self.caps.clone(),
            script: self.script.clone(),
            faults: self.faults.clone(),
            fired: BTreeSet::new(),
            load: self.load.clone(),
            stats: self.stats.clone(),
            holds_model: false,
            utterance: None,
            consumed: 0,
            next: 0,
            endpointed: false,
        }))
    }
}

#[derive(Debug)]
struct ScriptedAdapter {
    caps: AsrCapabilities,
    script: Arc<Vec<UtteranceScript>>,
    faults: Arc<Vec<ScriptedFault>>,
    fired: BTreeSet<usize>,
    load: LoadBehavior,
    stats: Arc<ScriptedStats>,
    holds_model: bool,
    utterance: Option<u64>,
    consumed: u64,
    next: usize,
    endpointed: bool,
}

impl Drop for ScriptedAdapter {
    fn drop(&mut self) {
        if self.holds_model {
            self.stats.live.fetch_sub(1, SeqCst);
        }
    }
}

fn adapter_error(code: &'static str) -> CeError {
    CeError::new(ErrorCategory::Provider, code, "scripted adapter fault")
}

impl ScriptedAdapter {
    fn script(&self) -> Option<&UtteranceScript> {
        let u = self.utterance?;
        if self.script.is_empty() {
            return None;
        }
        self.script.get((u % self.script.len() as u64) as usize)
    }

    fn now_ms(&self) -> u32 {
        u32::try_from(self.consumed * 1_000 / u64::from(self.caps.sample_rate)).unwrap_or(u32::MAX)
    }

    fn with_words(&self, mut h: ScriptedHypothesis) -> AdapterHypothesis {
        if h.words.is_none() && self.caps.word_timestamps {
            let words: Vec<&str> = h.text.split_whitespace().collect();
            let span =
                (u64::from(h.at_ms) * u64::from(self.caps.sample_rate) / 1_000).min(self.consumed);
            let n = words.len() as u64;
            h.words = Some(
                words
                    .iter()
                    .enumerate()
                    .map(|(k, w)| AdapterWord {
                        text: (*w).into(),
                        start: span * k as u64 / n.max(1),
                        end: span * (k as u64 + 1) / n.max(1),
                        score: None,
                    })
                    .collect(),
            );
        }
        AdapterHypothesis {
            text: h.text,
            words: h.words,
            score: h.score,
            endpoint: false,
            language: h.language,
        }
    }

    fn latest(&self) -> Option<ScriptedHypothesis> {
        let now = self.now_ms();
        self.script()?
            .hypotheses
            .iter()
            .rev()
            .find(|h| h.at_ms <= now)
            .cloned()
    }
}

impl AsrAdapter for ScriptedAdapter {
    fn load(&mut self) -> Result<LoadOutcome, CeError> {
        if self.load.delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(self.load.delay_ms));
        }
        if self.load.panic {
            panic!("scripted adapter: load panic");
        }
        if self.load.fail {
            return Err(adapter_error("asr.scripted_load_failed"));
        }
        let reused = self.stats.live.fetch_add(1, SeqCst) > 0;
        self.holds_model = true;
        if reused {
            self.stats.reuses.fetch_add(1, SeqCst);
        } else {
            self.stats.loads.fetch_add(1, SeqCst);
        }
        Ok(LoadOutcome { reused })
    }

    fn begin(&mut self, utterance: u64, _language: &LanguageHint) -> Result<(), CeError> {
        self.utterance = Some(utterance);
        self.consumed = 0;
        self.next = 0;
        self.endpointed = false;
        Ok(())
    }

    fn accept(&mut self, samples: &[f32]) -> Result<Option<AdapterHypothesis>, CeError> {
        let u = self.utterance.unwrap_or(0);
        self.consumed += samples.len() as u64;
        let now = self.now_ms();
        for (i, f) in self.faults.clone().iter().enumerate() {
            match f {
                ScriptedFault::AlwaysError { from_utterance } if u >= *from_utterance => {
                    return Err(adapter_error("asr.scripted_error"));
                }
                _ if self.fired.contains(&i) => {}
                ScriptedFault::Error { utterance, at_ms }
                    if utterance.is_none_or(|x| x == u) && now >= *at_ms =>
                {
                    self.fired.insert(i);
                    return Err(adapter_error("asr.scripted_error"));
                }
                ScriptedFault::Panic { utterance, at_ms } if *utterance == u && now >= *at_ms => {
                    self.fired.insert(i);
                    panic!("scripted adapter: accept panic");
                }
                ScriptedFault::Stall {
                    utterance,
                    at_ms,
                    ms,
                } if *utterance == u && now >= *at_ms => {
                    self.fired.insert(i);
                    std::thread::sleep(Duration::from_millis(*ms));
                }
                _ => {}
            }
        }
        let Some(script) = self.script().cloned() else {
            return Ok(None);
        };
        let avail = script.hypotheses.iter().rposition(|h| h.at_ms <= now);
        let mut out = None;
        if let Some(i) = avail {
            if i + 1 > self.next {
                self.next = i + 1;
                out = Some(self.with_words(script.hypotheses[i].clone()));
            }
        }
        if !self.endpointed && script.endpoint_at_ms.is_some_and(|e| now >= e) {
            self.endpointed = true;
            let mut h = match out {
                Some(h) => h,
                None => self
                    .latest()
                    .map(|h| self.with_words(h))
                    .unwrap_or_default(),
            };
            h.endpoint = true;
            out = Some(h);
        }
        Ok(out)
    }

    fn finalize(&mut self) -> Result<AdapterHypothesis, CeError> {
        let u = self.utterance.unwrap_or(0);
        for (i, f) in self.faults.iter().enumerate() {
            if let ScriptedFault::FinalizeError { utterance } = f {
                if *utterance == u && !self.fired.contains(&i) {
                    self.fired.insert(i);
                    return Err(adapter_error("asr.scripted_finalize_failed"));
                }
            }
        }
        let fin = self.script().and_then(|s| s.final_text.clone());
        let h = match fin {
            Some(text) => AdapterHypothesis {
                text,
                ..Default::default()
            },
            None => self
                .latest()
                .map(|h| self.with_words(h))
                .unwrap_or_default(),
        };
        self.utterance = None;
        Ok(h)
    }

    fn reset(&mut self) {
        self.utterance = None;
        self.consumed = 0;
        self.next = 0;
    }
}

/// A reference utterance on the adapter-rate timeline (samples since the
/// recognizer's anchor; for replay from the start of the stream that is
/// simply the audio position at the adapter rate).
#[derive(Clone, Debug, PartialEq)]
pub struct ReferenceUtterance {
    /// First speech sample.
    pub start: u64,
    /// One past the last speech sample.
    pub end: u64,
    /// Normalized reference words.
    pub words: Vec<String>,
}

/// Oracle behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ReferenceNoise {
    /// Share of partial hypotheses whose last word is shown truncated (a
    /// provisional correction follows).
    pub truncate_rate: f32,
    /// Share of partial hypotheses in which the word two back from the end
    /// is replaced by "uh" (a correction of an earlier word).
    pub rewrite_rate: f32,
    /// Share of reference words substituted by "uh" in every hypothesis
    /// including the final (a fixed error floor).
    pub substitution_rate: f32,
    /// Seed.
    pub seed: u64,
}

/// Factory of reference (oracle) adapters.
#[derive(Debug, Clone)]
pub struct ReferenceFactory {
    refs: Arc<Vec<ReferenceUtterance>>,
    noise: ReferenceNoise,
}

impl ReferenceFactory {
    /// Adapter id.
    pub const ID: &'static str = "ce.asr.reference";

    /// Over `refs` (sorted by start).
    pub fn new(refs: Vec<ReferenceUtterance>, noise: ReferenceNoise) -> Self {
        Self {
            refs: Arc::new(refs),
            noise,
        }
    }
}

impl AsrFactory for ReferenceFactory {
    fn capabilities(&self) -> AsrCapabilities {
        AsrCapabilities {
            revises_partials: self.noise.truncate_rate > 0.0 || self.noise.rewrite_rate > 0.0,
            endpointing: false,
            requires_load: false,
            ..base_caps(Self::ID)
        }
    }

    fn create(&self) -> Result<Box<dyn AsrAdapter>, CeError> {
        Ok(Box::new(ReferenceAdapter {
            refs: self.refs.clone(),
            noise: self.noise,
            start: 0,
            consumed: 0,
            hypotheses: 0,
            utterance: 0,
            last: String::new(),
        }))
    }
}

#[derive(Debug)]
struct ReferenceAdapter {
    refs: Arc<Vec<ReferenceUtterance>>,
    noise: ReferenceNoise,
    start: u64,
    consumed: u64,
    hypotheses: u64,
    utterance: u64,
    last: String,
}

/// Deterministic hash → [0, 1).
fn unit(seed: u64, a: u64, b: u64) -> f32 {
    let mut x =
        seed ^ a.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ b.wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^= x >> 33;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

impl ReferenceAdapter {
    /// Words whose (uniformly assumed) end falls in (start, pos].
    fn revealed(&self) -> Vec<String> {
        let pos = self.start + self.consumed;
        let mut out = Vec::new();
        for (i, r) in self.refs.iter().enumerate() {
            if r.start >= pos || r.end <= self.start || r.words.is_empty() {
                continue;
            }
            let n = r.words.len() as u64;
            let dur = r.end.saturating_sub(r.start).max(1);
            for (k, w) in r.words.iter().enumerate() {
                let at = r.start + dur * (k as u64 + 1) / n;
                if at > self.start && at <= pos {
                    let sub =
                        unit(self.noise.seed, i as u64, k as u64) < self.noise.substitution_rate;
                    out.push(if sub { "uh".to_owned() } else { w.clone() });
                }
            }
        }
        out
    }
}

impl AsrAdapter for ReferenceAdapter {
    fn load(&mut self) -> Result<LoadOutcome, CeError> {
        Ok(LoadOutcome { reused: false })
    }

    fn position_hint(&mut self, absolute_start_sample: u64) {
        self.start = absolute_start_sample;
    }

    fn begin(&mut self, utterance: u64, _language: &LanguageHint) -> Result<(), CeError> {
        self.utterance = utterance;
        self.consumed = 0;
        self.last.clear();
        Ok(())
    }

    fn accept(&mut self, samples: &[f32]) -> Result<Option<AdapterHypothesis>, CeError> {
        self.consumed += samples.len() as u64;
        let mut words = self.revealed();
        self.hypotheses += 1;
        let (u, h, n) = (self.utterance, self.hypotheses, self.noise);
        if !words.is_empty() && unit(n.seed ^ 1, u, h) < n.truncate_rate {
            if let Some(last) = words.last_mut() {
                let keep = last.chars().count().div_ceil(2);
                *last = last.chars().take(keep).collect();
            }
        }
        if words.len() >= 3 && unit(n.seed ^ 2, u, h) < n.rewrite_rate {
            let i = words.len() - 3;
            words[i] = "uh".into();
        }
        let text = words.join(" ");
        if text == self.last {
            return Ok(None);
        }
        self.last.clone_from(&text);
        Ok(Some(AdapterHypothesis {
            text,
            ..Default::default()
        }))
    }

    fn finalize(&mut self) -> Result<AdapterHypothesis, CeError> {
        let text = self.revealed().join(" ");
        self.start += self.consumed;
        self.consumed = 0;
        Ok(AdapterHypothesis {
            text,
            ..Default::default()
        })
    }

    fn reset(&mut self) {
        self.start += self.consumed;
        self.consumed = 0;
        self.last.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(a: &mut dyn AsrAdapter, ms: u32) -> Option<AdapterHypothesis> {
        a.accept(&vec![0.0; (FAKE_RATE / 1_000 * ms) as usize])
            .unwrap()
    }

    #[test]
    fn scripted_adapter_plays_hypotheses_endpoints_and_finals() {
        let f = ScriptedFactory::new(vec![
            UtteranceScript::progressive("the cat sat", 200).endpoint_at(700),
        ]);
        let caps = f.capabilities();
        assert_eq!(caps.adapter, "ce.asr.scripted");
        assert_eq!(f.stats().created.load(SeqCst), 0, "discovery is free");
        let mut a = f.create().unwrap();
        assert!(!a.load().unwrap().reused);
        a.begin(0, &LanguageHint::Auto).unwrap();
        assert!(feed(a.as_mut(), 100).is_none());
        assert_eq!(feed(a.as_mut(), 100).unwrap().text, "the");
        assert!(feed(a.as_mut(), 100).is_none());
        assert_eq!(feed(a.as_mut(), 300).unwrap().text, "the cat sat");
        let e = feed(a.as_mut(), 100).unwrap();
        assert!(e.endpoint && e.text == "the cat sat");
        assert_eq!(a.finalize().unwrap().text, "the cat sat");
        // A second adapter reuses the live model.
        let mut b = f.create().unwrap();
        assert!(b.load().unwrap().reused);
        drop(a);
        drop(b);
        assert_eq!(f.stats().live.load(SeqCst), 0);
    }

    #[test]
    fn scripted_word_timings_are_synthetic_and_only_when_declared() {
        let f = ScriptedFactory::new(vec![UtteranceScript::timed(&[(200, "a b")])])
            .with_capabilities(|c| c.word_timestamps = true);
        let mut a = f.create().unwrap();
        a.begin(0, &LanguageHint::Auto).unwrap();
        let h = feed(a.as_mut(), 300).unwrap();
        let w = h.words.unwrap();
        assert_eq!((w[0].start, w[0].end, w[1].end), (0, 1_600, 3_200));
    }

    #[test]
    fn reference_adapter_reveals_words_in_proportion_and_respects_utterance_starts() {
        let refs = vec![ReferenceUtterance {
            start: 16_000,
            end: 32_000,
            words: vec!["one".into(), "two".into(), "three".into(), "four".into()],
        }];
        let f = ReferenceFactory::new(refs.clone(), ReferenceNoise::default());
        assert!(!f.capabilities().endpointing);
        let mut a = f.create().unwrap();
        a.position_hint(16_000);
        a.begin(0, &LanguageHint::Auto).unwrap();
        assert_eq!(feed(a.as_mut(), 250).unwrap().text, "one");
        assert!(feed(a.as_mut(), 100).is_none(), "unchanged → no hypothesis");
        assert_eq!(feed(a.as_mut(), 400).unwrap().text, "one two three");
        assert_eq!(a.finalize().unwrap().text, "one two three");
        // The next utterance continues where the last one ended.
        a.begin(1, &LanguageHint::Auto).unwrap();
        assert_eq!(feed(a.as_mut(), 500).unwrap().text, "four");
        // Substitutions are deterministic and also in finals.
        let f = ReferenceFactory::new(
            refs,
            ReferenceNoise {
                substitution_rate: 1.0,
                ..Default::default()
            },
        );
        let mut a = f.create().unwrap();
        a.position_hint(16_000);
        a.begin(0, &LanguageHint::Auto).unwrap();
        feed(a.as_mut(), 1_000);
        assert_eq!(a.finalize().unwrap().text, "uh uh uh uh");
    }
}
