//! Stream requests, lifecycle, resolved formats and status.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::auth::AuthorizationDenial;
use super::device::AudioDeviceId;
use super::format::{ChannelLayout, SampleFormat, StreamFormat};
use crate::error::{CeError, ErrorReport};
use crate::ids::{AudioStreamId, CorrelationId, SessionId};

/// Direction of a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioDirection {
    /// Device → CE (microphone).
    Capture,
    /// CE → device (speaker).
    Playback,
}

impl AudioDirection {
    /// Stable lowercase name.
    pub const fn as_str(self) -> &'static str {
        match self {
            AudioDirection::Capture => "capture",
            AudioDirection::Playback => "playback",
        }
    }
}

/// Lifecycle of one audio stream.
///
/// ```text
/// Configured ─open─► Open ─start─► Running ─stop─► Stopped ─start─► Running
///      │               │              │              │
///      │               ├──────────────┴──────────────┴──► Invalidated (device lost)
///      │               ├──────────────┴──────────────┴──► Failed
///      └───────────────┴──────────────┴──────────────┴──► Closed (terminal)
/// Invalidated ─► Closed      Failed ─► Closed
/// ```
///
/// * **Configured** — request validated, format negotiated, worker spawned;
///   no device resources yet.
/// * **Open** — device stream built; callbacks not running.
/// * **Running** — callbacks running; data flowing.
/// * **Stopped** — paused after running; device still held; may restart.
/// * **Invalidated** — the device disappeared or the backend invalidated the
///   stream. CE never restarts it silently: the host must close it and open
///   a new stream.
/// * **Failed** — an operation failed or the worker died.
/// * **Closed** — device released and callbacks quiesced (terminal).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioStreamState {
    /// Request accepted; nothing opened yet.
    Configured,
    /// Device stream built, not running.
    Open,
    /// Running.
    Running,
    /// Paused after running.
    Stopped,
    /// Device lost or stream invalidated by the backend.
    Invalidated,
    /// Failed.
    Failed,
    /// Released (terminal).
    Closed,
}

impl AudioStreamState {
    /// All states.
    pub const ALL: [AudioStreamState; 7] = [
        AudioStreamState::Configured,
        AudioStreamState::Open,
        AudioStreamState::Running,
        AudioStreamState::Stopped,
        AudioStreamState::Invalidated,
        AudioStreamState::Failed,
        AudioStreamState::Closed,
    ];

    /// The transition table.
    pub const fn can_transition_to(self, next: AudioStreamState) -> bool {
        use AudioStreamState::*;
        matches!(
            (self, next),
            (Configured, Open)
                | (Configured, Failed)
                | (Configured, Closed)
                | (Open, Running)
                | (Open, Invalidated)
                | (Open, Failed)
                | (Open, Closed)
                | (Running, Stopped)
                | (Running, Invalidated)
                | (Running, Failed)
                | (Running, Closed)
                | (Stopped, Running)
                | (Stopped, Invalidated)
                | (Stopped, Failed)
                | (Stopped, Closed)
                | (Invalidated, Closed)
                | (Failed, Closed)
        )
    }

    /// Validate a transition.
    pub fn transition_to(self, next: AudioStreamState) -> Result<AudioStreamState, CeError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(CeError::lifecycle(
                "lifecycle.illegal_audio_stream_transition",
                "illegal audio stream state transition",
            )
            .with_context("from", self.to_string())
            .with_context("to", next.to_string()))
        }
    }

    /// Whether the stream holds device resources.
    pub const fn holds_device(self) -> bool {
        matches!(
            self,
            AudioStreamState::Open
                | AudioStreamState::Running
                | AudioStreamState::Stopped
                | AudioStreamState::Invalidated
                | AudioStreamState::Failed
        )
    }

    /// Stable lowercase name.
    pub const fn as_str(self) -> &'static str {
        match self {
            AudioStreamState::Configured => "configured",
            AudioStreamState::Open => "open",
            AudioStreamState::Running => "running",
            AudioStreamState::Stopped => "stopped",
            AudioStreamState::Invalidated => "invalidated",
            AudioStreamState::Failed => "failed",
            AudioStreamState::Closed => "closed",
        }
    }
}

impl fmt::Display for AudioStreamState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which device to open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "device")]
pub enum DeviceSelector {
    /// The current default device for the direction.
    Default,
    /// A specific device from the catalog.
    Device(AudioDeviceId),
}

/// How capture channels are delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChannelMode {
    /// Deliver the device's channels unchanged.
    #[default]
    Native,
    /// Average all channels into one (capture only). A plain mean: no
    /// weighting, filtering or gain — not DSP.
    DownmixToMono,
}

/// Optional format preferences. `None` means "use the device default".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct FormatPreference {
    /// Preferred rate in Hz.
    pub sample_rate: Option<u32>,
    /// Preferred device channel count.
    pub channels: Option<u16>,
    /// Preferred native sample format.
    pub sample_format: Option<SampleFormat>,
    /// Preferred frames per device callback.
    pub buffer_frames: Option<u32>,
}

