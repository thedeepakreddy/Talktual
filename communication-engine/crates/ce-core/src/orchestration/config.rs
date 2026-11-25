//! Pipeline specification and orchestration settings.

use serde::{Deserialize, Serialize};

use crate::error::CeError;
use crate::intel::AnalysisConfig;
use crate::semantic::SemanticConfig;
use crate::speech::RecognitionConfig;
use crate::translation::TranslationConfig;

/// A stage of a pipeline. The set is fixed: a new kind needs a dependency
/// rule (ADR-0030).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    /// Phase 3 audio intelligence (activity, segments).
    Analysis,
    /// Phase 4 speech recognition.
    Recognition,
    /// Phase 5 semantic interpretation.
    Semantics,
    /// Phase 6 translation.
    Translation,
}

impl StageKind {
    /// Stable name.
    pub fn as_str(self) -> &'static str {
        match self {
            StageKind::Analysis => "analysis",
            StageKind::Recognition => "recognition",
            StageKind::Semantics => "semantics",
            StageKind::Translation => "translation",
        }
    }
}

/// Whether the pipeline can run without the stage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Requirement {
    /// Its failure fails the pipeline.
    #[default]
    Required,
    /// Its failure degrades the pipeline (explicitly).
    Optional,
}

/// A stage's configuration (`None` in a [`StageSpec`] = engine default).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "config", rename_all = "snake_case")]
pub enum StageConfig {
    /// Analysis.
    Analysis(Box<AnalysisConfig>),
    /// Recognition.
    Recognition(Box<RecognitionConfig>),
    /// Semantics.
    Semantics(Box<SemanticConfig>),
    /// Translation.
    Translation(Box<TranslationConfig>),
}

impl StageConfig {
    /// The stage kind this configuration belongs to.
    pub fn kind(&self) -> StageKind {
        match self {
            StageConfig::Analysis(_) => StageKind::Analysis,
            StageConfig::Recognition(_) => StageKind::Recognition,
            StageConfig::Semantics(_) => StageKind::Semantics,
            StageConfig::Translation(_) => StageKind::Translation,
        }
    }

    /// Validate with the stage's own rules.
    pub fn validate(&self) -> Result<(), CeError> {
        match self {
            StageConfig::Analysis(c) => c.validate(),
            StageConfig::Recognition(c) => c.validate(),
            StageConfig::Semantics(c) => c.validate(),
            StageConfig::Translation(c) => c.validate(),
        }
    }

    /// The stage's own configuration fingerprint (no secrets).
    pub fn fingerprint(&self) -> String {
        match self {
            StageConfig::Analysis(c) => c.fingerprint(),
            StageConfig::Recognition(c) => c.fingerprint(),
            StageConfig::Semantics(c) => c.fingerprint(),
            StageConfig::Translation(c) => c.fingerprint(),
        }
    }
}

/// One stage of a pipeline specification.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageSpec {
    /// Kind.
    pub kind: StageKind,
    /// Required or optional.
    #[serde(default)]
    pub requirement: Requirement,
    /// Configuration; `None` uses the engine's default for the stage.
    #[serde(default)]
    pub config: Option<StageConfig>,
}

impl StageSpec {
    /// A required stage with the engine default configuration.
    pub fn required(kind: StageKind) -> Self {
        Self {
            kind,
            requirement: Requirement::Required,
            config: None,
        }
    }

    /// An optional stage with the engine default configuration.
    pub fn optional(kind: StageKind) -> Self {
        Self {
            kind,
            requirement: Requirement::Optional,
            config: None,
        }
    }

    /// With an explicit configuration.
    pub fn with_config(mut self, config: StageConfig) -> Self {
        self.config = Some(config);
        self
    }
}

/// How a pipeline stops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopMode {
    /// Upstream first: recognition ends and downstream workers finish what
    /// they already have (bounded by the module operation timeout).
    #[default]
    Drain,
    /// Downstream first: pending work is discarded.
    Cancel,
}

/// Optional audio-to-output budgets (monotonic). A miss is reported as a
/// typed outcome; the result is still published and never replaces newer
/// state (ADR-0032).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeadlinePolicy {
    /// Utterance audio end → final transcript published.
    pub transcript_final_ms: Option<u32>,
    /// Utterance audio end → semantic interpretation finalized.
    pub semantic_final_ms: Option<u32>,
    /// Utterance audio end → final translation published.
    pub translation_final_ms: Option<u32>,
}

impl DeadlinePolicy {
    /// The budget for a stage's final output, if any.
    pub fn budget(&self, stage: StageKind) -> Option<u32> {
        match stage {
            StageKind::Recognition => self.transcript_final_ms,
            StageKind::Semantics => self.semantic_final_ms,
            StageKind::Translation => self.translation_final_ms,
            StageKind::Analysis => None,
        }
    }

