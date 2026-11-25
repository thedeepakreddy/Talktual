//! Sample-clock timeline: normalization, re-anchors, gaps, ordering and
//! integrity (ADR-0032).
//!
//! * The stream's sample clock (`frame_index`) is authoritative. **Media
//!   time** is `frames / rate` — exact, so normalization is defensible.
//! * Reported `time_range`s are CE monotonic time derived from a host
//!   anchor (`anchor_time + (frame − anchor_frame) / rate`). They are kept
//!   next to media time, never rewritten.
//! * The offset `monotonic(start) − media(start)` is constant within one
//!   anchoring. A change of more than one frame (+1 µs rounding slack) is a
//!   **re-anchor** — a new mapping segment, reported, never smoothed.
//! * A zero `time_range` while frames advance means the producer had no
//!   anchor: timing is **unknown** (media time only; nothing invented).
//! * Uncertainty of the monotonic mapping is `unknown` unless the audio
//!   layer supplies a bound; device and host clocks are never assumed to
//!   be synchronized.

use std::collections::BTreeMap;

use ce_core::audio::frame::frames_to_duration;
use ce_core::orchestration::{StageKind, TimelineKind, TimelineStatus, TimingUncertainty};
use ce_core::time::MonotonicTime;

/// A span on the timeline, normalized next to its reported times.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NormalizedSpan {
    /// Sample-clock frames [start, end).
    pub frames: (u64, u64),
    /// Media time (ns since frame 0, exact).
    pub media_ns: (u64, u64),
    /// Reported monotonic times (as produced, unchanged).
    pub reported: (MonotonicTime, MonotonicTime),
    /// Mapping segment of the producing stage (0-based).
    pub segment: u32,
    /// Whether the reported times are anchored (else media time only).
    pub timing_known: bool,
}

#[derive(Clone, Debug, Default)]
struct Track {
    offset: Option<i128>,
    segment: u32,
    last_ordinal: Option<u64>,
    last_start: Option<u64>,
}

/// The timeline of one stream.
#[derive(Clone, Debug)]
pub struct Timeline {
    rate: u32,
    uncertainty: TimingUncertainty,
    tracks: BTreeMap<StageKind, Track>,
    lost_frames: u64,
    last: Option<NormalizedSpan>,
}

impl Timeline {
    /// A timeline for a stream whose sample clock runs at `rate`.
    pub fn new(rate: u32, uncertainty: TimingUncertainty) -> Self {
        Self {
            rate,
            uncertainty,
            tracks: BTreeMap::new(),
            lost_frames: 0,
            last: None,
        }
    }

    /// Media time of a frame (ns).
    pub fn media_ns(&self, frame: u64) -> u64 {
        u64::try_from(frames_to_duration(frame, self.rate).as_nanos()).unwrap_or(u64::MAX)
    }

    fn tolerance_ns(&self) -> i128 {
        let frame = if self.rate == 0 {
            0
        } else {
            (1_000_000_000_i128 + i128::from(self.rate) - 1) / i128::from(self.rate)
        };
        frame + 1_000
    }

    /// Observe a timed output of `stage`. `ordinal` identifies the unit
    /// (utterance, segment) when it has one: a new unit must not start
    /// before the previous one. Returns the normalized span and findings.
    pub fn observe(
        &mut self,
        stage: StageKind,
        ordinal: Option<u64>,
        frames: (u64, u64),
        reported: (MonotonicTime, MonotonicTime),
    ) -> (NormalizedSpan, Vec<TimelineKind>) {
        let mut findings = Vec::new();
        let media = (self.media_ns(frames.0), self.media_ns(frames.1));
        let tol = self.tolerance_ns();
        let track = self.tracks.entry(stage).or_default();
        // Ordering (by causal unit, not arrival).
        if let Some(o) = ordinal {
            match track.last_ordinal {
                Some(last) if o > last => {
                    if track.last_start.is_some_and(|s| frames.0 < s) {
                        findings.push(TimelineKind::OutOfOrder);
                    }
                    track.last_ordinal = Some(o);
                    track.last_start = Some(frames.0);
                }
                Some(_) => {}
                None => {
                    track.last_ordinal = Some(o);
                    track.last_start = Some(frames.0);
                }
            }
        }
        let unknown = reported.0.as_nanos() == 0 && reported.1.as_nanos() == 0 && frames.1 > 0;
        let malformed = frames.1 < frames.0 || reported.1 < reported.0;
        let mut timing_known = !unknown;
        if malformed {
            findings.push(TimelineKind::MalformedRange);
            timing_known = false;
        } else if unknown {
            findings.push(TimelineKind::UnknownTiming);
        } else {
            let reported_span =
                i128::from(reported.1.as_nanos()) - i128::from(reported.0.as_nanos());
            let media_span = i128::from(media.1) - i128::from(media.0);
            if (reported_span - media_span).abs() > tol {
                findings.push(TimelineKind::DurationMismatch);
            }
            let offset = i128::from(reported.0.as_nanos()) - i128::from(media.0);
            if let Some(prev) = track.offset {
                if (offset - prev).abs() > tol {
                    findings.push(TimelineKind::ReAnchor);
                    track.segment += 1;
                }
            }
            track.offset = Some(offset);
        }
        let span = NormalizedSpan {
            frames,
            media_ns: media,
            reported,
            segment: track.segment,
            timing_known,
        };
        if timing_known {
            self.last = Some(span);
        }
        (span, findings)
    }

    /// Record an upstream gap of `lost` frames (when known).
    pub fn gap(&mut self, lost: Option<u64>) -> TimelineKind {
        self.lost_frames = self.lost_frames.saturating_add(lost.unwrap_or(0));
        TimelineKind::Gap
    }

