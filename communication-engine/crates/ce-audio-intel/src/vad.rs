//! Voice-activity detector adapters.
//!
//! A detector turns each [`Hop`] into a score in [0, 1] with documented
//! semantics ([`ScoreSemantics`]). Detectors are replaceable through
//! [`VoiceActivityDetector`]; the pipeline only relies on the trait.
//!
//! * [`SpectralVad`] (`ce.vad.spectral`, default) — adaptive noise floor →
//!   SNR evidence, combined with periodicity, speech-band energy ratio and
//!   spectral flatness through a fixed logistic model whose weights were set
//!   on the **development** fixtures only (see the evaluation report).
//! * [`EnergyVad`] (`ce.vad.energy`) — adaptive-floor SNR only; the
//!   degraded-mode fallback.
//! * [`ScriptedVad`] — deterministic fake for tests and fault injection
//!   (scripted scores, per-hop delay, panic at a hop).
//!
//! Scores are **heuristic confidences, not probabilities**: monotonic in the
//! evidence, uncalibrated, not comparable across detectors.

use std::time::Duration;

use ce_core::intel::{ScoreSemantics, VadDescriptor};

use crate::framer::Hop;

/// One detector output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VadObservation {
    /// Score in [0, 1] (see the detector's score semantics).
    pub score: f32,
    /// False while the detector is still adapting (warm-up).
    pub settled: bool,
}

/// A replaceable voice-activity detector.
pub trait VoiceActivityDetector: Send {
    /// Static facts.
    fn descriptor(&self) -> VadDescriptor;
    /// Forget all adaptive state (after a discontinuity).
    fn reset(&mut self);
    /// Whether it uses spectral features (disabled in degraded modes).
    fn needs_spectral(&self) -> bool;
    /// Score one hop.
    fn score(&mut self, hop: &Hop) -> VadObservation;
    /// Whether it needs the hop's mono samples (external / neural
    /// detectors). Default: features are enough.
    fn needs_audio(&self) -> bool {
        false
    }
    /// Score one hop given its mono samples at `rate` Hz. Called instead of
    /// [`score`](Self::score) when [`needs_audio`](Self::needs_audio) is true.
    fn score_audio(&mut self, hop: &Hop, mono: &[f32], rate: u32) -> VadObservation {
        let _ = (mono, rate);
        self.score(hop)
    }
}

/// Hops after which the floor counts as settled.
const WARMUP_HOPS: u32 = 30;
/// Absolute gate: below this level nothing is activity.
const GATE_DBFS: f32 = -65.0;

/// Adaptive noise-floor tracker: follows dips quickly (30 % of the
/// difference per hop) and rises slowly (3 dB/s), so speech pauses pull it
/// down and steady noise eventually becomes the floor.
#[derive(Clone, Debug)]
pub struct FloorTracker {
    floor: Option<f32>,
    rise_per_hop: f32,
    hops: u32,
}

impl FloorTracker {
    /// A tracker for `hop_ms` hops.
    pub fn new(hop_ms: u32) -> Self {
        Self {
            floor: None,
            rise_per_hop: 3.0 * hop_ms as f32 / 1000.0,
            hops: 0,
        }
    }

    /// Update with a hop level and return the floor.
    pub fn update(&mut self, level_db: f32) -> f32 {
        self.hops = self.hops.saturating_add(1);
        let f = match self.floor {
            None => level_db,
            Some(f) if level_db < f => f + 0.3 * (level_db - f),
            Some(f) => (f + self.rise_per_hop).min(level_db),
        };
        self.floor = Some(f);
        f
    }

    /// Current floor.
    pub fn floor(&self) -> Option<f32> {
        self.floor
    }

    /// Whether enough hops were seen.
    pub fn settled(&self) -> bool {
        self.hops >= WARMUP_HOPS
    }

    /// Forget everything.
    pub fn reset(&mut self) {
        self.floor = None;
        self.hops = 0;
    }
}

fn sigmoid(z: f32) -> f32 {
    1.0 / (1.0 + (-z).exp())
}

/// Logistic model weights of [`SpectralVad`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpectralWeights {
    /// SNR (dB) at which energy evidence is neutral.
    pub snr_mid: f32,
    /// dB per logit unit.
    pub snr_scale: f32,
    /// Weight of (periodicity − 0.45).
    pub periodicity: f32,
    /// Weight of (speech ratio − 0.5).
    pub speech_ratio: f32,
    /// Weight of (flatness − 0.35) (subtracted).
    pub flatness: f32,
    /// Weight of (level modulation − 10 dB) / 10: the 10th–90th percentile
    /// level range over the last 500 ms. Speech is syllabically modulated;
    /// steady noise and sustained music are not.
    pub modulation: f32,
}

impl SpectralWeights {
    /// The v1 weights (set on development fixtures; see evaluation report).
    pub const V1: SpectralWeights = SpectralWeights {
        snr_mid: 6.0,
        snr_scale: 2.5,
        periodicity: 5.0,
        speech_ratio: 4.0,
        flatness: 6.0,
        modulation: 2.0,
    };
}

