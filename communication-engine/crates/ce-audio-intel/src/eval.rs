//! Evaluation and replay.
//!
//! * [`read_wav`] — a minimal PCM16 / float32 WAV reader for fixtures.
//! * [`Scenario`] — a deterministic stream built from fixture utterances
//!   (leading silence, pauses between utterances, trailing silence) under a
//!   [`Condition`] (clean, white/pink noise or music at an SNR, clipping, low
//!   level) and a device format (rate, channels, block size).
//! * Reference labels come from the **clean** utterances by energy trimming
//!   (frame level within 40 dB of the utterance peak and above -55 dBFS,
//!   gaps < 200 ms bridged, regions < 50 ms removed). They are oracle-style
//!   acoustic labels, not human annotations — see the evaluation report.
//! * [`replay`] runs a [`StreamAnalyzer`] over a scenario and returns every
//!   event plus timing; [`score`] computes the metrics in [`Metrics`].
//!
//! Everything is deterministic except the measured processing time.

use std::path::{Path, PathBuf};
use std::time::Instant;

use ce_core::ids::AudioStreamId;
use ce_core::intel::{
    ActivityState, AnalysisConfig, AnalysisEndReason, AnalysisEvent, QualityReason,
    SegmentCloseReason, Usability,
};
use ce_core::{CeError, ErrorCategory};

use crate::analyzer::StreamAnalyzer;
use crate::synth;
use crate::vad::VoiceActivityDetector;

/// Label/metric grid: 10 ms.
pub const GRID_MS: u32 = 10;
/// The fixture rate.
pub const FIXTURE_RATE: u32 = 16_000;

/// Read a mono or multi-channel PCM16 / float32 WAV into interleaved f32.
pub fn read_wav(path: &Path) -> Result<(u32, u16, Vec<f32>), CeError> {
    let bad = |m: &'static str| {
        CeError::new(ErrorCategory::InvalidInput, "eval.bad_wav", m)
            .with_context("path", path.display().to_string())
    };
    let bytes = std::fs::read(path).map_err(|e| bad("cannot read file").with_source(e))?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad("not a RIFF/WAVE file"));
    }
    let (mut fmt, mut data) = (None, None);
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let len =
            u32::from_le_bytes([bytes[i + 4], bytes[i + 5], bytes[i + 6], bytes[i + 7]]) as usize;
        let body = &bytes[i + 8..(i + 8 + len).min(bytes.len())];
        match id {
            b"fmt " => fmt = Some(body),
            b"data" => data = Some(body),
            _ => {}
        }
        i += 8 + len + (len & 1);
    }
    let (fmt, data) = (
        fmt.ok_or_else(|| bad("no fmt chunk"))?,
        data.ok_or_else(|| bad("no data chunk"))?,
    );
    if fmt.len() < 16 {
        return Err(bad("short fmt chunk"));
    }
    let tag = u16::from_le_bytes([fmt[0], fmt[1]]);
    let channels = u16::from_le_bytes([fmt[2], fmt[3]]);
    let rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
    let bits = u16::from_le_bytes([fmt[14], fmt[15]]);
    let samples = match (tag, bits) {
        (1, 16) => data
            .chunks_exact(2)
            .map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32768.0)
            .collect(),
        (3, 32) => data
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
        _ => return Err(bad("only PCM16 and float32 WAV are supported")),
    };
    Ok((rate, channels.max(1), samples))
}

/// Directory of the bundled speech fixtures.
pub fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/speech")
}

/// Fixture split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Split {
    /// Used to set parameters.
    Dev,
    /// Held out; reported only.
    Test,
}

