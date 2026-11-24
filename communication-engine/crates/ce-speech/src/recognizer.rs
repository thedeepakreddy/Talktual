//! The deterministic per-stream recognizer.
//!
//! [`StreamRecognizer`] turns capture blocks plus Phase 3 analysis events
//! into speech events. It is single-threaded and deterministic for
//! deterministic adapters (wall-clock time only enters through
//! [`StreamRecognizer::load`], [`StreamRecognizer::note_load`] and [`StreamRecognizer::record_processing`]),
//! so replay and the live worker share one implementation.
//!
//! **Timeline.** Blocks are downmixed and resampled to the adapter rate.
//! Adapter sample `a` (counted from the current *anchor*) maps to stream
//! frame `anchor.frame + ⌊a · in_rate / adapter_rate⌋` and to time
//! `anchor.time + frames / in_rate`, exactly; nothing is shifted. The
//! anchor is the first block, and is re-established after every
//! discontinuity: utterances never span a gap.
//!
//! **Delimitation.** With `ActivityHints`, Phase 3 activity onsets queue a
//! *start* boundary (backdated to the onset, served from a pre-roll ring),
//! and segment ends (silence policy) or endpoint candidates at the
//! configured tier queue an *end* boundary (`trailing_ms` later). Hints are
//! hints: the adapter's own endpoint and `max_utterance_ms` still apply,
//! and while the analysis sheds load or has stopped the recognizer falls
//! back to `Continuous` (reported with `HintsChanged`). With `Continuous`,
//! an utterance starts with the first audio after the previous one and
//! ends on an adapter endpoint or the maximum duration.
//!
//! **Faults.** An adapter error cancels the open utterance
//! (`Cancelled`, reason `Failure`), resets the adapter and reports
//! `Reset { AdapterError }`; three consecutive errors fail the recognition.
//! Malformed adapter output is sanitized and counted, never trusted.

use std::collections::VecDeque;

use ce_core::audio::AudioBlock;
use ce_core::audio::frame::frames_to_duration;
use ce_core::ids::AudioStreamId;
use ce_core::intel::model::DiscontinuityContext;
use ce_core::intel::{ActivityState, AnalysisEvent, DegradedMode, SegmentCloseReason};
use ce_core::privacy::RedactionPolicy;
use ce_core::speech::{
    AsrCapabilities, FinalReason, HintSource, LanguageDecision, LanguageObservation, LanguageScope,
    LanguageSource, LanguageTag, RecognitionConfig, RecognitionCounters, RecognitionEndReason,
    RecognitionMode, RecognitionStatus, RecognizerRef, ResetReason, RevisionStatus, Score,
    SegmentationMode, SpeechCapabilities, SpeechEvent, SpeechMeta, TranscriptRevision, WordTiming,
};
use ce_core::time::MonotonicTime;
use ce_core::{CeError, ErrorCategory};
use std::sync::Arc;

use crate::adapter::{AdapterHypothesis, AdapterWord, AsrAdapter, sanitize};
use crate::language::{self, LanguageIdentifier, StreamLanguage};
use crate::metrics::Reservoir;
use crate::resample::{Resampler, downmix};
use crate::transcript::{TextRevision, UtteranceText};

/// Consecutive adapter errors that fail the recognition.
pub const MAX_CONSECUTIVE_ERRORS: u32 = 3;
/// Queued hint boundaries kept (older ones are dropped and counted).
const MAX_BOUNDARIES: usize = 64;
/// Latency observations kept for percentiles.
const RESERVOIR: usize = 1_024;
/// Blocks between two `QueuePressure` events.
const PRESSURE_EVERY: u64 = 50;

#[derive(Clone, Copy, Debug)]
struct Anchor {
    frame: u64,
    time: MonotonicTime,
    in_rate: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Boundary {
    Start(u64),
    End {
        target: u64,
        reason: FinalReason,
        speech_end: u64,
    },
}

#[derive(Debug)]
struct Open {
    id: u64,
    start: u64,
    seq_first: u64,
    fed: u64,
    text: UtteranceText,
    pending: Vec<f32>,
    last_partial_ms: Option<u64>,
    first_partial: bool,
    discontinuity: Option<DiscontinuityContext>,
    words: Option<Vec<AdapterWord>>,
    score: Option<f32>,
    language: Option<LanguageTag>,
    endpoint: bool,
}

/// Static setup of a recognizer.
#[derive(Debug, Clone)]
pub struct RecognizerSetup {
    /// Stream.
    pub stream: AudioStreamId,
    /// Validated configuration.
    pub config: RecognitionConfig,
    /// The adapter's capabilities (from its factory).
    pub capabilities: AsrCapabilities,
    /// Text language identifier.
    pub identifier: Option<Arc<dyn LanguageIdentifier>>,
    /// Phase 3 analysis is running on the stream (required for
    /// `ActivityHints`; without it the recognizer starts in `Continuous`).
    pub analysis_available: bool,
}

/// The per-stream recognizer.
pub struct StreamRecognizer {
    stream: AudioStreamId,
    cfg: RecognitionConfig,
    caps: AsrCapabilities,
    adapter: Box<dyn AsrAdapter>,
    lid: Option<Arc<dyn LanguageIdentifier>>,
    stream_lang: StreamLanguage,
    recognizer: RecognizerRef,
    speech_caps: SpeechCapabilities,
    rate: u32,

