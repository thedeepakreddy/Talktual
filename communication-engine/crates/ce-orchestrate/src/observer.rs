//! The per-pipeline observer: correlation, revision reconciliation, causal
//! validation, timeline and latency (ADR-0032).
//!
//! The runtime feeds it every **admitted** stage event of the pipeline's
//! stream (after generation admission, before publication) with the
//! current monotonic time. It **annotates and counts**; it never changes a
//! payload. Within a worker generation the worker is authoritative for its
//! own ordering — dropping, say, a translation result would break Phase 6's
//! gapless-sequence contract — so findings are reported, not enforced.
//! One observer lives for one pipeline epoch (a restart creates a new one:
//! a new recognition worker numbers utterances from 0 again).

use std::collections::BTreeMap;

use ce_core::intel::AnalysisEvent;
use ce_core::orchestration::{
    CausalKind, DeadlinePolicy, LatencyMetric, LatencySummary, OrchestrationCounters, StageKind,
    TimelineKind, TimelineStatus, TimingUncertainty,
};
use ce_core::semantic::SemanticEvent;
use ce_core::speech::{RevisionStatus, TranscriptRevision};
use ce_core::time::MonotonicTime;
use ce_core::translation::{TranslationEvent, TranslationState};

use crate::latency::{DeadlineOutcome, Reservoir, deadline};
use crate::reconcile::{RevisionClass, SourceClass, TranscriptTable};
use crate::timeline::Timeline;

/// Something the runtime reports (as an operational event and a counter).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Finding {
    /// A causal or revision-contract violation.
    Causal {
        /// Stage of the output.
        stage: StageKind,
        /// Utterance.
        utterance: u64,
        /// Revision referred to.
        revision: u32,
        /// Kind.
        kind: CausalKind,
    },
    /// A timeline finding.
    Timeline {
        /// Stage.
        stage: StageKind,
        /// Kind.
        kind: TimelineKind,
        /// Frame, if known.
        frame: Option<u64>,
    },
    /// A final output missed its budget.
    Deadline {
        /// Stage.
        stage: StageKind,
        /// Utterance.
        utterance: u64,
        /// Latency (ms).
        latency_ms: u32,
        /// Budget (ms).
        budget_ms: u32,
    },
}

/// The observer of one pipeline epoch.
#[derive(Clone, Debug)]
pub struct Observer {
    table: TranscriptTable,
    timeline: Timeline,
    deadlines: DeadlinePolicy,
    capacity: usize,
    translation_seq: BTreeMap<u64, u32>,
    /// Translation views of utterances below this were forgotten.
    seq_evicted_below: u64,
    reservoirs: BTreeMap<LatencyMetric, Reservoir>,
    counters: OrchestrationCounters,
}

fn ms_between(later: MonotonicTime, earlier: MonotonicTime) -> f32 {
    later.saturating_since(earlier).as_secs_f32() * 1_000.0
}

impl Observer {
    /// An observer for a stream sampled at `rate`, remembering `utterances`
    /// utterances.
    pub fn new(
        rate: u32,
        utterances: usize,
        deadlines: DeadlinePolicy,
        uncertainty: TimingUncertainty,
    ) -> Self {
        Self {
            table: TranscriptTable::new(utterances),
            timeline: Timeline::new(rate, uncertainty),
            deadlines,
            capacity: utterances.max(1),
            translation_seq: BTreeMap::new(),
            seq_evicted_below: 0,
            reservoirs: BTreeMap::new(),
            counters: OrchestrationCounters::default(),
        }
    }

    fn count_timeline(&mut self, kind: TimelineKind) {
        match kind {
            TimelineKind::ReAnchor => self.counters.re_anchors += 1,
            TimelineKind::UnknownTiming => self.counters.unknown_timing += 1,
            TimelineKind::Gap => self.counters.gaps += 1,
            _ => self.counters.timeline_anomalies += 1,
        }
    }

    fn record(&mut self, metric: LatencyMetric, ms: f32) {
        self.reservoirs.entry(metric).or_default().record(ms);
    }

