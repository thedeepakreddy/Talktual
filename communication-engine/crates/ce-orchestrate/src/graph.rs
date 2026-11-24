//! Pipeline graph: dependency rules, validation, order, fingerprint.
//!
//! Rules (ADR-0030):
//!
//! | Stage | Hard dependency | Soft input |
//! |---|---|---|
//! | analysis | — (the capture stream) | — |
//! | recognition | analysis if `segmentation = activity_hints` | — |
//! | semantics | recognition | — |
//! | translation | recognition | semantics if `context.use_semantic` |
//!
//! A pipeline is valid when it is non-empty, has no duplicate kind, every
//! hard dependency is present, no required stage hard-depends on an
//! optional one, every configuration validates and the dependency graph
//! is acyclic. A missing soft input is a **note**, not an error.

use ce_core::error::CeError;
use ce_core::orchestration::{
    DeadlinePolicy, Requirement, StageConfig, StageKind, StageSpec, StopMode,
};
use ce_core::speech::SegmentationMode;

/// A stage with its effective configuration (defaults filled in).
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedStage {
    /// Kind.
    pub kind: StageKind,
    /// Requirement.
    pub requirement: Requirement,
    /// Effective configuration.
    pub config: StageConfig,
}

impl ResolvedStage {
    /// Fill a spec's missing configuration from `default`.
    pub fn resolve(spec: &StageSpec, default: impl FnOnce(StageKind) -> StageConfig) -> Self {
        Self {
            kind: spec.kind,
            requirement: spec.requirement,
            config: spec.config.clone().unwrap_or_else(|| default(spec.kind)),
        }
    }

    fn hard_deps(&self) -> Vec<StageKind> {
        match &self.config {
            StageConfig::Analysis(_) => vec![],
            StageConfig::Recognition(c) => {
                if c.segmentation == SegmentationMode::ActivityHints {
                    vec![StageKind::Analysis]
                } else {
                    vec![]
                }
            }
            StageConfig::Semantics(_) => vec![StageKind::Recognition],
            StageConfig::Translation(_) => vec![StageKind::Recognition],
        }
    }

    fn soft_inputs(&self) -> Vec<(StageKind, &'static str)> {
        match &self.config {
            StageConfig::Translation(c) if c.context.use_semantic => {
                vec![(StageKind::Semantics, "translation.hints_unavailable")]
            }
            _ => vec![],
        }
    }
}

/// A validated pipeline graph.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedGraph {
    /// Stages in execution (topological) order.
    pub stages: Vec<ResolvedStage>,
    /// Notes about absent soft inputs (stable codes).
    pub notes: Vec<String>,
    /// Stop mode.
    pub stop: StopMode,
    /// Deadlines.
    pub deadlines: DeadlinePolicy,
    /// Fingerprint (versioned; configuration fingerprints, no secrets).
    pub fingerprint: String,
}

impl ValidatedGraph {
    /// The stage of a kind, if present.
    pub fn stage(&self, kind: StageKind) -> Option<&ResolvedStage> {
        self.stages.iter().find(|s| s.kind == kind)
    }

    /// Hard dependencies of a stage (within this graph).
    pub fn dependencies(&self, kind: StageKind) -> Vec<StageKind> {
        self.stage(kind).map(|s| s.hard_deps()).unwrap_or_default()
    }

    /// Stages that hard-depend (directly or transitively) on `kind`.
    pub fn dependents(&self, kind: StageKind) -> Vec<StageKind> {
        let mut out: Vec<StageKind> = Vec::new();
        let mut frontier = vec![kind];
        while let Some(k) = frontier.pop() {
            for s in &self.stages {
                if s.hard_deps().contains(&k) && !out.contains(&s.kind) {
                    out.push(s.kind);
                    frontier.push(s.kind);
                }
            }
        }
        out
    }

    /// Stop order. `Drain`: the transcript chain upstream first
    /// (recognition, then its dependents, which finish what they already
    /// have), and analysis — a hint provider — **last**, so recognition keeps
    /// its activity hints until it ends. `Cancel`: execution order reversed
    /// (downstream first, pending work discarded).
    pub fn stop_order(&self) -> Vec<StageKind> {
        let mut order: Vec<StageKind> = self.stages.iter().map(|s| s.kind).collect();
        match self.stop {
            StopMode::Cancel => order.reverse(),
            StopMode::Drain => {
                if let Some(i) = order.iter().position(|k| *k == StageKind::Analysis) {
                    let a = order.remove(i);
                    order.push(a);
                }
            }
        }
        order
    }
}

fn invalid(code: &'static str, message: &'static str) -> CeError {
    CeError::invalid_config(code, message)
}

