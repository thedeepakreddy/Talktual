//! Deterministic synthetic signals and block builders for tests, replay and
//! evaluation. No randomness source other than a seeded xorshift.
//!
//! The "speech-like" generator is a harmonic source (pitch 110–190 Hz,
//! slowly gliding) through two formant-like resonances with a 4 Hz syllabic
//! envelope. It is **not** speech; it exists to exercise the pipeline
//! deterministically. Speech performance is measured on the TTS fixtures.

use std::f32::consts::PI;

use ce_core::audio::{
    AudioBlock, AudioFrameMeta, ChannelLayout, SampleFormat, frame::frames_to_duration,
};
use ce_core::ids::{AudioStreamId, CorrelationId, SessionId};
use ce_core::time::MonotonicTime;
use uuid::Uuid;

/// A fixed stream id used by synthetic blocks.
pub const SYNTH_STREAM: AudioStreamId = AudioStreamId::from_uuid(Uuid::from_u128(0x5eed));

/// Build a block with exact sample-clock timing.
pub fn block(
    sequence: u64,
    frame_index: u64,
    rate: u32,
    channels: u16,
    samples: Vec<f32>,
) -> AudioBlock {
    let frames = (samples.len() / channels.max(1) as usize) as u32;
    let ts = MonotonicTime::from_nanos(frames_to_duration(frame_index, rate).as_nanos() as u64);
    AudioBlock {
        meta: AudioFrameMeta {
            stream_id: SYNTH_STREAM,
            session_id: SessionId::from_uuid(Uuid::from_u128(1)),
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(2)),
            sequence,
            frame_index,
            frames,
            sample_rate: rate,
            channels,
            layout: ChannelLayout::from_channels(channels),
            sample_format: SampleFormat::F32,
            timestamp: ts,
            host_time: ts,
            device_time: None,
            discontinuity: None,
        },
        samples,
    }
}

/// Split interleaved samples into consecutive blocks of `frames_per_block`.
pub fn blocks(
    samples: &[f32],
    rate: u32,
    channels: u16,
    frames_per_block: usize,
) -> Vec<AudioBlock> {
    let step = frames_per_block * channels as usize;
    samples
        .chunks(step.max(1))
        .enumerate()
        .map(|(i, c)| {
            block(
                i as u64,
                (i * frames_per_block) as u64,
                rate,
                channels,
                c.to_vec(),
            )
        })
        .collect()
}

/// A sine of `amp` amplitude.
pub fn sine(freq: f32, rate: u32, amp: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| amp * (2.0 * PI * freq * i as f32 / rate as f32).sin())
        .collect()
}

/// Seeded xorshift64*.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    /// Seeded generator (seed 0 is remapped).
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    /// Uniform in [-1, 1).
    pub fn next_f32(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let v = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        ((v >> 40) as f32 / (1u64 << 23) as f32) - 1.0
    }
}

/// Uniform white noise of peak `amp`.
pub fn noise(seed: u64, n: usize, amp: f32) -> Vec<f32> {
    let mut r = Rng::new(seed);
    (0..n).map(|_| amp * r.next_f32()).collect()
}

/// Pink-ish (−3 dB/oct approximation, Paul Kellet's filter) noise.
pub fn pink(seed: u64, n: usize, amp: f32) -> Vec<f32> {
    let mut r = Rng::new(seed);
    let (mut b0, mut b1, mut b2) = (0.0f32, 0.0f32, 0.0f32);
    (0..n)
        .map(|_| {
            let w = r.next_f32();
            b0 = 0.99765 * b0 + w * 0.0990460;
            b1 = 0.96300 * b1 + w * 0.2965164;
            b2 = 0.57000 * b2 + w * 1.0526913;
            amp * 0.25 * (b0 + b1 + b2 + w * 0.1848)
        })
        .collect()
}

/// Steady "music-like" signal: a sustained three-note chord with vibrato.
pub fn chord(rate: u32, amp: f32, n: usize) -> Vec<f32> {
    let f = [261.6f32, 329.6, 392.0];
    (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let vib = 1.0 + 0.004 * (2.0 * PI * 5.0 * t).sin();
            amp / 3.0
                * f.iter()
                    .map(|fr| {
                        (2.0 * PI * fr * vib * t).sin() + 0.3 * (4.0 * PI * fr * vib * t).sin()
                    })
                    .sum::<f32>()
        })
        .collect()
}

