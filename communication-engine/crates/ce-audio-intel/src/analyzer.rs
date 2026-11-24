//! Per-stream analysis pipeline: framer → detector → stabilizer →
//! segmenter, with quality, conditions, adaptive policy and the resource
//! governor. Single-threaded and deterministic: the same blocks, loads and
//! configuration always yield the same events (replay relies on this).
//!
//! Discontinuities are never bridged: a gap closes the open segment
//! (`Discontinuity`), ends activity at the last analysed hop, resets every
//! adaptive state and is reported once. In `Shedding` mode activity and
//! segmentation are suspended and only quality continues on the analysed
//! half of the blocks; skipped blocks count as lost frames.

use ce_core::CeError;
use ce_core::audio::AudioBlock;
use ce_core::ids::AudioStreamId;
use ce_core::intel::model::DiscontinuityContext;
use ce_core::intel::{
    ActivityConfig, ActivityProfile, ActivityState, AlgorithmRef, AnalysisCapabilities,
    AnalysisConfig, AnalysisCounters, AnalysisEndReason, AnalysisEvent, AnalysisStatus,
    DegradedMode, PolicyDecision, ResultMeta, SegmentCloseReason, VadAlgorithm, VadDescriptor,
};
use ce_core::privacy::RedactionPolicy;
use ce_core::time::MonotonicTime;

use crate::activity::{Stabilizer, StabilizerParams, Transition};
use crate::framer::{Framer, FramerItem, Gap, Hop};
use crate::policy::{AdaptivePolicy, ResourceGovernor};
use crate::quality::{ALL_CONDITIONS, ConditionChange, ConditionTracker, QualityAnalyzer};
use crate::segment::{Point, SegmentEvent, Segmenter, Span};
use crate::vad::{EnergyVad, SpectralVad, VoiceActivityDetector};

/// The analysis pipeline for one stream.
pub struct StreamAnalyzer {
    stream: AudioStreamId,
    cfg: AnalysisConfig,
    algorithm: AlgorithmRef,
    descriptor: VadDescriptor,
    framer: Framer,
    primary: Box<dyn VoiceActivityDetector>,
    fallback: Option<EnergyVad>,
    stabilizer: Stabilizer,
    base_params: StabilizerParams,
    segmenter: Segmenter,
    quality: Option<QualityAnalyzer>,
    conditions: ConditionTracker,
    policy: AdaptivePolicy,
    governor: ResourceGovernor,
    counters: AnalysisCounters,
    items: Vec<FramerItem>,
    seg_out: Vec<SegmentEvent>,
    cond_out: Vec<ConditionChange>,
    pol_out: Vec<(PolicyDecision, bool)>,
    recent_gap: Option<(DiscontinuityContext, MonotonicTime)>,
    prev_block_end: Option<u64>,
    last_hop: Option<Hop>,
    activity_start: Option<Point>,
    shed_skip: bool,
    ended: Option<AnalysisEndReason>,
}

impl std::fmt::Debug for StreamAnalyzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamAnalyzer")
            .field("stream", &self.stream)
            .field("algorithm", &self.algorithm)
            .field("counters", &self.counters)
            .finish_non_exhaustive()
    }
}

fn ms(a: MonotonicTime, b: MonotonicTime) -> u32 {
    u32::try_from(b.saturating_since(a).as_millis()).unwrap_or(u32::MAX)
}

impl StreamAnalyzer {
    /// The configured detector.
    pub fn new(stream: AudioStreamId, cfg: AnalysisConfig) -> Result<Self, CeError> {
        let vad: Box<dyn VoiceActivityDetector> = match cfg.vad {
            VadAlgorithm::Spectral => Box::new(SpectralVad::new(cfg.hop_ms)),
            VadAlgorithm::Energy => Box::new(EnergyVad::new(cfg.hop_ms)),
        };
        Self::with_detector(stream, cfg, vad)
    }

