//! Candidate segments, endpoint candidates and silence windows.
//!
//! These are **acoustic** boundaries derived from stabilized activity:
//!
//! * A segment opens at an activity onset boundary and closes when the
//!   following pause reaches `close_silence_ms` (`SilencePolicy`), when it
//!   reaches `max_segment_ms` (`MaxDuration`, a continuation opens), or when
//!   the caller closes it (discontinuity, stop, stream end, resource policy,
//!   failure). Activity that resumes before `close_silence_ms` stays in the
//!   same segment.
//! * An endpoint candidate is emitted once per tier of
//!   `endpoint_tiers_ms` as a pause grows. It means "activity has paused
//!   this long", never "the speaker has finished": turn-taking decisions
//!   belong to later phases.
//! * A silence window is any inactive stretch ≥ `min_silence_report_ms`,
//!   reported when it ends.
//!
//! Spans are half-open `[start, end)` on the sample clock.

use ce_core::intel::{ActivityState, EndpointConfidence, SegmentCloseReason, SegmentConfig};
use ce_core::time::MonotonicTime;

use crate::activity::Transition;
use crate::dsp::power_db;
use crate::framer::Hop;

/// A position on the stream's sample clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    /// Source block sequence.
    pub sequence: u64,
    /// Frame index.
    pub frame: u64,
    /// Monotonic time.
    pub time: MonotonicTime,
}

impl Point {
    /// Start of a hop.
    pub fn start_of(h: &Hop) -> Self {
        Self {
            sequence: h.sequence,
            frame: h.frame_start,
            time: h.time,
        }
    }

    /// End of a hop.
    pub fn end_of(h: &Hop) -> Self {
        Self {
            sequence: h.sequence,
            frame: h.frame_start + h.frames as u64,
            time: h.end_time,
        }
    }
}

/// A half-open span.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// Start.
    pub start: Point,
    /// End.
    pub end: Point,
}

impl Span {
    /// Duration in ms.
    pub fn duration_ms(&self) -> u32 {
        ms_between(self.start.time, self.end.time)
    }
}

fn ms_between(a: MonotonicTime, b: MonotonicTime) -> u32 {
    u32::try_from(b.saturating_since(a).as_millis()).unwrap_or(u32::MAX)
}

/// Output of the segmenter.
#[derive(Clone, Debug, PartialEq)]
pub enum SegmentEvent {
    /// Segment opened.
    Started {
        /// Span (start = onset; end = start).
        span: Span,
        /// Segment number.
        segment: u64,
        /// Continues a `MaxDuration` close.
        continuation: bool,
    },
    /// Segment closed.
    Ended {
        /// [first activity, last activity).
        span: Span,
        /// Segment number.
        segment: u64,
        /// Why.
        reason: SegmentCloseReason,
        /// Mean score over active hops.
        mean_score: Option<f32>,
    },
    /// Endpoint candidate.
    Endpoint {
        /// The pause so far.
        span: Span,
        /// Segment number.
        segment: u64,
        /// Pause length, ms.
        silence_ms: u32,
        /// Tier index.
        tier: u8,
        /// Confidence.
        confidence: EndpointConfidence,
    },
    /// Silence window ended.
    Silence {
        /// The silence.
        span: Span,
        /// Mean level, dBFS.
        mean_level_dbfs: f32,
    },
}

#[derive(Clone, Debug)]
struct Open {
    id: u64,
    start: Point,
    last_active_end: Point,
    score_sum: f64,
    score_n: u32,
    pause_start: Option<Point>,
    tiers_emitted: usize,
}

#[derive(Clone, Debug)]
struct Silence {
    start: Point,
    end: Point,
    power_sum: f64,
    hops: u32,
}

/// The segmenter.
#[derive(Clone, Debug)]
pub struct Segmenter {
    cfg: SegmentConfig,
    open: Option<Open>,
    silence: Option<Silence>,
    next_id: u64,
    pending_continuation: bool,
    last_end: Option<Point>,
}

