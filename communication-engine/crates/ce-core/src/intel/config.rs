//! Analysis configuration. Every field has a documented default; partial
//! documents override defaults; unknown fields are rejected. The
//! [`AnalysisConfig::fingerprint`] identifies a configuration in results.

use serde::{Deserialize, Serialize};

use crate::error::CeError;

/// Which voice-activity detector to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VadAlgorithm {
    /// `ce.vad.spectral` — CE's native detector: adaptive noise floor, SNR,
    /// speech-band energy, spectral flatness and periodicity. Any sample
    /// rate 8–96 kHz. Default (ADR-0014).
    Spectral,
    /// `ce.vad.energy` — adaptive-floor energy detector. Cheapest; also the
    /// degraded-mode fallback.
    Energy,
}

impl VadAlgorithm {
    /// Stable id.
    pub const fn id(self) -> &'static str {
        match self {
            VadAlgorithm::Spectral => "ce.vad.spectral",
            VadAlgorithm::Energy => "ce.vad.energy",
        }
    }
}

/// Named stabilization presets. `Custom` uses [`ActivityConfig`] as given.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityProfile {
    /// Fast onsets, short hangover.
    Responsive,
    /// Default trade-off.
    Balanced,
    /// Slow onsets, long hangover: fewer false activations, more delay.
    Conservative,
    /// Use the explicit values.
    Custom,
}

/// Hysteresis / stabilization of detector scores.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ActivityConfig {
    /// Preset; for anything but `Custom` the values below are replaced by
    /// the preset's.
    pub profile: ActivityProfile,
    /// Score at or above which a frame counts towards onset. Default 0.6.
    pub onset_threshold: f32,
    /// Score below which a frame counts towards offset (< onset). Default 0.4.
    pub offset_threshold: f32,
    /// Consecutive above-onset time required to become active, ms. Default 40.
    pub onset_min_ms: u32,
    /// Below-offset time required to become inactive (hangover), ms.
    /// Default 240.
    pub offset_hangover_ms: u32,
}

impl Default for ActivityConfig {
    fn default() -> Self {
        Self::preset(ActivityProfile::Balanced)
    }
}

impl ActivityConfig {
    /// The values of a preset.
    pub fn preset(profile: ActivityProfile) -> Self {
        let (on, off, min, hang) = match profile {
            ActivityProfile::Responsive => (0.55, 0.4, 20, 150),
            ActivityProfile::Balanced | ActivityProfile::Custom => (0.6, 0.4, 40, 240),
            ActivityProfile::Conservative => (0.7, 0.45, 80, 400),
        };
        Self {
            profile,
            onset_threshold: on,
            offset_threshold: off,
            onset_min_ms: min,
            offset_hangover_ms: hang,
        }
    }

    /// Values after applying the profile.
    pub fn effective(&self) -> Self {
        match self.profile {
            ActivityProfile::Custom => self.clone(),
            p => Self::preset(p),
        }
    }
}

/// Segmentation and endpoint-candidate policy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SegmentConfig {
    /// Silence after the last activity that closes a segment, ms. Default 700.
    pub close_silence_ms: u32,
    /// Silence durations at which endpoint candidates are emitted, ms,
    /// ascending, each < `close_silence_ms`. Default [250, 500].
    pub endpoint_tiers_ms: Vec<u32>,
    /// Longest segment before it is closed (`max_duration`) and a
    /// continuation opened, ms. Default 30000.
    pub max_segment_ms: u32,
    /// Shortest silence reported as a silence window, ms. Default 200.
    pub min_silence_report_ms: u32,
}

impl Default for SegmentConfig {
    fn default() -> Self {
        Self {
            close_silence_ms: 700,
            endpoint_tiers_ms: vec![250, 500],
            max_segment_ms: 30_000,
            min_silence_report_ms: 200,
        }
    }
}

/// Quality analysis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QualityConfig {
    /// Enable quality analysis. Default true.
    pub enabled: bool,
    /// Aggregation window, ms. Default 1000.
    pub window_ms: u32,
    /// Sample magnitude counted as clipped. Default 0.999.
    pub clip_level: f32,
}

impl Default for QualityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            window_ms: 1_000,
            clip_level: 0.999,
        }
    }
}

/// Signal-condition indicators.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConditionConfig {
    /// Enable indicators (requires quality analysis). Default true.
    pub enabled: bool,
    /// Inactivity that counts as sustained silence, ms. Default 2000.
    pub sustained_silence_ms: u32,
    /// Noise floor (dBFS) above which inactive input counts as noisy.
    /// Default -50.
    pub noise_floor_dbfs: f32,
    /// Window RMS level (dBFS) above which input counts as overloaded.
    /// Default -6.
    pub overload_dbfs: f32,
    /// Clipped-sample ratio above which clipping is indicated. Default 0.001.
    pub clipping_ratio: f32,
}