    format: Option<(u32, u16)>,
    expected_seq: Option<u64>,
    expected_frame: Option<u64>,
    resampler: Option<Resampler>,
    anchor: Option<Anchor>,
    pos: u64,
    ring: VecDeque<f32>,
    ring_start: u64,
    block_starts: VecDeque<(u64, u64)>,
    scratch: Vec<f32>,
    resampled: Vec<f32>,

    utt: Option<Open>,
    boundaries: VecDeque<Boundary>,
    last_end: u64,
    activity_active: bool,
    hints: HintSource,
    analysis_available: bool,
    shedding: bool,
    next_utterance: u64,

    mode: RecognitionMode,
    overruns: u32,
    healthy: u32,
    blocks_since_pressure: u64,

    loaded: bool,
    started: bool,
    ended: Option<RecognitionEndReason>,
    consecutive_errors: u32,
    counters: RecognitionCounters,
    processing: Reservoir,
    first_partial: Reservoir,
    final_delay: Reservoir,
    pending_discontinuity: Option<DiscontinuityContext>,
    deferred: VecDeque<AnalysisEvent>,
}

impl std::fmt::Debug for StreamRecognizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamRecognizer")
            .field("stream", &self.stream)
            .field("adapter", &self.caps.adapter)
            .field("mode", &self.mode)
            .field("hints", &self.hints)
            .finish_non_exhaustive()
    }
}

/// The stream frame a hint refers to (`None`: not tied to audio).
fn hint_frame(e: &AnalysisEvent) -> Option<u64> {
    match e {
        AnalysisEvent::ActivityChanged { meta, .. }
        | AnalysisEvent::EndpointCandidate { meta, .. } => Some(meta.frame_range.0),
        AnalysisEvent::SegmentEnded { meta, .. } => Some(meta.frame_range.1),
        _ => None,
    }
}

fn ms_to_samples(ms: u32, rate: u32) -> u64 {
    u64::from(ms) * u64::from(rate) / 1_000
}

fn samples_to_ms(samples: u64, rate: u32) -> u64 {
    samples * 1_000 / u64::from(rate.max(1))
}

impl StreamRecognizer {
    /// A recognizer over an unloaded adapter.
    pub fn new(setup: RecognizerSetup, adapter: Box<dyn AsrAdapter>) -> Result<Self, CeError> {
        setup.config.validate()?;
        let caps = setup.capabilities;
        if caps.sample_rate == 0 || caps.sample_rate > 384_000 {
            return Err(CeError::invalid_input(
                "speech.adapter_rate_invalid",
                "the adapter declares an unusable sample rate",
            ));
        }
        let cfg = setup.config;
        let lid_fp = setup
            .identifier
            .as_deref()
            .map(|l| language::fingerprint(l, &cfg.language_id))
            .unwrap_or_else(|| crate::fnv(&format!("lid.none|{:?}", cfg.language_id)));
        let recognizer = RecognizerRef {
            adapter: caps.adapter.clone(),
            version: caps.version.clone(),
            model_fingerprint: caps.model.as_ref().and_then(|m| m.fingerprint.clone()),
            config_fingerprint: cfg.fingerprint(),
        };
        let speech_caps = SpeechCapabilities {
            recognizer: caps.clone(),
            language_identifier: setup
                .identifier
                .as_deref()
                .map(|l| (l.id().to_owned(), l.version().to_owned())),
            identifiable_languages: setup
                .identifier
                .as_deref()
                .map(LanguageIdentifier::languages)
                .unwrap_or_default(),
            code_switching: false,
        };
        let hints =
            if cfg.segmentation == SegmentationMode::ActivityHints && setup.analysis_available {
                HintSource::Activity
            } else {
                HintSource::Continuous
            };
        Ok(Self {
            stream: setup.stream,
            stream_lang: StreamLanguage::new(cfg.language_id.clone(), &cfg.language, lid_fp),
            rate: caps.sample_rate,
            lid: setup.identifier,
            recognizer,
            speech_caps,
            caps,
            adapter,
            format: None,
            expected_seq: None,
            expected_frame: None,
            resampler: None,
            anchor: None,
            pos: 0,
            ring: VecDeque::new(),
            ring_start: 0,
            block_starts: VecDeque::new(),
            scratch: Vec::new(),
            resampled: Vec::new(),
            utt: None,
            boundaries: VecDeque::new(),
            last_end: 0,
            activity_active: false,
            hints,
            analysis_available: setup.analysis_available,
            shedding: false,
            next_utterance: 0,
            mode: RecognitionMode::Full,
            overruns: 0,
            healthy: 0,
            blocks_since_pressure: PRESSURE_EVERY,
            loaded: false,
            started: false,
            ended: None,
            consecutive_errors: 0,
            counters: RecognitionCounters::default(),
            processing: Reservoir::new(RESERVOIR),
            first_partial: Reservoir::new(RESERVOIR),
            final_delay: Reservoir::new(RESERVOIR),
            pending_discontinuity: None,
            deferred: VecDeque::new(),
            cfg,
        })
    }

    /// Stream.
    pub fn stream(&self) -> AudioStreamId {
        self.stream
    }

    /// Recognizer identity.
    pub fn recognizer(&self) -> &RecognizerRef {
        &self.recognizer
    }

