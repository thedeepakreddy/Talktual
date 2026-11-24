//! Time semantics and the injectable [`Clock`].
//!
//! Every event carries two timestamps with different guarantees:
//!
//! * [`WallTime`] — UTC wall-clock time for humans and correlation with
//!   external systems. It can jump (NTP, manual changes) and is **never** used
//!   for ordering or durations inside CE.
//! * [`MonotonicTime`] — elapsed time since the runtime's clock origin. It never
//!   goes backwards within one runtime and is meaningful only within that
//!   runtime.
//!
//! The authoritative order of events is the envelope `sequence` number, not
//! either timestamp (see `docs/architecture/events.md`).
//!
//! Timeouts inside the runtime use the async runtime's timer, not [`Clock`], so
//! tests control them with the async runtime's paused-time facility.

use std::fmt;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Nanoseconds since the Unix epoch, UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WallTime(u64);

impl WallTime {
    /// Construct from nanoseconds since the Unix epoch.
    pub const fn from_unix_nanos(nanos: u64) -> Self {
        Self(nanos)
    }

    /// Nanoseconds since the Unix epoch.
    pub const fn unix_nanos(self) -> u64 {
        self.0
    }
}

/// Elapsed time since a runtime's clock origin.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct MonotonicTime(u64);

impl MonotonicTime {
    /// Construct from nanoseconds since the clock origin.
    pub const fn from_nanos(nanos: u64) -> Self {
        Self(nanos)
    }

    /// Nanoseconds since the clock origin.
    pub const fn as_nanos(self) -> u64 {
        self.0
    }

    /// Duration since the clock origin.
    pub const fn as_duration(self) -> Duration {
        Duration::from_nanos(self.0)
    }

    /// Saturating difference `self - earlier`.
    pub fn saturating_since(self, earlier: MonotonicTime) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(earlier.0))
    }
}

/// Source of time for timestamps. Injectable so tests are deterministic.
pub trait Clock: Send + Sync + fmt::Debug {
    /// Current wall-clock time.
    fn wall(&self) -> WallTime;
    /// Current monotonic time. Must never decrease.
    fn monotonic(&self) -> MonotonicTime;
}

/// The production clock, backed by `std::time`.
///
/// Note: `std::time::SystemTime::now` is unavailable on
/// `wasm32-unknown-unknown`; hosts on that target must inject their own clock.
#[derive(Debug, Clone, Copy)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    /// Create a clock whose monotonic origin is "now".
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn wall(&self) -> WallTime {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        WallTime(nanos)
    }

    fn monotonic(&self) -> MonotonicTime {
        MonotonicTime(u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_is_monotonic_and_after_epoch() {
        let clock = SystemClock::new();
        let a = clock.monotonic();
        let b = clock.monotonic();
        assert!(b >= a);
        assert!(clock.wall().unix_nanos() > 1_600_000_000_000_000_000);
        assert_eq!(b.saturating_since(a) + a.as_duration(), b.as_duration());
        assert_eq!(
            a.saturating_since(b),
            Duration::ZERO.max(a.saturating_since(b))
        );
    }
}