/// Topologically order `n` nodes whose hard dependencies are `deps[i]`
/// (indices). Kahn's algorithm; ties are broken by index, so the order is
/// deterministic. Returns the nodes on a cycle if there is one.
pub fn toposort(deps: &[Vec<usize>]) -> Result<Vec<usize>, Vec<usize>> {
    let n = deps.len();
    let mut indegree: Vec<usize> = deps.iter().map(Vec::len).collect();
    let mut order = Vec::with_capacity(n);
    let mut done = vec![false; n];
    while let Some(next) = (0..n).find(|&i| !done[i] && indegree[i] == 0) {
        done[next] = true;
        order.push(next);
        for (j, d) in deps.iter().enumerate() {
            indegree[j] -= d.iter().filter(|&&x| x == next).count();
        }
    }
    if order.len() == n {
        Ok(order)
    } else {
        Err((0..n).filter(|&i| !done[i]).collect())
    }
}

/// Validate resolved stages and policies into a graph.
pub fn validate(
    stages: Vec<ResolvedStage>,
    stop: StopMode,
    deadlines: DeadlinePolicy,
) -> Result<ValidatedGraph, CeError> {
    if stages.is_empty() {
        return Err(invalid(
            "orchestration.empty_pipeline",
            "a pipeline needs at least one stage",
        ));
    }
    for (i, s) in stages.iter().enumerate() {
        if s.config.kind() != s.kind {
            return Err(invalid(
                "orchestration.config_mismatch",
                "a stage's configuration must be of its own kind",
            )
            .with_context("stage", s.kind.as_str()));
        }
        s.config
            .validate()
            .map_err(|e| e.with_context("stage", s.kind.as_str()))?;
        if stages[..i].iter().any(|o| o.kind == s.kind) {
            return Err(invalid(
                "orchestration.duplicate_stage",
                "a stage kind appears more than once",
            )
            .with_context("stage", s.kind.as_str()));
        }
    }
    let index = |k: StageKind| stages.iter().position(|s| s.kind == k);
    let mut deps: Vec<Vec<usize>> = Vec::with_capacity(stages.len());
    for s in &stages {
        let mut d = Vec::new();
        for k in s.hard_deps() {
            let Some(j) = index(k) else {
                return Err(invalid(
                    "orchestration.missing_dependency",
                    "a stage's required input stage is not in the pipeline",
                )
                .with_context("stage", s.kind.as_str())
                .with_context("needs", k.as_str()));
            };
            if s.requirement == Requirement::Required
                && stages[j].requirement == Requirement::Optional
            {
                return Err(invalid(
                    "orchestration.required_depends_on_optional",
                    "a required stage cannot depend on an optional one",
                )
                .with_context("stage", s.kind.as_str())
                .with_context("needs", k.as_str()));
            }
            d.push(j);
        }
        deps.push(d);
    }
    let order = toposort(&deps).map_err(|cycle| {
        invalid(
            "orchestration.cycle",
            "the pipeline's dependencies form a cycle",
        )
        .with_context(
            "stages",
            cycle
                .iter()
                .map(|&i| stages[i].kind.as_str())
                .collect::<Vec<_>>()
                .join(","),
        )
    })?;
    let mut notes = Vec::new();
    for s in &stages {
        for (k, code) in s.soft_inputs() {
            if index(k).is_none() {
                notes.push(code.to_string());
            }
        }
    }
    let ordered: Vec<ResolvedStage> = order.into_iter().map(|i| stages[i].clone()).collect();
    let mut canonical = String::from("pipeline.v1");
    for s in &ordered {
        canonical.push_str(&format!(
            "|{}:{:?}:{}",
            s.kind.as_str(),
            s.requirement,
            s.config.fingerprint()
        ));
    }
    canonical.push_str(&format!("|stop:{stop:?}|deadlines:{deadlines:?}"));
    Ok(ValidatedGraph {
        fingerprint: fnv(&canonical),
        stages: ordered,
        notes,
        stop,
        deadlines,
    })
}

