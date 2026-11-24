//! Adaptive and resource policy.
//!
//! **Adaptive policy** (input-driven, per quality window; all decisions are
//! deterministic, documented and reversible, and each change is an event):
//!
//! | Decision | Apply | Revert |
//! |---|---|---|
//! | `ConservativeSegmentation` | SNR < 12 dB or `SustainedNoise`, 2 windows | SNR ≥ 18 dB and no `SustainedNoise`, 3 windows |
//! | `SuppressLevelEstimates` | `Clipping` or `Overload` active | both clear |
//! | `LowConfidenceActivity` | usability `Unusable` | any other usability |
//!
//! **Resource policy** (load-driven, per block): `load` is processing time
//! divided by the block's audio duration; `backlog` is blocks waiting in
//! the analysis queue. After `degrade_after` consecutive blocks over budget
//! (load > `max_load` or backlog > half the queue) the mode steps down
//! (Full → Reduced → Shedding); after `recover_after` consecutive blocks
//! comfortably within budget (load < `max_load`/2 and backlog ≤ 1) it steps
//! back up one level. Capture is never affected: the tap drops instead.

use ce_core::intel::{
    AnalysisBudget, ConditionKind, DegradedMode, PolicyDecision, QualityMetrics, Usability,
};

/// Input-driven decisions.
#[derive(Clone, Debug, Default)]
pub struct AdaptivePolicy {
    enabled: bool,
    active: Vec<PolicyDecision>,
    bad_windows: u32,
    good_windows: u32,
}

impl AdaptivePolicy {
    /// A policy; when `enabled` is false it never decides anything.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            ..Self::default()
        }
    }

    /// Active decisions.
    pub fn active(&self) -> &[PolicyDecision] {
        &self.active
    }

    /// Whether `d` is active.
    pub fn is_active(&self, d: PolicyDecision) -> bool {
        self.active.contains(&d)
    }

    fn set(&mut self, d: PolicyDecision, on: bool, out: &mut Vec<(PolicyDecision, bool)>) {
        if on == self.is_active(d) {
            return;
        }
        if on {
            self.active.push(d);
        } else {
            self.active.retain(|x| *x != d);
        }
        out.push((d, on));
    }

    /// Evaluate after a quality window. Returns changes `(decision, active)`.
    pub fn on_window(
        &mut self,
        m: &QualityMetrics,
        conditions: &[ConditionKind],
        out: &mut Vec<(PolicyDecision, bool)>,
    ) {
        if !self.enabled {
            return;
        }
        let noise = conditions.contains(&ConditionKind::SustainedNoise);
        let bad = noise || m.snr_db.is_some_and(|s| s < 12.0);
        let good = !noise && m.snr_db.is_some_and(|s| s >= 18.0);
        if bad {
            self.bad_windows += 1;
            self.good_windows = 0;
        } else if good {
            self.good_windows += 1;
            self.bad_windows = 0;
        } else {
            self.bad_windows = 0;
            self.good_windows = 0;
        }
        if self.bad_windows >= 2 {
            self.set(PolicyDecision::ConservativeSegmentation, true, out);
        } else if self.good_windows >= 3 {
            self.set(PolicyDecision::ConservativeSegmentation, false, out);
        }
        let levels_unreliable = conditions.contains(&ConditionKind::Clipping)
            || conditions.contains(&ConditionKind::Overload);
        self.set(
            PolicyDecision::SuppressLevelEstimates,
            levels_unreliable,
            out,
        );
        self.set(
            PolicyDecision::LowConfidenceActivity,
            m.usability == Usability::Unusable,
            out,
        );
    }

    /// Revert everything (stop / reset).
    pub fn clear(&mut self, out: &mut Vec<(PolicyDecision, bool)>) {
        for d in std::mem::take(&mut self.active) {
            out.push((d, false));
        }
        self.bad_windows = 0;
        self.good_windows = 0;
    }
}

/// Load-driven operating level.
#[derive(Clone, Debug)]
pub struct ResourceGovernor {
    budget: AnalysisBudget,
    mode: DegradedMode,
    over: u32,
    under: u32,
    max_load_permille: u32,
    max_backlog: u32,
}

/// A mode change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeChange {
    /// Previous.
    pub from: DegradedMode,
    /// New.
    pub to: DegradedMode,
    /// Load, permille.
    pub load_permille: u32,
    /// Backlog, blocks.
    pub backlog: u32,
}

impl ResourceGovernor {
    /// A governor starting at `Full`.
    pub fn new(budget: AnalysisBudget) -> Self {
        Self {
            budget,
            mode: DegradedMode::Full,
            over: 0,
            under: 0,
            max_load_permille: 0,
            max_backlog: 0,
        }
    }

    /// Current mode.
    pub fn mode(&self) -> DegradedMode {
        self.mode
    }

    /// Highest load seen, permille.
    pub fn max_load_permille(&self) -> u32 {
        self.max_load_permille
    }

    /// Highest backlog seen.
    pub fn max_backlog(&self) -> u32 {
        self.max_backlog
    }