/// Load a split's utterances (sorted by file name), 16 kHz mono.
pub fn load_split(split: Split) -> Result<Vec<(String, Vec<f32>)>, CeError> {
    let prefix = match split {
        Split::Dev => "dev_",
        Split::Test => "test_",
    };
    let dir = fixture_dir();
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|e| {
            CeError::new(
                ErrorCategory::InvalidInput,
                "eval.no_fixtures",
                "fixture directory missing",
            )
            .with_source(e)
        })?
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .filter(|n| n.starts_with(prefix) && n.ends_with(".wav"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|n| {
            let (rate, ch, s) = read_wav(&dir.join(&n))?;
            if rate != FIXTURE_RATE || ch != 1 {
                return Err(CeError::new(
                    ErrorCategory::InvalidInput,
                    "eval.bad_fixture",
                    "fixtures must be 16 kHz mono",
                ));
            }
            Ok((n, s))
        })
        .collect()
}

/// Acoustic condition applied to the whole stream.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Condition {
    /// Speech plus a -70 dBFS white floor.
    Clean,
    /// White noise at an SNR (dB, relative to active speech RMS).
    White(f32),
    /// Pink noise at an SNR.
    Pink(f32),
    /// A sustained chord ("music") at an SNR.
    Music(f32),
    /// Speech amplified by this many dB and hard-clipped at full scale.
    Clipped(f32),
    /// Speech attenuated by this many dB.
    Quiet(f32),
}

impl Condition {
    /// Short label.
    pub fn label(&self) -> String {
        match self {
            Condition::Clean => "clean".into(),
            Condition::White(s) => format!("white {s} dB"),
            Condition::Pink(s) => format!("pink {s} dB"),
            Condition::Music(s) => format!("music {s} dB"),
            Condition::Clipped(g) => format!("clipped +{g} dB"),
            Condition::Quiet(g) => format!("quiet -{g} dB"),
        }
    }
}

/// A device format variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    /// Rate.
    pub rate: u32,
    /// Channels (the signal is duplicated, second channel at -6 dB).
    pub channels: u16,
    /// Block length in frames.
    pub block_frames: usize,
}

impl Format {
    /// 16 kHz mono, 10 ms blocks.
    pub const BASE: Format = Format {
        rate: 16_000,
        channels: 1,
        block_frames: 160,
    };
}

/// A reference region on the 10 ms grid: [start, end).
pub type Region = (u32, u32);

/// A built stream.
#[derive(Clone, Debug)]
pub struct Scenario {
    /// Description.
    pub name: String,
    /// Device format.
    pub format: Format,
    /// Interleaved samples at `format.rate`.
    pub samples: Vec<f32>,
    /// Reference activity regions.
    pub reference: Vec<Region>,
    /// Utterance ends on the grid (each followed by ≥ 1 s of non-speech).
    pub utterance_ends: Vec<u32>,
    /// Per input utterance: [first labelled start, last labelled end) on
    /// the grid (`None` if nothing was labelled), in input order.
    pub utterance_spans: Vec<Option<Region>>,
    /// Condition.
    pub condition: Condition,
}

fn grid_levels(s: &[f32]) -> Vec<f32> {
    let n = (FIXTURE_RATE * GRID_MS / 1000) as usize;
    s.chunks(n)
        .map(|c| crate::dsp::power_db(c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32))
        .collect()
}

/// Reference regions of one clean utterance (grid units, utterance-relative).
pub fn label_utterance(s: &[f32]) -> Vec<Region> {
    let lv = grid_levels(s);
    let peak = lv.iter().copied().fold(-120.0f32, f32::max);
    let on: Vec<bool> = lv.iter().map(|l| *l > peak - 40.0 && *l > -55.0).collect();
    let mut regions: Vec<Region> = Vec::new();
    let mut i = 0;
    while i < on.len() {
        if on[i] {
            let start = i;
            while i < on.len() && on[i] {
                i += 1;
            }
            regions.push((start as u32, i as u32));
        } else {
            i += 1;
        }
    }
    // Bridge gaps < 200 ms, then drop regions < 50 ms.
    let mut merged: Vec<Region> = Vec::new();
    for r in regions {
        match merged.last_mut() {
            Some(last) if r.0 - last.1 < 200 / GRID_MS => last.1 = r.1,
            _ => merged.push(r),
        }
    }
    merged.retain(|r| r.1 - r.0 >= 50 / GRID_MS);
    merged
}

