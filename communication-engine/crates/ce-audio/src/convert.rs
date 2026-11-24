//! Sample conversion between native formats and CE's normalized `f32`.
//!
//! Integer ↔ float uses the conventional power-of-two scale: a signed
//! N-bit sample `s` maps to `s / 2^(N-1)`, so `MIN` maps to exactly -1.0 and
//! `MAX` to slightly below +1.0; unsigned formats are offset by `2^(N-1)`
//! first. Float → integer multiplies by `2^(N-1)`, rounds half away from
//! zero and clamps to the integer range (+1.0 therefore clips to `MAX`).
//! Integer → `f32` → integer is lossless for 8- and 16-bit formats. For
//! 32-bit formats the normalized `f32` keeps 24 bits of precision, so a
//! round trip can differ by up to 2^7 = 128 LSB (about -138 dBFS); this is
//! inherent to an `f32` normalization contract (ADR-0012). Non-finite floats
//! (NaN, ±∞) are replaced by silence.
//!
//! These functions are allocation-free and branch-light: safe to call from
//! a device callback.

use ce_core::audio::SampleFormat;

/// A native sample type CE converts to and from `f32`.
pub trait RawSample: Copy + Send + 'static {
    /// The matching format tag.
    const FORMAT: SampleFormat;
    /// The silent value.
    const SILENCE: Self;
    /// Convert to normalized `f32`.
    fn to_f32(self) -> f32;
    /// Convert from normalized `f32` (clamped, rounded).
    fn from_f32(v: f32) -> Self;
}

