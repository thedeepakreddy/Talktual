//! Window/feature pipeline: Phase 2 blocks → fixed analysis hops with
//! features, plus explicit discontinuity markers.
//!
//! * **Hop**: `hop_ms` (10/20/30 ms) of frames, no overlap. Time-domain
//!   features (level, peak, clipping, DC, zero-crossing rate, per-channel
//!   level, non-finite count) are computed per hop on every channel / the
//!   mono mix.
//! * **Spectral window**: the last 40 ms of the mono mix, decimated by an
//!   integer factor to ~16 kHz (boxcar average — adequate for features
//!   below 4 kHz, not a production resampler), Hann-windowed, zero-padded
//!   FFT. Yields speech-band energy ratio (300–3400 Hz of 50–8000 Hz),
//!   spectral flatness (300–4000 Hz) and periodicity (peak normalized
//!   autocorrelation for 70–400 Hz pitch, corrected for the window's own
//!   autocorrelation). Cost is independent of the device rate.
//! * **Timestamps**: a hop's time is its first frame's sample-clock time,
//!   from the block `timestamp` plus the offset within the block.
//! * **Discontinuities**: a Phase 2 discontinuity flag, a source-sequence
//!   gap (analysis tap drops) or a format change emits a [`Gap`] and resets
//!   all windowed state; a partial hop before a gap is discarded (counted).
//! * **Malformed input**: non-finite samples are replaced by 0 and counted;
//!   a trailing partial frame in a block is ignored.
//!
//! All buffers are allocated once per format.

use ce_core::audio::{AudioBlock, frame::frames_to_duration};
use ce_core::intel::model::DiscontinuityContext;
use ce_core::time::MonotonicTime;
use ce_core::{CeError, ErrorCategory};

use crate::dsp::{Fft, hann, power_db};

/// Supported rate range for analysis.
pub const MIN_RATE: u32 = 8_000;
/// Supported rate range for analysis.
pub const MAX_RATE: u32 = 96_000;
/// Channels whose individual levels are tracked (imbalance).
pub const TRACKED_CHANNELS: usize = 8;
/// Spectral window length.
pub const SPECTRAL_WINDOW_MS: u32 = 40;

/// Spectral features of the window ending at a hop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spectral {
    /// Energy in 300–3400 Hz over energy in 50–8000 Hz (or Nyquist).
    pub speech_ratio: f32,
    /// Geometric / arithmetic mean of power in 300–4000 Hz (0 tonal … 1 flat).
    pub flatness: f32,
    /// Peak normalized autocorrelation for 70–400 Hz periods, [0, 1].
    pub periodicity: f32,
}

/// Features of one hop.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HopFeatures {
    /// Mean square of the mono mix.
    pub mean_square: f32,
    /// RMS level of the mono mix, dBFS.
    pub level_db: f32,
    /// Peak |sample| over all channels.
    pub peak: f32,
    /// Samples at or above the clip level (all channels).
    pub clipped: u32,
    /// Samples in the hop (frames × channels).
    pub samples: u32,
    /// Non-finite samples replaced by 0.
    pub non_finite: u32,
    /// Sum of mono samples (for DC).
    pub dc_sum: f32,
    /// Zero-crossing rate of the mono mix (crossings per frame).
    pub zcr: f32,
    /// Mean square of the first `TRACKED_CHANNELS` channels.
    pub channel_ms: [f32; TRACKED_CHANNELS],
    /// Number of valid entries in `channel_ms`.
    pub channels: u8,
    /// Spectral features, when enabled and the window is full.
    pub spectral: Option<Spectral>,
}

/// One analysis hop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hop {
    /// Hops since analysis start.
    pub index: u64,
    /// Sample-clock frame of the first frame.
    pub frame_start: u64,
    /// Frames in the hop.
    pub frames: u32,
    /// Sample-clock time of the first frame.
    pub time: MonotonicTime,
    /// Time just after the last frame.
    pub end_time: MonotonicTime,
    /// Source block sequence containing the hop's last frame.
    pub sequence: u64,
    /// Features.
    pub features: HopFeatures,
    /// Range of this hop's mono samples in [`Framer::hop_audio`] (valid
    /// until the next `push`; empty unless audio export is enabled).
    pub audio: (u32, u32),
}