fn rms_over(s: &[f32], regions: &[Region]) -> f32 {
    let n = (FIXTURE_RATE * GRID_MS / 1000) as usize;
    let (mut sum, mut cnt) = (0.0f64, 0usize);
    for r in regions {
        let a = (r.0 as usize * n).min(s.len());
        let b = (r.1 as usize * n).min(s.len());
        sum += s[a..b].iter().map(|v| f64::from(v * v)).sum::<f64>();
        cnt += b - a;
    }
    if cnt == 0 {
        0.0
    } else {
        (sum / cnt as f64).sqrt() as f32
    }
}

/// Build a scenario: 1.2 s lead, utterances separated by 1.0 s, 1.5 s tail.
pub fn build(
    name: &str,
    utterances: &[(String, Vec<f32>)],
    condition: Condition,
    format: Format,
) -> Scenario {
    let grid = (FIXTURE_RATE * GRID_MS / 1000) as usize;
    let lead = vec![0.0f32; FIXTURE_RATE as usize * 12 / 10];
    let gap = vec![0.0f32; FIXTURE_RATE as usize];
    let tail = vec![0.0f32; FIXTURE_RATE as usize * 15 / 10];
    let mut speech = lead;
    let mut reference = Vec::new();
    let mut ends = Vec::new();
    let mut spans = Vec::new();
    for (k, (_, u)) in utterances.iter().enumerate() {
        if k > 0 {
            speech.extend_from_slice(&gap);
        }
        // Pad each utterance to the grid so labels align exactly.
        let mut u = u.clone();
        u.resize(u.len().div_ceil(grid) * grid, 0.0);
        let off = (speech.len() / grid) as u32;
        let regs = label_utterance(&u);
        if let Some(last) = regs.last() {
            ends.push(off + last.1);
        }
        spans.push(
            regs.first()
                .zip(regs.last())
                .map(|(a, b)| (a.0 + off, b.1 + off)),
        );
        reference.extend(regs.iter().map(|r| (r.0 + off, r.1 + off)));
        speech.extend_from_slice(&u);
    }
    speech.extend_from_slice(&tail);
    let speech_rms = rms_over(&speech, &reference).max(1e-6);
    let n = speech.len();
    let mut mix = speech.clone();
    let noise_at = |mut v: Vec<f32>, snr: f32| {
        synth::scale_to_rms(&mut v, speech_rms / 10f32.powf(snr / 20.0));
        v
    };
    match condition {
        Condition::Clean => {}
        Condition::White(snr) => {
            synth::mix_into(&mut mix, &noise_at(synth::noise(11, n, 1.0), snr))
        }
        Condition::Pink(snr) => synth::mix_into(&mut mix, &noise_at(synth::pink(12, n, 1.0), snr)),
        Condition::Music(snr) => {
            synth::mix_into(&mut mix, &noise_at(synth::chord(FIXTURE_RATE, 1.0, n), snr))
        }
        Condition::Clipped(gain) => {
            let g = 10f32.powf(gain / 20.0);
            for v in &mut mix {
                *v *= g;
            }
            synth::clip(&mut mix, 1.0);
        }
        Condition::Quiet(att) => {
            let g = 10f32.powf(-att / 20.0);
            for v in &mut mix {
                *v *= g;
            }
        }
    }
    // A realistic analog floor (-70 dBFS) so silence is never digital zero.
    synth::mix_into(
        &mut mix,
        &noise_at(synth::noise(13, n, 1.0), 20.0 * (speech_rms.log10()) + 70.0),
    );
    let resampled = synth::resample_linear(&mix, FIXTURE_RATE, format.rate);
    let samples = if format.channels > 1 {
        synth::to_channels(&resampled, format.channels, &[1.0, 0.5])
    } else {
        resampled
    };
    Scenario {
        name: name.into(),
        format,
        samples,
        reference,
        utterance_ends: ends,
        utterance_spans: spans,
        condition,
    }
}

