//! Worker generations and stale-output admission (ADR-0031).
//!
//! Every stage worker start gets a fresh generation from one
//! engine-wide counter. A worker's outputs carry its generation; the
//! runtime admits an output only if the stream's current worker for that
//! stage has the same generation. Everything else is a stale output of an
//! earlier (stopped, abandoned or replaced) worker.

/// Engine-wide monotonic generation source (starts at 1).
#[derive(Clone, Debug, Default)]
pub struct GenerationCounter {
    last: u64,
}

impl GenerationCounter {
    /// The next generation.
    pub fn issue(&mut self) -> u64 {
        self.last += 1;
        self.last
    }

    /// The last generation handed out (0 if none).
    pub fn last(&self) -> u64 {
        self.last
    }
}

/// Whether an output may be published.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    /// It belongs to the current worker.
    Current,
    /// It belongs to an earlier worker; `current` is the live one, if any.
    Stale {
        /// The current generation for the stream and stage, if a worker runs.
        current: Option<u64>,
    },
}

/// Admit an output of `generation` given the stream's `current` worker
/// generation for the stage.
pub fn admit(current: Option<u64>, generation: u64) -> Admission {
    if current == Some(generation) {
        Admission::Current
    } else {
        Admission::Stale { current }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generations_are_monotonic_and_old_ones_are_stale() {
        let mut g = GenerationCounter::default();
        let first = g.issue();
        let second = g.issue();
        assert!(second > first);
        assert_eq!(g.last(), second);
        assert_eq!(admit(Some(second), second), Admission::Current);
        assert_eq!(
            admit(Some(second), first),
            Admission::Stale {
                current: Some(second)
            },
            "an output of the replaced worker after a restart"
        );
        assert_eq!(
            admit(None, first),
            Admission::Stale { current: None },
            "an output after stop or abandonment"
        );
    }
}