/// Deterministic speech-like signal of peak about `amp` (see module docs).
pub fn speechlike(seed: u64, rate: u32, amp: f32, n: usize) -> Vec<f32> {
    let mut r = Rng::new(seed);
    let base = 110.0 + 80.0 * (r.next_f32() * 0.5 + 0.5);
    let mut phase = 0.0f32;
    // Two resonators (formant-like) implemented as 2-pole filters.
    let res = |fc: f32, bw: f32| {
        let rr = (-PI * bw / rate as f32).exp();
        let th = 2.0 * PI * fc / rate as f32;
        (2.0 * rr * th.cos(), -rr * rr, 1.0 - rr)
    };
    let (a1, a2, g1) = res(700.0, 130.0);
    let (b1, b2, g2) = res(1_500.0, 200.0);
    let (mut y1, mut y2, mut z1, mut z2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut out: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let f0 = base * (1.0 + 0.1 * (2.0 * PI * 0.7 * t).sin());
            phase += f0 / rate as f32;
            phase -= phase.floor();
            // Band-limited-ish glottal pulse train: sum of 12 harmonics, 1/k.
            let src: f32 = (1..=12)
                .filter(|k| *k as f32 * f0 < 0.45 * rate as f32)
                .map(|k| (2.0 * PI * k as f32 * phase).sin() / k as f32)
                .sum();
            let y = g1 * src + a1 * y1 + a2 * y2;
            y2 = y1;
            y1 = y;
            let z = g2 * src + b1 * z1 + b2 * z2;
            z2 = z1;
            z1 = z;
            // Syllabic envelope: 4 Hz, never fully closed.
            let env = 0.55 + 0.45 * (2.0 * PI * 4.0 * t).sin();
            env * (y + 0.6 * z)
        })
        .collect();
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-9);
    for v in &mut out {
        *v *= amp / peak;
    }
    out
}

/// Add `b` into `a` (lengths may differ; `a` keeps its length).
pub fn mix_into(a: &mut [f32], b: &[f32]) {
    for (x, y) in a.iter_mut().zip(b) {
        *x += y;
    }
}

/// Scale so the RMS equals `rms`.
pub fn scale_to_rms(s: &mut [f32], rms: f32) {
    let cur = (s.iter().map(|v| v * v).sum::<f32>() / s.len().max(1) as f32).sqrt();
    if cur > 0.0 {
        for v in s.iter_mut() {
            *v *= rms / cur;
        }
    }
}

/// Hard-clip to ±`limit`.
pub fn clip(s: &mut [f32], limit: f32) {
    for v in s.iter_mut() {
        *v = v.clamp(-limit, limit);
    }
}

/// Mono → interleaved `channels`, with per-channel gains (missing gains = 1).
pub fn to_channels(mono: &[f32], channels: u16, gains: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(mono.len() * channels as usize);
    for v in mono {
        for c in 0..channels as usize {
            out.push(v * gains.get(c).copied().unwrap_or(1.0));
        }
    }
    out
}

/// Linear-interpolation resampler (fixtures to other device rates in tests;
/// not a production resampler).
pub fn resample_linear(s: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || s.is_empty() {
        return s.to_vec();
    }
    let n = (s.len() as u64 * to as u64 / from as u64) as usize;
    (0..n)
        .map(|i| {
            let x = i as f64 * from as f64 / to as f64;
            let k = x.floor() as usize;
            let fr = (x - k as f64) as f32;
            let a = s[k.min(s.len() - 1)];
            let b = s[(k + 1).min(s.len() - 1)];
            a + fr * (b - a)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generators_are_deterministic_and_bounded() {
        assert_eq!(noise(3, 100, 1.0), noise(3, 100, 1.0));
        assert_ne!(noise(3, 100, 1.0), noise(4, 100, 1.0));
        assert!(noise(9, 10_000, 0.5).iter().all(|v| v.abs() <= 0.5));
        let s = speechlike(1, 16_000, 0.3, 16_000);
        assert!((s.iter().fold(0.0f32, |m, v| m.max(v.abs())) - 0.3).abs() < 1e-4);
        let b = blocks(&to_channels(&s, 2, &[1.0, 0.5]), 16_000, 2, 160);
        assert_eq!(b.len(), 100);
        assert_eq!(b[3].meta.frame_index, 480);
        assert_eq!(b[3].meta.timestamp.as_nanos(), 30_000_000);
        assert_eq!(resample_linear(&s, 16_000, 48_000).len(), 48_000);
    }
}
