//! Audio contracts (Phase 2).
//!
//! Backend-independent data model for audio capture and playback:
//!
//! * [`format`](mod@format) — sample formats, channel layouts, stream formats and the
//!   ranges a device supports;
//! * [`device`] — device descriptors, the device catalog, backend info and
//!   platform permission state;
//! * [`stream`] — stream requests, the stream lifecycle table, resolved
//!   formats, counters and status;
//! * [`negotiate`](mod@negotiate) — pure format negotiation (requested → resolved);
//! * [`frame`] — frame/block metadata, timestamps and discontinuities;
//! * [`auth`] — the capture authorization gate (consent + permission);
//! * [`event`] — audio event payloads;
//! * [`config`] — audio configuration.
//!
//! Nothing here depends on an audio library or async runtime. Backend
//! adapters and the real-time handoff live in the `ce-audio` crate. See
//! `docs/architecture/audio.md`.

pub mod auth;
pub mod config;
pub mod device;
pub mod event;
pub mod format;
pub mod frame;
pub mod negotiate;
pub mod stream;

pub use auth::{AuthorizationDenial, CaptureAuthorization, evaluate_capture_authorization};
pub use config::{AudioConfig, DeliveryOverflowPolicy};
pub use device::{
    AudioDeviceDescriptor, AudioDeviceId, BackendInfo, DeviceCatalog, IdStability,
    PlatformPermission,
};
pub use event::{AudioEvent, OverflowStage};
pub use format::{
    BufferSizeRange, ChannelLayout, SampleFormat, StreamFormat, SupportedFormatRange,
};
pub use frame::{AudioBlock, AudioFrameMeta, Discontinuity, DiscontinuityKind};
pub use negotiate::negotiate;
pub use stream::{
    AudioDirection, AudioStreamState, AudioStreamStatus, ChannelMode, DeliveredFormat,
    DeviceSelector, FormatAdjustment, FormatPreference, InvalidationReason, ResolvedFormat,
    StreamCounters, StreamInfo, StreamRequest, TerminalReason,
};
