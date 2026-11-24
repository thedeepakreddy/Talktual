//! Translation configuration (Phase 6): typed, validated, bounded,
//! fingerprinted.

use serde::{Deserialize, Serialize};

use crate::error::CeError;
use crate::speech::LanguageTag;

/// Where the source language comes from. An explicit language always wins
/// over detection; an uncertain detection is never silently resolved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "policy")]
pub enum SourceLanguagePolicy {
    /// Use Phase 4's utterance language decision. If Phase 4 abstained,
    /// the adapter's own detection is used when the adapter declares it;
    /// otherwise the utterance is reported `unavailable(uncertain_source)`.
    #[default]
    Detected,
    /// The source language is declared; detection is ignored (a
    /// disagreeing Phase 4 decision is reported as a diagnostic).
    Explicit {
        /// Language.
        language: LanguageTag,
    },
}

/// Which source revisions are translated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PartialPolicy {
    /// Finals only (batch per utterance). Default.
    #[default]
    Off,
    /// Partial revisions: only their committed text is translated; results
    /// are `stable` and superseded by longer stable results and the final.
    Committed,
    /// Partial revisions: the whole snapshot is translated; results are
    /// `provisional`.
    Full,
}

/// Use of Phase 5 context.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ContextPolicy {
    /// Use final Phase 5 entity observations of the stream as hints.
    /// Default true.
    pub use_semantic: bool,
    /// Most hints passed per request (0..=64). Default 16.
    pub max_hints: u32,
    /// Only hints from this many most recent utterances (0 = same
    /// utterance only) (0..=16). Default 1.
    pub max_utterance_distance: u32,
    /// Total characters of hint mentions per request (0..=4096). Default 512.
    pub max_hint_chars: u32,
}

impl Default for ContextPolicy {
    fn default() -> Self {
        Self {
            use_semantic: true,
            max_hints: 16,
            max_utterance_distance: 1,
            max_hint_chars: 512,
        }
    }
}

/// Privacy boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct TranslationPrivacy {
    /// Allow adapters whose locality is `remote` (text leaves the device).
    /// Default false: such adapters are refused at start and are never used
    /// as a fallback.
    pub allow_remote: bool,
}

/// Bounds and failure policy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TranslationBudget {
    /// Pending inputs (4..=1024). A newer partial replaces a queued partial
    /// of the same utterance. Default 32.
    pub queue: u32,
    /// Longest source text, characters (1..=20000). Default 2000.
    pub max_input_chars: u32,
    /// Longest accepted output, characters (1..=40000). Default 4000.
    pub max_output_chars: u32,
    /// A result later than this is discarded as `timeout`, ms
    /// (10..=60000). Default 2000.
    pub deadline_ms: u64,
    /// Consecutive adapter errors that fail the worker (1..=100). Default 5.
    pub max_consecutive_errors: u32,
    /// No heartbeat for this long → stalled, ms (100..=600000). Default 5000.
    pub stall_timeout_ms: u64,
    /// Retries of a failed request on the same adapter (0..=2). Default 0:
    /// no automatic retry.
    pub max_retries: u32,
}

impl Default for TranslationBudget {
    fn default() -> Self {
        Self {
            queue: 32,
            max_input_chars: 2_000,
            max_output_chars: 4_000,
            deadline_ms: 2_000,
            max_consecutive_errors: 5,
            stall_timeout_ms: 5_000,
            max_retries: 0,
        }
    }
}

/// Per-stream translation configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct TranslationConfig {
    /// Adapter id (`None`: the first registered adapter that supports the
    /// pair and privacy policy).
    pub adapter: Option<String>,
    /// Explicit fallback adapters, tried in order only when the selected
    /// adapter reports an unsupported pair or no coverage. Never includes a
    /// remote adapter unless `privacy.allow_remote`. Default empty.
    pub fallback: Vec<String>,
    /// Target language (required to start).
    pub target: Option<LanguageTag>,
    /// Source language policy.
    pub source: SourceLanguagePolicy,
    /// Which revisions are translated.
    pub partials: PartialPolicy,
    /// Terminology profile id (`None`: none).
    pub terminology: Option<String>,
    /// Phase 5 context use.
    pub context: ContextPolicy,
    /// Privacy.
    pub privacy: TranslationPrivacy,
    /// Bounds.
    pub budget: TranslationBudget,
}

/// Engine-wide translation settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TranslationSettings {
    /// Default per-stream configuration.
    pub translation: TranslationConfig,
    /// Concurrent translation workers across the engine (1..=64). Default 4.
    pub max_concurrent: u32,
}

impl Default for TranslationSettings {
    fn default() -> Self {
        Self {
            translation: TranslationConfig::default(),
            max_concurrent: 4,
        }
    }
}

fn bad(field: &'static str, allowed: &'static str) -> CeError {
    CeError::invalid_config(
        "config.translation_invalid",
        "translation configuration value is invalid",
    )
    .with_context("field", field)
    .with_context("allowed", allowed)
}

fn id_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

