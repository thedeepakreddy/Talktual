//! Format negotiation: requested preferences + device capabilities →
//! resolved format. Pure and deterministic.
//!
//! Policy (ADR-0012), applied to the device's supported ranges:
//!
//! 1. Targets: each preference, else the device default, else 48 kHz /
//!    the device default channel count (else 1) / `f32`.
//! 2. Rank candidate ranges lexicographically by
//!    (a) whether the target rate lies in the range, then distance to it;
//!    (b) channel distance from the target;
//!    (c) sample format: exact target first, then CE's preference order.
//!    Rate is weighted first because changing it later needs resampling,
//!    which is lossy and out of Phase 2 scope; channel and format
//!    conversion are cheap and exact enough.
//! 3. The resolved rate is the target clamped into the chosen range.
//! 4. Every honoured-or-not preference is reported in `adjustments`.

use super::format::{
    BufferSizeRange, ChannelLayout, SampleFormat, StreamFormat, SupportedFormatRange,
};
use super::stream::{
    ChannelMode, DeliveredFormat, FormatAdjustment, FormatPreference, ResolvedFormat,
};
use crate::error::{CeError, ErrorCategory};

/// Rate used when neither the request nor the device states one.
pub const FALLBACK_SAMPLE_RATE: u32 = 48_000;