/// FNV-1a 64-bit, hex (the same scheme as the stage fingerprints).
fn fnv(s: &str) -> String {
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
    use ce_core::speech::RecognitionConfig;
    use ce_core::translation::TranslationConfig;

    fn default(kind: StageKind) -> StageConfig {
        match kind {
            StageKind::Analysis => StageConfig::Analysis(Box::default()),
            StageKind::Recognition => StageConfig::Recognition(Box::new(RecognitionConfig {
                segmentation: SegmentationMode::Continuous,
                ..RecognitionConfig::default()
            })),
            StageKind::Semantics => StageConfig::Semantics(Box::default()),
            StageKind::Translation => StageConfig::Translation(Box::default()),
        }
    }

    fn stages(specs: &[StageSpec]) -> Vec<ResolvedStage> {
        specs
            .iter()
            .map(|s| ResolvedStage::resolve(s, default))
            .collect()
    }

    fn ok(specs: &[StageSpec]) -> ValidatedGraph {
        validate(stages(specs), StopMode::Drain, DeadlinePolicy::default()).unwrap()
    }

    fn err(specs: &[StageSpec]) -> String {
        validate(stages(specs), StopMode::Drain, DeadlinePolicy::default())
            .unwrap_err()
            .code()
            .to_string()
    }

    use StageKind::*;

    #[test]
    fn order_follows_dependencies_not_declaration() {
        let g = ok(&[
            StageSpec::optional(Translation),
            StageSpec::optional(Semantics),
            StageSpec::required(Recognition),
        ]);
        let order: Vec<StageKind> = g.stages.iter().map(|s| s.kind).collect();
        assert_eq!(order, vec![Recognition, Translation, Semantics]);
        assert_eq!(g.stop_order(), vec![Recognition, Translation, Semantics]);
        assert_eq!(g.dependents(Recognition), vec![Translation, Semantics]);
        assert!(g.notes.is_empty(), "{:?}", g.notes);
        let cancel = validate(
            stages(&[
                StageSpec::required(Recognition),
                StageSpec::optional(Translation),
            ]),
            StopMode::Cancel,
            DeadlinePolicy::default(),
        )
        .unwrap();
        assert_eq!(cancel.stop_order(), vec![Translation, Recognition]);
    }

    #[test]
    fn minimal_and_branched_pipelines_are_valid() {
        let g = ok(&[StageSpec::required(Recognition)]);
        assert_eq!(g.stages.len(), 1);
        assert!(g.notes.is_empty());
        let g = ok(&[StageSpec::required(Analysis)]);
        assert!(g.notes.is_empty());
        let g = ok(&[
            StageSpec::required(Analysis),
            StageSpec::required(Recognition),
            StageSpec::optional(Semantics),
            StageSpec::optional(Translation),
        ]);
        assert_eq!(g.stages.len(), 4);
        assert_eq!(
            g.stop_order(),
            vec![Recognition, Semantics, Translation, Analysis],
            "drain: transcript chain first, the hint provider last"
        );
    }

    #[test]
    fn missing_dependencies_and_bad_requirements_fail_before_activation() {
        assert_eq!(
            err(&[StageSpec::required(Translation)]),
            "orchestration.missing_dependency"
        );
        assert_eq!(
            err(&[
                StageSpec::optional(Recognition),
                StageSpec::required(Semantics)
            ]),
            "orchestration.required_depends_on_optional"
        );
        assert_eq!(
            err(&[
                StageSpec::required(Recognition),
                StageSpec::required(Recognition)
            ]),
            "orchestration.duplicate_stage"
        );
        assert_eq!(err(&[]), "orchestration.empty_pipeline");
        let hints = StageSpec::required(Recognition).with_config(StageConfig::Recognition(
            Box::new(RecognitionConfig {
                segmentation: SegmentationMode::ActivityHints,
                ..RecognitionConfig::default()
            }),
        ));
        assert_eq!(
            err(std::slice::from_ref(&hints)),
            "orchestration.missing_dependency"
        );
        ok(&[StageSpec::required(Analysis), hints]);
        let wrong =
            StageSpec::required(Recognition).with_config(StageConfig::Translation(Box::default()));
        assert_eq!(err(&[wrong]), "orchestration.config_mismatch");
    }

    #[test]
    fn soft_inputs_are_notes() {
        let mut t = TranslationConfig::default();
        t.context.use_semantic = true;
        let g = ok(&[
            StageSpec::required(Recognition),
            StageSpec::required(Translation).with_config(StageConfig::Translation(Box::new(t))),
        ]);
        assert!(
            g.notes
                .contains(&"translation.hints_unavailable".to_string())
        );
    }

    #[test]
    fn toposort_is_deterministic_and_finds_cycles() {
        assert_eq!(
            toposort(&[vec![], vec![0], vec![0], vec![1, 2]]),
            Ok(vec![0, 1, 2, 3])
        );
        assert_eq!(toposort(&[vec![2], vec![], vec![1]]), Ok(vec![1, 2, 0]));
        assert_eq!(
            toposort(&[vec![1], vec![2], vec![0], vec![]]),
            Err(vec![0, 1, 2])
        );
        assert_eq!(toposort(&[vec![0]]), Err(vec![0]), "self loop");
        assert_eq!(toposort(&[]), Ok(vec![]));
    }

    #[test]
    fn fingerprint_is_stable_and_config_sensitive() {
        let a = ok(&[
            StageSpec::required(Recognition),
            StageSpec::optional(Translation),
        ]);
        let b = ok(&[
            StageSpec::optional(Translation),
            StageSpec::required(Recognition),
        ]);
        assert_eq!(
            a.fingerprint, b.fingerprint,
            "declaration order does not matter"
        );
        let c = ok(&[
            StageSpec::required(Recognition),
            StageSpec::required(Translation),
        ]);
        assert_ne!(a.fingerprint, c.fingerprint, "requirement matters");
        assert_eq!(a.fingerprint.len(), 16);
    }
}
