//! Input quality and signal-condition indicators.
//!
//! **Quality** is measured over fixed windows (`window_ms`) of hops:
//! level, peak, clipping ratio, DC offset, channel imbalance, non-finite
//! samples, lost frames and the activity ratio, plus a noise floor (10th
//! percentile of *inactive* hop levels over the last 3 s) and an SNR
//! estimate (active level − floor). A value that cannot be measured from
//! the data available is `None`, never guessed. Usability is the worst
//! severity among the reasons (thresholds documented on
//! [`QualityReason`]). A change is reported only after two consecutive
//! windows agree (debounce).
//!
//! **Conditions** are a bounded, documented set describing the signal:
//! sustained silence, probable activity, sustained noise, clipping,
//! overload and dropouts. None describes a person.

use std::collections::VecDeque;

use ce_core::intel::{
    ActivityState, ConditionConfig, ConditionEvidence, ConditionKind, QualityMetrics,
    QualityReason, Usability,
};

use crate::dsp::{magnitude_db, power_db};
use crate::framer::{Hop, TRACKED_CHANNELS};

/// Inactive-hop history for the noise floor: 3 s.
const FLOOR_HISTORY_MS: u32 = 3_000;
/// Inactive hops needed before a floor is reported.
const FLOOR_MIN_HOPS: usize = 30;

fn severity(r: QualityReason, m: &QualityMetrics, window_frames: u64) -> Usability {
    match r {
        QualityReason::NoSignal => Usability::Unusable,
        QualityReason::NonFinite => Usability::Poor,
        QualityReason::LowSnr if m.snr_db.is_some_and(|s| s < 6.0) => Usability::Poor,
        QualityReason::Clipping if m.clipping_ratio > 0.005 => Usability::Poor,
        QualityReason::Dropouts if window_frames > 0 && m.lost_frames * 10 > window_frames => {
            Usability::Poor
        }
        _ => Usability::Fair,
    }
}

#[derive(Default, Clone, Debug)]
struct Window {
    hops: u32,
    frames: u64,
    sum_sq: f64,
    active_sum_sq: f64,
    active_hops: u32,
    peak: f32,
    clipped: u64,
    samples: u64,
    non_finite: u64,
    dc_sum: f64,
    channel_sq: [f64; TRACKED_CHANNELS],
    channels: u8,
    lost_frames: u64,
}

/// Rolling quality analysis.
#[derive(Clone, Debug)]
pub struct QualityAnalyzer {
    window_hops: u32,
    window_ms: u32,
    win: Window,
    floor_hist: VecDeque<f32>,
    floor_cap: usize,
    reported: Option<(Usability, Vec<QualityReason>)>,
    candidate: Option<(Usability, Vec<QualityReason>)>,
    last: Option<QualityMetrics>,
}

/// A finished window.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowResult {
    /// Metrics.
    pub metrics: QualityMetrics,
    /// Whether usability/reasons changed (debounced): report it.
    pub changed: bool,
}

impl QualityAnalyzer {
    /// A window of `window_ms` over `hop_ms` hops.
    pub fn new(window_ms: u32, hop_ms: u32) -> Self {
        Self {
            window_hops: (window_ms / hop_ms).max(1),
            window_ms,
            win: Window::default(),
            floor_hist: VecDeque::new(),
            floor_cap: (FLOOR_HISTORY_MS / hop_ms) as usize,
            reported: None,
            candidate: None,
            last: None,
        }
    }

    /// The last finished window.
    pub fn last(&self) -> Option<&QualityMetrics> {
        self.last.as_ref()
    }

    /// Current noise floor, dBFS (10th percentile of inactive levels).
    pub fn noise_floor(&self) -> Option<f32> {
        if self.floor_hist.len() < FLOOR_MIN_HOPS {
            return None;
        }
        let mut v: Vec<f32> = self.floor_hist.iter().copied().collect();
        v.sort_by(f32::total_cmp);
        Some(v[v.len() / 10])
    }

