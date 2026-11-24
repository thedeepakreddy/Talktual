//! Sample formats, channel layouts and stream formats.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::CeError;

/// Lowest sample rate CE accepts, in Hz.
pub const MIN_SAMPLE_RATE: u32 = 1_000;
/// Highest sample rate CE accepts, in Hz.
pub const MAX_SAMPLE_RATE: u32 = 768_000;
/// Highest channel count CE accepts.
pub const MAX_CHANNELS: u16 = 32;

/// A native (device-side) sample format CE can convert to and from.
///
/// Formats a backend offers outside this set (24-bit packed, 64-bit integer,
/// DSD) are skipped during discovery and counted in
/// `AudioDeviceDescriptor::skipped_formats`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleFormat {
    /// Signed 8-bit integer.
    I8,
    /// Unsigned 8-bit integer (silence = 128).
    U8,
    /// Signed 16-bit integer.
    I16,
    /// Unsigned 16-bit integer (silence = 32768).
    U16,
    /// Signed 32-bit integer.
    I32,
    /// Unsigned 32-bit integer.
    U32,
    /// 32-bit IEEE float, nominal range [-1.0, 1.0].
    F32,
    /// 64-bit IEEE float, nominal range [-1.0, 1.0].
    F64,
}

impl SampleFormat {
    /// All formats, in CE's preference order for negotiation (most preferred
    /// first): float avoids conversion loss; 16-bit is universally supported.
    pub const PREFERENCE: [SampleFormat; 8] = [
        SampleFormat::F32,
        SampleFormat::I16,
        SampleFormat::I32,
        SampleFormat::F64,
        SampleFormat::U16,
        SampleFormat::I8,
        SampleFormat::U8,
        SampleFormat::U32,
    ];

    /// Bytes per sample.
    pub const fn bytes(self) -> usize {
        match self {
            SampleFormat::I8 | SampleFormat::U8 => 1,
            SampleFormat::I16 | SampleFormat::U16 => 2,
            SampleFormat::I32 | SampleFormat::U32 | SampleFormat::F32 => 4,
            SampleFormat::F64 => 8,
        }
    }

    /// Whether the format is floating point.
    pub const fn is_float(self) -> bool {
        matches!(self, SampleFormat::F32 | SampleFormat::F64)
    }

    /// Position in [`SampleFormat::PREFERENCE`] (0 = most preferred).
    pub fn preference_rank(self) -> usize {
        Self::PREFERENCE
            .iter()
            .position(|f| *f == self)
            .unwrap_or(Self::PREFERENCE.len())
    }

    /// Stable lowercase name.
    pub const fn as_str(self) -> &'static str {
        match self {
            SampleFormat::I8 => "i8",
            SampleFormat::U8 => "u8",
            SampleFormat::I16 => "i16",
            SampleFormat::U16 => "u16",
            SampleFormat::I32 => "i32",
            SampleFormat::U32 => "u32",
            SampleFormat::F32 => "f32",
            SampleFormat::F64 => "f64",
        }
    }
}

impl fmt::Display for SampleFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Channel layout. Backends generally report only a channel count, so CE
/// does not invent speaker positions: one channel is `Mono`, two channels are
/// treated as `Stereo` (left, right — the near-universal convention), and
/// anything else is `Discrete(n)` with unknown positions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "layout", content = "channels")]
pub enum ChannelLayout {
    /// One channel.
    Mono,
    /// Two channels, conventionally left then right.
    Stereo,
    /// `n` channels with unknown positions.
    Discrete(u16),
}

impl ChannelLayout {
    /// Layout for a channel count.
    pub const fn from_channels(channels: u16) -> Self {
        match channels {
            1 => ChannelLayout::Mono,
            2 => ChannelLayout::Stereo,
            n => ChannelLayout::Discrete(n),
        }
    }

    /// Channel count.
    pub const fn channels(self) -> u16 {
        match self {
            ChannelLayout::Mono => 1,
            ChannelLayout::Stereo => 2,
            ChannelLayout::Discrete(n) => n,
        }
    }
}

/// A concrete stream format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StreamFormat {
    /// Frames per second.
    pub sample_rate: u32,
    /// Interleaved channels per frame.
    pub channels: u16,
    /// Sample encoding.
    pub sample_format: SampleFormat,
}

impl StreamFormat {
    /// Construct and validate.
    pub fn new(
        sample_rate: u32,
        channels: u16,
        sample_format: SampleFormat,
    ) -> Result<Self, CeError> {
        let f = Self {
            sample_rate,
            channels,
            sample_format,
        };
        f.validate()?;
        Ok(f)
    }

    /// Check rate and channel bounds.
    pub fn validate(&self) -> Result<(), CeError> {
        if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&self.sample_rate) {
            return Err(CeError::invalid_input(
                "audio.invalid_sample_rate",
                "sample rate outside the supported range",
            )
            .with_context("sample_rate", self.sample_rate.to_string()));
        }
        if self.channels == 0 || self.channels > MAX_CHANNELS {
            return Err(CeError::invalid_input(
                "audio.invalid_channels",
                "channel count outside the supported range",
            )
            .with_context("channels", self.channels.to_string()));
        }
        Ok(())
    }

    /// Channel layout derived from the channel count.
    pub const fn layout(&self) -> ChannelLayout {
        ChannelLayout::from_channels(self.channels)
    }
}