/// A discontinuity seen by the framer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gap {
    /// What happened.
    pub context: DiscontinuityContext,
    /// Sample-clock frame where analysis resumes.
    pub resume_frame: u64,
    /// Time where analysis resumes.
    pub resume_time: MonotonicTime,
    /// Source sequence of the resuming block.
    pub sequence: u64,
    /// Frames of an incomplete hop discarded before the gap.
    pub discarded_frames: u32,
}

/// Framer output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FramerItem {
    /// A complete hop.
    Hop(Hop),
    /// A discontinuity (state downstream must reset).
    Gap(Gap),
}

struct Format {
    rate: u32,
    channels: usize,
    hop_frames: u32,
    decim: usize,
    spec_rate: f32,
    win: usize,
    fft: Fft,
    hann: Vec<f32>,
    /// Autocorrelation of the window, for unbiasing.
    win_ac: Vec<f32>,
    history: Vec<f32>,
    hist_pos: usize,
    hist_filled: usize,
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Format {
    fn new(rate: u32, channels: usize, hop_ms: u32) -> Result<Self, CeError> {
        if !(MIN_RATE..=MAX_RATE).contains(&rate) || channels == 0 {
            return Err(CeError::new(
                ErrorCategory::InvalidInput,
                "analysis.unsupported_format",
                "analysis supports 8–96 kHz with at least one channel",
            )
            .with_context("sample_rate", rate.to_string())
            .with_context("channels", channels.to_string()));
        }
        let hop_frames = (rate as u64 * hop_ms as u64 / 1000) as u32;
        let decim = ((rate / 16_000) as usize).max(1);
        let spec_rate = rate as f32 / decim as f32;
        let win = (spec_rate * SPECTRAL_WINDOW_MS as f32 / 1000.0) as usize;
        let n = (2 * win).next_power_of_two();
        let fft = Fft::new(n);
        let hann = hann(win);
        let (mut re, mut im) = (vec![0.0; n], vec![0.0; n]);
        re[..win].copy_from_slice(&hann);
        fft.forward(&mut re, &mut im);
        for k in 0..n {
            re[k] = re[k] * re[k] + im[k] * im[k];
            im[k] = 0.0;
        }
        fft.inverse(&mut re, &mut im);
        let win_ac = re[..win].to_vec();
        Ok(Self {
            rate,
            channels,
            hop_frames,
            decim,
            spec_rate,
            win,
            fft,
            hann,
            win_ac,
            history: vec![0.0; win],
            hist_pos: 0,
            hist_filled: 0,
            re: vec![0.0; n],
            im: vec![0.0; n],
        })
    }

    fn push_history(&mut self, v: f32) {
        self.history[self.hist_pos] = v;
        self.hist_pos = (self.hist_pos + 1) % self.win;
        self.hist_filled = (self.hist_filled + 1).min(self.win);
    }

    fn reset(&mut self) {
        self.hist_pos = 0;
        self.hist_filled = 0;
    }

    fn spectral(&mut self) -> Option<Spectral> {
        if self.hist_filled < self.win {
            return None;
        }
        let n = self.fft.len();
        let win = self.win;
        for i in 0..win {
            let v = self.history[(self.hist_pos + i) % win];
            self.re[i] = v * self.hann[i];
        }
        self.re[win..].fill(0.0);
        self.im.fill(0.0);
        self.fft.forward(&mut self.re, &mut self.im);
        let bin_hz = self.spec_rate / n as f32;
        let nyq = self.spec_rate / 2.0;
        let (mut total, mut speech, mut flat_log, mut flat_lin, mut flat_n) =
            (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0u32);
        for k in 0..=n / 2 {
            let p = self.re[k] * self.re[k] + self.im[k] * self.im[k];
            let f = k as f32 * bin_hz;
            if (50.0..=8_000.0_f32.min(nyq)).contains(&f) {
                total += f64::from(p);
            }
            if (300.0..=3_400.0).contains(&f) {
                speech += f64::from(p);
            }
            if (300.0..=4_000.0_f32.min(nyq)).contains(&f) {
                flat_log += f64::from(p + 1e-12).ln();
                flat_lin += f64::from(p);
                flat_n += 1;
            }
            // Reuse the buffers for the autocorrelation: |X|^2.
            self.re[k] = p;
            self.im[k] = 0.0;
            if k > 0 && k < n / 2 {
                self.re[n - k] = p;
                self.im[n - k] = 0.0;
            }
        }
        let speech_ratio = if total > 1e-12 {
            (speech / total) as f32
        } else {
            0.0
        };
        let flatness = if flat_n > 0 && flat_lin > 1e-12 {
            let n = f64::from(flat_n);
            ((flat_log / n).exp() / (flat_lin / n)) as f32
        } else {
            1.0
        };
        self.fft.inverse(&mut self.re, &mut self.im);
        let r0 = self.re[0];
        let mut periodicity = 0.0f32;
        if r0 > 1e-10 {
            let lo = (self.spec_rate / 400.0) as usize;
            let hi = ((self.spec_rate / 70.0) as usize).min(win - 1);
            for lag in lo.max(1)..=hi {
                let w = self.win_ac[lag] / self.win_ac[0];
                if w > 0.05 {
                    let r = (self.re[lag] / r0) / w;
                    periodicity = periodicity.max(r);
                }
            }
        }
        Some(Spectral {
            speech_ratio: speech_ratio.clamp(0.0, 1.0),
            flatness: flatness.clamp(0.0, 1.0),
            periodicity: periodicity.clamp(0.0, 1.0),
        })
    }
}

#[derive(Default)]
struct HopAcc {
    frames: u32,
    frame_start: u64,
    time: MonotonicTime,
    sum_sq: f64,
    peak: f32,
    clipped: u32,
    non_finite: u32,
    dc_sum: f64,
    crossings: u32,
    last_sign: i8,
    channel_sq: [f64; TRACKED_CHANNELS],
    decim_acc: f32,
    decim_n: usize,
}

/// Turns blocks into hops.
pub struct Framer {
    hop_ms: u32,
    clip_level: f32,
    spectral_enabled: bool,
    format: Option<Format>,
    acc: HopAcc,
    hop_index: u64,
    expected_sequence: Option<u64>,
    expected_frame: Option<u64>,
    audio_enabled: bool,
    cur_mono: Vec<f32>,
    hop_audio: Vec<f32>,
}

impl std::fmt::Debug for Framer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Framer")
            .field("hop_ms", &self.hop_ms)
            .field("hop_index", &self.hop_index)
            .finish_non_exhaustive()
    }
}

