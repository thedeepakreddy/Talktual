//! Small, allocation-free (after construction) DSP primitives: an
//! in-place radix-2 FFT, a Hann window and level conversions.
//!
//! Hand-written rather than a dependency (ADR-0014): one transform size per
//! stream, power-of-two only, tested against a direct DFT.

use std::f32::consts::PI;

/// Floor used to avoid `log10(0)`: -120 dB.
pub const LEVEL_FLOOR: f32 = 1e-12;

/// Mean-square power → dB (10·log10), floored at -120 dB.
pub fn power_db(mean_square: f32) -> f32 {
    10.0 * mean_square.max(LEVEL_FLOOR).log10()
}

/// Magnitude → dB (20·log10), floored at -120 dB.
pub fn magnitude_db(magnitude: f32) -> f32 {
    20.0 * magnitude.max(1e-6).log10()
}

/// An in-place iterative radix-2 complex FFT of fixed size.
#[derive(Debug, Clone)]
pub struct Fft {
    n: usize,
    cos: Vec<f32>,
    sin: Vec<f32>,
    bitrev: Vec<usize>,
}

impl Fft {
    /// A transform of size `n` (power of two, ≥ 2).
    pub fn new(n: usize) -> Self {
        assert!(
            n.is_power_of_two() && n >= 2,
            "FFT size must be a power of two"
        );
        let bits = n.trailing_zeros();
        let bitrev = (0..n)
            .map(|i| i.reverse_bits() >> (usize::BITS - bits))
            .collect();
        let (cos, sin) = (0..n / 2)
            .map(|k| {
                let a = -2.0 * PI * k as f32 / n as f32;
                (a.cos(), a.sin())
            })
            .unzip();
        Self {
            n,
            cos,
            sin,
            bitrev,
        }
    }

    /// Size.
    pub fn len(&self) -> usize {
        self.n
    }

    /// Always false (size ≥ 2).
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Forward transform in place (`re`, `im` of length `n`).
    pub fn forward(&self, re: &mut [f32], im: &mut [f32]) {
        let n = self.n;
        for i in 0..n {
            let j = self.bitrev[i];
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let step = n / len;
            for start in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let (wr, wi) = (self.cos[k * step], self.sin[k * step]);
                    let a = start + k;
                    let b = a + len / 2;
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len *= 2;
        }
    }

    /// Inverse transform in place, scaled by 1/n.
    pub fn inverse(&self, re: &mut [f32], im: &mut [f32]) {
        for v in im.iter_mut() {
            *v = -*v;
        }
        self.forward(re, im);
        let s = 1.0 / self.n as f32;
        for (r, i) in re.iter_mut().zip(im.iter_mut()) {
            *r *= s;
            *i = -*i * s;
        }
    }
}

/// A periodic Hann window of length `n`.
pub fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_matches_a_direct_dft_and_inverts() {
        let n = 64;
        let x: Vec<f32> = (0..n).map(|i| ((i * 7 % 13) as f32 - 6.0) / 6.0).collect();
        let (mut re, mut im) = (x.clone(), vec![0.0; n]);
        let fft = Fft::new(n);
        fft.forward(&mut re, &mut im);
        for k in 0..n {
            let (mut dr, mut di) = (0.0f32, 0.0f32);
            for (t, v) in x.iter().enumerate() {
                let a = -2.0 * PI * (k * t) as f32 / n as f32;
                dr += v * a.cos();
                di += v * a.sin();
            }
            assert!(
                (dr - re[k]).abs() < 1e-3 && (di - im[k]).abs() < 1e-3,
                "bin {k}"
            );
        }
        fft.inverse(&mut re, &mut im);
        for (a, b) in re.iter().zip(&x) {
            assert!((a - b).abs() < 1e-4);
        }
    }

    #[test]
    fn levels_and_window() {
        assert!((power_db(1.0) - 0.0).abs() < 1e-6);
        assert!(
            (power_db(0.5) + 3.0103).abs() < 1e-3,
            "full-scale sine is -3.01 dBFS"
        );
        assert_eq!(power_db(0.0), -120.0);
        assert!((magnitude_db(0.5) + 6.0206).abs() < 1e-3);
        let w = hann(8);
        assert_eq!(w[0], 0.0);
        assert!((w[4] - 1.0).abs() < 1e-6);
    }
}