    /// An explicit detector (tests, fault injection, alternative adapters).
    pub fn with_detector(
        stream: AudioStreamId,
        cfg: AnalysisConfig,
        primary: Box<dyn VoiceActivityDetector>,
    ) -> Result<Self, CeError> {
        cfg.validate()?;
        let descriptor = primary.descriptor();
        let algorithm = AlgorithmRef {
            vad: descriptor.id.clone(),
            version: descriptor.version.clone(),
            config_fingerprint: cfg.fingerprint(),
        };
        let base_params = StabilizerParams::from_config(&cfg.activity, cfg.hop_ms);
        let mut framer = Framer::new(cfg.hop_ms, cfg.quality.clip_level);
        framer.set_spectral(primary.needs_spectral());
        framer.set_audio(primary.needs_audio());
        Ok(Self {
            stream,
            algorithm,
            descriptor,
            framer,
            fallback: (primary.needs_spectral() || primary.needs_audio())
                .then(|| EnergyVad::new(cfg.hop_ms)),
            primary,
            stabilizer: Stabilizer::new(base_params),
            base_params,
            segmenter: Segmenter::new(cfg.segment.clone()),
            quality: cfg
                .quality
                .enabled
                .then(|| QualityAnalyzer::new(cfg.quality.window_ms, cfg.hop_ms)),
            conditions: ConditionTracker::new(cfg.conditions.clone(), cfg.hop_ms),
            policy: AdaptivePolicy::new(cfg.adaptive_policy),
            governor: ResourceGovernor::new(cfg.budget.clone()),
            counters: AnalysisCounters::default(),
            items: Vec::new(),
            seg_out: Vec::new(),
            cond_out: Vec::new(),
            pol_out: Vec::new(),
            recent_gap: None,
            prev_block_end: None,
            last_hop: None,
            activity_start: None,
            shed_skip: false,
            ended: None,
            cfg,
        })
    }

    /// Stream.
    pub fn stream(&self) -> AudioStreamId {
        self.stream
    }

    /// Algorithm identity.
    pub fn algorithm(&self) -> &AlgorithmRef {
        &self.algorithm
    }

    /// Capabilities.
    pub fn capabilities(&self) -> AnalysisCapabilities {
        AnalysisCapabilities {
            vad: self.descriptor.clone(),
            quality: self.cfg.quality.enabled,
            conditions: if self.cfg.conditions.enabled {
                ALL_CONDITIONS.to_vec()
            } else {
                Vec::new()
            },
            overlap_detection: false,
        }
    }

    /// Current mode.
    pub fn mode(&self) -> DegradedMode {
        self.governor.mode()
    }

    /// Counters.
    pub fn counters(&self) -> AnalysisCounters {
        let mut c = self.counters;
        c.max_load_permille = self.governor.max_load_permille();
        c.max_backlog = self.governor.max_backlog();
        c
    }

    /// Record the tap's cumulative drop count.
    pub fn set_tap_dropped(&mut self, total: u64) {
        self.counters.tap_dropped_blocks = total;
    }

    /// Status snapshot.
    pub fn status(&self, running: bool) -> AnalysisStatus {
        AnalysisStatus {
            stream: self.stream,
            algorithm: self.algorithm.clone(),
            capabilities: self.capabilities(),
            running,
            activity: self.stabilizer.state(),
            mode: self.mode(),
            policies: self.policy.active().to_vec(),
            conditions: self.conditions.active().to_vec(),
            quality: self.quality.as_ref().and_then(|q| q.last().cloned()),
            counters: self.counters(),
            ended: self.ended,
        }
    }

    /// Emit `Started`.
    pub fn start(&mut self, out: &mut Vec<AnalysisEvent>) {
        out.push(AnalysisEvent::Started {
            stream: self.stream,
            algorithm: self.algorithm.clone(),
            capabilities: self.capabilities(),
        });
    }

    fn meta(&self, span: Span) -> ResultMeta {
        // A result is shaped by a gap while the detector re-adapts after it
        // (its warm-up, at least one hop).
        let warmup =
            u64::from(self.descriptor.warmup_hops.max(1)) * u64::from(self.cfg.hop_ms) * 1_000_000;
        let discontinuity = self.recent_gap.and_then(|(ctx, at)| {
            (span.start.time.as_nanos() < at.as_nanos().saturating_add(warmup)).then_some(ctx)
        });
        ResultMeta {
            stream: self.stream,
            sequence_range: (span.start.sequence, span.end.sequence),
            frame_range: (span.start.frame, span.end.frame),
            time_range: (span.start.time, span.end.time),
            discontinuity,
            mode: self.mode(),
        }
    }