impl TranslationConfig {
    /// Validate ranges and relationships (the target may be unset here; a
    /// start without a target fails with `translation.no_target`).
    pub fn validate(&self) -> Result<(), CeError> {
        if self.adapter.as_deref().is_some_and(|a| !id_ok(a)) {
            return Err(bad("adapter", "1..=64 chars of [A-Za-z0-9._-]"));
        }
        if self.fallback.len() > 4 || self.fallback.iter().any(|a| !id_ok(a)) {
            return Err(bad("fallback", "at most 4 adapter ids"));
        }
        if self
            .adapter
            .as_ref()
            .is_some_and(|a| self.fallback.contains(a))
        {
            return Err(bad("fallback", "must not repeat the primary adapter"));
        }
        if let (Some(t), SourceLanguagePolicy::Explicit { language }) = (&self.target, &self.source)
        {
            if t.primary() == language.primary() {
                return Err(bad(
                    "target",
                    "must differ from the explicit source language",
                ));
            }
        }
        if self.terminology.as_deref().is_some_and(|a| !id_ok(a)) {
            return Err(bad("terminology", "1..=64 chars of [A-Za-z0-9._-]"));
        }
        let c = &self.context;
        if c.max_hints > 64 || c.max_utterance_distance > 16 || c.max_hint_chars > 4_096 {
            return Err(bad(
                "context",
                "max_hints <= 64, distance <= 16, chars <= 4096",
            ));
        }
        let b = &self.budget;
        if !(4..=1_024).contains(&b.queue)
            || !(1..=20_000).contains(&b.max_input_chars)
            || !(1..=40_000).contains(&b.max_output_chars)
            || !(10..=60_000).contains(&b.deadline_ms)
            || !(1..=100).contains(&b.max_consecutive_errors)
            || !(100..=600_000).contains(&b.stall_timeout_ms)
            || b.max_retries > 2
        {
            return Err(bad(
                "budget",
                "queue 4..=1024, input 1..=20000, output 1..=40000, deadline 10..=60000 ms, errors 1..=100, stall 100..=600000 ms, retries <= 2",
            ));
        }
        Ok(())
    }

    /// A stable 64-bit fingerprint (FNV-1a over the canonical debug form;
    /// stable within a CE release), 16 hex digits.
    pub fn fingerprint(&self) -> String {
        crate::speech::config::fnv(&format!("translation.v1|{self:?}"))
    }
}

impl TranslationSettings {
    /// Validate.
    pub fn validate(&self) -> Result<(), CeError> {
        if !(1..=64).contains(&self.max_concurrent) {
            return Err(bad("max_concurrent", "1..=64"));
        }
        self.translation.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(s: &str) -> LanguageTag {
        LanguageTag::parse(s).unwrap()
    }

    #[test]
    fn defaults_validate_round_trip_and_fingerprint() {
        let c = TranslationConfig::default();
        c.validate().unwrap();
        TranslationSettings::default().validate().unwrap();
        assert_eq!(c.partials, PartialPolicy::Off, "batch by default");
        assert!(!c.privacy.allow_remote, "local only by default");
        assert_eq!(c.budget.max_retries, 0, "no automatic retry by default");
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<TranslationConfig>(&json).unwrap(), c);
        let p: TranslationConfig = serde_json::from_str(
            r#"{"target":"es","source":{"policy":"explicit","language":"en"},"partials":"committed"}"#,
        )
        .unwrap();
        assert_eq!(p.target, Some(tag("es")));
        assert_eq!(
            p.source,
            SourceLanguagePolicy::Explicit {
                language: tag("en")
            }
        );
        assert!(serde_json::from_str::<TranslationConfig>(r#"{"bogus":1}"#).is_err());
        assert_ne!(p.fingerprint(), c.fingerprint());
        assert_eq!(c.fingerprint().len(), 16);
    }

    #[test]
    fn invalid_values_are_rejected_with_one_code() {
        type Mutation = Box<dyn Fn(&mut TranslationConfig)>;
        let cases: Vec<Mutation> = vec![
            Box::new(|c| c.adapter = Some("bad id".into())),
            Box::new(|c| c.fallback = vec!["a".into(); 5]),
            Box::new(|c| {
                c.adapter = Some("a".into());
                c.fallback = vec!["a".into()];
            }),
            Box::new(|c| {
                c.target = Some(tag("en-GB"));
                c.source = SourceLanguagePolicy::Explicit {
                    language: tag("en"),
                };
            }),
            Box::new(|c| c.context.max_hints = 65),
            Box::new(|c| c.budget.queue = 2),
            Box::new(|c| c.budget.max_input_chars = 0),
            Box::new(|c| c.budget.deadline_ms = 1),
            Box::new(|c| c.budget.max_retries = 3),
            Box::new(|c| c.terminology = Some(String::new())),
        ];
        for (i, m) in cases.iter().enumerate() {
            let mut c = TranslationConfig::default();
            m(&mut c);
            assert_eq!(
                c.validate().unwrap_err().code(),
                "config.translation_invalid",
                "case {i}"
            );
        }
        let s = TranslationSettings {
            max_concurrent: 0,
            ..Default::default()
        };
        assert!(s.validate().is_err());
    }
}