/// A request to open a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamRequest {
    /// Capture or playback.
    pub direction: AudioDirection,
    /// Which device.
    pub device: DeviceSelector,
    /// Format preferences.
    #[serde(default)]
    pub format: FormatPreference,
    /// Channel delivery (capture only; must be `Native` for playback).
    #[serde(default)]
    pub channel_mode: ChannelMode,
}

impl StreamRequest {
    /// Capture from the default device with device-default format.
    pub fn capture() -> Self {
        Self {
            direction: AudioDirection::Capture,
            device: DeviceSelector::Default,
            format: FormatPreference::default(),
            channel_mode: ChannelMode::Native,
        }
    }

    /// Playback to the default device with device-default format.
    pub fn playback() -> Self {
        Self {
            direction: AudioDirection::Playback,
            ..Self::capture()
        }
    }

    /// Validate field ranges and combinations.
    pub fn validate(&self) -> Result<(), CeError> {
        let bad = |code: &'static str, msg: &'static str| CeError::invalid_input(code, msg);
        if self.direction == AudioDirection::Playback && self.channel_mode != ChannelMode::Native {
            return Err(bad(
                "audio.invalid_channel_mode",
                "channel conversion applies to capture only",
            ));
        }
        if let Some(rate) = self.format.sample_rate {
            if !(super::format::MIN_SAMPLE_RATE..=super::format::MAX_SAMPLE_RATE).contains(&rate) {
                return Err(bad(
                    "audio.invalid_sample_rate",
                    "sample rate outside the supported range",
                )
                .with_context("sample_rate", rate.to_string()));
            }
        }
        if let Some(ch) = self.format.channels {
            if ch == 0 || ch > super::format::MAX_CHANNELS {
                return Err(bad(
                    "audio.invalid_channels",
                    "channel count outside the supported range",
                )
                .with_context("channels", ch.to_string()));
            }
        }
        if let Some(frames) = self.format.buffer_frames {
            if frames == 0 || frames > MAX_BUFFER_FRAMES {
                return Err(bad(
                    "audio.invalid_buffer_frames",
                    "buffer size outside the supported range",
                )
                .with_context("buffer_frames", frames.to_string()));
            }
        }
        Ok(())
    }
}

/// Largest buffer (frames per callback) a request may ask for.
pub const MAX_BUFFER_FRAMES: u32 = 192_000;

/// A difference between what was requested and what was resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "field")]
pub enum FormatAdjustment {
    /// Sample rate differs.
    SampleRate {
        /// Requested.
        requested: u32,
        /// Resolved.
        resolved: u32,
    },
    /// Device channel count differs.
    Channels {
        /// Requested.
        requested: u16,
        /// Resolved.
        resolved: u16,
    },
    /// Native sample format differs.
    SampleFormat {
        /// Requested.
        requested: SampleFormat,
        /// Resolved.
        resolved: SampleFormat,
    },
    /// Buffer size differs or could not be honoured.
    BufferFrames {
        /// Requested.
        requested: u32,
        /// Resolved (`None` = backend default, actual size unknown before open).
        resolved: Option<u32>,
    },
}

/// What CE hands to (capture) or accepts from (playback) the host:
/// always interleaved `f32` at the device's sample rate (no resampling in
/// Phase 2; see ADR-0012).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveredFormat {
    /// Frames per second (equals the device rate).
    pub sample_rate: u32,
    /// Channels per frame.
    pub channels: u16,
    /// Layout.
    pub layout: ChannelLayout,
    /// Always [`SampleFormat::F32`].
    pub sample_format: SampleFormat,
}

/// The outcome of negotiation, refined after the device opened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedFormat {
    /// The requested preferences.
    pub requested: FormatPreference,
    /// The device-side format actually opened.
    pub device: StreamFormat,
    /// The CE-side format delivered/accepted.
    pub delivered: DeliveredFormat,
    /// Buffer size asked of the backend (`None` = backend default).
    pub requested_buffer_frames: Option<u32>,
    /// Frames per callback reported by the backend after open, if known.
    pub actual_buffer_frames: Option<u32>,
    /// Every field that differs from the request.
    pub adjustments: Vec<FormatAdjustment>,
}

impl ResolvedFormat {
    /// Whether every stated preference was honoured exactly.
    pub fn is_exact(&self) -> bool {
        self.adjustments.is_empty()
    }
}

/// Static facts about an opened stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamInfo {
    /// Stream id.
    pub id: AudioStreamId,
    /// Owning session.
    pub session_id: SessionId,
    /// The session's correlation id.
    pub correlation_id: CorrelationId,
    /// Direction.
    pub direction: AudioDirection,
    /// The device opened.
    pub device: AudioDeviceId,
    /// Whether the stream was opened on the default device.
    pub opened_on_default: bool,
    /// The original request.
    pub request: StreamRequest,
    /// Resolved formats.
    pub format: ResolvedFormat,
    /// Frames per delivered capture block / maximum frames per playback write.
    pub block_frames: u32,
}