    fn validate(&self) -> Result<(), CeError> {
        for v in [
            self.transcript_final_ms,
            self.semantic_final_ms,
            self.translation_final_ms,
        ]
        .into_iter()
        .flatten()
        {
            if !(1..=600_000).contains(&v) {
                return Err(bad("deadlines", "1..=600000 ms"));
            }
        }
        Ok(())
    }
}

/// A pipeline: stages of one capture stream, with policies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineSpec {
    /// Stages (order is irrelevant to execution; it only breaks ties).
    pub stages: Vec<StageSpec>,
    /// Stop behaviour.
    #[serde(default)]
    pub stop: StopMode,
    /// Deadlines.
    #[serde(default)]
    pub deadlines: DeadlinePolicy,
}

impl PipelineSpec {
    /// A pipeline of the given stages with default policies.
    pub fn new(stages: Vec<StageSpec>) -> Self {
        Self {
            stages,
            stop: StopMode::default(),
            deadlines: DeadlinePolicy::default(),
        }
    }

    /// Structural checks that need no engine context: non-empty, at most 8
    /// stages, configurations match their stage and validate, policies in
    /// range. Dependency rules are checked by `ce-orchestrate`.
    pub fn validate(&self) -> Result<(), CeError> {
        if self.stages.is_empty() || self.stages.len() > 8 {
            return Err(bad("stages", "1..=8 stages"));
        }
        for s in &self.stages {
            if let Some(c) = &s.config {
                if c.kind() != s.kind {
                    return Err(bad("config", "a configuration of the stage's own kind")
                        .with_context("stage", s.kind.as_str()));
                }
                c.validate()?;
            }
        }
        self.deadlines.validate()
    }
}

/// Engine-wide orchestration settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OrchestrationSettings {
    /// Concurrent pipelines (1..=64). Default 8.
    pub max_pipelines: u32,
    /// Utterances the observer remembers per pipeline (16..=4096). Default 256.
    pub observer_utterances: u32,
    /// Abandoned stage workers whose threads are still alive, engine-wide,
    /// at which **new stage worker starts are refused** (1..=1024; default
    /// 16). An abandoned worker (stop timeout or stall) keeps its thread
    /// and its signal forwarder until the thread exits; this cap bounds
    /// that burden. Total stage threads stay below this value plus the
    /// running-worker limits of the stages (see `orchestration.md` §5).
    pub max_abandoned_workers: u32,
}

impl Default for OrchestrationSettings {
    fn default() -> Self {
        Self {
            max_pipelines: 8,
            observer_utterances: 256,
            max_abandoned_workers: 16,
        }
    }
}

impl OrchestrationSettings {
    /// Validate.
    pub fn validate(&self) -> Result<(), CeError> {
        if !(1..=64).contains(&self.max_pipelines) {
            return Err(bad("max_pipelines", "1..=64"));
        }
        if !(16..=4096).contains(&self.observer_utterances) {
            return Err(bad("observer_utterances", "16..=4096"));
        }
        if !(1..=1024).contains(&self.max_abandoned_workers) {
            return Err(bad("max_abandoned_workers", "1..=1024"));
        }
        Ok(())
    }
}

fn bad(field: &'static str, allowed: &'static str) -> CeError {
    CeError::invalid_config(
        "config.orchestration_invalid",
        "orchestration configuration value is invalid",
    )
    .with_context("field", field)
    .with_context("allowed", allowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_round_trips_and_validates() {
        let spec = PipelineSpec::new(vec![
            StageSpec::required(StageKind::Recognition),
            StageSpec::optional(StageKind::Translation)
                .with_config(StageConfig::Translation(Box::default())),
        ]);
        spec.validate().unwrap();
        let json = serde_json::to_value(&spec).unwrap();
        assert_eq!(json["stages"][1]["requirement"], "optional");
        assert_eq!(json["stages"][1]["config"]["kind"], "translation");
        let back: PipelineSpec = serde_json::from_value(json).unwrap();
        assert_eq!(back, spec);
    }

    #[test]
    fn invalid_specs_are_rejected() {
        assert!(PipelineSpec::new(vec![]).validate().is_err());
        let wrong = PipelineSpec::new(vec![
            StageSpec::required(StageKind::Recognition)
                .with_config(StageConfig::Translation(Box::default())),
        ]);
        assert_eq!(
            wrong.validate().unwrap_err().code(),
            "config.orchestration_invalid"
        );
        let mut late = PipelineSpec::new(vec![StageSpec::required(StageKind::Recognition)]);
        late.deadlines.transcript_final_ms = Some(0);
        assert!(late.validate().is_err());
        assert!(
            OrchestrationSettings {
                max_pipelines: 0,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        OrchestrationSettings::default().validate().unwrap();
    }
}
