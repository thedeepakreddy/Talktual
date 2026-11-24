//! A deterministic scripted adapter for lifecycle, race and failure tests.
//! No network, model or credential.
//!
//! The script maps source text to output with a closure and can inject:
//! per-call delay, a panic at call `n`, an error at call `n`, a hang (long
//! delay) at call `n`, malformed output, no-coverage answers and a detected
//! source language. Calls are counted from creation (or reset).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Duration;

use ce_core::intel::ScoreSemantics;
use ce_core::semantic::Locality;
use ce_core::speech::LanguageTag;
use ce_core::translation::{LanguagePair, TranslationCapabilities, TranslationConfig};
use ce_core::{CeError, ErrorCategory};

use crate::adapter::{AdapterOutcome, TranslationAdapter, TranslationFactory, TranslationRequest};

type Script = dyn Fn(&TranslationRequest<'_>) -> AdapterOutcome + Send + Sync;

/// Faults to inject.
#[derive(Clone, Debug, Default)]
pub struct Faults {
    /// Sleep this long on every call.
    pub delay: Duration,
    /// Panic on this call (0-based).
    pub panic_at: Option<u64>,
    /// Return an error on these calls.
    pub error_at: Vec<u64>,
    /// Sleep `hang` on this call.
    pub hang_at: Option<(u64, Duration)>,
    /// Fail `load`.
    pub load_error: bool,
}

/// Factory for [`ScriptedAdapter`]s.
#[derive(Clone)]
pub struct ScriptedFactory {
    caps: TranslationCapabilities,
    script: Arc<Script>,
    faults: Faults,
    calls: Arc<AtomicU64>,
}

impl std::fmt::Debug for ScriptedFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptedFactory")
            .field("adapter", &self.caps.adapter)
            .finish_non_exhaustive()
    }
}

/// Capabilities of a scripted adapter for `pairs` (all features on).
pub fn scripted_capabilities(id: &str, pairs: &[(&str, &str)]) -> TranslationCapabilities {
    TranslationCapabilities {
        adapter: id.into(),
        version: "1.0.0".into(),
        locality: Locality::InProcess,
        pairs: pairs
            .iter()
            .map(|(s, t)| LanguagePair {
                source: LanguageTag::parse(s).expect("valid tag"),
                target: LanguageTag::parse(t).expect("valid tag"),
            })
            .collect(),
        detects_source: false,
        incremental_output: false,
        partial_input: true,
        context_hints: true,
        terminology: true,
        placeholders: true,
        max_input_chars: 10_000,
        deterministic: true,
        requires_load: false,
        score_semantics: Some(ScoreSemantics::HeuristicConfidence),
        model: None,
    }
}

impl ScriptedFactory {
    /// A factory whose adapters answer with `script`.
    pub fn new(
        caps: TranslationCapabilities,
        script: impl Fn(&TranslationRequest<'_>) -> AdapterOutcome + Send + Sync + 'static,
    ) -> Self {
        Self {
            caps,
            script: Arc::new(script),
            faults: Faults::default(),
            calls: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Uppercases the text and tags it with the target (`"[es] HELLO"`).
    /// Placeholders are kept.
    pub fn echo(caps: TranslationCapabilities) -> Self {
        Self::new(caps, |r| AdapterOutcome::Translated {
            text: format!("[{}] {}", r.target, r.text.to_uppercase()),
            score: Some(0.9),
            detected_source: None,
        })
    }

    /// With faults.
    pub fn with_faults(mut self, faults: Faults) -> Self {
        self.faults = faults;
        self
    }

    /// Calls made so far by all adapters of this factory (test probe).
    pub fn calls(&self) -> u64 {
        self.calls.load(Relaxed)
    }
}

impl TranslationFactory for ScriptedFactory {
    fn capabilities(&self) -> TranslationCapabilities {
        self.caps.clone()
    }

    fn create(&self, _: &TranslationConfig) -> Result<Box<dyn TranslationAdapter>, CeError> {
        Ok(Box::new(ScriptedAdapter {
            script: self.script.clone(),
            faults: self.faults.clone(),
            calls: self.calls.clone(),
            own_calls: 0,
        }))
    }
}

/// The scripted adapter.
pub struct ScriptedAdapter {
    script: Arc<Script>,
    faults: Faults,
    calls: Arc<AtomicU64>,
    own_calls: u64,
}

impl std::fmt::Debug for ScriptedAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptedAdapter")
            .field("calls", &self.own_calls)
            .finish_non_exhaustive()
    }
}

impl TranslationAdapter for ScriptedAdapter {
    fn load(&mut self) -> Result<(), CeError> {
        if self.faults.load_error {
            return Err(CeError::new(
                ErrorCategory::Provider,
                "translation.load_failed",
                "scripted load failure",
            ));
        }
        Ok(())
    }

    fn translate(&mut self, request: &TranslationRequest<'_>) -> Result<AdapterOutcome, CeError> {
        let n = self.own_calls;
        self.own_calls += 1;
        self.calls.fetch_add(1, Relaxed);
        if !self.faults.delay.is_zero() {
            std::thread::sleep(self.faults.delay);
        }
        if self.faults.panic_at == Some(n) {
            panic!("scripted translation panic at call {n}");
        }
        if let Some((at, d)) = self.faults.hang_at {
            if at == n {
                std::thread::sleep(d);
            }
        }
        if self.faults.error_at.contains(&n) {
            return Err(CeError::new(
                ErrorCategory::Provider,
                "translation.scripted_error",
                "scripted adapter error",
            ));
        }
        Ok((self.script)(request))
    }

    fn reset(&mut self) {
        self.own_calls = 0;
    }
}