/// A replay result.
#[derive(Clone, Debug)]
pub struct Replay {
    /// Every event, in order.
    pub events: Vec<AnalysisEvent>,
    /// Processing time, seconds.
    pub processing_s: f64,
    /// Audio duration, seconds.
    pub audio_s: f64,
    /// Hop length in frames at the scenario rate.
    pub hop_frames: u64,
}

/// Run `cfg` (and optionally a detector) over `scenario`. Load is fed as 0
/// so the result is deterministic.
pub fn replay(
    scenario: &Scenario,
    cfg: &AnalysisConfig,
    detector: Option<Box<dyn VoiceActivityDetector>>,
) -> Result<Replay, CeError> {
    let stream = AudioStreamId::from_uuid(uuid::Uuid::from_u128(0xe7a1));
    let mut a = match detector {
        Some(d) => StreamAnalyzer::with_detector(stream, cfg.clone(), d)?,
        None => StreamAnalyzer::new(stream, cfg.clone())?,
    };
    let mut events = Vec::new();
    a.start(&mut events);
    let blocks = synth::blocks(
        &scenario.samples,
        scenario.format.rate,
        scenario.format.channels,
        scenario.format.block_frames,
    );
    let t0 = Instant::now();
    for b in &blocks {
        a.push_block(b, &mut events)?;
        a.note_load(0.0, 0, &mut events);
    }
    let processing_s = t0.elapsed().as_secs_f64();
    a.finish(AnalysisEndReason::StreamClosed, &mut events);
    let frames = scenario.samples.len() / scenario.format.channels as usize;
    Ok(Replay {
        events,
        processing_s,
        audio_s: frames as f64 / f64::from(scenario.format.rate),
        hop_frames: u64::from(scenario.format.rate * cfg.hop_ms / 1000),
    })
}

/// Evaluation metrics for one scenario.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metrics {
    /// Frame precision.
    pub precision: f32,
    /// Frame recall.
    pub recall: f32,
    /// Frame F1.
    pub f1: f32,
    /// Predicted regions not touching any reference region (±100 ms), per hour.
    pub false_activations_per_hour: f32,
    /// Reference-active time predicted inactive, ms.
    pub missed_ms: u32,
    /// Mean |onset error| for detected regions, ms.
    pub onset_error_ms: f32,
    /// Mean |offset error| for detected regions, ms.
    pub offset_error_ms: f32,
    /// Mean onset decision latency (boundary → decision), ms.
    pub onset_latency_ms: f32,
    /// Mean predicted regions per detected reference region (1 = ideal).
    pub fragmentation: f32,
    /// Predicted regions covering more than one reference region.
    pub merges: u32,
    /// Utterance ends followed by a `SilencePolicy` segment close within 300 ms.
    pub endpoint_hits: u32,
    /// Utterance ends.
    pub endpoints: u32,
    /// `SilencePolicy` closes not within 300 ms of an utterance end.
    pub false_endpoints: u32,
    /// Mean delay from utterance end to the segment-close decision, ms.
    pub endpoint_delay_ms: f32,
    /// Segments closed.
    pub segments: u32,
    /// Usability of the last quality window reported.
    pub final_usability: Option<Usability>,
    /// Reasons seen in any quality report.
    pub quality_reasons: Vec<QualityReason>,
    /// Processing time / audio time.
    pub real_time_factor: f32,
}

fn to_grid(frame: u64, hop_frames: u64, hop_ms: u32) -> u32 {
    ((frame / hop_frames.max(1)) as u32) * hop_ms / GRID_MS
}

