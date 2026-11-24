//! A scripted adapter for lifecycle, failure, revision and concurrency
//! tests: it replays programmed outputs and failures, and can be slow.
//! Deterministic; never used for quality evaluation.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ce_core::semantic::{
    Certainty, Entity, Epistemic, Evidence, EvidenceKind, Locality, SemanticCapabilities,
    SemanticConfig, SemanticInput, TextSpan, Uncertainty,
};
use ce_core::{CeError, ErrorCategory};

use crate::adapter::{ContextView, Draft, DraftPayload, SemanticAdapter, SemanticFactory};

/// One programmed step.
#[derive(Clone, Debug)]
pub enum Step {
    /// Return these drafts.
    Output(Vec<Draft>),
    /// Mark every whitespace-separated word as an entity of category
    /// `word` (handy for revision tests).
    Words,
    /// Return an error.
    Error,
    /// Panic.
    Panic,
    /// Sleep, then behave like `Words`.
    Sleep(Duration),
}

/// Shared script and call log.
#[derive(Debug, Default)]
pub struct Script {
    steps: VecDeque<Step>,
    /// Default when the script is exhausted.
    pub fallback: Option<Step>,
    /// Texts analyzed, in order (test inspection only).
    pub calls: Vec<(u64, u32, String)>,
    /// Context sizes seen, in order.
    pub context_sizes: Vec<usize>,
}

impl Script {
    /// A script of steps, then `Words`.
    pub fn new(steps: impl IntoIterator<Item = Step>) -> Arc<Mutex<Script>> {
        Arc::new(Mutex::new(Script {
            steps: steps.into_iter().collect(),
            fallback: Some(Step::Words),
            ..Script::default()
        }))
    }
}

/// Factory with configurable capabilities.
#[derive(Debug, Clone)]
pub struct ScriptedFactory {
    /// Shared script.
    pub script: Arc<Mutex<Script>>,
    /// Capabilities reported.
    pub capabilities: SemanticCapabilities,
}

impl ScriptedFactory {
    /// In-process, everything supported.
    pub fn new(script: Arc<Mutex<Script>>) -> Self {
        Self {
            script,
            capabilities: SemanticCapabilities {
                adapter: "ce.semantic.scripted".into(),
                version: "1.0.0".into(),
                locality: Locality::InProcess,
                deterministic: true,
                entities: true,
                relations: true,
                references: true,
                acts: true,
                categories: vec!["word".into()],
                normalizes: Vec::new(),
                act_taxonomies: Vec::new(),
                languages: Vec::new(),
                score_semantics: None,
                requires_load: false,
            },
        }
    }

    /// Report this locality (for policy tests).
    pub fn with_locality(mut self, l: Locality) -> Self {
        self.capabilities.locality = l;
        self
    }
}

impl SemanticFactory for ScriptedFactory {
    fn capabilities(&self) -> SemanticCapabilities {
        self.capabilities.clone()
    }

    fn create(&self, _config: &SemanticConfig) -> Result<Box<dyn SemanticAdapter>, CeError> {
        Ok(Box::new(ScriptedAdapter {
            script: self.script.clone(),
        }))
    }
}

/// The scripted adapter.
#[derive(Debug)]
pub struct ScriptedAdapter {
    script: Arc<Mutex<Script>>,
}

/// Every word of `text` as a `word` entity.
pub fn words(text: &str) -> Vec<Draft> {
    let mut out = Vec::new();
    let mut start = None;
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().chain(std::iter::once(&' ')).enumerate() {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                let mention: String = chars[s..i].iter().collect();
                let span = TextSpan {
                    start: s as u32,
                    end: i as u32,
                };
                out.push(Draft {
                    key: format!("entity:word:{}", mention.to_lowercase()),
                    payload: DraftPayload::Entity(Entity {
                        category: "word".into(),
                        mention,
                        normalized: None,
                        attributes: Vec::new(),
                    }),
                    span,
                    epistemic: Epistemic::Observation,
                    uncertainty: Uncertainty::tier(Certainty::High),
                    evidence: vec![Evidence {
                        kind: EvidenceKind::Explicit,
                        span: Some(span),
                        context_item: None,
                        rule: "scripted.word".into(),
                    }],
                });
                start = None;
            }
            _ => {}
        }
    }
    out
}

impl SemanticAdapter for ScriptedAdapter {
    fn analyze(
        &mut self,
        input: &SemanticInput,
        context: &ContextView,
    ) -> Result<Vec<Draft>, CeError> {
        let step = {
            let mut s = self
                .script
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            s.calls
                .push((input.utterance, input.revision, input.text.clone()));
            s.context_sizes.push(context.items.len());
            s.steps.pop_front().or_else(|| s.fallback.clone())
        };
        match step {
            Some(Step::Output(d)) => Ok(d),
            Some(Step::Words) | None => Ok(words(&input.text)),
            Some(Step::Error) => Err(CeError::new(
                ErrorCategory::Provider,
                "semantic.scripted_error",
                "scripted adapter error",
            )),
            Some(Step::Panic) => panic!("scripted adapter panic"),
            Some(Step::Sleep(d)) => {
                std::thread::sleep(d);
                Ok(words(&input.text))
            }
        }
    }
}