    /// Emit `Started`.
    pub fn start(&mut self, out: &mut Vec<SpeechEvent>) {
        if self.started {
            return;
        }
        self.started = true;
        out.push(SpeechEvent::Started {
            stream: self.stream,
            recognizer: self.recognizer.clone(),
            capabilities: Box::new(self.speech_caps.clone()),
        });
    }

    /// Load and warm up the adapter (expensive; the worker calls it on its
    /// own thread). `ModelReady` is emitted on success.
    pub fn load(&mut self, out: &mut Vec<SpeechEvent>) -> Result<(), CeError> {
        let t0 = std::time::Instant::now();
        let outcome = self.adapter.load()?;
        let ms = u32::try_from(t0.elapsed().as_millis()).unwrap_or(u32::MAX);
        self.note_model_ready(ms, outcome.reused, out);
        Ok(())
    }

    /// Record a completed load.
    pub fn note_model_ready(&mut self, load_ms: u32, reused: bool, out: &mut Vec<SpeechEvent>) {
        self.loaded = true;
        self.counters.load_ms = Some(load_ms);
        out.push(SpeechEvent::ModelReady {
            stream: self.stream,
            load_ms,
            reused,
        });
    }

    /// Record wall-clock processing time of one block, ms.
    pub fn record_processing(&mut self, ms: f32) {
        self.processing.record(ms);
    }

    /// Audio held in the pre-roll / feed ring plus queued hints (bounded:
    /// diagnostics and the bounded-memory tests).
    pub fn buffered(&self) -> (usize, usize, usize) {
        (self.ring.len(), self.boundaries.len(), self.deferred.len())
    }

    /// Whether recognition has ended.
    pub fn ended(&self) -> Option<RecognitionEndReason> {
        self.ended
    }

    // ------------------------------------------------------------ timeline

    fn to_index(&self, frame: u64) -> Option<u64> {
        let a = self.anchor?;
        let d = frame.checked_sub(a.frame)?;
        Some(
            u64::try_from(u128::from(d) * u128::from(self.rate) / u128::from(a.in_rate.max(1)))
                .unwrap_or(u64::MAX),
        )
    }

    fn to_frame(&self, idx: u64) -> u64 {
        match self.anchor {
            Some(a) => {
                a.frame
                    + u64::try_from(u128::from(idx) * u128::from(a.in_rate) / u128::from(self.rate))
                        .unwrap_or(u64::MAX)
            }
            None => 0,
        }
    }