/// Level history for the modulation feature.
#[derive(Clone, Debug)]
struct Modulation {
    levels: std::collections::VecDeque<f32>,
    cap: usize,
    scratch: Vec<f32>,
}

impl Modulation {
    fn new(hop_ms: u32) -> Self {
        let cap = (500 / hop_ms.max(1)) as usize;
        Self {
            levels: std::collections::VecDeque::with_capacity(cap),
            cap,
            scratch: Vec::with_capacity(cap),
        }
    }

    /// Push a level; returns the p10–p90 range in dB once half full.
    fn update(&mut self, level: f32) -> Option<f32> {
        if self.levels.len() == self.cap {
            self.levels.pop_front();
        }
        self.levels.push_back(level.max(-100.0));
        if self.levels.len() < self.cap / 2 {
            return None;
        }
        self.scratch.clear();
        self.scratch.extend(self.levels.iter().copied());
        self.scratch.sort_by(f32::total_cmp);
        let n = self.scratch.len();
        Some(self.scratch[n * 9 / 10] - self.scratch[n / 10])
    }

    fn reset(&mut self) {
        self.levels.clear();
    }
}

/// CE's native spectral detector.
#[derive(Clone, Debug)]
pub struct SpectralVad {
    floor: FloorTracker,
    modulation: Modulation,
    weights: SpectralWeights,
}

impl SpectralVad {
    /// With the v1 weights.
    pub fn new(hop_ms: u32) -> Self {
        Self::with_weights(hop_ms, SpectralWeights::V1)
    }

    /// With explicit weights (evaluation and tuning).
    pub fn with_weights(hop_ms: u32, weights: SpectralWeights) -> Self {
        Self {
            floor: FloorTracker::new(hop_ms),
            modulation: Modulation::new(hop_ms),
            weights,
        }
    }
}

impl VoiceActivityDetector for SpectralVad {
    fn descriptor(&self) -> VadDescriptor {
        VadDescriptor {
            id: "ce.vad.spectral".into(),
            version: "1.0.0".into(),
            score_semantics: ScoreSemantics::HeuristicConfidence,
            min_sample_rate: crate::framer::MIN_RATE,
            max_sample_rate: crate::framer::MAX_RATE,
            deterministic: true,
            lookahead_ms: 0,
            warmup_hops: WARMUP_HOPS,
        }
    }

    fn reset(&mut self) {
        self.floor.reset();
        self.modulation.reset();
    }

    fn needs_spectral(&self) -> bool {
        true
    }

    fn score(&mut self, hop: &Hop) -> VadObservation {
        let level = hop.features.level_db;
        let floor = self.floor.update(level);
        let modulation = self.modulation.update(level);
        let settled = self.floor.settled();
        if level < GATE_DBFS {
            return VadObservation {
                score: 0.0,
                settled,
            };
        }
        let w = &self.weights;
        let mut z = (level - floor - w.snr_mid) / w.snr_scale;
        if let Some(m) = modulation {
            z += w.modulation * (m.min(30.0) - 10.0) / 10.0;
        }
        if let Some(s) = hop.features.spectral {
            z += w.periodicity * (s.periodicity - 0.45) + w.speech_ratio * (s.speech_ratio - 0.5)
                - w.flatness * (s.flatness - 0.35);
        }
        VadObservation {
            score: sigmoid(z),
            settled,
        }
    }
}

/// Adaptive-floor energy detector.
#[derive(Clone, Debug)]
pub struct EnergyVad {
    floor: FloorTracker,
}

impl EnergyVad {
    /// A detector for `hop_ms` hops.
    pub fn new(hop_ms: u32) -> Self {
        Self {
            floor: FloorTracker::new(hop_ms),
        }
    }
}

impl VoiceActivityDetector for EnergyVad {
    fn descriptor(&self) -> VadDescriptor {
        VadDescriptor {
            id: "ce.vad.energy".into(),
            version: "1.0.0".into(),
            score_semantics: ScoreSemantics::HeuristicConfidence,
            min_sample_rate: crate::framer::MIN_RATE,
            max_sample_rate: crate::framer::MAX_RATE,
            deterministic: true,
            lookahead_ms: 0,
            warmup_hops: WARMUP_HOPS,
        }
    }

    fn reset(&mut self) {
        self.floor.reset();
    }

    fn needs_spectral(&self) -> bool {
        false
    }

    fn score(&mut self, hop: &Hop) -> VadObservation {
        let level = hop.features.level_db;
        let floor = self.floor.update(level);
        let score = if level < GATE_DBFS {
            0.0
        } else {
            sigmoid((level - floor - 9.0) / 2.5)
        };
        VadObservation {
            score,
            settled: self.floor.settled(),
        }
    }
}

/// Deterministic fake detector with fault injection.
pub struct ScriptedVad {
    script: Box<dyn Fn(u64) -> f32 + Send>,
    delay: Duration,
    panic_at: Option<u64>,
    hops: u64,
}