/// Buffer sizes (frames per callback) a device accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum BufferSizeRange {
    /// The backend cannot tell.
    Unknown,
    /// An inclusive range.
    Range {
        /// Minimum frames per callback.
        min: u32,
        /// Maximum frames per callback.
        max: u32,
    },
}

/// One family of formats a device supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SupportedFormatRange {
    /// Channel count.
    pub channels: u16,
    /// Lowest supported rate (Hz), already clamped to CE's limits.
    pub min_sample_rate: u32,
    /// Highest supported rate (Hz), already clamped to CE's limits.
    pub max_sample_rate: u32,
    /// Sample encoding.
    pub sample_format: SampleFormat,
    /// Buffer size range.
    pub buffer: BufferSizeRange,
}

impl SupportedFormatRange {
    /// Clamp a backend-reported range to CE limits. Returns `None` when the
    /// range is empty, inverted or outside the limits (an invalid backend
    /// response), so callers can skip and count it.
    pub fn sanitized(self) -> Option<Self> {
        if self.channels == 0
            || self.channels > MAX_CHANNELS
            || self.min_sample_rate > self.max_sample_rate
        {
            return None;
        }
        let min = self.min_sample_rate.max(MIN_SAMPLE_RATE);
        let max = self.max_sample_rate.min(MAX_SAMPLE_RATE);
        if min > max {
            return None;
        }
        let buffer = match self.buffer {
            BufferSizeRange::Range { min, max } if min == 0 || min > max => {
                BufferSizeRange::Unknown
            }
            other => other,
        };
        Some(Self {
            min_sample_rate: min,
            max_sample_rate: max,
            buffer,
            ..self
        })
    }

    /// Whether `rate` lies in the range.
    pub fn contains_rate(&self, rate: u32) -> bool {
        (self.min_sample_rate..=self.max_sample_rate).contains(&rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_follow_channel_counts_without_inventing_positions() {
        assert_eq!(ChannelLayout::from_channels(1), ChannelLayout::Mono);
        assert_eq!(ChannelLayout::from_channels(2), ChannelLayout::Stereo);
        assert_eq!(ChannelLayout::from_channels(6), ChannelLayout::Discrete(6));
        for n in 1..=MAX_CHANNELS {
            assert_eq!(ChannelLayout::from_channels(n).channels(), n);
        }
    }

    #[test]
    fn stream_formats_are_validated() {
        assert!(StreamFormat::new(48_000, 2, SampleFormat::I16).is_ok());
        for (rate, ch) in [(0, 1), (999, 1), (768_001, 1), (48_000, 0), (48_000, 33)] {
            assert!(
                StreamFormat::new(rate, ch, SampleFormat::F32).is_err(),
                "{rate} {ch}"
            );
        }
        assert_eq!(
            StreamFormat::new(44_100, 6, SampleFormat::F32)
                .unwrap()
                .layout(),
            ChannelLayout::Discrete(6)
        );
    }

    #[test]
    fn sample_format_properties_and_preference() {
        assert_eq!(SampleFormat::I16.bytes(), 2);
        assert_eq!(SampleFormat::F64.bytes(), 8);
        assert!(SampleFormat::F32.is_float() && !SampleFormat::I32.is_float());
        assert_eq!(SampleFormat::F32.preference_rank(), 0);
        assert!(SampleFormat::I16.preference_rank() < SampleFormat::U8.preference_rank());
        assert_eq!(
            serde_json::to_string(&SampleFormat::U16).unwrap(),
            "\"u16\""
        );
    }

    #[test]
    fn backend_ranges_are_sanitized_or_rejected() {
        let r = |ch, min, max| SupportedFormatRange {
            channels: ch,
            min_sample_rate: min,
            max_sample_rate: max,
            sample_format: SampleFormat::I16,
            buffer: BufferSizeRange::Range { min: 0, max: 10 },
        };
        // The ALSA null device reports 1..=u32::MAX; CE clamps it.
        let s = r(2, 1, u32::MAX).sanitized().unwrap();
        assert_eq!(
            (s.min_sample_rate, s.max_sample_rate),
            (MIN_SAMPLE_RATE, MAX_SAMPLE_RATE)
        );
        assert_eq!(
            s.buffer,
            BufferSizeRange::Unknown,
            "a zero minimum buffer is not trusted"
        );
        assert!(r(0, 8_000, 48_000).sanitized().is_none());
        assert!(r(2, 48_000, 8_000).sanitized().is_none());
        assert!(r(2, 1, 500).sanitized().is_none());
        assert!(r(40, 8_000, 48_000).sanitized().is_none());
        assert!(
            r(1, 8_000, 48_000)
                .sanitized()
                .unwrap()
                .contains_rate(16_000)
        );
    }
}