    fn conditions_enabled(&self) -> bool {
        self.cfg.conditions.enabled && self.mode() == DegradedMode::Full
    }

    /// Analyse one block. Errors only for unsupported formats.
    pub fn push_block(
        &mut self,
        block: &AudioBlock,
        out: &mut Vec<AnalysisEvent>,
    ) -> Result<(), CeError> {
        if self.ended.is_some() {
            return Ok(());
        }
        if self.mode() == DegradedMode::Shedding {
            self.shed_skip = !self.shed_skip;
            if self.shed_skip {
                self.counters.shed_blocks += 1;
                if let Some(q) = &mut self.quality {
                    q.note_lost(u64::from(block.meta.frames));
                }
                return Ok(());
            }
        }
        self.counters.blocks += 1;
        let mut items = std::mem::take(&mut self.items);
        items.clear();
        let res = self.framer.push(block, &mut |i| items.push(i));
        for item in items.drain(..) {
            match item {
                FramerItem::Gap(g) => self.on_gap(g, out),
                FramerItem::Hop(h) => self.on_hop(&h, out),
            }
        }
        self.items = items;
        self.prev_block_end = Some(block.meta.end_frame_index());
        res
    }

    fn end_activity(&mut self, reason: SegmentCloseReason, out: &mut Vec<AnalysisEvent>) {
        let active_at = self
            .last_hop
            .filter(|_| self.stabilizer.state() == ActivityState::Active);
        if let Some(last) = active_at {
            let end = Point::end_of(&last);
            let span = Span { start: end, end };
            out.push(AnalysisEvent::ActivityChanged {
                meta: self.meta(span),
                state: ActivityState::Inactive,
                score: None,
                decision_latency_ms: 0,
            });
        }
        self.stabilizer.reset();
        self.activity_start = None;
        self.segmenter.close_all(reason, &mut self.seg_out);
        self.flush_segments(out);
    }

    fn on_gap(&mut self, g: Gap, out: &mut Vec<AnalysisEvent>) {
        let lost = match g.context {
            DiscontinuityContext::Capture {
                lost_frames: Some(n),
                ..
            } => n,
            _ => self
                .prev_block_end
                .map_or(0, |e| g.resume_frame.saturating_sub(e)),
        } + u64::from(g.discarded_frames);
        if let Some(q) = &mut self.quality {
            q.note_lost(lost);
        }
        self.counters.discontinuities += 1;
        self.primary.reset();
        if let Some(f) = &mut self.fallback {
            f.reset();
        }
        let shedding = self.mode() == DegradedMode::Shedding;
        if shedding && matches!(g.context, DiscontinuityContext::AnalysisDrop { .. }) {
            // Expected while shedding: counted, not reported per block.
            return;
        }
        self.end_activity(SegmentCloseReason::Discontinuity, out);
        let at = Point {
            sequence: g.sequence,
            frame: g.resume_frame,
            time: g.resume_time,
        };
        self.recent_gap = Some((g.context, g.resume_time));
        out.push(AnalysisEvent::Discontinuity {
            meta: self.meta(Span { start: at, end: at }),
            context: g.context,
        });
    }

    fn primary_score(&mut self, hop: &Hop) -> f32 {
        if self.primary.needs_audio() {
            let rate = self.framer.rate().unwrap_or(0);
            let mono = self.framer.hop_audio(hop);
            self.primary.score_audio(hop, mono, rate).score
        } else {
            self.primary.score(hop).score
        }
    }

    fn score(&mut self, hop: &Hop) -> f32 {
        let full = self.mode() == DegradedMode::Full;
        match &mut self.fallback {
            Some(fb) if full => {
                fb.score(hop); // keep the fallback's floor warm
                self.primary_score(hop)
            }
            Some(fb) => fb.score(hop).score,
            None => self.primary_score(hop),
        }
    }