impl Segmenter {
    /// A segmenter with `cfg` (validated).
    pub fn new(cfg: SegmentConfig) -> Self {
        Self {
            cfg,
            open: None,
            silence: None,
            next_id: 0,
            pending_continuation: false,
            last_end: None,
        }
    }

    /// Segments closed so far (ids are 0-based).
    pub fn segments_opened(&self) -> u64 {
        self.next_id
    }

    /// Whether a segment is open.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    fn open_segment(&mut self, at: Point, continuation: bool, out: &mut Vec<SegmentEvent>) {
        let id = self.next_id;
        self.next_id += 1;
        self.open = Some(Open {
            id,
            start: at,
            last_active_end: at,
            score_sum: 0.0,
            score_n: 0,
            pause_start: None,
            tiers_emitted: 0,
        });
        out.push(SegmentEvent::Started {
            span: Span { start: at, end: at },
            segment: id,
            continuation,
        });
    }

    fn close_silence(&mut self, end: Point, out: &mut Vec<SegmentEvent>) {
        if let Some(s) = self.silence.take() {
            let span = Span {
                start: s.start,
                end,
            };
            if span.duration_ms() >= self.cfg.min_silence_report_ms {
                let mean = if s.hops > 0 {
                    (s.power_sum / s.hops as f64) as f32
                } else {
                    0.0
                };
                out.push(SegmentEvent::Silence {
                    span,
                    mean_level_dbfs: power_db(mean),
                });
            }
        }
    }

    fn close_segment(&mut self, reason: SegmentCloseReason, out: &mut Vec<SegmentEvent>) {
        if let Some(o) = self.open.take() {
            let end = o.pause_start.unwrap_or(o.last_active_end);
            out.push(SegmentEvent::Ended {
                span: Span {
                    start: o.start,
                    end,
                },
                segment: o.id,
                reason,
                mean_score: (o.score_n > 0).then(|| (o.score_sum / o.score_n as f64) as f32),
            });
        }
    }

    /// Feed one hop with its score, the state after the stabilizer step and
    /// the transition it decided (if any).
    pub fn step(
        &mut self,
        hop: &Hop,
        score: f32,
        state: ActivityState,
        transition: Option<&Transition>,
        out: &mut Vec<SegmentEvent>,
    ) {
        let here_end = Point::end_of(hop);
        self.last_end = Some(here_end);
        if let Some(t) = transition {
            let boundary = Point::start_of(&t.boundary);
            match t.state {
                ActivityState::Active => {
                    self.close_silence(boundary, out);
                    if self.open.is_none() {
                        let cont = std::mem::take(&mut self.pending_continuation);
                        self.open_segment(boundary, cont, out);
                    }
                    let o = self.open.as_mut().expect("just opened");
                    o.pause_start = None;
                    o.tiers_emitted = 0;
                    // The qualifying run before the deciding hop was scored
                    // while still inactive; count it as active.
                    let before = t.decided.index.saturating_sub(t.boundary.index);
                    o.score_sum += t.mean_score as f64 * before as f64;
                    o.score_n += before as u32;
                }
                ActivityState::Inactive => {
                    if let Some(o) = &mut self.open {
                        o.pause_start = Some(boundary);
                        // Hangover hops were scored while still active; they
                        // belong to the pause.
                        let run = t.decided.index.saturating_sub(t.boundary.index);
                        o.score_sum -= t.mean_score as f64 * run as f64;
                        o.score_n = o.score_n.saturating_sub(run as u32);
                    }
                    self.pending_continuation = false;
                    self.silence = Some(Silence {
                        start: boundary,
                        end: boundary,
                        power_sum: 0.0,
                        hops: 0,
                    });
                }
            }
        }
        match state {
            ActivityState::Active => {
                if self.open.is_none() {
                    // Activity continuing after a MaxDuration close.
                    self.open_segment(Point::start_of(hop), true, out);
                }
                let max = self.cfg.max_segment_ms;
                let o = self.open.as_mut().expect("segment open while active");
                o.score_sum += score as f64;
                o.score_n += 1;
                o.last_active_end = here_end;
                if ms_between(o.start.time, here_end.time) >= max {
                    self.close_segment(SegmentCloseReason::MaxDuration, out);
                    self.pending_continuation = true;
                }
            }
            ActivityState::Inactive => {
                match &mut self.silence {
                    Some(s) => {
                        s.power_sum += hop.features.mean_square as f64;
                        s.hops += 1;
                        s.end = here_end;
                    }
                    None => {
                        self.silence = Some(Silence {
                            start: Point::start_of(hop),
                            end: here_end,
                            power_sum: hop.features.mean_square as f64,
                            hops: 1,
                        });
                    }
                }
                let tiers = &self.cfg.endpoint_tiers_ms;
                let close = self.cfg.close_silence_ms;
                let mut close_now = false;
                let paused = self
                    .open
                    .as_mut()
                    .and_then(|o| o.pause_start.map(|ps| (o, ps)));
                if let Some((o, ps)) = paused {
                    let silence_ms = ms_between(ps.time, here_end.time);
                    while o.tiers_emitted < tiers.len() && silence_ms >= tiers[o.tiers_emitted] {
                        let tier = o.tiers_emitted;
                        let confidence = if tier + 1 == tiers.len() {
                            EndpointConfidence::High
                        } else if tier == 0 {
                            EndpointConfidence::Low
                        } else {
                            EndpointConfidence::Medium
                        };
                        out.push(SegmentEvent::Endpoint {
                            span: Span {
                                start: ps,
                                end: here_end,
                            },
                            segment: o.id,
                            silence_ms,
                            tier: tier as u8,
                            confidence,
                        });
                        o.tiers_emitted += 1;
                    }
                    close_now = silence_ms >= close;
                }
                if close_now {
                    self.close_segment(SegmentCloseReason::SilencePolicy, out);
                }
            }
        }
    }

    /// Close everything (gap, stop, stream end, resource policy, failure).
    /// The silence window, if any, ends at the last hop seen.
    pub fn close_all(&mut self, reason: SegmentCloseReason, out: &mut Vec<SegmentEvent>) {
        self.close_segment(reason, out);
        if let Some(end) = self.last_end {
            self.close_silence(end, out);
        }
        self.silence = None;
        self.pending_continuation = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::{Stabilizer, StabilizerParams};
    use crate::testutil::hop;

    fn cfg() -> SegmentConfig {
        SegmentConfig {
            close_silence_ms: 300,
            endpoint_tiers_ms: vec![100, 200],
            max_segment_ms: 1_000,
            min_silence_report_ms: 100,
        }
    }

    fn drive(scores: &[f32], cfg: SegmentConfig) -> Vec<SegmentEvent> {
        let mut st = Stabilizer::new(StabilizerParams {
            onset_threshold: 0.6,
            offset_threshold: 0.4,
            onset_hops: 2,
            hangover_hops: 5,
        });
        let mut seg = Segmenter::new(cfg);
        let mut out = Vec::new();
        for (i, s) in scores.iter().enumerate() {
            let h = hop(i as u64);
            let t = st.step(&h, *s);
            seg.step(&h, *s, st.state(), t.as_ref(), &mut out);
        }
        seg.close_all(SegmentCloseReason::StreamEnd, &mut out);
        out
    }

    fn pattern(parts: &[(f32, usize)]) -> Vec<f32> {
        parts
            .iter()
            .flat_map(|(v, n)| std::iter::repeat_n(*v, *n))
            .collect()
    }

    #[test]
    fn a_segment_with_endpoints_and_silences() {
        // 20 hops silence, 50 active, 40 silence (10 ms hops).
        let ev = drive(&pattern(&[(0.0, 20), (0.9, 50), (0.0, 40)]), cfg());
        let kinds: Vec<_> = ev
            .iter()
            .map(|e| match e {
                SegmentEvent::Started { .. } => "start",
                SegmentEvent::Ended { .. } => "end",
                SegmentEvent::Endpoint { .. } => "ep",
                SegmentEvent::Silence { .. } => "sil",
            })
            .collect();
        assert_eq!(kinds, ["sil", "start", "ep", "ep", "end", "sil"]);
        let SegmentEvent::Started {
            span, continuation, ..
        } = &ev[1]
        else {
            panic!()
        };
        assert_eq!((span.start.frame, *continuation), (20 * 160, false));
        let SegmentEvent::Endpoint {
            silence_ms,
            confidence,
            tier,
            span,
            ..
        } = &ev[2]
        else {
            panic!()
        };
        assert_eq!(
            (*silence_ms, *tier, *confidence),
            (100, 0, EndpointConfidence::Low)
        );
        assert_eq!(
            span.start.frame,
            70 * 160,
            "pause starts at the offset boundary"
        );
        let SegmentEvent::Endpoint { confidence, .. } = &ev[3] else {
            panic!()
        };
        assert_eq!(*confidence, EndpointConfidence::High);
        let SegmentEvent::Ended {
            span,
            reason,
            mean_score,
            ..
        } = &ev[4]
        else {
            panic!()
        };
        assert_eq!(
            (span.duration_ms(), *reason),
            (500, SegmentCloseReason::SilencePolicy)
        );
        assert!((mean_score.unwrap() - 0.9).abs() < 1e-4);
        let SegmentEvent::Silence { span, .. } = &ev[0] else {
            panic!()
        };
        assert_eq!(span.duration_ms(), 200);
        let SegmentEvent::Silence { span, .. } = &ev[5] else {
            panic!()
        };
        assert_eq!(span.duration_ms(), 400);
    }

    #[test]
    fn short_pauses_stay_inside_one_segment() {
        let ev = drive(
            &pattern(&[(0.9, 30), (0.0, 20), (0.9, 30), (0.0, 40)]),
            cfg(),
        );
        let starts = ev
            .iter()
            .filter(|e| matches!(e, SegmentEvent::Started { .. }))
            .count();
        let ends: Vec<_> = ev
            .iter()
            .filter_map(|e| {
                if let SegmentEvent::Ended { span, .. } = e {
                    Some(span.duration_ms())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!((starts, ends), (1, vec![800]));
        // The 200 ms pause reached both tiers before activity resumed; the
        // final pause reached both again.
        let eps = ev
            .iter()
            .filter(|e| matches!(e, SegmentEvent::Endpoint { .. }))
            .count();
        assert_eq!(eps, 4);
    }

    #[test]
    fn long_activity_splits_with_continuations_and_stop_closes() {
        let ev = drive(&pattern(&[(0.9, 250)]), cfg());
        let mut v = Vec::new();
        for e in &ev {
            match e {
                SegmentEvent::Started { continuation, .. } => {
                    v.push(format!("s{}", *continuation as u8))
                }
                SegmentEvent::Ended { reason, span, .. } => {
                    v.push(format!("e{:?}{}", reason, span.duration_ms()))
                }
                _ => {}
            }
        }
        assert_eq!(
            v,
            [
                "s0",
                "eMaxDuration1000",
                "s1",
                "eMaxDuration1000",
                "s1",
                "eStreamEnd500"
            ]
        );
    }

    #[test]
    fn close_all_mid_pause_ends_at_the_pause_start() {
        let ev = drive(&pattern(&[(0.9, 30), (0.0, 10)]), cfg());
        let SegmentEvent::Ended { span, reason, .. } = ev
            .iter()
            .find(|e| matches!(e, SegmentEvent::Ended { .. }))
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (span.duration_ms(), *reason),
            (300, SegmentCloseReason::StreamEnd)
        );
    }
}
