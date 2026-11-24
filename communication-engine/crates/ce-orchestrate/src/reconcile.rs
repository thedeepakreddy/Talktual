//! Bounded transcript-revision reconciliation (ADR-0021 semantics) and
//! classification of derived outputs by the revision they refer to.

use std::collections::BTreeMap;

use ce_core::speech::RevisionStatus;
use ce_core::time::MonotonicTime;

/// What a transcript revision is, relative to what was seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevisionClass {
    /// Newer than anything seen for its utterance (or a new utterance).
    Applied,
    /// Equal to the latest seen revision.
    Duplicate,
    /// Older than the latest seen revision.
    Stale,
    /// The utterance had already ended (final or cancelled).
    AfterFinal,
    /// The utterance is older than the bounded window (already forgotten).
    Evicted,
}

/// What a derived output's source revision is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceClass {
    /// The latest revision of its utterance.
    Current,
    /// An older revision of a known utterance (allowed; replaced downstream).
    Superseded {
        /// The latest revision.
        latest: u32,
    },
    /// Never seen: a causal violation.
    Unknown,
    /// Outside the bounded window.
    Evicted,
}

#[derive(Clone, Copy, Debug)]
struct Utterance {
    latest: u32,
    ended: bool,
    audio_end: MonotonicTime,
    frame_end: u64,
}

/// Bounded table of utterances (oldest forgotten first).
#[derive(Clone, Debug)]
pub struct TranscriptTable {
    capacity: usize,
    utterances: BTreeMap<u64, Utterance>,
    /// Every utterance `< evicted_below` that is not in the table was
    /// forgotten (utterance numbers only grow).
    evicted_below: u64,
}

impl TranscriptTable {
    /// A table remembering at most `capacity` utterances (≥ 1).
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            utterances: BTreeMap::new(),
            evicted_below: 0,
        }
    }

    /// Utterances remembered.
    pub fn len(&self) -> usize {
        self.utterances.len()
    }

    /// Whether empty.
    pub fn is_empty(&self) -> bool {
        self.utterances.is_empty()
    }

    /// Record a transcript revision.
    pub fn apply(
        &mut self,
        utterance: u64,
        revision: u32,
        status: RevisionStatus,
        audio_end: MonotonicTime,
        frame_end: u64,
    ) -> RevisionClass {
        let ended = status != RevisionStatus::Partial;
        if let Some(u) = self.utterances.get_mut(&utterance) {
            if u.ended {
                return RevisionClass::AfterFinal;
            }
            if revision < u.latest {
                return RevisionClass::Stale;
            }
            if revision == u.latest {
                return RevisionClass::Duplicate;
            }
            u.latest = revision;
            u.ended = ended;
            u.audio_end = audio_end;
            u.frame_end = frame_end;
            return RevisionClass::Applied;
        }
        if utterance < self.evicted_below {
            return RevisionClass::Evicted;
        }
        self.utterances.insert(
            utterance,
            Utterance {
                latest: revision,
                ended,
                audio_end,
                frame_end,
            },
        );
        while self.utterances.len() > self.capacity {
            if let Some((&oldest, _)) = self.utterances.iter().next() {
                self.utterances.remove(&oldest);
                self.evicted_below = self.evicted_below.max(oldest + 1);
            }
        }
        RevisionClass::Applied
    }

    /// Classify a derived output's source.
    pub fn source(&self, utterance: u64, revision: u32) -> SourceClass {
        match self.utterances.get(&utterance) {
            Some(u) if revision == u.latest => SourceClass::Current,
            Some(u) if revision < u.latest => SourceClass::Superseded { latest: u.latest },
            Some(_) => SourceClass::Unknown,
            None if utterance < self.evicted_below => SourceClass::Evicted,
            None => SourceClass::Unknown,
        }
    }

    /// Audio end (reported monotonic) of the utterance's latest revision.
    pub fn audio_end(&self, utterance: u64) -> Option<MonotonicTime> {
        self.utterances.get(&utterance).map(|u| u.audio_end)
    }

    /// Sample-clock end of the utterance's latest revision.
    pub fn frame_end(&self, utterance: u64) -> Option<u64> {
        self.utterances.get(&utterance).map(|u| u.frame_end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RevisionStatus::*;

    fn t(n: u64) -> MonotonicTime {
        MonotonicTime::from_nanos(n)
    }

    #[test]
    fn revisions_must_increase_and_end_once() {
        let mut tt = TranscriptTable::new(8);
        assert_eq!(tt.apply(0, 0, Partial, t(1), 10), RevisionClass::Applied);
        assert_eq!(tt.apply(0, 0, Partial, t(1), 10), RevisionClass::Duplicate);
        assert_eq!(tt.apply(0, 2, Partial, t(2), 20), RevisionClass::Applied);
        assert_eq!(tt.apply(0, 1, Partial, t(2), 20), RevisionClass::Stale);
        assert_eq!(tt.apply(0, 3, Final, t(3), 30), RevisionClass::Applied);
        assert_eq!(tt.apply(0, 4, Partial, t(4), 40), RevisionClass::AfterFinal);
        assert_eq!(tt.apply(1, 0, Cancelled, t(5), 50), RevisionClass::Applied);
        assert_eq!(tt.apply(1, 1, Final, t(5), 50), RevisionClass::AfterFinal);
        assert_eq!(tt.audio_end(0), Some(t(3)));
        assert_eq!(tt.frame_end(0), Some(30));
    }

    #[test]
    fn derived_outputs_are_classified_by_source() {
        let mut tt = TranscriptTable::new(8);
        tt.apply(5, 0, Partial, t(1), 1);
        tt.apply(5, 1, Final, t(2), 2);
        assert_eq!(tt.source(5, 1), SourceClass::Current);
        assert_eq!(tt.source(5, 0), SourceClass::Superseded { latest: 1 });
        assert_eq!(
            tt.source(5, 2),
            SourceClass::Unknown,
            "a revision never emitted"
        );
        assert_eq!(tt.source(6, 0), SourceClass::Unknown);
    }

    #[test]
    fn the_window_is_bounded_and_eviction_is_distinguished() {
        let mut tt = TranscriptTable::new(3);
        for u in 0..10 {
            tt.apply(u, 0, Final, t(u), u);
        }
        assert_eq!(tt.len(), 3);
        assert_eq!(tt.source(2, 0), SourceClass::Evicted);
        assert_eq!(tt.source(9, 0), SourceClass::Current);
        assert_eq!(tt.source(42, 0), SourceClass::Unknown);
        assert_eq!(tt.apply(1, 1, Final, t(1), 1), RevisionClass::Evicted);
    }
}