#[inline]
fn finite_or_zero(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

macro_rules! signed_small {
    ($t:ty, $fmt:ident, $scale:expr) => {
        impl RawSample for $t {
            const FORMAT: SampleFormat = SampleFormat::$fmt;
            const SILENCE: Self = 0;
            #[inline]
            fn to_f32(self) -> f32 {
                f32::from(self) / $scale
            }
            #[inline]
            fn from_f32(v: f32) -> Self {
                let scaled = (finite_or_zero(v) * $scale).round();
                scaled.clamp(f32::from(<$t>::MIN), f32::from(<$t>::MAX)) as $t
            }
        }
    };
}

macro_rules! unsigned_small {
    ($t:ty, $fmt:ident, $scale:expr) => {
        impl RawSample for $t {
            const FORMAT: SampleFormat = SampleFormat::$fmt;
            const SILENCE: Self = $scale as $t;
            #[inline]
            fn to_f32(self) -> f32 {
                (f32::from(self) - $scale) / $scale
            }
            #[inline]
            fn from_f32(v: f32) -> Self {
                let scaled = (finite_or_zero(v) * $scale).round() + $scale;
                scaled.clamp(0.0, f32::from(<$t>::MAX)) as $t
            }
        }
    };
}

signed_small!(i8, I8, 128.0);
signed_small!(i16, I16, 32_768.0);
unsigned_small!(u8, U8, 128.0);
unsigned_small!(u16, U16, 32_768.0);

const I32_SCALE: f64 = 2_147_483_648.0;

impl RawSample for i32 {
    const FORMAT: SampleFormat = SampleFormat::I32;
    const SILENCE: Self = 0;
    #[inline]
    fn to_f32(self) -> f32 {
        (f64::from(self) / I32_SCALE) as f32
    }
    #[inline]
    fn from_f32(v: f32) -> Self {
        let scaled = (f64::from(finite_or_zero(v)) * I32_SCALE).round();
        scaled.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
    }
}

impl RawSample for u32 {
    const FORMAT: SampleFormat = SampleFormat::U32;
    const SILENCE: Self = 2_147_483_648;
    #[inline]
    fn to_f32(self) -> f32 {
        ((f64::from(self) - I32_SCALE) / I32_SCALE) as f32
    }
    #[inline]
    fn from_f32(v: f32) -> Self {
        let scaled = (f64::from(finite_or_zero(v)) * I32_SCALE).round() + I32_SCALE;
        scaled.clamp(0.0, f64::from(u32::MAX)) as u32
    }
}

impl RawSample for f32 {
    const FORMAT: SampleFormat = SampleFormat::F32;
    const SILENCE: Self = 0.0;
    #[inline]
    fn to_f32(self) -> f32 {
        finite_or_zero(self)
    }
    /// Clamped to [-1, 1] on the way to a device: out-of-range float output
    /// is clipped rather than passed to hardware.
    #[inline]
    fn from_f32(v: f32) -> Self {
        finite_or_zero(v).clamp(-1.0, 1.0)
    }
}

impl RawSample for f64 {
    const FORMAT: SampleFormat = SampleFormat::F64;
    const SILENCE: Self = 0.0;
    #[inline]
    fn to_f32(self) -> f32 {
        finite_or_zero(self as f32)
    }
    #[inline]
    fn from_f32(v: f32) -> Self {
        f64::from(finite_or_zero(v).clamp(-1.0, 1.0))
    }
}

/// Append `frames` of interleaved `input` (with `channels` channels) to
/// `out`, either unchanged or averaged to mono.
///
/// Averaging is a plain arithmetic mean per frame (no gain, weighting or
/// filtering); it cannot exceed the input range, so no clipping occurs.
pub fn append_frames(input: &[f32], channels: usize, downmix: bool, out: &mut Vec<f32>) {
    if channels == 0 {
        return;
    }
    if !downmix || channels == 1 {
        out.extend_from_slice(&input[..input.len() - input.len() % channels]);
        return;
    }
    let inv = 1.0 / channels as f32;
    for frame in input.chunks_exact(channels) {
        out.push(frame.iter().sum::<f32>() * inv);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_exact<T: RawSample + PartialEq + std::fmt::Debug>(
        values: impl Iterator<Item = T>,
    ) {
        for v in values {
            assert_eq!(T::from_f32(v.to_f32()), v, "{v:?}");
        }
    }

    #[test]
    fn integer_round_trips_are_lossless() {
        round_trip_exact(i8::MIN..=i8::MAX);
        round_trip_exact(u8::MIN..=u8::MAX);
        round_trip_exact(i16::MIN..=i16::MAX);
        round_trip_exact(u16::MIN..=u16::MAX);
    }

    #[test]
    fn thirty_two_bit_round_trips_are_within_24_bit_precision() {
        for v in [i32::MIN, -1_000_000_007, -1, 0, 1, 7, 123_456_789, i32::MAX] {
            let back = i32::from_f32(v.to_f32());
            assert!(
                (i64::from(back) - i64::from(v)).abs() <= 128,
                "{v} -> {back}"
            );
        }
        for v in [u32::MIN, 1, 1_000_000_007, 2_147_483_648, u32::MAX] {
            let back = u32::from_f32(v.to_f32());
            assert!(
                (i64::from(back) - i64::from(v)).abs() <= 128,
                "{v} -> {back}"
            );
        }
        assert_eq!(
            i32::from_f32(1i32.to_f32()),
            1,
            "small signed values survive"
        );
    }

    #[test]
    fn scale_endpoints_and_silence() {
        assert_eq!(i16::MIN.to_f32(), -1.0);
        assert_eq!(0i16.to_f32(), 0.0);
        assert!(i16::MAX.to_f32() < 1.0);
        assert_eq!(128u8.to_f32(), 0.0);
        assert_eq!(0u8.to_f32(), -1.0);
        assert_eq!(32_768u16.to_f32(), 0.0);
        assert_eq!(i32::MIN.to_f32(), -1.0);
        assert_eq!(u8::SILENCE.to_f32(), 0.0);
        assert_eq!(u16::SILENCE.to_f32(), 0.0);
        assert_eq!(u32::SILENCE.to_f32(), 0.0);
    }

    #[test]
    fn float_to_integer_clips_and_rounds() {
        assert_eq!(i16::from_f32(1.0), i16::MAX, "+1.0 clips");
        assert_eq!(i16::from_f32(2.5), i16::MAX);
        assert_eq!(i16::from_f32(-3.0), i16::MIN);
        assert_eq!(
            i16::from_f32(0.5 / 32_768.0),
            1,
            "rounds half away from zero"
        );
        assert_eq!(i16::from_f32(-0.5 / 32_768.0), -1);
        assert_eq!(i16::from_f32(0.4 / 32_768.0), 0);
        assert_eq!(u8::from_f32(1.0), 255);
        assert_eq!(u8::from_f32(-1.0), 0);
        assert_eq!(i8::from_f32(-1.0), i8::MIN);
        assert_eq!(i32::from_f32(1.0), i32::MAX);
        assert_eq!(u32::from_f32(-1.5), 0);
        assert_eq!(
            f32::from_f32(1.5),
            1.0,
            "float output is clamped for the device"
        );
        assert_eq!(f64::from_f32(-7.0), -1.0);
    }

    #[test]
    fn non_finite_values_become_silence() {
        for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(i16::from_f32(v), 0);
            assert_eq!(u8::from_f32(v), 128);
            assert_eq!(f32::from_f32(v), 0.0);
            assert_eq!(v.to_f32(), 0.0);
        }
        assert_eq!(f64::NAN.to_f32(), 0.0);
        assert_eq!(
            (1.0e300_f64).to_f32(),
            0.0,
            "overflow to inf becomes silence"
        );
    }

    #[test]
    fn capture_float_passes_through_unclamped() {
        assert_eq!(1.25f32.to_f32(), 1.25);
        assert_eq!((-0.5f64).to_f32(), -0.5);
    }

    #[test]
    fn frames_are_appended_or_downmixed_by_mean() {
        let stereo = [0.5, -0.5, 1.0, 0.0, -1.0, -1.0];
        let mut out = Vec::new();
        append_frames(&stereo, 2, true, &mut out);
        assert_eq!(out, vec![0.0, 0.5, -1.0]);
        out.clear();
        append_frames(&stereo, 2, false, &mut out);
        assert_eq!(out, stereo.to_vec());
        out.clear();
        // Six channels: not assumed to be stereo.
        append_frames(&[0.6, 0.0, 0.0, 0.0, 0.0, 0.0], 6, true, &mut out);
        assert!((out[0] - 0.1).abs() < 1e-6);
        out.clear();
        // A trailing partial frame is ignored.
        append_frames(&[0.1, 0.2, 0.3], 2, false, &mut out);
        assert_eq!(out, vec![0.1, 0.2]);
        out.clear();
        append_frames(&[1.0, 1.0], 0, true, &mut out);
        assert!(out.is_empty());
    }
}