    fn on_hop(&mut self, hop: &Hop, out: &mut Vec<AnalysisEvent>) {
        self.counters.hops += 1;
        self.counters.non_finite_samples += u64::from(hop.features.non_finite);
        self.last_hop = Some(*hop);
        let shedding = self.mode() == DegradedMode::Shedding;
        let mut state = ActivityState::Inactive;
        if !shedding {
            let score = self.score(hop);
            let t = self.stabilizer.step(hop, score);
            state = self.stabilizer.state();
            if let Some(t) = &t {
                self.emit_transition(t, out);
            }
            self.segmenter
                .step(hop, score, state, t.as_ref(), &mut self.seg_out);
            self.flush_segments(out);
        }
        if self.conditions_enabled() {
            self.conditions.on_hop(state, &mut self.cond_out);
            self.flush_conditions(out);
        }
        let suppress = self
            .policy
            .is_active(PolicyDecision::SuppressLevelEstimates);
        let Some(result) = self
            .quality
            .as_mut()
            .and_then(|q| q.push(hop, state, suppress))
        else {
            return;
        };
        let span_start = {
            let window_frames =
                u64::from(hop.frames) * u64::from(self.cfg.quality.window_ms / self.cfg.hop_ms);
            let frames_back = window_frames.saturating_sub(u64::from(hop.frames));
            let back = ce_core::audio::frame::frames_to_duration(
                frames_back,
                (u64::from(hop.frames) * 1000 / u64::from(self.cfg.hop_ms)) as u32,
            );
            Point {
                sequence: hop.sequence,
                frame: hop.frame_start.saturating_sub(frames_back),
                time: MonotonicTime::from_nanos(
                    hop.time.as_nanos().saturating_sub(back.as_nanos() as u64),
                ),
            }
        };
        let span = Span {
            start: span_start,
            end: Point::end_of(hop),
        };
        let mut metrics = result.metrics;
        if self.policy.is_active(PolicyDecision::LowConfidenceActivity) {
            // Activity statistics are not meaningful on unusable input.
            metrics.activity_ratio = 0.0;
        }
        if result.changed {
            out.push(AnalysisEvent::QualityChanged {
                meta: self.meta(span),
                metrics: metrics.clone(),
            });
        }
        if self.conditions_enabled() {
            self.conditions.on_window(&metrics, &mut self.cond_out);
            self.flush_conditions(out);
        }
        let conds = self.conditions.active().to_vec();
        self.policy.on_window(&metrics, &conds, &mut self.pol_out);
        let changes = std::mem::take(&mut self.pol_out);
        for (decision, active) in &changes {
            out.push(AnalysisEvent::PolicyChanged {
                meta: self.meta(span),
                decision: *decision,
                active: *active,
            });
            if *decision == PolicyDecision::ConservativeSegmentation {
                let p = if *active {
                    self.base_params.stricter(StabilizerParams::from_config(
                        &ActivityConfig::preset(ActivityProfile::Conservative),
                        self.cfg.hop_ms,
                    ))
                } else {
                    self.base_params
                };
                self.stabilizer.set_params(p);
            }
        }
        self.pol_out = changes;
        self.pol_out.clear();
    }

    fn emit_transition(&mut self, t: &Transition, out: &mut Vec<AnalysisEvent>) {
        let span = Span {
            start: Point::start_of(&t.boundary),
            end: Point::end_of(&t.decided),
        };
        let low_conf = self.policy.is_active(PolicyDecision::LowConfidenceActivity);
        out.push(AnalysisEvent::ActivityChanged {
            meta: self.meta(span),
            state: t.state,
            score: (!low_conf).then_some(t.mean_score),
            decision_latency_ms: ms(t.boundary.time, t.decided.end_time),
        });
        self.activity_start = (t.state == ActivityState::Active).then_some(span.start);
    }

