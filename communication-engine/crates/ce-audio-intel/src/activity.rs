//! Activity stabilization: hysteresis over detector scores.
//!
//! * Inactive → Active after `onset_hops` consecutive hops with score ≥
//!   `onset_threshold`. The boundary is backdated to the first of them; the
//!   decision latency is the time from the boundary to the deciding hop.
//! * Active → Inactive after `hangover_hops` consecutive hops with score <
//!   `offset_threshold` (hangover). Scores between the thresholds neither
//!   start nor end activity (dead band), which suppresses flapping.
//! * `reset()` forgets runs (after a discontinuity); the caller closes any
//!   open activity explicitly — a gap is never bridged.

use ce_core::intel::{ActivityConfig, ActivityState};

use crate::framer::Hop;

/// A decided transition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transition {
    /// New state.
    pub state: ActivityState,
    /// First hop of the qualifying run (the boundary).
    pub boundary: Hop,
    /// Hop at which the change was decided.
    pub decided: Hop,
    /// Mean score over the qualifying run.
    pub mean_score: f32,
}

/// Effective thresholds in hops.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StabilizerParams {
    /// Onset threshold.
    pub onset_threshold: f32,
    /// Offset threshold.
    pub offset_threshold: f32,
    /// Hops required for onset.
    pub onset_hops: u32,
    /// Hops required for offset.
    pub hangover_hops: u32,
}

impl StabilizerParams {
    /// From configuration and hop length.
    pub fn from_config(cfg: &ActivityConfig, hop_ms: u32) -> Self {
        let c = cfg.effective();
        let hops = |ms: u32| ms.div_ceil(hop_ms).max(1);
        Self {
            onset_threshold: c.onset_threshold,
            offset_threshold: c.offset_threshold,
            onset_hops: hops(c.onset_min_ms),
            hangover_hops: hops(c.offset_hangover_ms),
        }
    }

    /// The stricter of two parameter sets (for conservative segmentation).
    pub fn stricter(self, other: StabilizerParams) -> Self {
        Self {
            onset_threshold: self.onset_threshold.max(other.onset_threshold),
            offset_threshold: self
                .offset_threshold
                .max(other.offset_threshold)
                .min(self.onset_threshold.max(other.onset_threshold) - 0.05),
            onset_hops: self.onset_hops.max(other.onset_hops),
            hangover_hops: self.hangover_hops.max(other.hangover_hops),
        }
    }
}

/// The hysteresis state machine.
#[derive(Clone, Debug)]
pub struct Stabilizer {
    params: StabilizerParams,
    state: ActivityState,
    run: u32,
    run_start: Option<Hop>,
    run_score: f32,
}

impl Stabilizer {
    /// Start inactive.
    pub fn new(params: StabilizerParams) -> Self {
        Self {
            params,
            state: ActivityState::Inactive,
            run: 0,
            run_start: None,
            run_score: 0.0,
        }
    }

    /// Current state.
    pub fn state(&self) -> ActivityState {
        self.state
    }

    /// Current parameters.
    pub fn params(&self) -> StabilizerParams {
        self.params
    }

    /// Change parameters (takes effect on the next hop; runs are kept).
    pub fn set_params(&mut self, params: StabilizerParams) {
        self.params = params;
    }

    /// Forget runs and return to Inactive (after a discontinuity).
    pub fn reset(&mut self) {
        self.state = ActivityState::Inactive;
        self.run = 0;
        self.run_start = None;
        self.run_score = 0.0;
    }

