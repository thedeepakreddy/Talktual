//! Frame/block metadata, timestamps and discontinuities.
//!
//! ## Clock policy
//!
//! * `frame_index` — position in the stream's **sample clock**: frames since
//!   the stream first started, *including* frames that were lost. It is the
//!   authoritative timeline for a stream.
//! * `timestamp` — CE monotonic time ([`MonotonicTime`], ns since the
//!   runtime clock origin) of the block's first frame, derived from the
//!   sample clock: `anchor_time + (frame_index - anchor_frame) / rate`. The
//!   anchor is the host time when data was first observed after a start, so
//!   absolute accuracy is bounded by the worker poll interval plus one device
//!   buffer; relative timing within a run is exact in sample terms.
//! * `host_time` — CE monotonic time when the block was assembled. Comparing
//!   it with `timestamp` over time reveals drift between the device clock and
//!   the host clock, which CE never assumes to match.
//! * `device_time` — the backend's own stream clock for the first frame, when
//!   the backend reports one. A different time base; never mixed with the
//!   others.
//!
//! Units: nanoseconds for times, frames for positions.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::format::{ChannelLayout, SampleFormat};
use crate::ids::{AudioStreamId, CorrelationId, SessionId};
use crate::time::MonotonicTime;

/// Why the sample clock has a gap before a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscontinuityKind {
    /// The device ring was full; the callback dropped incoming frames.
    DeviceOverflow,
    /// The delivery queue was full; whole blocks were dropped.
    DeliveryOverflow,
    /// The stream was stopped and restarted; real time elapsed with no data.
    StreamRestart,
}

/// A gap immediately before a block's first frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discontinuity {
    /// Kind.
    pub kind: DiscontinuityKind,
    /// Frames missing from the sample clock, when known. For
    /// `StreamRestart` this is `None` (the gap is in real time, not in
    /// captured frames, and `frame_index` does not advance).
    pub lost_frames: Option<u64>,
}

/// Metadata for one delivered block of audio.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioFrameMeta {
    /// Stream.
    pub stream_id: AudioStreamId,
    /// Owning session.
    pub session_id: SessionId,
    /// Session correlation id.
    pub correlation_id: CorrelationId,
    /// Per-stream block sequence, starting at 0, incremented for every block
    /// produced — including blocks later dropped from a full queue, so a
    /// sequence gap reveals drops.
    pub sequence: u64,
    /// Sample-clock position of the first frame.
    pub frame_index: u64,
    /// Frames in the block (≥ 1).
    pub frames: u32,
    /// Frames per second.
    pub sample_rate: u32,
    /// Channels per frame.
    pub channels: u16,
    /// Layout.
    pub layout: ChannelLayout,
    /// Sample format of the data (always `f32` in Phase 2).
    pub sample_format: SampleFormat,
    /// Monotonic time of the first frame (sample-clock derived).
    pub timestamp: MonotonicTime,
    /// Monotonic time the block was assembled.
    pub host_time: MonotonicTime,
    /// Backend stream-clock time of the first frame, if reported.
    pub device_time: Option<Duration>,
    /// Gap immediately before this block, if any.
    pub discontinuity: Option<Discontinuity>,
}

impl AudioFrameMeta {
    /// Duration of the block.
    pub fn duration(&self) -> Duration {
        frames_to_duration(u64::from(self.frames), self.sample_rate)
    }

    /// Sample-clock position just after the block.
    pub fn end_frame_index(&self) -> u64 {
        self.frame_index + u64::from(self.frames)
    }
}

/// A block of interleaved samples plus metadata.
///
/// The samples are **biometric** data: CE never logs them, puts them in
/// events or persists them. Ownership passes to the receiver; the block is
/// an independent copy (the device ring is reused immediately).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioBlock {
    /// Metadata.
    pub meta: AudioFrameMeta,
    /// `meta.frames * meta.channels` interleaved samples.
    pub samples: Vec<f32>,
}

/// Exact duration of `frames` at `rate` (rounded down to whole ns).
pub fn frames_to_duration(frames: u64, rate: u32) -> Duration {
    if rate == 0 {
        return Duration::ZERO;
    }
    let nanos = u128::from(frames) * 1_000_000_000 / u128::from(rate);
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

/// Whole frames in `duration` at `rate` (rounded down).
pub fn duration_to_frames(duration: Duration, rate: u32) -> u64 {
    let frames = duration.as_nanos() * u128::from(rate) / 1_000_000_000;
    u64::try_from(frames).unwrap_or(u64::MAX)
}

/// Frames in `ms` milliseconds at `rate`, at least 1.
pub fn ms_to_frames(ms: u32, rate: u32) -> u32 {
    let frames = u64::from(ms) * u64::from(rate) / 1000;
    u32::try_from(frames.max(1)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_arithmetic_is_exact_and_saturating() {
        assert_eq!(frames_to_duration(48_000, 48_000), Duration::from_secs(1));
        assert_eq!(frames_to_duration(480, 48_000), Duration::from_millis(10));
        assert_eq!(frames_to_duration(1, 44_100), Duration::from_nanos(22_675));
        assert_eq!(frames_to_duration(10, 0), Duration::ZERO);
        assert_eq!(duration_to_frames(Duration::from_millis(10), 16_000), 160);
        assert_eq!(
            duration_to_frames(Duration::from_nanos(22_675), 44_100),
            0,
            "rounds down"
        );
        assert_eq!(ms_to_frames(10, 48_000), 480);
        assert_eq!(ms_to_frames(0, 48_000), 1, "never zero");
        assert_eq!(
            frames_to_duration(u64::MAX, 1_000),
            Duration::from_nanos(u64::MAX)
        );
        // 24 hours at 768 kHz does not overflow.
        let day = 768_000u64 * 86_400;
        assert_eq!(
            frames_to_duration(day, 768_000),
            Duration::from_secs(86_400)
        );
    }
}