impl std::fmt::Debug for ScriptedVad {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptedVad")
            .field("hops", &self.hops)
            .finish_non_exhaustive()
    }
}

impl ScriptedVad {
    /// Score hop `i` (counted from the last reset) with `script(i)`.
    pub fn new(script: impl Fn(u64) -> f32 + Send + 'static) -> Self {
        Self {
            script: Box::new(script),
            delay: Duration::ZERO,
            panic_at: None,
            hops: 0,
        }
    }

    /// Busy-wait this long per hop (overload injection).
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    /// Panic when scoring hop `n` (failure injection).
    pub fn panic_at(mut self, n: u64) -> Self {
        self.panic_at = Some(n);
        self
    }
}

impl VoiceActivityDetector for ScriptedVad {
    fn descriptor(&self) -> VadDescriptor {
        VadDescriptor {
            id: "ce.vad.scripted".into(),
            version: "1.0.0".into(),
            score_semantics: ScoreSemantics::HeuristicConfidence,
            min_sample_rate: crate::framer::MIN_RATE,
            max_sample_rate: crate::framer::MAX_RATE,
            deterministic: true,
            lookahead_ms: 0,
            warmup_hops: 0,
        }
    }

    fn reset(&mut self) {
        self.hops = 0;
    }

    fn needs_spectral(&self) -> bool {
        false
    }

    fn score(&mut self, _hop: &Hop) -> VadObservation {
        let i = self.hops;
        self.hops += 1;
        if self.panic_at == Some(i) {
            panic!("scripted VAD panic at hop {i}");
        }
        if !self.delay.is_zero() {
            let start = std::time::Instant::now();
            while start.elapsed() < self.delay {
                std::hint::spin_loop();
            }
        }
        VadObservation {
            score: (self.script)(i).clamp(0.0, 1.0),
            settled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framer::{Framer, FramerItem};
    use crate::testutil::{block, noise, sine, speechlike};

    fn scores(vad: &mut dyn VoiceActivityDetector, samples: &[f32], rate: u32) -> Vec<f32> {
        let mut f = Framer::new(10, 0.999);
        let mut out = Vec::new();
        for (i, c) in samples.chunks(rate as usize / 100).enumerate() {
            f.push(
                &block(
                    i as u64,
                    (i * rate as usize / 100) as u64,
                    rate,
                    1,
                    c.to_vec(),
                ),
                &mut |it| {
                    if let FramerItem::Hop(h) = it {
                        out.push(vad.score(&h).score);
                    }
                },
            )
            .unwrap();
        }
        out
    }

    #[test]
    fn floor_tracker_follows_dips_fast_and_rises_slowly() {
        let mut t = FloorTracker::new(10);
        assert_eq!(t.update(-30.0), -30.0);
        let f = t.update(-60.0);
        assert!((f + 39.0).abs() < 1e-4, "{f}");
        for _ in 0..100 {
            t.update(-20.0);
        }
        let f = t.floor().unwrap();
        assert!(f < -20.0 && f > -50.0, "rises 3 dB/s: {f}");
        assert!(t.settled());
        t.reset();
        assert!(t.floor().is_none() && !t.settled());
    }

    #[test]
    fn spectral_vad_separates_speechlike_activity_from_silence_and_noise() {
        let rate = 16_000;
        let mut s = vec![0.0f32; rate as usize];
        s.extend(speechlike(1, rate, 0.2, rate as usize));
        let sc = scores(&mut SpectralVad::new(10), &s, rate);
        let silence = &sc[10..90];
        let active: f32 = sc[110..190].iter().sum::<f32>() / 80.0;
        assert!(silence.iter().all(|v| *v < 0.1));
        assert!(active > 0.6, "mean active score {active}");
        // Stationary white noise settles below the onset threshold.
        let n = noise(3, rate as usize, 0.05);
        let mut long = n.clone();
        long.extend(noise(4, rate as usize, 0.05));
        let sc = scores(&mut SpectralVad::new(10), &long, rate);
        let tail: f32 = sc[150..].iter().sum::<f32>() / (sc.len() - 150) as f32;
        assert!(tail < 0.3, "steady noise tail {tail}");
        let _ = sine(1.0, rate, 0.0, 1);
    }

    #[test]
    fn energy_vad_and_gate() {
        let rate = 16_000;
        let mut s = vec![0.0f32; 8_000];
        s.extend(sine(300.0, rate, 0.0003, 8_000)); // -73 dBFS: below the gate
        let sc = scores(&mut EnergyVad::new(10), &s, rate);
        assert!(sc.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn scripted_vad_is_deterministic_and_injects_faults() {
        let mut v = ScriptedVad::new(|i| if i % 2 == 0 { 1.0 } else { 0.0 });
        let h = crate::testutil::hop(0);
        assert_eq!(v.score(&h).score, 1.0);
        assert_eq!(v.score(&h).score, 0.0);
        v.reset();
        assert_eq!(v.score(&h).score, 1.0);
        let mut p = ScriptedVad::new(|_| 0.5).panic_at(1);
        p.score(&h);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.score(&h))).is_err());
    }
}
