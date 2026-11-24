use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ce_core::ids::IdGenerator;
use ce_core::observability::{ObservabilitySink, Observation};
use ce_core::time::{Clock, MonotonicTime, WallTime};
use uuid::Uuid;

/// A clock that only moves when told to.
#[derive(Debug)]
pub struct ManualClock {
    now: Mutex<(u64, u64)>,
}

impl ManualClock {
    /// Start at the given wall time (ns since epoch) with monotonic time 0.
    pub fn new(wall_unix_nanos: u64) -> Self {
        Self {
            now: Mutex::new((wall_unix_nanos, 0)),
        }
    }

    /// Advance both clocks.
    pub fn advance(&self, by: Duration) {
        let nanos = u64::try_from(by.as_nanos()).unwrap_or(u64::MAX);
        let mut now = self.now.lock().unwrap_or_else(|p| p.into_inner());
        now.0 = now.0.saturating_add(nanos);
        now.1 = now.1.saturating_add(nanos);
    }

    /// Move only the wall clock backwards (simulates an NTP correction).
    pub fn rewind_wall(&self, by: Duration) {
        let nanos = u64::try_from(by.as_nanos()).unwrap_or(u64::MAX);
        let mut now = self.now.lock().unwrap_or_else(|p| p.into_inner());
        now.0 = now.0.saturating_sub(nanos);
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new(1_700_000_000_000_000_000)
    }
}

impl Clock for ManualClock {
    fn wall(&self) -> WallTime {
        WallTime::from_unix_nanos(self.now.lock().unwrap_or_else(|p| p.into_inner()).0)
    }

    fn monotonic(&self) -> MonotonicTime {
        MonotonicTime::from_nanos(self.now.lock().unwrap_or_else(|p| p.into_inner()).1)
    }
}

/// Generates UUIDs 1, 2, 3, … for reproducible identifiers.
#[derive(Debug, Default)]
pub struct SequentialIds {
    next: AtomicU64,
}

impl IdGenerator for SequentialIds {
    fn generate(&self) -> Uuid {
        Uuid::from_u128(u128::from(self.next.fetch_add(1, Ordering::Relaxed) + 1))
    }
}

/// Captures every observation the runtime records.
#[derive(Debug, Default)]
pub struct RecordingSink {
    records: Mutex<Vec<Observation>>,
}

impl RecordingSink {
    /// Everything recorded so far.
    pub fn records(&self) -> Vec<Observation> {
        self.records
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// All recorded field values and labels concatenated, for leak checks.
    pub fn rendered(&self) -> String {
        self.records()
            .iter()
            .map(|o| format!("{o:?}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl ObservabilitySink for RecordingSink {
    fn record(&self, observation: &Observation) {
        self.records
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(observation.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_is_deterministic_and_wall_can_jump_back() {
        let c = ManualClock::new(1_000);
        assert_eq!(c.monotonic().as_nanos(), 0);
        c.advance(Duration::from_nanos(500));
        assert_eq!(
            (c.wall().unix_nanos(), c.monotonic().as_nanos()),
            (1_500, 500)
        );
        c.rewind_wall(Duration::from_nanos(1_000));
        assert_eq!(
            (c.wall().unix_nanos(), c.monotonic().as_nanos()),
            (500, 500)
        );
    }

    #[test]
    fn sequential_ids_count_from_one() {
        let ids = SequentialIds::default();
        assert_eq!(ids.generate(), Uuid::from_u128(1));
        assert_eq!(ids.generate(), Uuid::from_u128(2));
    }
}