impl Default for ConditionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sustained_silence_ms: 2_000,
            noise_floor_dbfs: -50.0,
            overload_dbfs: -6.0,
            clipping_ratio: 0.001,
        }
    }
}

/// Per-stream resource budget and degradation thresholds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnalysisBudget {
    /// Capacity of the tap between the capture worker and the analysis
    /// worker, in blocks. When full, blocks are dropped for analysis only
    /// (capture is unaffected). Default 50 (≈ 0.5 s of 10 ms blocks).
    pub queue_blocks: u32,
    /// Processing time per block, as a fraction of the block's duration,
    /// above which a block counts as an overrun. Default 0.5.
    pub max_load: f32,
    /// Consecutive overruns (or backlog above half capacity) that step down
    /// one degraded level. Default 5.
    pub degrade_after: u32,
    /// Consecutive healthy blocks that step back up one level. Default 100.
    pub recover_after: u32,
    /// A worker that processed nothing for this long while input was queued
    /// is declared stalled, ms. Default 2000.
    pub stall_timeout_ms: u64,
    /// Whether overload steps the degraded mode down (Full → Reduced →
    /// Shedding). When false the analysis always runs in `Full` and overload
    /// only shows as tap drops. Default true.
    pub degrade: bool,
}

impl Default for AnalysisBudget {
    fn default() -> Self {
        Self {
            queue_blocks: 50,
            max_load: 0.5,
            degrade_after: 5,
            recover_after: 100,
            stall_timeout_ms: 2_000,
            degrade: true,
        }
    }
}

/// Complete analysis configuration (Phase 3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnalysisConfig {
    /// Detector.
    pub vad: VadAlgorithm,
    /// Analysis hop (frame) length, ms: 10, 20 or 30. Spectral features use
    /// a Hann window of two hops (50 % overlap). Default 10.
    pub hop_ms: u32,
    /// Stabilization.
    pub activity: ActivityConfig,
    /// Segmentation and endpoint candidates.
    pub segment: SegmentConfig,
    /// Quality analysis.
    pub quality: QualityConfig,
    /// Condition indicators.
    pub conditions: ConditionConfig,
    /// Adapt stabilization to degraded input (noise, low SNR). Default true.
    pub adaptive_policy: bool,
    /// Resource budget.
    pub budget: AnalysisBudget,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            vad: VadAlgorithm::Spectral,
            hop_ms: 10,
            activity: ActivityConfig::default(),
            segment: SegmentConfig::default(),
            quality: QualityConfig::default(),
            conditions: ConditionConfig::default(),
            adaptive_policy: true,
            budget: AnalysisBudget::default(),
        }
    }
}

fn bad(field: &'static str, allowed: &'static str) -> CeError {
    CeError::invalid_config(
        "config.analysis_invalid",
        "analysis configuration value is invalid",
    )
    .with_context("field", field)
    .with_context("allowed", allowed)
}

impl AnalysisConfig {
    /// Validate ranges and relationships.
    pub fn validate(&self) -> Result<(), CeError> {
        if !matches!(self.hop_ms, 10 | 20 | 30) {
            return Err(bad("hop_ms", "10, 20 or 30"));
        }
        let a = self.activity.effective();
        let unit = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
        if !unit(a.onset_threshold)
            || !unit(a.offset_threshold)
            || a.offset_threshold >= a.onset_threshold
        {
            return Err(bad("activity thresholds", "0 <= offset < onset <= 1"));
        }
        if a.onset_min_ms > 1_000 || a.offset_hangover_ms > 5_000 {
            return Err(bad(
                "activity timings",
                "onset_min_ms <= 1000, offset_hangover_ms <= 5000",
            ));
        }
        let s = &self.segment;
        if !(50..=10_000).contains(&s.close_silence_ms) {
            return Err(bad("segment.close_silence_ms", "50..=10000"));
        }
        if s.endpoint_tiers_ms.len() > 4
            || s.endpoint_tiers_ms.windows(2).any(|w| w[0] >= w[1])
            || s.endpoint_tiers_ms
                .iter()
                .any(|t| *t == 0 || *t >= s.close_silence_ms)
        {
            return Err(bad(
                "segment.endpoint_tiers_ms",
                "at most 4 ascending values in 1..close_silence_ms",
            ));
        }
        if !(1_000..=600_000).contains(&s.max_segment_ms) {
            return Err(bad("segment.max_segment_ms", "1000..=600000"));
        }
        if !(100..=10_000).contains(&self.quality.window_ms) {
            return Err(bad("quality.window_ms", "100..=10000"));
        }
        if !(self.quality.clip_level > 0.5 && self.quality.clip_level <= 1.0) {
            return Err(bad("quality.clip_level", "(0.5, 1.0]"));
        }
        let c = &self.conditions;
        if !c.noise_floor_dbfs.is_finite()
            || !c.overload_dbfs.is_finite()
            || !(0.0..1.0).contains(&c.clipping_ratio)
        {
            return Err(bad("conditions", "finite levels, clipping_ratio in [0, 1)"));
        }
        let b = &self.budget;
        if !(4..=10_000).contains(&b.queue_blocks)
            || !(b.max_load > 0.0 && b.max_load <= 10.0)
            || b.degrade_after == 0
            || b.recover_after == 0
            || !(100..=600_000).contains(&b.stall_timeout_ms)
        {
            return Err(bad(
                "budget",
                "queue_blocks 4..=10000, max_load (0,10], counts >= 1, stall 100..=600000 ms",
            ));
        }
        Ok(())
    }

