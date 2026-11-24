//! Bounded latency statistics (same method as `ce-speech`: nearest rank
//! over the most recent values).

use ce_core::speech::Percentiles;

/// Keeps the most recent `cap` observations (bounded memory) and reports
/// percentiles over them.
#[derive(Clone, Debug)]
pub struct Reservoir {
    values: std::collections::VecDeque<f32>,
    cap: usize,
    count: u64,
    max: f32,
}

impl Reservoir {
    /// Holds at most `cap` values.
    pub fn new(cap: usize) -> Self {
        Self {
            values: std::collections::VecDeque::with_capacity(cap.min(4_096)),
            cap: cap.max(1),
            count: 0,
            max: 0.0,
        }
    }

    /// Record one value (non-finite values are ignored).
    pub fn record(&mut self, v: f32) {
        if !v.is_finite() {
            return;
        }
        if self.values.len() == self.cap {
            self.values.pop_front();
        }
        self.values.push_back(v);
        self.count += 1;
        self.max = self.max.max(v);
    }

    /// Nearest-rank percentiles over the retained values (`count` is all
    /// values ever recorded; `max` too).
    pub fn summary(&self) -> Percentiles {
        if self.values.is_empty() {
            return Percentiles::default();
        }
        let mut v: Vec<f32> = self.values.iter().copied().collect();
        v.sort_by(f32::total_cmp);
        let rank = |p: f64| v[(((p * v.len() as f64).ceil() as usize).max(1) - 1).min(v.len() - 1)];
        Percentiles {
            count: self.count,
            p50: rank(0.50),
            p95: rank(0.95),
            p99: rank(0.99),
            max: self.max,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentiles_and_bounded_memory() {
        let mut r = Reservoir::new(100);
        assert_eq!(r.summary(), Percentiles::default());
        for i in 1..=100 {
            r.record(i as f32);
        }
        r.record(f32::NAN);
        let s = r.summary();
        assert_eq!(
            (s.count, s.p50, s.p95, s.p99, s.max),
            (100, 50.0, 95.0, 99.0, 100.0)
        );
        for _ in 0..1_000 {
            r.record(1.0);
        }
        assert_eq!(r.values.len(), 100);
        assert_eq!(r.summary().max, 100.0, "max covers everything recorded");
    }
}