    /// Record one block's load and the queue backlog.
    pub fn note(&mut self, load: f32, backlog: u32) -> Option<ModeChange> {
        let permille = (load.max(0.0) * 1000.0).min(u32::MAX as f32) as u32;
        self.max_load_permille = self.max_load_permille.max(permille);
        self.max_backlog = self.max_backlog.max(backlog);
        let b = &self.budget;
        if !b.degrade {
            return None;
        }
        let over = load > b.max_load || backlog > b.queue_blocks / 2;
        let under = load < b.max_load / 2.0 && backlog <= 1;
        if over {
            self.over += 1;
            self.under = 0;
        } else if under {
            self.under += 1;
            self.over = 0;
        } else {
            self.over = 0;
            self.under = 0;
        }
        let next = if self.over >= b.degrade_after {
            match self.mode {
                DegradedMode::Full => Some(DegradedMode::Reduced),
                DegradedMode::Reduced => Some(DegradedMode::Shedding),
                DegradedMode::Shedding => None,
            }
        } else if self.under >= b.recover_after {
            match self.mode {
                DegradedMode::Shedding => Some(DegradedMode::Reduced),
                DegradedMode::Reduced => Some(DegradedMode::Full),
                DegradedMode::Full => None,
            }
        } else {
            None
        }?;
        self.over = 0;
        self.under = 0;
        let from = self.mode;
        self.mode = next;
        Some(ModeChange {
            from,
            to: next,
            load_permille: permille,
            backlog,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(snr: Option<f32>, usability: Usability) -> QualityMetrics {
        QualityMetrics {
            window_ms: 1000,
            level_dbfs: -20.0,
            peak_dbfs: -10.0,
            clipping_ratio: 0.0,
            noise_floor_dbfs: None,
            snr_db: snr,
            dc_offset: 0.0,
            channel_imbalance_db: None,
            non_finite_samples: 0,
            lost_frames: 0,
            activity_ratio: 0.5,
            usability,
            reasons: vec![],
        }
    }

    #[test]
    fn adaptive_decisions_apply_and_revert_with_hysteresis() {
        let mut p = AdaptivePolicy::new(true);
        let mut out = Vec::new();
        p.on_window(&metrics(Some(8.0), Usability::Fair), &[], &mut out);
        assert!(out.is_empty());
        p.on_window(&metrics(Some(8.0), Usability::Fair), &[], &mut out);
        assert_eq!(out, vec![(PolicyDecision::ConservativeSegmentation, true)]);
        out.clear();
        for _ in 0..2 {
            p.on_window(&metrics(Some(25.0), Usability::Good), &[], &mut out);
        }
        assert!(out.is_empty(), "needs three good windows");
        p.on_window(
            &metrics(Some(25.0), Usability::Good),
            &[ConditionKind::Clipping],
            &mut out,
        );
        assert_eq!(
            out,
            vec![
                (PolicyDecision::ConservativeSegmentation, false),
                (PolicyDecision::SuppressLevelEstimates, true)
            ]
        );
        out.clear();
        p.on_window(&metrics(None, Usability::Unusable), &[], &mut out);
        assert_eq!(
            out,
            vec![
                (PolicyDecision::SuppressLevelEstimates, false),
                (PolicyDecision::LowConfidenceActivity, true)
            ]
        );
        out.clear();
        p.clear(&mut out);
        assert_eq!(out, vec![(PolicyDecision::LowConfidenceActivity, false)]);
        let mut off = AdaptivePolicy::new(false);
        off.on_window(&metrics(None, Usability::Unusable), &[], &mut out);
        assert!(off.active().is_empty());
    }

    #[test]
    fn governor_degrades_stepwise_and_recovers() {
        let budget = AnalysisBudget {
            degrade_after: 3,
            recover_after: 4,
            ..AnalysisBudget::default()
        };
        let mut g = ResourceGovernor::new(budget);
        let mut changes = Vec::new();
        for _ in 0..6 {
            changes.extend(g.note(0.9, 0));
        }
        assert_eq!(
            changes.iter().map(|c| c.to).collect::<Vec<_>>(),
            vec![DegradedMode::Reduced, DegradedMode::Shedding]
        );
        assert!(g.note(5.0, 40).is_none(), "already at the lowest level");
        changes.clear();
        for _ in 0..8 {
            changes.extend(g.note(0.1, 0));
        }
        assert_eq!(
            changes.iter().map(|c| c.to).collect::<Vec<_>>(),
            vec![DegradedMode::Reduced, DegradedMode::Full]
        );
        // Backlog alone counts as over budget; middling load resets runs.
        for _ in 0..2 {
            assert!(g.note(0.1, 30).is_none());
        }
        assert!(g.note(0.4, 0).is_none());
        for _ in 0..2 {
            assert!(g.note(0.1, 30).is_none());
        }
        assert_eq!(g.max_load_permille(), 5000);
        assert_eq!(g.max_backlog(), 40);
        // Degradation can be switched off: load is still recorded.
        let mut off = ResourceGovernor::new(AnalysisBudget {
            degrade: false,
            degrade_after: 1,
            ..AnalysisBudget::default()
        });
        assert!((0..10).all(|_| off.note(9.0, 49).is_none()));
        assert_eq!(
            (off.mode(), off.max_load_permille()),
            (DegradedMode::Full, 9000)
        );
    }
}
