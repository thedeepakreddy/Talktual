//! The backend adapter boundary.
//!
//! An adapter wraps one audio library or platform API and exposes it through
//! CE-owned types only. Adapters never see sessions, consent or events, and
//! hosts never see adapter types.
//!
//! ## Threading contract
//!
//! * `enumerate` and `open_*` may block (device I/O). The runtime calls them
//!   off the control-plane task: enumeration on a blocking thread, opens on
//!   the stream's worker thread.
//! * A [`BackendStream`] is created, started, stopped and dropped on its
//!   stream's worker thread; it need not be `Send`.
//! * **Callback quiescence:** when a `BackendStream` is dropped, the adapter
//!   must guarantee that the device callback will not run again and that the
//!   endpoint it was given has been released, before `drop` returns.
//! * Device callbacks may only call the endpoint methods (and
//!   [`crate::FaultReporter`]); see `docs/architecture/realtime.md`.

use std::fmt;

use ce_core::CeError;
use ce_core::audio::{
    BackendInfo, IdStability, PlatformPermission, StreamFormat, SupportedFormatRange,
};
use tokio::sync::mpsc;

use crate::rt::{CaptureEndpoint, PlaybackEndpoint};
use crate::worker::WorkerReport;

/// A device as an adapter reports it. The runtime maps `key` to an opaque
/// CE device id and sanitizes the format ranges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendDevice {
    /// Backend-specific key used to reopen the device (may be personal data).
    pub key: String,
    /// Human-readable name (personal data).
    pub name: String,
    /// How stable `key` is.
    pub key_stability: IdStability,
    /// Capture supported.
    pub supports_input: bool,
    /// Playback supported.
    pub supports_output: bool,
    /// Current default input.
    pub is_default_input: bool,
    /// Current default output.
    pub is_default_output: bool,
    /// Capture format ranges (unsanitized).
    pub input_formats: Vec<SupportedFormatRange>,
    /// Playback format ranges (unsanitized).
    pub output_formats: Vec<SupportedFormatRange>,
    /// Preferred capture format.
    pub default_input_format: Option<StreamFormat>,
    /// Preferred playback format.
    pub default_output_format: Option<StreamFormat>,
    /// Native formats skipped because CE cannot represent them.
    pub skipped_formats: u32,
}

/// Result of an enumeration: devices plus non-fatal per-device problems.
#[derive(Clone, Debug, Default)]
pub struct BackendEnumeration {
    /// Devices.
    pub devices: Vec<BackendDevice>,
    /// Problems (devices skipped, partial failures).
    pub problems: Vec<CeError>,
}

/// What to open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenSpec {
    /// Device key; `None` opens the backend's current default.
    pub device_key: Option<String>,
    /// Exact device-side format (already negotiated).
    pub format: StreamFormat,
    /// Frames per callback; `None` = backend default.
    pub buffer_frames: Option<u32>,
}

/// A device stream owned by a worker thread.
pub trait BackendStream {
    /// Start (or resume) callbacks.
    fn start(&mut self) -> Result<(), CeError>;
    /// Pause callbacks; the device stays open.
    fn stop(&mut self) -> Result<(), CeError>;
    /// Frames per callback as reported by the backend, if known.
    fn buffer_frames(&self) -> Option<u32>;
}

/// Unsolicited notices from an adapter (hotplug, permission changes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendNotice {
    /// The device list or a default device may have changed; the runtime
    /// re-enumerates and diffs.
    DevicesChanged,
    /// The platform capture permission changed.
    PermissionChanged(PlatformPermission),
}

/// Everything that flows from the audio layer into the runtime core.
#[derive(Debug)]
pub enum AudioSignal {
    /// From a stream worker.
    Worker(WorkerReport),
    /// From the backend adapter.
    Backend(BackendNotice),
}

/// Handle an adapter uses to push [`BackendNotice`]s. Never blocks: notices
/// are idempotent hints, so a full queue drops them (the runtime also polls).
#[derive(Clone, Debug)]
pub struct BackendNotifier {
    tx: mpsc::Sender<AudioSignal>,
}

impl BackendNotifier {
    /// Wrap a sender (used by the runtime).
    pub fn new(tx: mpsc::Sender<AudioSignal>) -> Self {
        Self { tx }
    }

    /// Push a notice; returns false if it was dropped.
    pub fn notify(&self, notice: BackendNotice) -> bool {
        self.tx.try_send(AudioSignal::Backend(notice)).is_ok()
    }
}

/// An audio backend adapter.
pub trait AudioBackend: Send + Sync + fmt::Debug + 'static {
    /// Static facts.
    fn info(&self) -> BackendInfo;

    /// List devices. May block.
    fn enumerate(&self) -> Result<BackendEnumeration, CeError>;

    /// The platform capture permission as far as this backend can tell.
    /// Adapters that cannot query it must return
    /// [`PlatformPermission::Unknown`] (which denies capture) unless the
    /// platform has no permission model, in which case
    /// [`PlatformPermission::NotRequiredByPlatform`].
    fn capture_permission(&self) -> PlatformPermission;

    /// Receive a notifier for unsolicited notices. Default: ignored
    /// (backends without hotplug support).
    fn set_notifier(&self, notifier: BackendNotifier) {
        let _ = notifier;
    }

    /// Build a capture stream (not started). May block.
    fn open_capture(
        &self,
        spec: &OpenSpec,
        endpoint: CaptureEndpoint,
    ) -> Result<Box<dyn BackendStream>, CeError>;

    /// Build a playback stream (not started). May block.
    fn open_playback(
        &self,
        spec: &OpenSpec,
        endpoint: PlaybackEndpoint,
    ) -> Result<Box<dyn BackendStream>, CeError>;
}

/// Wraps a backend and substitutes a host-supplied permission source — the
/// way a native shell reports the platform's microphone authorization (e.g.
/// from `AVAudioSession` or Android's `RECORD_AUDIO`), which audio libraries
/// typically cannot query.
pub struct PermissionOverride<B> {
    inner: B,
    source: Box<dyn Fn() -> PlatformPermission + Send + Sync>,
}

impl<B> PermissionOverride<B> {
    /// Use `source` for the capture permission.
    pub fn new(inner: B, source: impl Fn() -> PlatformPermission + Send + Sync + 'static) -> Self {
        Self {
            inner,
            source: Box::new(source),
        }
    }
}

impl<B: fmt::Debug> fmt::Debug for PermissionOverride<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PermissionOverride")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

impl<B: AudioBackend> AudioBackend for PermissionOverride<B> {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            permission_query: true,
            ..self.inner.info()
        }
    }
    fn enumerate(&self) -> Result<BackendEnumeration, CeError> {
        self.inner.enumerate()
    }
    fn capture_permission(&self) -> PlatformPermission {
        (self.source)()
    }
    fn set_notifier(&self, notifier: BackendNotifier) {
        self.inner.set_notifier(notifier);
    }
    fn open_capture(
        &self,
        spec: &OpenSpec,
        endpoint: CaptureEndpoint,
    ) -> Result<Box<dyn BackendStream>, CeError> {
        self.inner.open_capture(spec, endpoint)
    }
    fn open_playback(
        &self,
        spec: &OpenSpec,
        endpoint: PlaybackEndpoint,
    ) -> Result<Box<dyn BackendStream>, CeError> {
        self.inner.open_playback(spec, endpoint)
    }
}
