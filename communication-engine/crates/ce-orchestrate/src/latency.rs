//! Bounded latency reservoirs and deadline outcomes (monotonic only).

use ce_core::orchestration::{LatencyMetric, LatencySummary};

/// Samples retained per reservoir.
pub const RESERVOIR: usize = 512;

/// A bounded ring of recent samples (ms) plus an all-time count.
#[derive(Clone, Debug, Default)]
pub struct Reservoir {
    samples: Vec<f32>,
    next: usize,
    count: u64,
}

impl Reservoir {
    /// Record a sample (non-finite or negative values are clamped to 0).
    pub fn record(&mut self, ms: f32) {
        let v = if ms.is_finite() { ms.max(0.0) } else { 0.0 };
        if self.samples.len() < RESERVOIR {
            self.samples.push(v);
        } else {
            self.samples[self.next] = v;
        }
        self.next = (self.next + 1) % RESERVOIR;
        self.count += 1;
    }

    /// Samples recorded (all time).
    pub fn count(&self) -> u64 {
        self.count
    }

    /// Samples retained.
    pub fn retained(&self) -> usize {
        self.samples.len()
    }

    /// Summary, or `None` when empty.
    pub fn summary(&self, metric: LatencyMetric) -> Option<LatencySummary> {
        if self.samples.is_empty() {
            return None;
        }
        let mut s = self.samples.clone();
        s.sort_by(f32::total_cmp);
        let q = |p: f32| {
            // Nearest rank.
            let rank = ((p * s.len() as f32).ceil() as usize).clamp(1, s.len());
            s[rank - 1]
        };
        Some(LatencySummary {
            metric,
            count: self.count,
            p50_ms: q(0.5),
            p95_ms: q(0.95),
            max_ms: s[s.len() - 1],
        })
    }
}

/// Outcome of a deadline check.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DeadlineOutcome {
    /// No budget configured.
    NoBudget,
    /// Within budget.
    Met,
    /// Over budget.
    Missed {
        /// Measured (ms).
        latency_ms: f32,
        /// Budget (ms).
        budget_ms: u32,
    },
}

/// Check a latency against an optional budget.
pub fn deadline(budget_ms: Option<u32>, latency_ms: f32) -> DeadlineOutcome {
    match budget_ms {
        None => DeadlineOutcome::NoBudget,
        Some(b) if latency_ms > b as f32 => DeadlineOutcome::Missed {
            latency_ms,
            budget_ms: b,
        },
        Some(_) => DeadlineOutcome::Met,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservoir_is_bounded_and_summarizes() {
        let mut r = Reservoir::default();
        assert!(r.summary(LatencyMetric::TranscriptFinal).is_none());
        for i in 1..=100 {
            r.record(i as f32);
        }
        let s = r.summary(LatencyMetric::TranscriptFinal).unwrap();
        assert_eq!(
            (s.count, s.p50_ms, s.p95_ms, s.max_ms),
            (100, 50.0, 95.0, 100.0)
        );
        for _ in 0..2_000 {
            r.record(1.0);
        }
        r.record(f32::NAN);
        r.record(-3.0);
        assert_eq!(r.retained(), RESERVOIR);
        assert_eq!(r.count(), 2_102);
        assert_eq!(r.summary(LatencyMetric::Overhead).unwrap().max_ms, 1.0);
    }

    #[test]
    fn deadlines_are_typed() {
        assert_eq!(deadline(None, 10_000.0), DeadlineOutcome::NoBudget);
        assert_eq!(deadline(Some(500), 500.0), DeadlineOutcome::Met);
        assert_eq!(
            deadline(Some(500), 501.5),
            DeadlineOutcome::Missed {
                latency_ms: 501.5,
                budget_ms: 500
            }
        );
    }
}
