//! `ce.translate.memory` — a deterministic, offline translation-memory
//! adapter.
//!
//! A translation memory (TM) stores human-authored segment pairs with
//! provenance. The adapter answers a request only with a stored target
//! segment whose source matches the request **exactly after
//! normalization** (Unicode lowercase, punctuation removed, whitespace
//! collapsed). Anything else is `NoCoverage`: it never composes, guesses or
//! partially translates. It is therefore exact on covered segments and
//! useless outside them — a phrasebook / domain-memory baseline, not general
//! machine translation (see ADR-0028 and the Phase 6 evaluation).
//!
//! Capabilities: in-process, deterministic, no model, no network; pairs are
//! those present in the memory; no source detection; no partial input (a
//! prefix of an utterance does not match a stored segment); no context
//! hints; no terminology application (terminology is verified by the
//! processor, not applied); no placeholders.

use std::collections::BTreeMap;

use ce_core::semantic::Locality;
use ce_core::speech::LanguageTag;
use ce_core::translation::{LanguagePair, TranslationCapabilities, TranslationConfig};
use ce_core::{CeError, ErrorCategory};
use serde::{Deserialize, Serialize};

use crate::adapter::{AdapterOutcome, TranslationAdapter, TranslationFactory, TranslationRequest};

/// Adapter id.
pub const MEMORY_ID: &str = "ce.translate.memory";
/// Adapter version.
pub const MEMORY_VERSION: &str = "1.0.0";
/// Most segments in one memory.
pub const MAX_SEGMENTS: usize = 100_000;

/// One stored segment pair.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Segment {
    /// Source language.
    pub source_language: LanguageTag,
    /// Target language.
    pub target_language: LanguageTag,
    /// Source text.
    pub source: String,
    /// Target text.
    pub target: String,
    /// Where the pair came from.
    pub provenance: String,
}

/// Normalize for matching: lowercase, drop punctuation and symbols, collapse
/// whitespace.
pub fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() || c == '\'' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A validated, indexed translation memory.
#[derive(Clone, Debug, Default)]
pub struct TranslationMemory {
    index: BTreeMap<(String, String, String), String>,
    pairs: Vec<LanguagePair>,
    fingerprint: String,
    len: usize,
}

impl TranslationMemory {
    /// Build from segments. Fails on conflicting targets for the same
    /// normalized source and pair (a memory must be unambiguous), on empty
    /// texts and on oversize.
    pub fn new(segments: &[Segment]) -> Result<Self, CeError> {
        let bad = |what: &str| {
            CeError::invalid_config("translation.bad_memory", "translation memory is invalid")
                .with_context("problem", what.to_string())
        };
        if segments.len() > MAX_SEGMENTS {
            return Err(bad("too many segments"));
        }
        let mut index = BTreeMap::new();
        let mut pairs: Vec<LanguagePair> = Vec::new();
        let mut canon = String::new();
        for s in segments {
            let key_src = normalize(&s.source);
            if key_src.is_empty() || s.target.trim().is_empty() {
                return Err(bad("empty segment"));
            }
            if s.source.chars().count() > 2_000 || s.target.chars().count() > 4_000 {
                return Err(bad("segment too long"));
            }
            let key = (
                s.source_language.primary().to_string(),
                s.target_language.primary().to_string(),
                key_src,
            );
            let target = s.target.trim().to_string();
            if let Some(prev) = index.insert(key, target.clone()) {
                if prev != target {
                    return Err(bad("conflicting targets for one source segment"));
                }
            }
            let pair = LanguagePair {
                source: LanguageTag::parse(s.source_language.primary()).map_err(|_| bad("tag"))?,
                target: LanguageTag::parse(s.target_language.primary()).map_err(|_| bad("tag"))?,
            };
            if !pairs.contains(&pair) {
                pairs.push(pair);
            }
            canon.push_str(&format!(
                "{}|{}|{}|{}\n",
                s.source_language, s.target_language, s.source, s.target
            ));
        }
        pairs.sort();
        Ok(Self {
            len: index.len(),
            index,
            pairs,
            fingerprint: fnv(&canon),
        })
    }

    /// Parse a JSON memory (`{"segments": [...]}`).
    pub fn from_json(json: &str) -> Result<Self, CeError> {
        #[derive(Deserialize)]
        struct File {
            segments: Vec<Segment>,
        }
        let f: File = serde_json::from_str(json).map_err(|e| {
            CeError::new(
                ErrorCategory::InvalidConfig,
                "translation.bad_memory",
                "translation memory is not valid JSON",
            )
            .with_source(e)
        })?;
        Self::new(&f.segments)
    }