    /// Account frames lost to a discontinuity in the current window, and
    /// forget the floor history (it may no longer describe the input).
    pub fn note_lost(&mut self, frames: u64) {
        self.win.lost_frames += frames;
        self.floor_hist.clear();
    }

    /// Feed one hop. Returns a result when a window completes.
    /// `suppress_levels` withholds floor and SNR (policy decision).
    pub fn push(
        &mut self,
        hop: &Hop,
        state: ActivityState,
        suppress_levels: bool,
    ) -> Option<WindowResult> {
        let f = &hop.features;
        let w = &mut self.win;
        w.hops += 1;
        w.frames += hop.frames as u64;
        w.sum_sq += f.mean_square as f64;
        w.peak = w.peak.max(f.peak);
        w.clipped += f.clipped as u64;
        w.samples += f.samples as u64;
        w.non_finite += f.non_finite as u64;
        w.dc_sum += f.dc_sum as f64;
        w.channels = f.channels;
        for (a, b) in w.channel_sq.iter_mut().zip(f.channel_ms) {
            *a += b as f64;
        }
        if state == ActivityState::Active {
            w.active_sum_sq += f.mean_square as f64;
            w.active_hops += 1;
        } else {
            if self.floor_hist.len() == self.floor_cap {
                self.floor_hist.pop_front();
            }
            self.floor_hist.push_back(f.level_db);
        }
        if w.hops < self.window_hops {
            return None;
        }
        let w = std::mem::take(&mut self.win);
        let level = power_db((w.sum_sq / w.hops as f64) as f32);
        let floor = if suppress_levels {
            None
        } else {
            self.noise_floor()
        };
        let active_level =
            (w.active_hops > 0).then(|| power_db((w.active_sum_sq / w.active_hops as f64) as f32));
        let snr = match (active_level, floor) {
            (Some(a), Some(fl)) => Some(a - fl),
            _ => None,
        };
        let channels = (w.channels as usize).min(TRACKED_CHANNELS);
        let imbalance = if channels >= 2 {
            let lv: Vec<f32> = w.channel_sq[..channels]
                .iter()
                .map(|v| power_db((v / w.hops as f64) as f32))
                .collect();
            let max = lv.iter().copied().fold(f32::MIN, f32::max);
            let min = lv.iter().copied().fold(f32::MAX, f32::min);
            (max > -80.0).then_some(max - min)
        } else {
            None
        };
        let clipping_ratio = if w.samples > 0 {
            w.clipped as f32 / w.samples as f32
        } else {
            0.0
        };
        let dc = if w.samples > 0 {
            (w.dc_sum / w.samples as f64) as f32
        } else {
            0.0
        };
        let mut reasons = Vec::new();
        if level < -80.0 {
            reasons.push(QualityReason::NoSignal);
        }
        if active_level.is_some_and(|a| a < -45.0) {
            reasons.push(QualityReason::LowLevel);
        }
        if snr.is_some_and(|s| s < 15.0) {
            reasons.push(QualityReason::LowSnr);
        }
        if clipping_ratio > 0.0005 {
            reasons.push(QualityReason::Clipping);
        }
        if w.lost_frames > 0 {
            reasons.push(QualityReason::Dropouts);
        }
        if imbalance.is_some_and(|d| d > 20.0) {
            reasons.push(QualityReason::ChannelImbalance);
        }
        if dc.abs() > 0.05 {
            reasons.push(QualityReason::DcOffset);
        }
        if w.non_finite > 0 {
            reasons.push(QualityReason::NonFinite);
        }
        reasons.sort();
        let mut metrics = QualityMetrics {
            window_ms: self.window_ms,
            level_dbfs: level,
            peak_dbfs: magnitude_db(w.peak),
            clipping_ratio,
            noise_floor_dbfs: floor,
            snr_db: snr,
            dc_offset: dc,
            channel_imbalance_db: imbalance,
            non_finite_samples: w.non_finite,
            lost_frames: w.lost_frames,
            activity_ratio: w.active_hops as f32 / w.hops as f32,
            usability: Usability::Good,
            reasons: Vec::new(),
        };
        metrics.usability = reasons
            .iter()
            .map(|r| severity(*r, &metrics, w.frames + w.lost_frames))
            .max()
            .unwrap_or(Usability::Good);
        metrics.reasons = reasons;
        let key = (metrics.usability, metrics.reasons.clone());
        let changed = if self.reported.as_ref() == Some(&key) {
            self.candidate = None;
            false
        } else if self.candidate.as_ref() == Some(&key) {
            self.reported = Some(key);
            self.candidate = None;
            true
        } else {
            self.candidate = Some(key);
            false
        };
        self.last = Some(metrics.clone());
        Some(WindowResult { metrics, changed })
    }