    fn final_latency(
        &mut self,
        stage: StageKind,
        metric: LatencyMetric,
        utterance: u64,
        audio_end: MonotonicTime,
        now: MonotonicTime,
        out: &mut Vec<Finding>,
    ) {
        if audio_end.as_nanos() == 0 {
            return; // unknown timing: no latency is claimed
        }
        let ms = ms_between(now, audio_end);
        self.record(metric, ms);
        if let DeadlineOutcome::Missed {
            latency_ms,
            budget_ms,
        } = deadline(self.deadlines.budget(stage), ms)
        {
            self.counters.deadline_misses += 1;
            out.push(Finding::Deadline {
                stage,
                utterance,
                latency_ms: latency_ms.round().min(u32::MAX as f32) as u32,
                budget_ms,
            });
        }
    }

    fn classify_source(
        &mut self,
        stage: StageKind,
        utterance: u64,
        revision: u32,
        out: &mut Vec<Finding>,
    ) -> SourceClass {
        self.counters.derived_outputs += 1;
        let class = self.table.source(utterance, revision);
        match class {
            SourceClass::Current => {}
            SourceClass::Superseded { .. } => self.counters.superseded_outputs += 1,
            SourceClass::Evicted => self.counters.evicted_references += 1,
            SourceClass::Unknown => {
                self.counters.causal_violations += 1;
                out.push(Finding::Causal {
                    stage,
                    utterance,
                    revision,
                    kind: CausalKind::UnknownSource,
                });
            }
        }
        class
    }

    /// A transcript revision.
    pub fn on_transcript(&mut self, r: &TranscriptRevision, now: MonotonicTime) -> Vec<Finding> {
        let mut out = Vec::new();
        self.counters.transcript_revisions += 1;
        let u = r.meta.utterance;
        let class = self.table.apply(
            u,
            r.revision,
            r.status,
            r.meta.time_range.1,
            r.meta.frame_range.1,
        );
        let kind = match class {
            RevisionClass::Applied => None,
            RevisionClass::Duplicate => Some(CausalKind::DuplicateRevision),
            RevisionClass::Stale => Some(CausalKind::StaleRevision),
            RevisionClass::AfterFinal => Some(CausalKind::AfterFinal),
            RevisionClass::Evicted => {
                self.counters.evicted_references += 1;
                None
            }
        };
        if let Some(kind) = kind {
            self.counters.causal_violations += 1;
            out.push(Finding::Causal {
                stage: StageKind::Recognition,
                utterance: u,
                revision: r.revision,
                kind,
            });
        }
        let (span, findings) = self.timeline.observe(
            StageKind::Recognition,
            Some(u),
            r.meta.frame_range,
            r.meta.time_range,
        );
        for k in findings {
            self.count_timeline(k);
            out.push(Finding::Timeline {
                stage: StageKind::Recognition,
                kind: k,
                frame: Some(r.meta.frame_range.0),
            });
        }
        if r.status == RevisionStatus::Final && class == RevisionClass::Applied && span.timing_known
        {
            self.final_latency(
                StageKind::Recognition,
                LatencyMetric::TranscriptFinal,
                u,
                r.meta.time_range.1,
                now,
                &mut out,
            );
        }
        out
    }

    /// A semantic event.
    pub fn on_semantic(&mut self, e: &SemanticEvent, now: MonotonicTime) -> Vec<Finding> {
        let mut out = Vec::new();
        match e {
            SemanticEvent::Observation { observation, .. } => {
                self.classify_source(
                    StageKind::Semantics,
                    observation.source.utterance,
                    observation.source.revision,
                    &mut out,
                );
            }
            SemanticEvent::UtteranceFinalized {
                utterance,
                revision,
                ..
            } => {
                let class =
                    self.classify_source(StageKind::Semantics, *utterance, *revision, &mut out);
                if matches!(class, SourceClass::Current | SourceClass::Superseded { .. }) {
                    if let Some(end) = self.table.audio_end(*utterance) {
                        self.final_latency(
                            StageKind::Semantics,
                            LatencyMetric::SemanticFinal,
                            *utterance,
                            end,
                            now,
                            &mut out,
                        );
                    }
                }
            }
            _ => {}
        }
        out
    }