    /// Distinct entries.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Pairs present.
    pub fn pairs(&self) -> &[LanguagePair] {
        &self.pairs
    }

    /// Content fingerprint.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Look up a segment.
    pub fn lookup(&self, source: &LanguageTag, target: &LanguageTag, text: &str) -> Option<&str> {
        self.index
            .get(&(
                source.primary().to_string(),
                target.primary().to_string(),
                normalize(text),
            ))
            .map(String::as_str)
    }
}

fn fnv(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Factory for the memory adapter (the memory is shared read-only).
#[derive(Clone, Debug)]
pub struct MemoryFactory {
    memory: std::sync::Arc<TranslationMemory>,
}

impl MemoryFactory {
    /// Over `memory`.
    pub fn new(memory: TranslationMemory) -> Self {
        Self {
            memory: std::sync::Arc::new(memory),
        }
    }
}

impl TranslationFactory for MemoryFactory {
    fn capabilities(&self) -> TranslationCapabilities {
        TranslationCapabilities {
            adapter: MEMORY_ID.into(),
            version: MEMORY_VERSION.into(),
            locality: Locality::InProcess,
            pairs: self.memory.pairs().to_vec(),
            detects_source: false,
            incremental_output: false,
            partial_input: false,
            context_hints: false,
            terminology: false,
            placeholders: false,
            max_input_chars: 2_000,
            deterministic: true,
            requires_load: false,
            score_semantics: None,
            model: Some(ce_core::speech::ModelInfo {
                id: "translation-memory".into(),
                version: MEMORY_VERSION.into(),
                fingerprint: Some(self.memory.fingerprint().to_string()),
                size_bytes: None,
                license: "per-segment provenance".into(),
            }),
        }
    }

    fn create(&self, _: &TranslationConfig) -> Result<Box<dyn TranslationAdapter>, CeError> {
        Ok(Box::new(MemoryAdapter {
            memory: self.memory.clone(),
        }))
    }
}

/// The memory adapter.
#[derive(Debug)]
pub struct MemoryAdapter {
    memory: std::sync::Arc<TranslationMemory>,
}

impl TranslationAdapter for MemoryAdapter {
    fn translate(&mut self, r: &TranslationRequest<'_>) -> Result<AdapterOutcome, CeError> {
        let Some(source) = r.source else {
            // No detection: an unknown source cannot be looked up.
            return Ok(AdapterOutcome::UnsupportedPair);
        };
        if !self
            .memory
            .pairs()
            .iter()
            .any(|p| p.covers(source, r.target))
        {
            return Ok(AdapterOutcome::UnsupportedPair);
        }
        Ok(match self.memory.lookup(source, r.target, r.text) {
            Some(t) => AdapterOutcome::Translated {
                text: t.to_string(),
                score: None,
                detected_source: None,
            },
            None => AdapterOutcome::NoCoverage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(s: &str, t: &str, a: &str, b: &str) -> Segment {
        Segment {
            source_language: LanguageTag::parse(a).unwrap(),
            target_language: LanguageTag::parse(b).unwrap(),
            source: s.into(),
            target: t.into(),
            provenance: "test".into(),
        }
    }

    #[test]
    fn exact_normalized_matching_only_and_conflicts_rejected() {
        let tm = TranslationMemory::new(&[
            seg(
                "Where is the train station?",
                "¿Dónde está la estación de tren?",
                "en",
                "es",
            ),
            seg("Thank you", "Gracias", "en", "es"),
        ])
        .unwrap();
        let (en, es) = (
            LanguageTag::parse("en").unwrap(),
            LanguageTag::parse("es").unwrap(),
        );
        assert_eq!(
            tm.lookup(&en, &es, "WHERE IS THE TRAIN STATION"),
            Some("¿Dónde está la estación de tren?")
        );
        assert_eq!(
            tm.lookup(&en, &es, "where is the train"),
            None,
            "no partial matches"
        );
        assert_eq!(tm.lookup(&es, &en, "Gracias"), None, "pairs are directed");
        assert!(
            TranslationMemory::new(&[seg("a", "b", "en", "es"), seg("A!", "c", "en", "es")])
                .is_err()
        );
        assert!(TranslationMemory::new(&[seg("", "b", "en", "es")]).is_err());
        let json = r#"{"segments":[{"source_language":"en","target_language":"fr","source":"Hello","target":"Bonjour","provenance":"t"}]}"#;
        let tm = TranslationMemory::from_json(json).unwrap();
        assert_eq!(tm.pairs().len(), 1);
        assert_eq!(tm.fingerprint().len(), 16);
        assert!(TranslationMemory::from_json("{").is_err());
    }
}
