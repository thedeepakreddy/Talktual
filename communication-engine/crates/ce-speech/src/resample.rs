//! Mono downmix and a streaming resampler to the adapter's input rate.
//!
//! Anti-aliasing FIR (31-tap Hann-windowed sinc, cutoff 0.45 × the lower
//! rate) followed by linear interpolation at the exact rational position
//! `n · from / to`. The FIR is applied symmetrically around each input
//! position (zero history at the start), so **output sample n corresponds
//! exactly to input position n · from / to** — no group delay is introduced
//! into timestamps; output lags input by at most 16 input samples of
//! look-ahead. Adequate for speech recognition front ends; not a mastering
//! resampler.

use std::collections::VecDeque;
use std::f32::consts::PI;

const TAPS: usize = 31;
const HALF: i64 = (TAPS / 2) as i64;

/// Average interleaved channels into mono, appending to `out`.
pub fn downmix(samples: &[f32], channels: usize, out: &mut Vec<f32>) {
    let ch = channels.max(1);
    let inv = 1.0 / ch as f32;
    for frame in samples.chunks_exact(ch) {
        out.push(
            frame
                .iter()
                .map(|v| if v.is_finite() { *v } else { 0.0 })
                .sum::<f32>()
                * inv,
        );
    }
}

/// Streaming resampler.
#[derive(Clone, Debug)]
pub struct Resampler {
    from: u32,
    to: u32,
    taps: Vec<f32>,
    /// Input samples from absolute index `base`.
    hist: VecDeque<f32>,
    base: i64,
    pushed: i64,
    produced: u64,
}

impl Resampler {
    /// From `from` Hz to `to` Hz.
    pub fn new(from: u32, to: u32) -> Self {
        let fc = 0.45 * f64::from(from.min(to)) / f64::from(from);
        let mut taps: Vec<f32> = (0..TAPS)
            .map(|k| {
                let n = k as f64 - HALF as f64;
                let sinc = if n == 0.0 {
                    2.0 * fc
                } else {
                    (2.0 * std::f64::consts::PI * fc * n).sin() / (std::f64::consts::PI * n)
                };
                let w = 0.5 - 0.5 * (2.0 * PI * k as f32 / (TAPS - 1) as f32).cos();
                (sinc * f64::from(w)) as f32
            })
            .collect();
        let sum: f32 = taps.iter().sum();
        for t in &mut taps {
            *t /= sum;
        }
        Self {
            from,
            to,
            taps,
            hist: VecDeque::new(),
            base: 0,
            pushed: 0,
            produced: 0,
        }
    }

    /// Output samples produced so far.
    pub fn produced(&self) -> u64 {
        self.produced
    }

    fn at(&self, i: i64) -> f32 {
        if i < self.base || i >= self.pushed {
            return 0.0;
        }
        self.hist[(i - self.base) as usize]
    }

    fn filtered(&self, i: i64) -> f32 {
        self.taps
            .iter()
            .enumerate()
            .map(|(k, t)| t * self.at(i - HALF + k as i64))
            .sum()
    }

    /// Push input; append every output sample that is fully determined.
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.from == self.to {
            out.extend_from_slice(input);
            self.produced += input.len() as u64;
            return;
        }
        self.hist.extend(input.iter().copied());
        self.pushed += input.len() as i64;
        loop {
            let num = u128::from(self.produced) * u128::from(self.from);
            let i0 = (num / u128::from(self.to)) as i64;
            let frac = (num % u128::from(self.to)) as f32 / self.to as f32;
            // Needs input up to i0 + 1 + HALF.
            if i0 + 1 + HALF >= self.pushed {
                break;
            }
            let y0 = self.filtered(i0);
            let y1 = if frac == 0.0 {
                y0
            } else {
                self.filtered(i0 + 1)
            };
            out.push(y0 + frac * (y1 - y0));
            self.produced += 1;
        }
        // Keep what the next output still needs.
        let num = u128::from(self.produced) * u128::from(self.from);
        let next = (num / u128::from(self.to)) as i64;
        while self.base < next - HALF - 1 && !self.hist.is_empty() {
            self.hist.pop_front();
            self.base += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(f: f32, rate: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * PI * f * i as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn identity_and_exact_lengths() {
        let mut r = Resampler::new(16_000, 16_000);
        let mut out = Vec::new();
        r.push(&[0.1, 0.2], &mut out);
        assert_eq!(out, vec![0.1, 0.2]);
        for (from, to) in [
            (48_000, 16_000),
            (44_100, 16_000),
            (8_000, 16_000),
            (96_000, 16_000),
        ] {
            let mut r = Resampler::new(from, to);
            let mut out = Vec::new();
            let x = sine(440.0, from, from as usize);
            for c in x.chunks(333) {
                r.push(c, &mut out);
            }
            // Look-ahead is 16 input samples (more output samples when upsampling).
            let expected = to as usize;
            assert!(
                out.len() <= expected && expected - out.len() <= (17 * to / from).max(17) as usize,
                "{from}->{to}: {}",
                out.len()
            );
            assert_eq!(r.produced(), out.len() as u64);
        }
    }

    #[test]
    fn passband_is_kept_and_aliases_are_suppressed() {
        let rms = |v: &[f32]| (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt();
        let run = |f: f32| {
            let mut r = Resampler::new(48_000, 16_000);
            let mut out = Vec::new();
            r.push(&sine(f, 48_000, 48_000), &mut out);
            rms(&out[1_000..15_000])
        };
        assert!(
            (run(1_000.0) - 0.707).abs() < 0.03,
            "1 kHz passes: {}",
            run(1_000.0)
        );
        assert!(
            run(12_000.0) < 0.05,
            "12 kHz (above the new Nyquist) is attenuated: {}",
            run(12_000.0)
        );
        // Timing: an impulse at input n lands at output n·to/from.
        let mut imp = vec![0.0f32; 4_800];
        imp[3_000] = 1.0;
        let mut r = Resampler::new(48_000, 16_000);
        let mut out = Vec::new();
        r.push(&imp, &mut out);
        let peak = out
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert_eq!(peak, 1_000, "no group delay in the time mapping");
        let mut mono = Vec::new();
        downmix(&[1.0, 0.0, f32::NAN, 0.5], 2, &mut mono);
        assert_eq!(mono, vec![0.5, 0.25]);
    }
}