/// Negotiate a format.
///
/// `supported` must already be sanitized (see
/// [`SupportedFormatRange::sanitized`]). Fails with a `Device` error when the
/// device offers no usable format.
pub fn negotiate(
    preference: &FormatPreference,
    channel_mode: ChannelMode,
    supported: &[SupportedFormatRange],
    device_default: Option<&StreamFormat>,
) -> Result<ResolvedFormat, CeError> {
    if supported.is_empty() {
        return Err(CeError::new(
            ErrorCategory::Device,
            "audio.no_supported_formats",
            "the device reports no format CE can use",
        ));
    }
    let target_rate = preference
        .sample_rate
        .or(device_default.map(|d| d.sample_rate))
        .unwrap_or(FALLBACK_SAMPLE_RATE);
    let target_channels = preference
        .channels
        .or(device_default.map(|d| d.channels))
        .unwrap_or(1);
    let target_format = preference
        .sample_format
        .or(device_default.map(|d| d.sample_format))
        .unwrap_or(SampleFormat::F32);

    let score = |r: &SupportedFormatRange| {
        let rate_distance = if r.contains_rate(target_rate) {
            0
        } else if target_rate < r.min_sample_rate {
            r.min_sample_rate - target_rate
        } else {
            target_rate - r.max_sample_rate
        };
        let channel_distance = r.channels.abs_diff(target_channels);
        let format_rank = if r.sample_format == target_format {
            0
        } else {
            1 + r.sample_format.preference_rank()
        };
        (rate_distance, channel_distance, format_rank)
    };
    let chosen = supported
        .iter()
        .min_by_key(|r| score(r))
        .copied()
        .ok_or_else(|| CeError::internal("audio.negotiation_empty", "no candidate format"))?;

    let rate = target_rate.clamp(chosen.min_sample_rate, chosen.max_sample_rate);
    let device = StreamFormat::new(rate, chosen.channels, chosen.sample_format)?;

    let buffer_frames = preference.buffer_frames.map(|want| match chosen.buffer {
        BufferSizeRange::Range { min, max } => want.clamp(min, max),
        BufferSizeRange::Unknown => want,
    });

    let mut adjustments = Vec::new();
    if let Some(requested) = preference.sample_rate {
        if requested != rate {
            adjustments.push(FormatAdjustment::SampleRate {
                requested,
                resolved: rate,
            });
        }
    }
    if let Some(requested) = preference.channels {
        if requested != chosen.channels {
            adjustments.push(FormatAdjustment::Channels {
                requested,
                resolved: chosen.channels,
            });
        }
    }
    if let Some(requested) = preference.sample_format {
        if requested != chosen.sample_format {
            adjustments.push(FormatAdjustment::SampleFormat {
                requested,
                resolved: chosen.sample_format,
            });
        }
    }
    if let (Some(requested), Some(resolved)) = (preference.buffer_frames, buffer_frames) {
        if requested != resolved {
            adjustments.push(FormatAdjustment::BufferFrames {
                requested,
                resolved: Some(resolved),
            });
        }
    }

    let delivered_channels = match channel_mode {
        ChannelMode::Native => device.channels,
        ChannelMode::DownmixToMono => 1,
    };
    Ok(ResolvedFormat {
        requested: *preference,
        device,
        delivered: DeliveredFormat {
            sample_rate: device.sample_rate,
            channels: delivered_channels,
            layout: ChannelLayout::from_channels(delivered_channels),
            sample_format: SampleFormat::F32,
        },
        requested_buffer_frames: buffer_frames,
        actual_buffer_frames: None,
        adjustments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(ch: u16, min: u32, max: u32, fmt: SampleFormat) -> SupportedFormatRange {
        SupportedFormatRange {
            channels: ch,
            min_sample_rate: min,
            max_sample_rate: max,
            sample_format: fmt,
            buffer: BufferSizeRange::Range { min: 64, max: 4096 },
        }
    }

    fn pref(rate: Option<u32>, ch: Option<u16>, fmt: Option<SampleFormat>) -> FormatPreference {
        FormatPreference {
            sample_rate: rate,
            channels: ch,
            sample_format: fmt,
            buffer_frames: None,
        }
    }

    #[test]
    fn exact_requests_are_honoured_and_reported_exact() {
        let ranges = [
            range(2, 44_100, 48_000, SampleFormat::I16),
            range(1, 8_000, 48_000, SampleFormat::F32),
        ];
        let r = negotiate(
            &pref(Some(16_000), Some(1), Some(SampleFormat::F32)),
            ChannelMode::Native,
            &ranges,
            None,
        )
        .unwrap();
        assert_eq!(
            r.device,
            StreamFormat::new(16_000, 1, SampleFormat::F32).unwrap()
        );
        assert!(r.is_exact());
        assert_eq!(r.delivered.sample_format, SampleFormat::F32);
    }

    #[test]
    fn rate_is_prioritised_and_every_deviation_is_reported() {
        // 16 kHz is only available as stereo i16; mono f32 only at 48 kHz.
        let ranges = [
            range(2, 8_000, 16_000, SampleFormat::I16),
            range(1, 48_000, 48_000, SampleFormat::F32),
        ];
        let r = negotiate(
            &pref(Some(16_000), Some(1), Some(SampleFormat::F32)),
            ChannelMode::DownmixToMono,
            &ranges,
            None,
        )
        .unwrap();
        assert_eq!(
            r.device,
            StreamFormat::new(16_000, 2, SampleFormat::I16).unwrap()
        );
        assert_eq!(
            r.adjustments,
            vec![
                FormatAdjustment::Channels {
                    requested: 1,
                    resolved: 2
                },
                FormatAdjustment::SampleFormat {
                    requested: SampleFormat::F32,
                    resolved: SampleFormat::I16
                },
            ]
        );
        assert_eq!(
            (r.delivered.channels, r.delivered.layout),
            (1, ChannelLayout::Mono),
            "downmix delivered"
        );
    }

    #[test]
    fn unreachable_rates_are_clamped_to_the_nearest_supported_rate() {
        let ranges = [range(1, 44_100, 48_000, SampleFormat::I16)];
        let r = negotiate(
            &pref(Some(8_000), None, None),
            ChannelMode::Native,
            &ranges,
            None,
        )
        .unwrap();
        assert_eq!(r.device.sample_rate, 44_100);
        assert_eq!(
            r.adjustments,
            vec![FormatAdjustment::SampleRate {
                requested: 8_000,
                resolved: 44_100
            }]
        );
        let r = negotiate(
            &pref(Some(96_000), None, None),
            ChannelMode::Native,
            &ranges,
            None,
        )
        .unwrap();
        assert_eq!(r.device.sample_rate, 48_000);
    }

    #[test]
    fn defaults_come_from_the_device_then_fallbacks() {
        let ranges = [
            range(1, 8_000, 96_000, SampleFormat::F32),
            range(2, 8_000, 96_000, SampleFormat::I16),
        ];
        let default = StreamFormat::new(44_100, 2, SampleFormat::I16).unwrap();
        let r = negotiate(
            &FormatPreference::default(),
            ChannelMode::Native,
            &ranges,
            Some(&default),
        )
        .unwrap();
        assert_eq!(r.device, default);
        assert!(r.is_exact(), "no preferences stated, so nothing deviates");
        let r = negotiate(
            &FormatPreference::default(),
            ChannelMode::Native,
            &ranges,
            None,
        )
        .unwrap();
        assert_eq!(
            r.device,
            StreamFormat::new(FALLBACK_SAMPLE_RATE, 1, SampleFormat::F32).unwrap()
        );
    }

    #[test]
    fn format_ties_follow_preference_order_and_channels_are_not_assumed() {
        let ranges = [
            range(6, 48_000, 48_000, SampleFormat::U8),
            range(6, 48_000, 48_000, SampleFormat::I32),
            range(6, 48_000, 48_000, SampleFormat::I16),
        ];
        let r = negotiate(
            &pref(Some(48_000), Some(6), Some(SampleFormat::F32)),
            ChannelMode::Native,
            &ranges,
            None,
        )
        .unwrap();
        assert_eq!(r.device.sample_format, SampleFormat::I16);
        assert_eq!(r.delivered.layout, ChannelLayout::Discrete(6));
    }

    #[test]
    fn buffer_requests_are_clamped_and_reported() {
        let ranges = [range(1, 48_000, 48_000, SampleFormat::F32)];
        let mut p = pref(None, None, None);
        p.buffer_frames = Some(10);
        let r = negotiate(&p, ChannelMode::Native, &ranges, None).unwrap();
        assert_eq!(r.requested_buffer_frames, Some(64));
        assert_eq!(
            r.adjustments,
            vec![FormatAdjustment::BufferFrames {
                requested: 10,
                resolved: Some(64)
            }]
        );
    }

    #[test]
    fn no_formats_is_a_device_error() {
        let err =
            negotiate(&FormatPreference::default(), ChannelMode::Native, &[], None).unwrap_err();
        assert_eq!(
            (err.category(), err.code()),
            (ErrorCategory::Device, "audio.no_supported_formats")
        );
    }
}