    /// Summary.
    pub fn status(&self) -> TimelineStatus {
        let offset = self
            .last
            .map(|s| i128::from(s.reported.0.as_nanos()) - i128::from(s.media_ns.0))
            .map(|o| i64::try_from(o).unwrap_or(if o < 0 { i64::MIN } else { i64::MAX }));
        TimelineStatus {
            sample_rate: self.rate,
            segments: if self.last.is_some() {
                1 + self.tracks.values().map(|t| t.segment).max().unwrap_or(0)
            } else {
                0
            },
            media_end_ns: self.last.map(|s| s.media_ns.1),
            monotonic_end_ns: self.last.map(|s| s.reported.1.as_nanos()),
            offset_ns: offset,
            lost_frames: self.lost_frames,
            uncertainty: self.uncertainty,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;
    const ANCHOR: u64 = 5_000_000_000; // 5 s after the runtime origin

    fn mt(n: u64) -> MonotonicTime {
        MonotonicTime::from_nanos(n)
    }

    /// Reported times as the producers derive them: anchor + frames / rate.
    fn at(anchor: u64, f0: u64, f1: u64) -> (MonotonicTime, MonotonicTime) {
        let d = |f: u64| u64::try_from(frames_to_duration(f, RATE).as_nanos()).unwrap();
        (mt(anchor + d(f0)), mt(anchor + d(f1)))
    }

    #[test]
    fn consistent_spans_normalize_exactly_without_findings() {
        let mut tl = Timeline::new(RATE, TimingUncertainty::Unknown);
        let (s, f) = tl.observe(
            StageKind::Recognition,
            Some(0),
            (16_000, 32_000),
            at(ANCHOR, 16_000, 32_000),
        );
        assert!(f.is_empty(), "{f:?}");
        assert_eq!(s.media_ns, (1_000_000_000, 2_000_000_000));
        assert_eq!(
            s.reported,
            at(ANCHOR, 16_000, 32_000),
            "reported times are kept unchanged"
        );
        let (_, f) = tl.observe(
            StageKind::Recognition,
            Some(1),
            (40_000, 48_001),
            at(ANCHOR, 40_000, 48_001),
        );
        assert!(
            f.is_empty(),
            "rounding within one frame is not a finding: {f:?}"
        );
        let st = tl.status();
        assert_eq!(st.segments, 1);
        assert_eq!(st.offset_ns, Some(i64::try_from(ANCHOR).unwrap()));
        assert_eq!(st.media_end_ns, Some(3_000_062_500));
        assert_eq!(st.uncertainty, TimingUncertainty::Unknown);
    }

    #[test]
    fn a_changed_mapping_is_a_reanchor_not_smoothed() {
        let mut tl = Timeline::new(RATE, TimingUncertainty::Unknown);
        tl.observe(
            StageKind::Recognition,
            Some(0),
            (0, 8_000),
            at(ANCHOR, 0, 8_000),
        );
        // A stream restart: real time passed, the sample clock did not advance.
        let later = ANCHOR + 3_000_000_000;
        let (s, f) = tl.observe(
            StageKind::Recognition,
            Some(1),
            (8_000, 16_000),
            at(later, 8_000, 16_000),
        );
        assert_eq!(f, vec![TimelineKind::ReAnchor]);
        assert_eq!(s.segment, 1);
        assert_eq!(s.reported.0, at(later, 8_000, 16_000).0, "never rewritten");
        assert_eq!(tl.status().segments, 2);
        // Stages are tracked separately (each producer anchors itself).
        let (_, f) = tl.observe(StageKind::Analysis, None, (0, 160), at(ANCHOR + 7, 0, 160));
        assert!(f.is_empty());
    }

    #[test]
    fn integrity_ordering_unknown_timing_and_gaps() {
        let mut tl = Timeline::new(RATE, TimingUncertainty::Unknown);
        let (_, f) = tl.observe(StageKind::Recognition, Some(0), (10, 5), at(ANCHOR, 0, 0));
        assert_eq!(f, vec![TimelineKind::MalformedRange]);
        let (_, f) = tl.observe(StageKind::Recognition, Some(1), (100, 200), (mt(10), mt(5)));
        assert_eq!(f, vec![TimelineKind::MalformedRange]);
        let (_, f) = tl.observe(
            StageKind::Recognition,
            Some(2),
            (1_000, 2_000),
            (mt(ANCHOR), mt(ANCHOR + 1_000_000_000)),
        );
        assert_eq!(f, vec![TimelineKind::DurationMismatch]);
        let (s, f) = tl.observe(
            StageKind::Recognition,
            Some(3),
            (16_000, 20_000),
            (mt(0), mt(0)),
        );
        assert_eq!(f, vec![TimelineKind::UnknownTiming]);
        assert!(!s.timing_known);
        assert_eq!(s.media_ns.0, 1_000_000_000, "media time is still exact");
        let (_, f) = tl.observe(
            StageKind::Recognition,
            Some(4),
            (500, 900),
            at(ANCHOR, 500, 900),
        );
        assert!(f.contains(&TimelineKind::OutOfOrder), "{f:?}");
        let (_, f) = tl.observe(
            StageKind::Recognition,
            Some(4),
            (100, 900),
            at(ANCHOR, 100, 900),
        );
        assert!(
            !f.contains(&TimelineKind::OutOfOrder),
            "a revision of the same unit may move its start"
        );
        assert_eq!(tl.gap(Some(480)), TimelineKind::Gap);
        tl.gap(None);
        assert_eq!(tl.status().lost_frames, 480);
    }
}