    /// Discard the partial window (mode change); keeps the floor.
    pub fn reset_window(&mut self) {
        let lost = self.win.lost_frames;
        self.win = Window {
            lost_frames: lost,
            ..Window::default()
        };
    }
}

/// A condition change.
#[derive(Clone, Debug, PartialEq)]
pub struct ConditionChange {
    /// Condition.
    pub kind: ConditionKind,
    /// Active now.
    pub active: bool,
    /// Evidence.
    pub evidence: ConditionEvidence,
}

fn ev(value: f32, threshold: f32, unit: &str) -> ConditionEvidence {
    ConditionEvidence {
        value,
        threshold,
        unit: unit.into(),
    }
}

/// Tracks the condition set.
#[derive(Clone, Debug)]
pub struct ConditionTracker {
    cfg: ConditionConfig,
    hop_ms: u32,
    active: Vec<ConditionKind>,
    inactive_ms: u32,
    noisy_windows: u32,
    quiet_windows: u32,
}

/// All conditions the tracker can report.
pub const ALL_CONDITIONS: [ConditionKind; 6] = [
    ConditionKind::SustainedSilence,
    ConditionKind::ProbableActivity,
    ConditionKind::SustainedNoise,
    ConditionKind::Clipping,
    ConditionKind::Overload,
    ConditionKind::Dropout,
];

impl ConditionTracker {
    /// A tracker.
    pub fn new(cfg: ConditionConfig, hop_ms: u32) -> Self {
        Self {
            cfg,
            hop_ms,
            active: Vec::new(),
            inactive_ms: 0,
            noisy_windows: 0,
            quiet_windows: 0,
        }
    }

    /// Active conditions.
    pub fn active(&self) -> &[ConditionKind] {
        &self.active
    }

    /// Whether `k` is active.
    pub fn is_active(&self, k: ConditionKind) -> bool {
        self.active.contains(&k)
    }

    fn set(
        &mut self,
        kind: ConditionKind,
        on: bool,
        evidence: ConditionEvidence,
        out: &mut Vec<ConditionChange>,
    ) {
        if on == self.is_active(kind) {
            return;
        }
        if on {
            self.active.push(kind);
        } else {
            self.active.retain(|k| *k != kind);
        }
        out.push(ConditionChange {
            kind,
            active: on,
            evidence,
        });
    }

    /// Per-hop conditions (silence, activity).
    pub fn on_hop(&mut self, state: ActivityState, out: &mut Vec<ConditionChange>) {
        match state {
            ActivityState::Active => {
                self.inactive_ms = 0;
                self.set(
                    ConditionKind::SustainedSilence,
                    false,
                    ev(0.0, self.cfg.sustained_silence_ms as f32, "ms"),
                    out,
                );
                self.set(
                    ConditionKind::ProbableActivity,
                    true,
                    ev(1.0, 1.0, "state"),
                    out,
                );
            }
            ActivityState::Inactive => {
                self.inactive_ms = self.inactive_ms.saturating_add(self.hop_ms);
                self.set(
                    ConditionKind::ProbableActivity,
                    false,
                    ev(0.0, 1.0, "state"),
                    out,
                );
                if self.inactive_ms >= self.cfg.sustained_silence_ms {
                    let v = self.inactive_ms as f32;
                    self.set(
                        ConditionKind::SustainedSilence,
                        true,
                        ev(v, self.cfg.sustained_silence_ms as f32, "ms"),
                        out,
                    );
                }
            }
        }
    }

