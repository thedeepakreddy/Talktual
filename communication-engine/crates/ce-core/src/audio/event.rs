//! Audio event payloads, carried as `EventPayload::Audio`.
//!
//! All fields are operational: device names never appear (only opaque
//! [`AudioDeviceId`]s) and audio content never appears. Stream events are
//! dispatched with the stream's session id and correlation id on the
//! envelope.

use serde::{Deserialize, Serialize};

use super::auth::AuthorizationDenial;
use super::device::{AudioDeviceId, PlatformPermission};
use super::frame::DiscontinuityKind;
use super::stream::{AudioDirection, AudioStreamState, InvalidationReason, ResolvedFormat};
use crate::error::ErrorReport;
use crate::ids::AudioStreamId;

/// Where frames were lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverflowStage {
    /// Real-time ring between the device callback and the stream worker.
    DeviceRing,
    /// Bounded block queue between the worker and the host reader.
    DeliveryQueue,
}

/// Audio events. Non-exhaustive: additions are compatible.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "audio_event")]
#[non_exhaustive]
pub enum AudioEvent {
    /// The device catalog was (re-)enumerated.
    DevicesEnumerated {
        /// Input-capable devices.
        inputs: u32,
        /// Output-capable devices.
        outputs: u32,
    },
    /// Enumeration failed entirely.
    EnumerationFailed {
        /// Redacted error.
        error: ErrorReport,
    },
    /// A device appeared.
    DeviceAdded {
        /// Device.
        device: AudioDeviceId,
    },
    /// A device disappeared.
    DeviceRemoved {
        /// Device.
        device: AudioDeviceId,
    },
    /// The default device for a direction changed.
    DefaultDeviceChanged {
        /// Direction.
        direction: AudioDirection,
        /// Previous default.
        from: Option<AudioDeviceId>,
        /// New default.
        to: Option<AudioDeviceId>,
    },
    /// The platform capture permission changed.
    PermissionChanged {
        /// Previous state.
        from: PlatformPermission,
        /// New state.
        to: PlatformPermission,
    },
    /// A stream changed lifecycle state. `from` is `None` for creation.
    StreamStateChanged {
        /// Stream.
        stream: AudioStreamId,
        /// Direction.
        direction: AudioDirection,
        /// Previous state.
        from: Option<AudioStreamState>,
        /// New state.
        to: AudioStreamState,
    },
    /// The format of a stream was resolved (after open, with the actual
    /// buffer size when the backend reports one).
    StreamFormatResolved {
        /// Stream.
        stream: AudioStreamId,
        /// Device.
        device: AudioDeviceId,
        /// Requested and resolved formats.
        format: ResolvedFormat,
    },
    /// The backend rerouted a default-device stream to a new device. The
    /// stream keeps running; no CE restart happened.
    StreamRerouted {
        /// Stream.
        stream: AudioStreamId,
    },
    /// The default device changed while a stream opened on the default is
    /// still bound to the previous device. CE does not move it; the host
    /// decides whether to reopen.
    StreamDefaultDeviceDiverged {
        /// Stream.
        stream: AudioStreamId,
        /// The device the stream is still on.
        bound_to: AudioDeviceId,
        /// The new default.
        new_default: Option<AudioDeviceId>,
    },
    /// The stream can no longer run. Terminal until closed.
    StreamInvalidated {
        /// Stream.
        stream: AudioStreamId,
        /// Why.
        reason: InvalidationReason,
    },
    /// Frames were lost (coalesced: at most once per pressure interval).
    Overflow {
        /// Stream.
        stream: AudioStreamId,
        /// Where.
        stage: OverflowStage,
        /// Frames lost at this stage so far.
        lost_frames_total: u64,
    },
    /// Playback inserted silence (coalesced).
    Underflow {
        /// Stream.
        stream: AudioStreamId,
        /// Silent frames inserted so far.
        missing_frames_total: u64,
    },
    /// A delivered block carried a discontinuity (coalesced: the first per
    /// pressure interval, with the running total).
    Discontinuity {
        /// Stream.
        stream: AudioStreamId,
        /// Sequence of the flagged block.
        sequence: u64,
        /// Kind.
        kind: DiscontinuityKind,
        /// Frames lost, when known.
        lost_frames: Option<u64>,
        /// Discontinuities so far.
        total: u64,
    },
    /// A stream operation or worker failed.
    StreamFailed {
        /// Stream.
        stream: AudioStreamId,
        /// Redacted error.
        error: ErrorReport,
        /// Whether the stream is now unusable (must be closed).
        terminal: bool,
    },
    /// Capture was refused by the authorization gate (nothing was opened or
    /// started).
    CaptureDenied {
        /// Stream, if one was already configured.
        stream: Option<AudioStreamId>,
        /// Why.
        denial: AuthorizationDenial,
    },
    /// Authorization was lost while a capture stream existed; the stream was
    /// stopped and its device released.
    CaptureRevoked {
        /// Stream.
        stream: AudioStreamId,
        /// Why.
        denial: AuthorizationDenial,
    },
}

impl AudioEvent {
    /// Stable kind name, safe to log.
    pub fn kind(&self) -> &'static str {
        match self {
            AudioEvent::DevicesEnumerated { .. } => "devices_enumerated",
            AudioEvent::EnumerationFailed { .. } => "enumeration_failed",
            AudioEvent::DeviceAdded { .. } => "device_added",
            AudioEvent::DeviceRemoved { .. } => "device_removed",
            AudioEvent::DefaultDeviceChanged { .. } => "default_device_changed",
            AudioEvent::PermissionChanged { .. } => "permission_changed",
            AudioEvent::StreamStateChanged { .. } => "stream_state_changed",
            AudioEvent::StreamFormatResolved { .. } => "stream_format_resolved",
            AudioEvent::StreamRerouted { .. } => "stream_rerouted",
            AudioEvent::StreamDefaultDeviceDiverged { .. } => "stream_default_device_diverged",
            AudioEvent::StreamInvalidated { .. } => "stream_invalidated",
            AudioEvent::Overflow { .. } => "overflow",
            AudioEvent::Underflow { .. } => "underflow",
            AudioEvent::Discontinuity { .. } => "discontinuity",
            AudioEvent::StreamFailed { .. } => "stream_failed",
            AudioEvent::CaptureDenied { .. } => "capture_denied",
            AudioEvent::CaptureRevoked { .. } => "capture_revoked",
        }
    }
}