    /// Feed one hop's score.
    pub fn step(&mut self, hop: &Hop, score: f32) -> Option<Transition> {
        let p = self.params;
        let counts = match self.state {
            ActivityState::Inactive => score >= p.onset_threshold,
            ActivityState::Active => score < p.offset_threshold,
        };
        if !counts {
            self.run = 0;
            self.run_start = None;
            self.run_score = 0.0;
            return None;
        }
        if self.run == 0 {
            self.run_start = Some(*hop);
        }
        self.run += 1;
        self.run_score += score;
        let needed = match self.state {
            ActivityState::Inactive => p.onset_hops,
            ActivityState::Active => p.hangover_hops,
        };
        if self.run < needed {
            return None;
        }
        let boundary = self.run_start.unwrap_or(*hop);
        let mean_score = self.run_score / self.run as f32;
        self.state = match self.state {
            ActivityState::Inactive => ActivityState::Active,
            ActivityState::Active => ActivityState::Inactive,
        };
        self.run = 0;
        self.run_start = None;
        self.run_score = 0.0;
        Some(Transition {
            state: self.state,
            boundary,
            decided: *hop,
            mean_score,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::hop;
    use ce_core::intel::ActivityProfile;

    fn params() -> StabilizerParams {
        StabilizerParams {
            onset_threshold: 0.6,
            offset_threshold: 0.4,
            onset_hops: 3,
            hangover_hops: 5,
        }
    }

    fn run(s: &mut Stabilizer, scores: &[f32]) -> Vec<(u64, u64, ActivityState)> {
        scores
            .iter()
            .enumerate()
            .filter_map(|(i, sc)| {
                s.step(&hop(i as u64), *sc)
                    .map(|t| (t.boundary.index, t.decided.index, t.state))
            })
            .collect()
    }

    #[test]
    fn onset_and_offset_are_backdated_and_need_their_run_lengths() {
        let mut s = Stabilizer::new(params());
        let mut scores = vec![0.0; 5];
        scores.extend([0.9; 10]);
        scores.extend([0.1; 10]);
        let t = run(&mut s, &scores);
        assert_eq!(
            t,
            vec![
                (5, 7, ActivityState::Active),
                (15, 19, ActivityState::Inactive)
            ]
        );
    }

    #[test]
    fn short_blips_and_brief_dips_do_not_flap() {
        let mut s = Stabilizer::new(params());
        // Two-hop blips never reach the 3-hop onset.
        let blips: Vec<f32> = (0..40).map(|i| if i % 4 < 2 { 0.9 } else { 0.0 }).collect();
        assert!(run(&mut s, &blips).is_empty());
        // Once active, 4-hop dips stay inside the 5-hop hangover.
        let mut s = Stabilizer::new(params());
        let mut sc = vec![0.9; 5];
        for _ in 0..5 {
            sc.extend([0.1; 4]);
            sc.extend([0.9; 2]);
        }
        assert_eq!(run(&mut s, &sc).len(), 1);
        assert_eq!(s.state(), ActivityState::Active);
    }

    #[test]
    fn the_dead_band_between_thresholds_holds_the_current_state() {
        let mut s = Stabilizer::new(params());
        assert!(run(&mut s, &[0.5; 50]).is_empty());
        let mut s = Stabilizer::new(params());
        let mut sc = vec![0.9; 3];
        sc.extend([0.5; 50]);
        assert_eq!(
            run(&mut s, &sc).len(),
            1,
            "onset only; 0.5 is not below 0.4"
        );
    }

    #[test]
    fn reset_returns_to_inactive_and_params_follow_profiles() {
        let mut s = Stabilizer::new(params());
        run(&mut s, &[0.9; 3]);
        assert_eq!(s.state(), ActivityState::Active);
        s.reset();
        assert_eq!(s.state(), ActivityState::Inactive);
        let p = StabilizerParams::from_config(
            &ActivityConfig::preset(ActivityProfile::Conservative),
            10,
        );
        assert_eq!((p.onset_hops, p.hangover_hops), (8, 40));
        let p20 = StabilizerParams::from_config(&ActivityConfig::default(), 20);
        assert_eq!((p20.onset_hops, p20.hangover_hops), (2, 12));
        let strict = params().stricter(p);
        assert_eq!(
            (
                strict.onset_threshold,
                strict.onset_hops,
                strict.hangover_hops
            ),
            (0.7, 8, 40)
        );
        assert!(strict.offset_threshold < strict.onset_threshold);
    }
}