    /// Per-window conditions.
    pub fn on_window(&mut self, m: &QualityMetrics, out: &mut Vec<ConditionChange>) {
        let c = self.cfg.clone();
        // Sustained noise: inactive windows whose level is above the
        // configured floor, two in a row; cleared after two quiet windows.
        let noisy = m.activity_ratio == 0.0 && m.level_dbfs > c.noise_floor_dbfs;
        if noisy {
            self.noisy_windows += 1;
            self.quiet_windows = 0;
        } else {
            self.quiet_windows += 1;
            self.noisy_windows = 0;
        }
        if self.noisy_windows >= 2 {
            self.set(
                ConditionKind::SustainedNoise,
                true,
                ev(m.level_dbfs, c.noise_floor_dbfs, "dBFS"),
                out,
            );
        } else if self.quiet_windows >= 2 {
            self.set(
                ConditionKind::SustainedNoise,
                false,
                ev(m.level_dbfs, c.noise_floor_dbfs, "dBFS"),
                out,
            );
        }
        // Clipping with hysteresis (clears below half the threshold).
        if m.clipping_ratio > c.clipping_ratio {
            self.set(
                ConditionKind::Clipping,
                true,
                ev(m.clipping_ratio, c.clipping_ratio, "ratio"),
                out,
            );
        } else if m.clipping_ratio < c.clipping_ratio / 2.0 {
            self.set(
                ConditionKind::Clipping,
                false,
                ev(m.clipping_ratio, c.clipping_ratio, "ratio"),
                out,
            );
        }
        // Overload with 3 dB hysteresis.
        if m.level_dbfs > c.overload_dbfs {
            self.set(
                ConditionKind::Overload,
                true,
                ev(m.level_dbfs, c.overload_dbfs, "dBFS"),
                out,
            );
        } else if m.level_dbfs < c.overload_dbfs - 3.0 {
            self.set(
                ConditionKind::Overload,
                false,
                ev(m.level_dbfs, c.overload_dbfs, "dBFS"),
                out,
            );
        }
        // Dropouts: any lost frames in the window; cleared by a clean one.
        self.set(
            ConditionKind::Dropout,
            m.lost_frames > 0,
            ev(m.lost_frames as f32, 0.0, "frames"),
            out,
        );
    }