impl Framer {
    /// A framer for `hop_ms` hops.
    pub fn new(hop_ms: u32, clip_level: f32) -> Self {
        Self {
            hop_ms,
            clip_level,
            spectral_enabled: true,
            format: None,
            acc: HopAcc::default(),
            hop_index: 0,
            expected_sequence: None,
            expected_frame: None,
            audio_enabled: false,
            cur_mono: Vec::new(),
            hop_audio: Vec::new(),
        }
    }

    /// Export each hop's mono samples (for detectors that need audio).
    pub fn set_audio(&mut self, enabled: bool) {
        self.audio_enabled = enabled;
        self.cur_mono.clear();
    }

    /// The mono samples of `hop` (device rate), if exported by the last push.
    pub fn hop_audio(&self, hop: &Hop) -> &[f32] {
        let (a, b) = (hop.audio.0 as usize, hop.audio.1 as usize);
        self.hop_audio.get(a..b).unwrap_or(&[])
    }

    /// Current sample rate.
    pub fn rate(&self) -> Option<u32> {
        self.format.as_ref().map(|f| f.rate)
    }

    /// Enable or disable spectral features (degraded mode).
    pub fn set_spectral(&mut self, enabled: bool) {
        if !enabled {
            if let Some(f) = self.format.as_mut() {
                f.reset();
            }
        }
        self.spectral_enabled = enabled;
    }

    /// Hop length in frames for the current format.
    pub fn hop_frames(&self) -> Option<u32> {
        self.format.as_ref().map(|f| f.hop_frames)
    }

    fn reset_state(&mut self) -> u32 {
        let discarded = self.acc.frames;
        self.acc = HopAcc::default();
        self.cur_mono.clear();
        if let Some(f) = self.format.as_mut() {
            f.reset();
        }
        discarded
    }