/// Predicted activity regions (grid units) from `ActivityChanged` events.
pub fn predicted_regions(r: &Replay, hop_ms: u32) -> Vec<Region> {
    let mut out = Vec::new();
    let mut start = None;
    for e in &r.events {
        if let AnalysisEvent::ActivityChanged { meta, state, .. } = e {
            let at = to_grid(meta.frame_range.0, r.hop_frames, hop_ms);
            match state {
                ActivityState::Active => start = Some(at),
                ActivityState::Inactive => {
                    if let Some(s) = start.take() {
                        out.push((s, at.max(s)));
                    }
                }
            }
        }
    }
    out
}

fn overlaps(a: Region, b: Region, tol: u32) -> bool {
    a.0 < b.1 + tol && b.0 < a.1 + tol
}

/// Score a replay against its scenario.
pub fn score(s: &Scenario, r: &Replay, cfg: &AnalysisConfig) -> Metrics {
    let hop_ms = cfg.hop_ms;
    let pred = predicted_regions(r, hop_ms);
    let total = (s.samples.len() as u64 / s.format.channels as u64 * 1000
        / u64::from(s.format.rate)) as u32
        / GRID_MS;
    let mut refm = vec![false; total as usize + 1];
    let mut prm = vec![false; total as usize + 1];
    for (rs, m) in [(&s.reference, &mut refm), (&pred, &mut prm)] {
        for g in rs.iter() {
            for i in g.0..g.1.min(total) {
                m[i as usize] = true;
            }
        }
    }
    let tp = refm.iter().zip(&prm).filter(|(a, b)| **a && **b).count() as f32;
    let fp = refm.iter().zip(&prm).filter(|(a, b)| !**a && **b).count() as f32;
    let fneg = refm.iter().zip(&prm).filter(|(a, b)| **a && !**b).count() as f32;
    let precision = if tp + fp > 0.0 { tp / (tp + fp) } else { 0.0 };
    let recall = if tp + fneg > 0.0 {
        tp / (tp + fneg)
    } else {
        0.0
    };
    let f1 = if precision + recall > 0.0 {
        2.0 * precision * recall / (precision + recall)
    } else {
        0.0
    };
    let tol = 100 / GRID_MS;
    let false_act = pred
        .iter()
        .filter(|p| !s.reference.iter().any(|g| overlaps(**p, *g, tol)))
        .count() as f32;
    let hours = r.audio_s as f32 / 3600.0;
    let (mut on_err, mut off_err, mut frag, mut detected) = (0.0f32, 0.0f32, 0.0f32, 0u32);
    for g in &s.reference {
        let hits: Vec<&Region> = pred.iter().filter(|p| overlaps(**p, *g, 0)).collect();
        if let (Some(first), Some(last)) = (hits.first(), hits.last()) {
            detected += 1;
            on_err += (first.0 as f32 - g.0 as f32).abs() * GRID_MS as f32;
            off_err += (last.1 as f32 - g.1 as f32).abs() * GRID_MS as f32;
            frag += hits.len() as f32;
        }
    }
    let merges = pred
        .iter()
        .filter(|p| s.reference.iter().filter(|g| overlaps(**p, **g, 0)).count() > 1)
        .count() as u32;
    let lat: Vec<f32> = r
        .events
        .iter()
        .filter_map(|e| match e {
            AnalysisEvent::ActivityChanged {
                state: ActivityState::Active,
                decision_latency_ms,
                ..
            } => Some(*decision_latency_ms as f32),
            _ => None,
        })
        .collect();
    let closes: Vec<u32> = r
        .events
        .iter()
        .filter_map(|e| match e {
            AnalysisEvent::SegmentEnded {
                meta,
                reason: SegmentCloseReason::SilencePolicy,
                ..
            } => Some(to_grid(meta.frame_range.1, r.hop_frames, hop_ms)),
            _ => None,
        })
        .collect();
    let segments = r
        .events
        .iter()
        .filter(|e| matches!(e, AnalysisEvent::SegmentEnded { .. }))
        .count() as u32;
    let near = 300 / GRID_MS;
    let mut hits = 0;
    let mut delay = 0.0f32;
    for end in &s.utterance_ends {
        if let Some(c) = closes.iter().find(|c| c.abs_diff(*end) <= near) {
            hits += 1;
            delay +=
                (*c as f32 - *end as f32) * GRID_MS as f32 + cfg.segment.close_silence_ms as f32;
        }
    }
    let false_endpoints = closes
        .iter()
        .filter(|c| !s.utterance_ends.iter().any(|e| c.abs_diff(*e) <= near))
        .count() as u32;
    let mut reasons: Vec<QualityReason> = Vec::new();
    let mut final_usability = None;
    for e in &r.events {
        if let AnalysisEvent::QualityChanged { metrics, .. } = e {
            final_usability = Some(metrics.usability);
            for q in &metrics.reasons {
                if !reasons.contains(q) {
                    reasons.push(*q);
                }
            }
        }
    }
    reasons.sort();
    let mean = |v: f32, n: u32| if n > 0 { v / n as f32 } else { 0.0 };
    Metrics {
        precision,
        recall,
        f1,
        false_activations_per_hour: if hours > 0.0 { false_act / hours } else { 0.0 },
        missed_ms: fneg as u32 * GRID_MS,
        onset_error_ms: mean(on_err, detected),
        offset_error_ms: mean(off_err, detected),
        onset_latency_ms: if lat.is_empty() {
            0.0
        } else {
            lat.iter().sum::<f32>() / lat.len() as f32
        },
        fragmentation: mean(frag, detected),
        merges,
        endpoint_hits: hits,
        endpoints: s.utterance_ends.len() as u32,
        false_endpoints,
        endpoint_delay_ms: mean(delay, hits),
        segments,
        final_usability,
        quality_reasons: reasons,
        real_time_factor: if r.audio_s > 0.0 {
            (r.processing_s / r.audio_s) as f32
        } else {
            0.0
        },
    }
}