    /// A translation event.
    pub fn on_translation(&mut self, e: &TranslationEvent, now: MonotonicTime) -> Vec<Finding> {
        let mut out = Vec::new();
        match e {
            // A new worker run: consumer state resets, sequences restart.
            TranslationEvent::Started { .. } => {
                self.counters.translation_runs += 1;
                self.translation_seq.clear();
                self.seq_evicted_below = 0;
            }
            TranslationEvent::Result(r) => {
                let u = r.source.utterance;
                let seq_kind = match self.translation_seq.get(&u) {
                    Some(&last) if r.sequence <= last => Some(CausalKind::SequenceRegression),
                    Some(&last) if r.sequence > last + 1 => Some(CausalKind::SequenceGap),
                    None if r.sequence != 0 && u >= self.seq_evicted_below => {
                        Some(CausalKind::SequenceGap)
                    }
                    _ => None,
                };
                if let Some(kind) = seq_kind {
                    self.counters.causal_violations += 1;
                    out.push(Finding::Causal {
                        stage: StageKind::Translation,
                        utterance: u,
                        revision: r.source.revision,
                        kind,
                    });
                }
                let entry = self.translation_seq.entry(u).or_insert(r.sequence);
                *entry = (*entry).max(r.sequence);
                while self.translation_seq.len() > self.capacity {
                    if let Some((&oldest, _)) = self.translation_seq.iter().next() {
                        self.translation_seq.remove(&oldest);
                        self.seq_evicted_below = self.seq_evicted_below.max(oldest + 1);
                    }
                }
                self.classify_source(StageKind::Translation, u, r.source.revision, &mut out);
                self.record(LatencyMetric::TranslationQueue, r.timing.queued_ms);
                self.record(LatencyMetric::TranslationAdapter, r.timing.adapter_ms);
                if r.state == TranslationState::Final {
                    self.final_latency(
                        StageKind::Translation,
                        LatencyMetric::TranslationFinal,
                        u,
                        r.source.time_range.1,
                        now,
                        &mut out,
                    );
                }
            }
            _ => {}
        }
        out
    }

    /// An analysis event (timeline only).
    pub fn on_analysis(&mut self, e: &AnalysisEvent) -> Vec<Finding> {
        let (meta, ordinal) = match e {
            AnalysisEvent::SegmentStarted { meta, segment, .. }
            | AnalysisEvent::SegmentEnded { meta, segment, .. } => (meta, Some(*segment)),
            AnalysisEvent::ActivityChanged { meta, .. }
            | AnalysisEvent::EndpointCandidate { meta, .. }
            | AnalysisEvent::SilenceWindow { meta, .. }
            | AnalysisEvent::QualityChanged { meta, .. } => (meta, None),
            _ => return Vec::new(),
        };
        let (_, findings) = self.timeline.observe(
            StageKind::Analysis,
            ordinal,
            meta.frame_range,
            meta.time_range,
        );
        let mut out = Vec::new();
        for k in findings {
            self.count_timeline(k);
            out.push(Finding::Timeline {
                stage: StageKind::Analysis,
                kind: k,
                frame: Some(meta.frame_range.0),
            });
        }
        out
    }

    /// An upstream audio discontinuity.
    pub fn on_audio_gap(&mut self, lost_frames: Option<u64>) -> Vec<Finding> {
        let k = self.timeline.gap(lost_frames);
        self.count_timeline(k);
        vec![Finding::Timeline {
            stage: StageKind::Recognition,
            kind: k,
            frame: None,
        }]
    }

    /// A stale output of an earlier generation was rejected for this stream.
    pub fn on_stale_rejected(&mut self) {
        self.counters.stale_outputs_rejected += 1;
    }

    /// Orchestration overhead of one observed event (ms, measured by the
    /// runtime on its monotonic clock).
    pub fn record_overhead(&mut self, ms: f32) {
        self.record(LatencyMetric::Overhead, ms);
    }

    /// Counters.
    pub fn counters(&self) -> &OrchestrationCounters {
        &self.counters
    }

    /// Latency summaries, in metric order.
    pub fn latency(&self) -> Vec<LatencySummary> {
        self.reservoirs
            .iter()
            .filter_map(|(m, r)| r.summary(*m))
            .collect()
    }

    /// Timeline summary.
    pub fn timeline(&self) -> TimelineStatus {
        self.timeline.status()
    }

    /// Utterances remembered (bounded).
    pub fn remembered(&self) -> usize {
        self.table.len()
    }
}