    /// A stable 64-bit fingerprint of the effective configuration (FNV-1a
    /// over a canonical field listing), rendered as 16 hex digits.
    pub fn fingerprint(&self) -> String {
        let a = self.activity.effective();
        let s = &self.segment;
        let q = &self.quality;
        let c = &self.conditions;
        let b = &self.budget;
        let canonical = format!(
            "v1|{}|{}|{}|{}|{}|{}|{}|{:?}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            self.vad.id(),
            self.hop_ms,
            a.onset_threshold,
            a.offset_threshold,
            a.onset_min_ms,
            a.offset_hangover_ms,
            s.close_silence_ms,
            s.endpoint_tiers_ms,
            s.max_segment_ms,
            s.min_silence_report_ms,
            q.enabled,
            q.window_ms,
            q.clip_level,
            c.enabled,
            c.sustained_silence_ms,
            c.noise_floor_dbfs,
            c.overload_dbfs,
            c.clipping_ratio,
            self.adaptive_policy,
            b.queue_blocks,
            b.max_load,
            b.degrade_after,
            b.recover_after,
            b.stall_timeout_ms,
            b.degrade
        );
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in canonical.bytes() {
            h ^= u64::from(byte);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        format!("{h:016x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate_and_presets_apply() {
        AnalysisConfig::default().validate().unwrap();
        let c = ActivityConfig {
            profile: ActivityProfile::Conservative,
            onset_threshold: 0.1,
            ..ActivityConfig::default()
        };
        assert_eq!(
            c.effective().onset_threshold,
            0.7,
            "presets override explicit values"
        );
        let custom = ActivityConfig {
            profile: ActivityProfile::Custom,
            onset_threshold: 0.8,
            ..ActivityConfig::default()
        };
        assert_eq!(custom.effective().onset_threshold, 0.8);
    }

    #[test]
    fn invalid_values_are_rejected() {
        type Mutation = Box<dyn Fn(&mut AnalysisConfig)>;
        let cases: Vec<Mutation> = vec![
            Box::new(|c| c.hop_ms = 15),
            Box::new(|c| {
                c.activity = ActivityConfig {
                    profile: ActivityProfile::Custom,
                    onset_threshold: 0.3,
                    offset_threshold: 0.4,
                    ..ActivityConfig::default()
                }
            }),
            Box::new(|c| {
                c.activity = ActivityConfig {
                    profile: ActivityProfile::Custom,
                    onset_threshold: f32::NAN,
                    ..ActivityConfig::default()
                }
            }),
            Box::new(|c| c.segment.endpoint_tiers_ms = vec![500, 250]),
            Box::new(|c| c.segment.endpoint_tiers_ms = vec![800]),
            Box::new(|c| c.segment.close_silence_ms = 10),
            Box::new(|c| c.quality.window_ms = 10),
            Box::new(|c| c.quality.clip_level = 0.1),
            Box::new(|c| c.conditions.clipping_ratio = 2.0),
            Box::new(|c| c.budget.queue_blocks = 1),
            Box::new(|c| c.budget.max_load = 0.0),
        ];
        for (i, mutate) in cases.iter().enumerate() {
            let mut c = AnalysisConfig::default();
            mutate(&mut c);
            assert_eq!(
                c.validate().unwrap_err().code(),
                "config.analysis_invalid",
                "case {i}"
            );
        }
    }

    #[test]
    fn fingerprints_are_stable_and_sensitive() {
        let a = AnalysisConfig::default().fingerprint();
        assert_eq!(a, AnalysisConfig::default().fingerprint());
        assert_eq!(a.len(), 16);
        let mut c = AnalysisConfig::default();
        c.segment.close_silence_ms += 1;
        assert_ne!(c.fingerprint(), a);
        let parsed: AnalysisConfig =
            serde_json::from_str(r#"{"vad":"energy","segment":{"close_silence_ms":900}}"#).unwrap();
        assert_eq!(parsed.vad, VadAlgorithm::Energy);
        assert_eq!(parsed.segment.endpoint_tiers_ms, vec![250, 500]);
        assert!(serde_json::from_str::<AnalysisConfig>(r#"{"speaker_id":true}"#).is_err());
    }
}