/// A stable digest of an event list (FNV-1a over its JSON-free debug form),
/// for replay determinism checks.
pub fn digest(events: &[AnalysisEvent]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for e in events {
        for b in format!("{e:?}").bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

/// The standard condition set.
pub fn standard_conditions() -> Vec<Condition> {
    vec![
        Condition::Clean,
        Condition::White(20.0),
        Condition::White(10.0),
        Condition::White(5.0),
        Condition::Pink(10.0),
        Condition::Music(10.0),
        Condition::Clipped(20.0),
        Condition::Quiet(30.0),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_reader_and_labels() {
        let (rate, ch, s) = read_wav(&fixture_dir().join("dev_awb_yes.wav")).unwrap();
        assert_eq!((rate, ch), (16_000, 1));
        assert!(s.len() > 10_000);
        let regs = label_utterance(&s);
        assert!(!regs.is_empty());
        assert!(read_wav(Path::new("/nonexistent.wav")).is_err());
    }

    #[test]
    fn scenarios_are_deterministic_and_labelled() {
        let dev = load_split(Split::Dev).unwrap();
        assert_eq!(dev.len(), 4);
        let a = build("dev", &dev, Condition::White(10.0), Format::BASE);
        let b = build("dev", &dev, Condition::White(10.0), Format::BASE);
        assert_eq!(a.samples, b.samples);
        assert_eq!(a.utterance_ends.len(), 4);
        assert!(a.reference.first().unwrap().0 >= 120, "1.2 s lead");
        let st = build(
            "dev",
            &dev,
            Condition::Clean,
            Format {
                rate: 48_000,
                channels: 2,
                block_frames: 480,
            },
        );
        assert_eq!(st.samples.len(), a.samples.len() * 3 * 2);
    }
}