    fn frame_time(&self, frame: u64) -> MonotonicTime {
        match self.anchor {
            Some(a) => {
                let d = frames_to_duration(frame.saturating_sub(a.frame), a.in_rate);
                MonotonicTime::from_nanos(
                    a.time
                        .as_nanos()
                        .saturating_add(u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)),
                )
            }
            None => MonotonicTime::from_nanos(0),
        }
    }

    fn seq_at(&self, idx: u64) -> u64 {
        self.block_starts
            .iter()
            .rev()
            .find(|(s, _)| *s <= idx)
            .or(self.block_starts.front())
            .map_or(0, |(_, q)| *q)
    }

    fn meta(&self, u: &Open) -> SpeechMeta {
        let (f0, f1) = (self.to_frame(u.start), self.to_frame(u.fed));
        SpeechMeta {
            stream: self.stream,
            utterance: u.id,
            sequence_range: (
                u.seq_first,
                self.seq_at(u.fed.saturating_sub(1)).max(u.seq_first),
            ),
            frame_range: (f0, f1.max(f0)),
            time_range: (self.frame_time(f0), self.frame_time(f1.max(f0))),
            input_end_frame: self.to_frame(self.pos),
            discontinuity: u.discontinuity,
            mode: self.mode,
            hints: self.hints,
        }
    }

    fn chunk_samples(&self) -> usize {
        let n = ms_to_samples(self.cfg.chunk_ms, self.rate) as usize;
        match self.mode {
            RecognitionMode::Full => n.max(1),
            RecognitionMode::Throttled => (n * 4).max(1),
        }
    }

    fn max_samples(&self) -> u64 {
        let ms = self
            .caps
            .max_utterance_ms
            .map_or(self.cfg.finalization.max_utterance_ms, |m| {
                m.min(self.cfg.finalization.max_utterance_ms)
            });
        ms_to_samples(ms, self.rate).max(1)
    }

    // ---------------------------------------------------------------- input

    /// Feed one capture block.
    pub fn push_block(
        &mut self,
        block: &AudioBlock,
        out: &mut Vec<SpeechEvent>,
    ) -> Result<(), CeError> {
        if self.ended.is_some() {
            return Ok(());
        }
        let m = &block.meta;
        if m.sample_rate == 0 || m.channels == 0 || m.frames == 0 {
            // Malformed block: skipped; the sequence gap it leaves resets
            // the timeline at the next block.
            return Ok(());
        }
        self.counters.blocks += 1;
        self.blocks_since_pressure += 1;
        let fmt = (m.sample_rate, m.channels);
        let mut gap: Option<DiscontinuityContext> = None;
        if self.format.is_some_and(|f| f != fmt) {
            gap = Some(DiscontinuityContext::FormatChange);
        } else if self.format.is_some() {
            if let Some(d) = m.discontinuity {
                gap = Some(DiscontinuityContext::Capture {
                    kind: d.kind,
                    lost_frames: d.lost_frames,
                });
            } else if let Some(expected) = self.expected_seq {
                if m.sequence > expected {
                    gap = Some(DiscontinuityContext::AnalysisDrop {
                        blocks: m.sequence - expected,
                    });
                } else if self.expected_frame.is_some_and(|f| f != m.frame_index) {
                    gap = Some(DiscontinuityContext::AnalysisDrop { blocks: 0 });
                }
            }
        }
        if let Some(context) = gap {
            self.discontinuity(context, m.frame_index, out)?;
        }
        if self.anchor.is_none() || self.format != Some(fmt) {
            self.anchor = Some(Anchor {
                frame: m.frame_index,
                time: m.timestamp,
                in_rate: m.sample_rate,
            });
            self.format = Some(fmt);
            self.resampler = Some(Resampler::new(m.sample_rate, self.rate));
        }
        self.expected_seq = Some(m.sequence + 1);
        self.expected_frame = Some(m.end_frame_index());

        self.scratch.clear();
        downmix(&block.samples, usize::from(m.channels), &mut self.scratch);
        self.resampled.clear();
        if let Some(r) = self.resampler.as_mut() {
            r.push(&self.scratch, &mut self.resampled);
        }
        self.block_starts.push_back((self.pos, m.sequence));
        let n = self.resampled.len() as u64;
        self.ring.extend(self.resampled.iter().copied());
        self.pos += n;
        self.counters.audio_ms = self
            .counters
            .audio_ms
            .saturating_add(u64::from(m.frames) * 1_000 / u64::from(m.sample_rate));
        let mut res = Ok(());
        while let Some(h) = self.deferred.front() {
            if hint_frame(h).is_some_and(|f| f >= m.end_frame_index()) {
                break;
            }
            let h = self.deferred.pop_front().expect("front exists");
            res = self.apply_hint(&h, out);
            if res.is_err() {
                break;
            }
        }
        if res.is_ok() {
            res = self.drive(out);
        }
        self.trim();
        res
    }

    fn trim(&mut self) {
        let keep = ms_to_samples(self.cfg.preroll_ms, self.rate);
        let mut floor = self.pos.saturating_sub(keep);
        if let Some(u) = &self.utt {
            floor = floor.min(u.fed);
        }
        let n = (floor.saturating_sub(self.ring_start) as usize).min(self.ring.len());
        self.ring.drain(..n);
        self.ring_start += n as u64;
        while self.block_starts.len() > 1 && self.block_starts[1].0 <= self.ring_start {
            self.block_starts.pop_front();
        }
    }

    /// Feed one Phase 3 analysis event (a hint). A hint about audio the
    /// recognizer has not received yet waits for that audio, so it is
    /// always mapped onto the right timeline.
    pub fn push_hint(
        &mut self,
        event: &AnalysisEvent,
        out: &mut Vec<SpeechEvent>,
    ) -> Result<(), CeError> {
        if self.ended.is_some() {
            return Ok(());
        }
        if let Some(f) = hint_frame(event) {
            let behind = self.expected_frame.is_none_or(|e| f >= e);
            if behind || !self.deferred.is_empty() {
                if self.deferred.len() >= MAX_BOUNDARIES {
                    self.deferred.pop_front();
                    self.counters.hint_dropped += 1;
                }
                self.deferred.push_back(event.clone());
                return Ok(());
            }
        }
        self.apply_hint(event, out)
    }

    fn apply_hint(
        &mut self,
        event: &AnalysisEvent,
        out: &mut Vec<SpeechEvent>,
    ) -> Result<(), CeError> {
        let fin = &self.cfg.finalization;
        let trailing = ms_to_samples(fin.trailing_ms, self.rate);
        match event {
            AnalysisEvent::ActivityChanged { meta, state, .. } => {
                self.activity_active = *state == ActivityState::Active;
                if self.activity_active && self.hints == HintSource::Activity {
                    if let Some(i) = self.to_index(meta.frame_range.0) {
                        let lead = ms_to_samples(self.cfg.lead_in_ms, self.rate);
                        self.queue(Boundary::Start(i.saturating_sub(lead)));
                    }
                }
            }
            AnalysisEvent::SegmentEnded { meta, reason, .. }
                if *reason == SegmentCloseReason::SilencePolicy
                    && fin.on_segment_end
                    && self.hints == HintSource::Activity =>
            {
                if let Some(e) = self.to_index(meta.frame_range.1) {
                    self.queue(Boundary::End {
                        target: e + trailing,
                        reason: FinalReason::SegmentEnd,
                        speech_end: e,
                    });
                }
            }
            AnalysisEvent::EndpointCandidate { meta, tier, .. }
                if fin.on_endpoint_tier.is_some_and(|t| *tier >= t)
                    && self.hints == HintSource::Activity =>
            {
                if let Some(e) = self.to_index(meta.frame_range.0) {
                    self.queue(Boundary::End {
                        target: e + trailing,
                        reason: FinalReason::EndpointCandidate,
                        speech_end: e,
                    });
                }
            }
            AnalysisEvent::DegradedModeChanged { to, .. } => {
                self.shedding = *to == DegradedMode::Shedding;
                self.update_hints(out);
            }
            AnalysisEvent::Stopped { .. } | AnalysisEvent::Failed { .. } => {
                self.analysis_available = false;
                self.update_hints(out);
            }
            _ => return Ok(()),
        }
        self.drive(out)
    }

    fn queue(&mut self, b: Boundary) {
        if self.boundaries.len() >= MAX_BOUNDARIES {
            self.boundaries.pop_front();
            self.counters.hint_dropped += 1;
        }
        self.boundaries.push_back(b);
    }

    fn update_hints(&mut self, out: &mut Vec<SpeechEvent>) {
        let want = if self.cfg.segmentation == SegmentationMode::ActivityHints
            && self.analysis_available
            && !self.shedding
        {
            HintSource::Activity
        } else {
            HintSource::Continuous
        };
        if want != self.hints {
            self.hints = want;
            self.boundaries.clear();
            if want == HintSource::Activity && self.utt.is_none() {
                // Do not re-feed audio from before the switch.
                self.last_end = self.pos;
            }
            out.push(SpeechEvent::HintsChanged {
                stream: self.stream,
                hints: want,
            });
        }
    }

    /// Record load and backlog after a block; may throttle or recover, and
    /// reports tap drops.
    pub fn note_load(
        &mut self,
        load: f32,
        backlog: u32,
        tap_dropped_total: u64,
        out: &mut Vec<SpeechEvent>,
    ) {
        let permille = if load.is_finite() {
            (load.max(0.0) * 1_000.0).min(u32::MAX as f32) as u32
        } else {
            0
        };
        self.counters.max_load_permille = self.counters.max_load_permille.max(permille);
        self.counters.max_backlog = self.counters.max_backlog.max(backlog);
        if tap_dropped_total > self.counters.tap_dropped_blocks {
            self.counters.tap_dropped_blocks = tap_dropped_total;
            if self.blocks_since_pressure >= PRESSURE_EVERY {
                self.blocks_since_pressure = 0;
                out.push(SpeechEvent::QueuePressure {
                    stream: self.stream,
                    backlog,
                    dropped_total: tap_dropped_total,
                });
            }
        }
        let b = &self.cfg.budget;
        let over = load > b.max_load || backlog > b.queue_blocks / 2;
        if over {
            self.overruns += 1;
            self.healthy = 0;
        } else {
            self.healthy += 1;
            self.overruns = 0;
        }
        let to = match self.mode {
            RecognitionMode::Full if b.degrade && self.overruns >= b.degrade_after => {
                Some(RecognitionMode::Throttled)
            }
            RecognitionMode::Throttled if self.healthy >= b.recover_after => {
                Some(RecognitionMode::Full)
            }
            _ => None,
        };
        if let Some(to) = to {
            out.push(SpeechEvent::ModeChanged {
                stream: self.stream,
                from: self.mode,
                to,
                load_permille: permille,
            });
            self.mode = to;
            self.overruns = 0;
            self.healthy = 0;
        }
    }

    // ---------------------------------------------------------------- core

    fn open_point(&mut self) -> Option<u64> {
        let floor = self.last_end.max(self.ring_start);
        match self.hints {
            HintSource::Continuous => (floor < self.pos).then_some(floor),
            HintSource::Activity => {
                // Ends before the next start belong to finished utterances.
                while let Some(b) = self.boundaries.front().copied() {
                    match b {
                        Boundary::End { .. } => {
                            self.boundaries.pop_front();
                        }
                        Boundary::Start(s) => {
                            if s >= self.pos {
                                return None;
                            }
                            self.boundaries.pop_front();
                            return Some(s.max(floor));
                        }
                    }
                }
                None
            }
        }
    }

    fn open(&mut self, start: u64, out: &mut Vec<SpeechEvent>) -> Result<(), CeError> {
        let id = self.next_utterance;
        self.next_utterance += 1;
        self.counters.utterances += 1;
        // Stream-absolute position at the adapter rate (the oracle aligns
        // with references by it; real adapters ignore it).
        let abs = match self.anchor {
            Some(a) => u64::try_from(
                u128::from(self.to_frame(start)) * u128::from(self.rate)
                    / u128::from(a.in_rate.max(1)),
            )
            .unwrap_or(u64::MAX),
            None => start,
        };
        self.adapter.position_hint(abs);
        let u = Open {
            id,
            start,
            seq_first: self.seq_at(start),
            fed: start,
            text: UtteranceText::new(self.cfg.stability.clone()),
            pending: Vec::new(),
            last_partial_ms: None,
            first_partial: false,
            discontinuity: self.pending_discontinuity.take(),
            words: None,
            score: None,
            language: None,
            endpoint: false,
        };
        out.push(SpeechEvent::UtteranceStarted {
            meta: self.meta(&u),
        });
        self.utt = Some(u);
        if let Err(e) = self.adapter.begin(id, &self.cfg.language) {
            return self.adapter_failed(e, out);
        }
        Ok(())
    }

    /// The end boundary that applies to the open utterance, if any.
    /// Activity onsets inside audio already received belong to the open
    /// utterance (a pause shorter than the segment-close silence) and are
    /// dropped; ends of earlier stretches of audio are stale.
    fn end_boundary(&mut self) -> Option<(u64, FinalReason, u64)> {
        let start = self.utt.as_ref()?.start;
        while let Some(b) = self.boundaries.front().copied() {
            match b {
                Boundary::End {
                    target,
                    reason,
                    speech_end,
                } => {
                    if speech_end <= start {
                        self.boundaries.pop_front();
                        continue;
                    }
                    return Some((target, reason, speech_end));
                }
                Boundary::Start(s) if s < self.pos => {
                    self.boundaries.pop_front();
                }
                Boundary::Start(_) => return None,
            }
        }
        None
    }

    fn drive(&mut self, out: &mut Vec<SpeechEvent>) -> Result<(), CeError> {
        loop {
            if self.utt.is_none() {
                let Some(s) = self.open_point() else {
                    return Ok(());
                };
                self.open(s, out)?;
                if self.utt.is_none() {
                    continue; // begin failed and was handled
                }
            }
            let end = self.end_boundary();
            let (start, fed) = match &self.utt {
                Some(u) => (u.start, u.fed),
                None => continue,
            };
            let max_at = start + self.max_samples();
            let mut limit = self.pos.min(max_at);
            if let Some((target, _, _)) = end {
                limit = limit.min(target.max(fed));
            }
            if fed < limit {
                self.feed(limit, out)?;
            }
            let Some(u) = self.utt.as_ref() else {
                continue; // an adapter error cancelled it
            };
            let fed = u.fed;
            if u.endpoint && self.cfg.finalization.on_adapter_endpoint && self.caps.endpointing {
                self.finalize(FinalReason::AdapterEndpoint, None, out)?;
                if self.hints == HintSource::Activity && self.activity_active {
                    self.boundaries.push_front(Boundary::Start(fed));
                }
                continue;
            }
            if let Some((target, reason, speech_end)) = end {
                if fed >= target {
                    self.boundaries.pop_front();
                    self.finalize(reason, Some(speech_end), out)?;
                    continue;
                }
            }
            if fed >= max_at {
                self.finalize(FinalReason::MaxDuration, None, out)?;
                if self.hints == HintSource::Activity && self.activity_active {
                    // Activity continues: a continuation opens right away.
                    self.boundaries.push_front(Boundary::Start(fed));
                }
                continue;
            }
            return Ok(());
        }
    }

    /// Feed ring audio [utt.fed, limit) in chunks.
    fn feed(&mut self, limit: u64, out: &mut Vec<SpeechEvent>) -> Result<(), CeError> {
        let chunk = self.chunk_samples();
        loop {
            let Some(u) = self.utt.as_mut() else {
                return Ok(());
            };
            if u.fed >= limit || u.endpoint {
                return Ok(());
            }
            let need = chunk.saturating_sub(u.pending.len()) as u64;
            let take = need.min(limit - u.fed);
            let from = (u.fed - self.ring_start) as usize;
            u.pending
                .extend(self.ring.range(from..from + take as usize).copied());
            u.fed += take;
            if u.pending.len() >= chunk {
                let samples = std::mem::take(&mut u.pending);
                self.accept(&samples, out)?;
            }
        }
    }

    fn accept(&mut self, samples: &[f32], out: &mut Vec<SpeechEvent>) -> Result<(), CeError> {
        match self.adapter.accept(samples) {
            Ok(h) => {
                self.consecutive_errors = 0;
                if let Some(h) = h {
                    self.hypothesis(h, out);
                }
                Ok(())
            }
            Err(e) => self.adapter_failed(e, out),
        }
    }

    fn sanitized(&mut self, mut h: AdapterHypothesis, consumed: u64) -> AdapterHypothesis {
        let timings = self.caps.word_timestamps && self.cfg.word_timings;
        if sanitize(&mut h, consumed, timings).any() {
            self.counters.malformed_outputs += 1;
        }
        if !self.caps.word_timestamps || !self.cfg.word_timings {
            h.words = None;
        }
        if self.caps.score_semantics.is_none() {
            h.score = None;
        }
        if !self.caps.reports_language {
            h.language = None;
        }
        h
    }

    fn hypothesis(&mut self, h: AdapterHypothesis, out: &mut Vec<SpeechEvent>) {
        let Some(u) = self.utt.as_ref() else {
            return;
        };
        let consumed = u.fed - u.start - u.pending.len() as u64;
        let h = self.sanitized(h, consumed);
        let rate = self.rate;
        let interval = u64::from(self.cfg.partial_interval_ms);
        let partials_on =
            self.cfg.partials && self.caps.partial_results && self.mode == RecognitionMode::Full;
        let Some(u) = self.utt.as_mut() else {
            return;
        };
        u.endpoint |= h.endpoint;
        u.words = h.words.clone();
        u.score = h.score;
        if h.language.is_some() {
            u.language = h.language.clone();
        }
        let now_ms = samples_to_ms(consumed, rate);
        if !partials_on || u.last_partial_ms.is_some_and(|l| now_ms < l + interval) {
            return;
        }
        u.last_partial_ms = Some(now_ms);
        if let Some(rev) = u.text.partial(&h.text, now_ms) {
            if !u.first_partial {
                u.first_partial = true;
                let delay = samples_to_ms(self.pos.saturating_sub(u.start), rate);
                self.first_partial.record(delay as f32);
            }
            let r = self.revision(rev, RevisionStatus::Partial, None, None);
            out.push(SpeechEvent::Transcript(Box::new(r)));
        }
    }

    fn revision(
        &mut self,
        rev: TextRevision,
        status: RevisionStatus,
        final_reason: Option<FinalReason>,
        language: Option<LanguageObservation>,
    ) -> TranscriptRevision {
        self.counters.revisions += 1;
        let u = self.utt.as_ref().expect("revision of an open utterance");
        let meta = self.meta(u);
        let words = if status == RevisionStatus::Cancelled {
            None
        } else {
            u.words.as_ref().map(|ws| {
                let committed = rev.text.committed.split_whitespace().count();
                ws.iter()
                    .enumerate()
                    .map(|(k, w)| {
                        let f0 = self.to_frame(u.start + w.start);
                        let f1 = self.to_frame(u.start + w.end).max(f0);
                        WordTiming {
                            text: w.text.clone(),
                            frame_range: (f0, f1),
                            time_range: (self.frame_time(f0), self.frame_time(f1)),
                            score: w.score,
                            committed: k < committed,
                        }
                    })
                    .collect()
            })
        };
        let score = match (u.score, self.caps.score_semantics) {
            (Some(v), Some(semantics)) if status != RevisionStatus::Cancelled => Some(Score {
                value: v,
                semantics,
            }),
            _ => None,
        };
        TranscriptRevision {
            meta,
            revision: rev.revision,
            status,
            text: rev.text,
            delta: rev.delta,
            commit_amended: rev.commit_amended,
            words,
            score,
            language,
            final_reason,
        }
    }

    fn observe_language(
        &self,
        id: u64,
        text: &str,
        reported: Option<&LanguageTag>,
    ) -> Option<LanguageObservation> {
        let cfg = &self.cfg.language_id;
        if !cfg.enabled {
            return None;
        }
        match (&self.lid, reported) {
            (Some(lid), _) => Some(language::observe_utterance(
                lid.as_ref(),
                cfg,
                &self.cfg.language,
                id,
                text,
            )),
            (None, _) if matches!(self.cfg.language, ce_core::speech::LanguageHint::Fixed(_)) => {
                let ident =
                    crate::language::ScriptedIdentifier::new(Vec::new(), None, |_| Vec::new());
                Some(language::observe_utterance(
                    &ident,
                    cfg,
                    &self.cfg.language,
                    id,
                    text,
                ))
            }
            (None, Some(l)) => Some(LanguageObservation {
                scope: LanguageScope::Utterance { utterance: id },
                decision: LanguageDecision::Identified {
                    language: l.clone(),
                },
                candidates: vec![ce_core::speech::LanguageCandidate {
                    language: l.clone(),
                    score: None,
                }],
                score_semantics: None,
                source: LanguageSource::Recognizer {
                    adapter: self.caps.adapter.clone(),
                },
                evidence: language::letters(text),
                fingerprint: self.recognizer.config_fingerprint.clone(),
            }),
            (None, None) => None,
        }
    }

    /// Flush buffered audio, take the adapter's final and emit it.
    fn finalize(
        &mut self,
        reason: FinalReason,
        speech_end: Option<u64>,
        out: &mut Vec<SpeechEvent>,
    ) -> Result<(), CeError> {
        let pending = match self.utt.as_mut() {
            Some(u) => std::mem::take(&mut u.pending),
            None => return Ok(()),
        };
        if !pending.is_empty() {
            self.accept(&pending, out)?;
            if self.utt.is_none() {
                return Ok(()); // the flush failed and cancelled it
            }
        }
        let h = match self.adapter.finalize() {
            Ok(h) => h,
            Err(e) => return self.adapter_failed(e, out),
        };
        let Some(u) = self.utt.as_ref() else {
            return Ok(());
        };
        let (id, fed, consumed, earlier) = (u.id, u.fed, u.fed - u.start, u.language.clone());
        let h = self.sanitized(h, consumed);
        let reported = h.language.clone().or(earlier);
        let Some(u) = self.utt.as_mut() else {
            return Ok(());
        };
        if h.words.is_some() {
            u.words = h.words.clone();
        }
        if h.score.is_some() {
            u.score = h.score;
        }
        let rev = u.text.finalize(&h.text);
        let lang = self.observe_language(id, &rev.text.committed, reported.as_ref());
        if rev.commit_amended {
            self.counters.commit_amendments += 1;
        }
        if let Some(e) = speech_end {
            self.final_delay
                .record(samples_to_ms(self.pos.saturating_sub(e), self.rate) as f32);
        }
        let r = self.revision(rev, RevisionStatus::Final, Some(reason), lang.clone());
        out.push(SpeechEvent::Transcript(Box::new(r)));
        self.counters.finals += 1;
        self.utt = None;
        self.last_end = fed;
        if let Some(l) = lang {
            if let Some(stream) = self.stream_lang.observe(&l) {
                out.push(SpeechEvent::LanguageObserved {
                    stream: self.stream,
                    observation: stream,
                });
            }
        }
        Ok(())
    }

    /// Abandon the open utterance: committed text is kept, nothing follows.
    fn cancel(&mut self, reason: FinalReason, out: &mut Vec<SpeechEvent>) {
        let Some(u) = self.utt.as_mut() else {
            return;
        };
        let rev = u.text.cancel();
        let fed = u.fed;
        let r = self.revision(rev, RevisionStatus::Cancelled, Some(reason), None);
        out.push(SpeechEvent::Transcript(Box::new(r)));
        self.counters.cancelled += 1;
        self.utt = None;
        self.last_end = fed;
        self.adapter.reset();
    }

    fn adapter_failed(&mut self, e: CeError, out: &mut Vec<SpeechEvent>) -> Result<(), CeError> {
        self.counters.adapter_errors += 1;
        self.consecutive_errors += 1;
        if self.utt.is_some() {
            self.cancel(FinalReason::Failure, out);
        } else {
            self.adapter.reset();
        }
        // Audio of the failed attempt is not fed again.
        self.last_end = self.last_end.max(self.pos);
        out.push(SpeechEvent::Reset {
            stream: self.stream,
            reason: ResetReason::AdapterError,
        });
        if self.consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
            return Err(CeError::new(
                ErrorCategory::Provider,
                "speech.adapter_failed",
                "the speech recognizer failed repeatedly",
            )
            .with_context("adapter", self.caps.adapter.clone())
            .with_context("errors", self.consecutive_errors.to_string())
            .with_source(e));
        }
        Ok(())
    }

    fn discontinuity(
        &mut self,
        context: DiscontinuityContext,
        resume_frame: u64,
        out: &mut Vec<SpeechEvent>,
    ) -> Result<(), CeError> {
        self.counters.discontinuities += 1;
        if let Some(u) = self.utt.as_mut() {
            u.discontinuity = Some(context);
        }
        let res = if self.utt.is_some() {
            self.finalize(FinalReason::Discontinuity, None, out)
        } else {
            Ok(())
        };
        self.adapter.reset();
        out.push(SpeechEvent::Discontinuity {
            stream: self.stream,
            context,
            resume_frame,
        });
        out.push(SpeechEvent::Reset {
            stream: self.stream,
            reason: ResetReason::Discontinuity,
        });
        // New timeline from the resuming block.
        self.anchor = None;
        self.pos = 0;
        self.ring.clear();
        self.ring_start = 0;
        self.block_starts.clear();
        self.boundaries.clear();
        self.last_end = 0;
        // Activity that was in progress continues after the gap (the gap
        // may be the recognizer's own drops while Phase 3 saw everything):
        // resume at the first audio after it. A Phase 3 onset that follows
        // falls inside the resumed utterance and is absorbed.
        if self.hints == HintSource::Activity && self.activity_active {
            self.boundaries.push_back(Boundary::Start(0));
        }
        self.pending_discontinuity = Some(context);
        res
    }

    /// Cancel the open utterance and reset the adapter; recognition
    /// continues with the next audio.
    pub fn reset(&mut self, out: &mut Vec<SpeechEvent>) {
        if self.ended.is_some() {
            return;
        }
        if self.utt.is_some() {
            self.cancel(FinalReason::Reset, out);
        } else {
            self.adapter.reset();
        }
        self.boundaries.clear();
        self.last_end = self.pos;
        self.consecutive_errors = 0;
        out.push(SpeechEvent::Reset {
            stream: self.stream,
            reason: ResetReason::Requested,
        });
    }

    /// End recognition. The open utterance is finalized (stop, shutdown,
    /// stream end) or cancelled (revocation, failure; nothing more of its
    /// audio is used).
    pub fn finish(&mut self, reason: RecognitionEndReason, out: &mut Vec<SpeechEvent>) {
        if self.ended.is_some() {
            return;
        }
        if self.utt.is_some() {
            let fin = match reason {
                RecognitionEndReason::Stopped | RecognitionEndReason::Shutdown => {
                    Some(FinalReason::Stopped)
                }
                RecognitionEndReason::StreamClosed
                | RecognitionEndReason::StreamInvalidated
                | RecognitionEndReason::StreamFailed => Some(FinalReason::StreamEnd),
                RecognitionEndReason::Revoked => {
                    self.cancel(FinalReason::Stopped, out);
                    None
                }
                RecognitionEndReason::Failed | RecognitionEndReason::LoadFailed => {
                    self.cancel(FinalReason::Failure, out);
                    None
                }
            };
            if let Some(fin) = fin {
                if self.finalize(fin, None, out).is_err() && self.utt.is_some() {
                    self.cancel(FinalReason::Failure, out);
                }
            }
        }
        self.ended = Some(reason);
        out.push(SpeechEvent::Stopped {
            stream: self.stream,
            reason,
            counters: Box::new(self.counters()),
        });
    }

    /// Fail: `Failed` then `Stopped(Failed)`.
    pub fn fail(&mut self, error: &CeError, out: &mut Vec<SpeechEvent>) {
        if self.ended.is_some() {
            return;
        }
        out.push(SpeechEvent::Failed {
            stream: self.stream,
            error: error.report(&RedactionPolicy::DEFAULT),
        });
        let reason = if self.loaded {
            RecognitionEndReason::Failed
        } else {
            RecognitionEndReason::LoadFailed
        };
        self.finish(reason, out);
    }

    // ---------------------------------------------------------------- status

    /// Counters with current percentiles.
    pub fn counters(&self) -> RecognitionCounters {
        RecognitionCounters {
            processing_ms: self.processing.summary(),
            first_partial_ms: self.first_partial.summary(),
            final_delay_ms: self.final_delay.summary(),
            ..self.counters.clone()
        }
    }

    /// Status snapshot.
    pub fn status(&self) -> RecognitionStatus {
        RecognitionStatus {
            stream: self.stream,
            recognizer: self.recognizer.clone(),
            capabilities: self.speech_caps.clone(),
            running: self.ended.is_none(),
            loaded: self.loaded,
            mode: self.mode,
            hints: self.hints,
            open_utterance: self.utt.as_ref().map(|u| u.id),
            stream_language: self.stream_lang.current().cloned(),
            counters: self.counters(),
            ended: self.ended,
        }
    }
}