    /// Force a gap (e.g. blocks skipped by the resource policy).
    pub fn force_gap(
        &mut self,
        context: DiscontinuityContext,
        block: &AudioBlock,
        out: &mut impl FnMut(FramerItem),
    ) {
        let discarded = self.reset_state();
        out(FramerItem::Gap(Gap {
            context,
            resume_frame: block.meta.frame_index,
            resume_time: block.meta.timestamp,
            sequence: block.meta.sequence,
            discarded_frames: discarded,
        }));
        self.expected_sequence = Some(block.meta.sequence);
        self.expected_frame = Some(block.meta.frame_index);
    }

    /// Process one block.
    pub fn push(
        &mut self,
        block: &AudioBlock,
        out: &mut impl FnMut(FramerItem),
    ) -> Result<(), CeError> {
        self.hop_audio.clear();
        let m = &block.meta;
        let channels = usize::from(m.channels);
        let format_changed = self
            .format
            .as_ref()
            .is_none_or(|f| f.rate != m.sample_rate || f.channels != channels);
        let mut gap: Option<DiscontinuityContext> = None;
        if format_changed {
            if self.format.is_some() {
                gap = Some(DiscontinuityContext::FormatChange);
            }
            self.format = Some(Format::new(m.sample_rate, channels, self.hop_ms)?);
            self.acc = HopAcc::default();
            self.cur_mono.clear();
        } else if let Some(d) = m.discontinuity {
            gap = Some(DiscontinuityContext::Capture {
                kind: d.kind,
                lost_frames: d.lost_frames,
            });
        } else if let Some(expected) = self.expected_sequence {
            if m.sequence > expected {
                gap = Some(DiscontinuityContext::AnalysisDrop {
                    blocks: m.sequence - expected,
                });
            } else if self.expected_frame.is_some_and(|f| f != m.frame_index) {
                gap = Some(DiscontinuityContext::AnalysisDrop { blocks: 0 });
            }
        }
        if let Some(context) = gap {
            let discarded = self.reset_state();
            out(FramerItem::Gap(Gap {
                context,
                resume_frame: m.frame_index,
                resume_time: m.timestamp,
                sequence: m.sequence,
                discarded_frames: discarded,
            }));
        }
        self.expected_sequence = Some(m.sequence + 1);
        self.expected_frame = Some(m.end_frame_index());

        let Some(format) = self.format.as_mut() else {
            return Ok(());
        };
        let rate = format.rate;
        let hop_frames = format.hop_frames;
        let decim = format.decim;
        let inv_ch = 1.0 / channels as f32;
        let frames = (block.samples.len() / channels).min(m.frames as usize);
        for (i, frame) in block
            .samples
            .chunks_exact(channels)
            .take(frames)
            .enumerate()
        {
            let acc = &mut self.acc;
            if acc.frames == 0 {
                let offset = frames_to_duration(i as u64, rate);
                acc.frame_start = m.frame_index + i as u64;
                acc.time = MonotonicTime::from_nanos(
                    m.timestamp
                        .as_nanos()
                        .saturating_add(offset.as_nanos() as u64),
                );
            }
            let mut mono = 0.0f32;
            for (c, s) in frame.iter().enumerate() {
                let v = if s.is_finite() {
                    *s
                } else {
                    acc.non_finite += 1;
                    0.0
                };
                let a = v.abs();
                acc.peak = acc.peak.max(a);
                if a >= self.clip_level {
                    acc.clipped += 1;
                }
                if c < TRACKED_CHANNELS {
                    acc.channel_sq[c] += f64::from(v * v);
                }
                mono += v;
            }
            mono *= inv_ch;
            if self.audio_enabled {
                self.cur_mono.push(mono);
            }
            acc.sum_sq += f64::from(mono * mono);
            acc.dc_sum += f64::from(mono);
            let sign = if mono > 0.0 {
                1
            } else if mono < 0.0 {
                -1
            } else {
                0
            };
            if sign != 0 {
                if acc.last_sign != 0 && sign != acc.last_sign {
                    acc.crossings += 1;
                }
                acc.last_sign = sign;
            }
            if self.spectral_enabled {
                acc.decim_acc += mono;
                acc.decim_n += 1;
                if acc.decim_n == decim {
                    let v = acc.decim_acc / decim as f32;
                    acc.decim_acc = 0.0;
                    acc.decim_n = 0;
                    format.push_history(v);
                }
            }
            acc.frames += 1;
            if acc.frames == hop_frames {
                let n = f64::from(hop_frames);
                let mean_square = (acc.sum_sq / n) as f32;
                let mut channel_ms = [0.0f32; TRACKED_CHANNELS];
                for (c, v) in channel_ms
                    .iter_mut()
                    .enumerate()
                    .take(channels.min(TRACKED_CHANNELS))
                {
                    *v = (acc.channel_sq[c] / n) as f32;
                }
                let spectral = if self.spectral_enabled {
                    format.spectral()
                } else {
                    None
                };
                let end = MonotonicTime::from_nanos(acc.time.as_nanos().saturating_add(
                    frames_to_duration(u64::from(hop_frames), rate).as_nanos() as u64,
                ));
                let a0 = self.hop_audio.len() as u32;
                self.hop_audio.append(&mut self.cur_mono);
                let hop = Hop {
                    audio: (a0, self.hop_audio.len() as u32),
                    index: self.hop_index,
                    frame_start: acc.frame_start,
                    frames: hop_frames,
                    time: acc.time,
                    end_time: end,
                    sequence: m.sequence,
                    features: HopFeatures {
                        mean_square,
                        level_db: power_db(mean_square),
                        peak: acc.peak,
                        clipped: acc.clipped,
                        samples: hop_frames * channels as u32,
                        non_finite: acc.non_finite,
                        dc_sum: acc.dc_sum as f32,
                        zcr: acc.crossings as f32 / hop_frames as f32,
                        channel_ms,
                        channels: channels.min(TRACKED_CHANNELS) as u8,
                        spectral,
                    },
                };
                self.hop_index += 1;
                let last_sign = acc.last_sign;
                let (decim_acc, decim_n) = (acc.decim_acc, acc.decim_n);
                *acc = HopAcc {
                    last_sign,
                    decim_acc,
                    decim_n,
                    ..HopAcc::default()
                };
                out(FramerItem::Hop(hop));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{block, sine};
    use ce_core::audio::{Discontinuity, DiscontinuityKind};

    fn run(framer: &mut Framer, blocks: &[AudioBlock]) -> Vec<FramerItem> {
        let mut out = Vec::new();
        for b in blocks {
            framer.push(b, &mut |i| out.push(i)).unwrap();
        }
        out
    }

    #[test]
    fn hops_have_exact_timing_and_levels() {
        let mut f = Framer::new(10, 0.999);
        let samples = sine(440.0, 16_000, 0.5, 1_600);
        let blocks: Vec<_> = samples
            .chunks(160)
            .enumerate()
            .map(|(i, c)| block(i as u64, i as u64 * 160, 16_000, 1, c.to_vec()))
            .collect();
        let items = run(&mut f, &blocks);
        let hops: Vec<_> = items
            .iter()
            .filter_map(|i| {
                if let FramerItem::Hop(h) = i {
                    Some(*h)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(hops.len(), 10);
        for (k, h) in hops.iter().enumerate() {
            assert_eq!(h.frame_start, k as u64 * 160);
            assert_eq!(h.time.as_nanos(), k as u64 * 10_000_000);
            assert_eq!(h.end_time.as_nanos() - h.time.as_nanos(), 10_000_000);
            // A 0.5-amplitude sine: RMS 0.354 → -9.03 dBFS.
            assert!(
                (h.features.level_db + 9.03).abs() < 0.2,
                "{}",
                h.features.level_db
            );
        }
        let s = hops[9].features.spectral.expect("window full after 40 ms");
        assert!(s.periodicity > 0.8, "a 440 Hz tone is periodic: {s:?}");
        assert!(s.flatness < 0.2, "a tone is not flat: {s:?}");
    }

    #[test]
    fn blocks_need_not_align_with_hops_and_rates_are_rate_independent() {
        let mut f = Framer::new(10, 0.999);
        let samples = sine(200.0, 48_000, 0.25, 4_800 * 2);
        let stereo: Vec<f32> = samples.iter().flat_map(|s| [*s, *s]).collect();
        let blocks: Vec<_> = stereo
            .chunks(2 * 333)
            .enumerate()
            .map(|(i, c)| block(i as u64, i as u64 * 333, 48_000, 2, c.to_vec()))
            .collect();
        let hops: Vec<_> = run(&mut f, &blocks)
            .into_iter()
            .filter_map(|i| {
                if let FramerItem::Hop(h) = i {
                    Some(h)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(hops.len(), 20);
        assert!(
            hops.windows(2)
                .all(|w| w[1].frame_start == w[0].frame_start + 480)
        );
        assert_eq!(hops[0].features.channels, 2);
        let s = hops[19].features.spectral.unwrap();
        assert!(
            s.periodicity > 0.8 && s.speech_ratio < 0.2,
            "200 Hz is below the speech band: {s:?}"
        );
    }

    #[test]
    fn white_noise_is_flat_and_aperiodic() {
        let mut f = Framer::new(10, 0.999);
        let noise = crate::testutil::noise(7, 16_000, 0.3);
        let blocks: Vec<_> = noise
            .chunks(160)
            .enumerate()
            .map(|(i, c)| block(i as u64, i as u64 * 160, 16_000, 1, c.to_vec()))
            .collect();
        let last = run(&mut f, &blocks)
            .into_iter()
            .rev()
            .find_map(|i| {
                if let FramerItem::Hop(h) = i {
                    h.features.spectral
                } else {
                    None
                }
            })
            .unwrap();
        assert!(last.flatness > 0.5 && last.periodicity < 0.5, "{last:?}");
    }

    #[test]
    fn discontinuities_sequence_gaps_and_format_changes_reset_state() {
        let mut f = Framer::new(10, 0.999);
        let s = sine(300.0, 16_000, 0.3, 2_000);
        let mut blocks = vec![block(0, 0, 16_000, 1, s[..250].to_vec())];
        let mut flagged = block(1, 1_000, 16_000, 1, s[250..410].to_vec());
        flagged.meta.discontinuity = Some(Discontinuity {
            kind: DiscontinuityKind::DeviceOverflow,
            lost_frames: Some(750),
        });
        blocks.push(flagged);
        blocks.push(block(4, 1_160, 16_000, 1, s[410..570].to_vec())); // sequence gap
        blocks.push(block(5, 1_320, 48_000, 1, sine(300.0, 48_000, 0.3, 480))); // format change
        let items = run(&mut f, &blocks);
        let gaps: Vec<_> = items
            .iter()
            .filter_map(|i| {
                if let FramerItem::Gap(g) = i {
                    Some(*g)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(gaps.len(), 3);
        assert_eq!(
            gaps[0].discarded_frames, 90,
            "the partial hop before the gap is discarded"
        );
        assert!(matches!(
            gaps[0].context,
            DiscontinuityContext::Capture {
                kind: DiscontinuityKind::DeviceOverflow,
                lost_frames: Some(750)
            }
        ));
        assert_eq!(
            gaps[1].context,
            DiscontinuityContext::AnalysisDrop { blocks: 2 }
        );
        assert_eq!(gaps[2].context, DiscontinuityContext::FormatChange);
        assert_eq!(f.hop_frames(), Some(480));
    }

    #[test]
    fn malformed_input_is_sanitized_and_unsupported_formats_are_errors() {
        let mut f = Framer::new(10, 0.999);
        let mut s = vec![0.1f32; 160];
        s[3] = f32::NAN;
        s[4] = f32::INFINITY;
        s[5] = 1.5;
        let items = run(&mut f, &[block(0, 0, 16_000, 1, s)]);
        let FramerItem::Hop(h) = items[0] else {
            panic!()
        };
        assert_eq!((h.features.non_finite, h.features.clipped), (2, 1));
        assert!(h.features.level_db.is_finite());
        let mut f = Framer::new(10, 0.999);
        let err = f
            .push(&block(0, 0, 4_000, 1, vec![0.0; 40]), &mut |_| {})
            .unwrap_err();
        assert_eq!(err.code(), "analysis.unsupported_format");
        // Empty and short blocks produce nothing and fail nothing.
        let mut f = Framer::new(10, 0.999);
        assert!(
            run(
                &mut f,
                &[
                    block(0, 0, 16_000, 1, vec![]),
                    block(1, 0, 16_000, 1, vec![0.0; 5])
                ]
            )
            .is_empty()
        );
    }
}