/// Running counters for one stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StreamCounters {
    /// Device callbacks observed.
    pub callbacks: u64,
    /// Blocks delivered (capture) or accepted (playback).
    pub blocks: u64,
    /// Frames delivered (capture) or played from host data (playback).
    pub frames: u64,
    /// Frames lost because the device ring was full (capture).
    pub ring_overflow_frames: u64,
    /// Blocks dropped because the delivery queue was full (capture).
    pub queue_dropped_blocks: u64,
    /// Frames in those dropped blocks.
    pub queue_dropped_frames: u64,
    /// Frames of silence inserted because no data was ready (playback).
    pub underrun_frames: u64,
    /// Gaps produced: device-ring overflows, restarts and delivery-queue
    /// drops. Adjacent gaps can merge into one flagged block, so delivered
    /// flagged blocks never exceed this count.
    pub discontinuities: u64,
    /// Over/underruns reported by the backend itself.
    pub backend_xruns: u64,
    /// Highest device-ring fill observed, in permille of capacity.
    pub max_ring_fill_permille: u16,
}

/// Why a stream ended or became unusable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "reason", content = "detail")]
pub enum TerminalReason {
    /// Closed by the host or by shutdown/session close.
    Closed,
    /// Capture authorization was withdrawn or became invalid.
    Revoked(AuthorizationDenial),
    /// The device or stream became invalid.
    Invalidated(InvalidationReason),
    /// An operation failed.
    Failed(ErrorReport),
}

/// Why a stream was invalidated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvalidationReason {
    /// The backend reported the device unavailable.
    DeviceDisconnected,
    /// The backend invalidated the stream (e.g. the format changed).
    StreamInvalidated,
    /// Re-enumeration no longer lists the device.
    DeviceRemoved,
}

/// Snapshot of one stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioStreamStatus {
    /// Static facts.
    pub info: StreamInfo,
    /// Lifecycle state.
    pub state: AudioStreamState,
    /// Counters.
    pub counters: StreamCounters,
    /// Why it ended, if it has.
    pub terminal: Option<TerminalReason>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGAL: &[(AudioStreamState, AudioStreamState)] = {
        use AudioStreamState::*;
        &[
            (Configured, Open),
            (Configured, Failed),
            (Configured, Closed),
            (Open, Running),
            (Open, Invalidated),
            (Open, Failed),
            (Open, Closed),
            (Running, Stopped),
            (Running, Invalidated),
            (Running, Failed),
            (Running, Closed),
            (Stopped, Running),
            (Stopped, Invalidated),
            (Stopped, Failed),
            (Stopped, Closed),
            (Invalidated, Closed),
            (Failed, Closed),
        ]
    };

    #[test]
    fn stream_transition_matrix_is_exactly_the_documented_one() {
        for from in AudioStreamState::ALL {
            for to in AudioStreamState::ALL {
                let expected = LEGAL.contains(&(from, to));
                assert_eq!(from.can_transition_to(to), expected, "{from} -> {to}");
                match from.transition_to(to) {
                    Ok(s) => assert_eq!(s, to),
                    Err(e) => assert_eq!(e.code(), "lifecycle.illegal_audio_stream_transition"),
                }
            }
        }
        // Double stop and double close are illegal, and Invalidated never
        // returns to Running (no silent restart).
        use AudioStreamState::*;
        assert!(!Stopped.can_transition_to(Stopped));
        assert!(!Closed.can_transition_to(Closed));
        assert!(!Invalidated.can_transition_to(Running));
        assert!(Running.holds_device() && !Configured.holds_device() && !Closed.holds_device());
    }

    #[test]
    fn requests_are_validated() {
        assert!(StreamRequest::capture().validate().is_ok());
        let mut r = StreamRequest::playback();
        r.channel_mode = ChannelMode::DownmixToMono;
        assert_eq!(
            r.validate().unwrap_err().code(),
            "audio.invalid_channel_mode"
        );
        for (rate, ch, buf, code) in [
            (Some(10), None, None, "audio.invalid_sample_rate"),
            (None, Some(0), None, "audio.invalid_channels"),
            (None, Some(64), None, "audio.invalid_channels"),
            (None, None, Some(0), "audio.invalid_buffer_frames"),
            (
                None,
                None,
                Some(MAX_BUFFER_FRAMES + 1),
                "audio.invalid_buffer_frames",
            ),
        ] {
            let mut r = StreamRequest::capture();
            r.format = FormatPreference {
                sample_rate: rate,
                channels: ch,
                buffer_frames: buf,
                sample_format: None,
            };
            assert_eq!(r.validate().unwrap_err().code(), code);
        }
        let json = serde_json::to_string(&StreamRequest::capture()).unwrap();
        assert_eq!(
            serde_json::from_str::<StreamRequest>(&json).unwrap(),
            StreamRequest::capture()
        );
    }
}