    /// Clear everything (degraded mode / stop).
    pub fn clear_all(&mut self, out: &mut Vec<ConditionChange>) {
        for k in std::mem::take(&mut self.active) {
            out.push(ConditionChange {
                kind: k,
                active: false,
                evidence: ev(0.0, 0.0, "cleared"),
            });
        }
        self.inactive_ms = 0;
        self.noisy_windows = 0;
        self.quiet_windows = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::hop;

    fn level_hop(i: u64, db: f32) -> Hop {
        let mut h = hop(i);
        h.features.level_db = db;
        h.features.mean_square = 10f32.powf(db / 10.0);
        h.features.peak = h.features.mean_square.sqrt() * 1.4;
        h
    }

    #[test]
    fn floor_snr_and_debounced_usability() {
        let mut q = QualityAnalyzer::new(500, 10);
        let mut results = Vec::new();
        // 1 s quiet (-60), then 1 s active at -20, -60 floor → 40 dB SNR.
        for i in 0..100 {
            results.extend(q.push(&level_hop(i, -60.0), ActivityState::Inactive, false));
        }
        for i in 100..200 {
            results.extend(q.push(&level_hop(i, -20.0), ActivityState::Active, false));
        }
        assert_eq!(results.len(), 4);
        assert!(results[0].metrics.noise_floor_dbfs.is_some());
        let last = &results[3].metrics;
        assert!((last.snr_db.unwrap() - 40.0).abs() < 0.1, "{last:?}");
        assert_eq!(last.usability, Usability::Good);
        assert_eq!(
            results.iter().filter(|r| r.changed).count(),
            1,
            "Good is reported once, after two windows"
        );
        assert!(results[1].changed);
        // Suppression withholds floor and SNR.
        let r = (200..250)
            .find_map(|i| q.push(&level_hop(i, -20.0), ActivityState::Active, true))
            .unwrap();
        assert_eq!((r.metrics.noise_floor_dbfs, r.metrics.snr_db), (None, None));
    }

    #[test]
    fn reasons_cover_silence_clipping_dropouts_and_non_finite() {
        let mut q = QualityAnalyzer::new(100, 10);
        let r = (0..10)
            .find_map(|i| q.push(&level_hop(i, -100.0), ActivityState::Inactive, false))
            .unwrap();
        assert_eq!(
            (r.metrics.usability, r.metrics.reasons.clone()),
            (Usability::Unusable, vec![QualityReason::NoSignal])
        );
        q.note_lost(800);
        let r = (10..20)
            .find_map(|i| {
                let mut h = level_hop(i, -20.0);
                h.features.clipped = 2;
                h.features.non_finite = u32::from(i == 12);
                q.push(&h, ActivityState::Active, false)
            })
            .unwrap();
        assert_eq!(
            r.metrics.reasons,
            vec![
                QualityReason::Clipping,
                QualityReason::Dropouts,
                QualityReason::NonFinite
            ]
        );
        assert_eq!(r.metrics.usability, Usability::Poor);
        assert_eq!(r.metrics.lost_frames, 800);
        assert!(
            r.metrics.snr_db.is_none(),
            "floor history was reset by the gap"
        );
    }

    #[test]
    fn conditions_have_hysteresis_and_clear() {
        let mut c = ConditionTracker::new(ConditionConfig::default(), 10);
        let mut out = Vec::new();
        for _ in 0..200 {
            c.on_hop(ActivityState::Inactive, &mut out);
        }
        assert_eq!(out.len(), 1);
        assert_eq!(
            (out[0].kind, out[0].active),
            (ConditionKind::SustainedSilence, true)
        );
        out.clear();
        c.on_hop(ActivityState::Active, &mut out);
        assert_eq!(
            out.iter().map(|o| (o.kind, o.active)).collect::<Vec<_>>(),
            vec![
                (ConditionKind::SustainedSilence, false),
                (ConditionKind::ProbableActivity, true)
            ]
        );
        let mut m = QualityAnalyzer::new(100, 10);
        let noisy = (0..10)
            .find_map(|i| m.push(&level_hop(i, -30.0), ActivityState::Inactive, false))
            .unwrap()
            .metrics;
        out.clear();
        c.on_window(&noisy, &mut out);
        assert!(out.is_empty(), "one noisy window is not sustained");
        c.on_window(&noisy, &mut out);
        assert_eq!(out[0].kind, ConditionKind::SustainedNoise);
        let mut loud = noisy.clone();
        loud.level_dbfs = -3.0;
        loud.activity_ratio = 1.0;
        loud.clipping_ratio = 0.01;
        out.clear();
        c.on_window(&loud, &mut out);
        let kinds: Vec<_> = out.iter().filter(|o| o.active).map(|o| o.kind).collect();
        assert_eq!(
            kinds,
            vec![ConditionKind::Clipping, ConditionKind::Overload]
        );
        loud.level_dbfs = -7.0; // inside the 3 dB hysteresis
        loud.clipping_ratio = 0.0007; // inside the clipping hysteresis
        out.clear();
        c.on_window(&loud, &mut out);
        assert!(
            !out.iter()
                .any(|o| o.kind == ConditionKind::Overload || o.kind == ConditionKind::Clipping)
        );
        out.clear();
        c.clear_all(&mut out);
        assert!(out.iter().all(|o| !o.active) && c.active().is_empty());
    }
}