    fn flush_segments(&mut self, out: &mut Vec<AnalysisEvent>) {
        let evs = std::mem::take(&mut self.seg_out);
        for e in &evs {
            out.push(match e {
                SegmentEvent::Started {
                    span,
                    segment,
                    continuation,
                } => AnalysisEvent::SegmentStarted {
                    meta: self.meta(*span),
                    segment: *segment,
                    continuation: *continuation,
                },
                SegmentEvent::Ended {
                    span,
                    segment,
                    reason,
                    mean_score,
                } => {
                    self.counters.segments += 1;
                    AnalysisEvent::SegmentEnded {
                        meta: self.meta(*span),
                        segment: *segment,
                        duration_ms: span.duration_ms(),
                        reason: *reason,
                        mean_score: if self.policy.is_active(PolicyDecision::LowConfidenceActivity)
                        {
                            None
                        } else {
                            *mean_score
                        },
                    }
                }
                SegmentEvent::Endpoint {
                    span,
                    segment,
                    silence_ms,
                    tier,
                    confidence,
                } => AnalysisEvent::EndpointCandidate {
                    meta: self.meta(*span),
                    segment: *segment,
                    silence_ms: *silence_ms,
                    tier: *tier,
                    confidence: *confidence,
                },
                SegmentEvent::Silence {
                    span,
                    mean_level_dbfs,
                } => AnalysisEvent::SilenceWindow {
                    meta: self.meta(*span),
                    duration_ms: span.duration_ms(),
                    mean_level_dbfs: *mean_level_dbfs,
                },
            });
        }
        self.seg_out = evs;
        self.seg_out.clear();
    }

    fn flush_conditions(&mut self, out: &mut Vec<AnalysisEvent>) {
        let Some(last) = self.last_hop else {
            self.cond_out.clear();
            return;
        };
        let at = Point::end_of(&last);
        let changes = std::mem::take(&mut self.cond_out);
        for c in &changes {
            out.push(AnalysisEvent::ConditionChanged {
                meta: self.meta(Span { start: at, end: at }),
                condition: c.kind,
                active: c.active,
                evidence: c.evidence.clone(),
            });
        }
        self.cond_out = changes;
        self.cond_out.clear();
    }

    /// Record one block's processing load (time / audio duration) and the
    /// analysis queue backlog; may change the degraded mode.
    pub fn note_load(&mut self, load: f32, backlog: u32, out: &mut Vec<AnalysisEvent>) {
        if self.ended.is_some() {
            return;
        }
        let Some(change) = self.governor.note(load, backlog) else {
            return;
        };
        out.push(AnalysisEvent::DegradedModeChanged {
            stream: self.stream,
            from: change.from,
            to: change.to,
            load_permille: change.load_permille,
            backlog: change.backlog,
        });
        let full = change.to == DegradedMode::Full;
        self.framer
            .set_spectral(full && self.primary.needs_spectral());
        self.framer.set_audio(full && self.primary.needs_audio());
        if let Some(q) = &mut self.quality {
            q.reset_window();
        }
        if !full {
            self.conditions.clear_all(&mut self.cond_out);
            self.flush_conditions(out);
        }
        if change.to == DegradedMode::Shedding {
            self.end_activity(SegmentCloseReason::ResourcePolicy, out);
            self.shed_skip = false;
        }
        if change.from == DegradedMode::Shedding {
            self.primary.reset();
            if let Some(f) = &mut self.fallback {
                f.reset();
            }
        }
    }

    /// End analysis: closes open results and emits `Stopped`.
    pub fn finish(&mut self, reason: AnalysisEndReason, out: &mut Vec<AnalysisEvent>) {
        if self.ended.is_some() {
            return;
        }
        let close = match reason {
            AnalysisEndReason::Stopped | AnalysisEndReason::Shutdown => {
                SegmentCloseReason::ExplicitStop
            }
            AnalysisEndReason::Failed => SegmentCloseReason::Failure,
            AnalysisEndReason::StreamClosed
            | AnalysisEndReason::Revoked
            | AnalysisEndReason::StreamInvalidated
            | AnalysisEndReason::StreamFailed => SegmentCloseReason::StreamEnd,
        };
        self.end_activity(close, out);
        self.ended = Some(reason);
        out.push(AnalysisEvent::Stopped {
            stream: self.stream,
            reason,
            counters: self.counters(),
        });
    }

    /// Fail: emits `Failed`, then finishes with `Failed`.
    pub fn fail(&mut self, error: &CeError, out: &mut Vec<AnalysisEvent>) {
        if self.ended.is_some() {
            return;
        }
        out.push(AnalysisEvent::Failed {
            stream: self.stream,
            error: error.report(&RedactionPolicy::DEFAULT),
        });
        self.finish(AnalysisEndReason::Failed, out);
    }
}
