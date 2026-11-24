//! Audio orchestration inside the runtime core (Phase 2).
//!
//! The core is the single owner of stream state, the device catalog and the
//! capture authorization gate, so audio events share the runtime's one
//! sequence and audio state is never mutated concurrently. Device work runs
//! on stream worker threads (`ce-audio`); enumeration runs on a tracked
//! blocking task. The core waits on both with deadlines while it keeps
//! draining ingress, so nothing here can stall the control plane forever.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use ce_audio::{
    AudioBackend, AudioSignal, BackendEnumeration, OpenSpec, StreamContext, StreamEndpoint,
    StreamFault, WorkerCommand, WorkerConfig, WorkerHandle, WorkerReport, spawn_stream,
};
use ce_core::CeResult;
use ce_core::audio::frame::ms_to_frames;
use ce_core::audio::{
    AudioDeviceDescriptor, AudioDeviceId, AudioDirection, AudioEvent, AudioStreamState,
    AudioStreamStatus, AuthorizationDenial, CaptureAuthorization, DeviceCatalog, DeviceSelector,
    FormatAdjustment, InvalidationReason, OverflowStage, PlatformPermission, StreamCounters,
    StreamInfo, StreamRequest, SupportedFormatRange, TerminalReason,
    evaluate_capture_authorization, negotiate,
};
use ce_core::command::CommandOutcome;
use ce_core::error::{CeError, ErrorCategory};
use ce_core::event::EventPayload;
use ce_core::ids::{AudioStreamId, SessionId};
use ce_core::observability::{Level, Observation};
use ce_core::privacy::{ConsentPurpose, ConsentState};
use ce_core::session::SessionState;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::{Core, WaitError};
use crate::runtime::metrics::Metrics;

/// Capacity of the channel carrying worker reports and backend notices.
pub(crate) const AUDIO_SIGNAL_CAPACITY: usize = 256;
/// How long shutdown waits for stuck worker threads in total.
const ZOMBIE_POLL: Duration = Duration::from_millis(10);

/// What `open_audio` hands back to the facade.
#[derive(Debug)]
pub(crate) struct OpenedAudio {
    pub(crate) endpoint: StreamEndpoint,
}

struct StreamSlot {
    info: StreamInfo,
    state: AudioStreamState,
    handle: WorkerHandle,
    counters: StreamCounters,
    terminal: Option<TerminalReason>,
    /// The device the stream is bound to diverged from the default (reported once).
    divergence_reported: bool,
}

impl std::fmt::Debug for StreamSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamSlot")
            .field("id", &self.info.id)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// Audio state owned by the core.
pub(crate) struct AudioState {
    backend: Arc<dyn AudioBackend>,
    signal_tx: mpsc::Sender<AudioSignal>,
    streams: BTreeMap<AudioStreamId, StreamSlot>,
    /// Worker threads that did not exit in time; joined when they finish.
    zombies: Vec<WorkerHandle>,
    catalog: Option<DeviceCatalog>,
    /// Backend key → CE id; never reused for another key.
    key_map: HashMap<String, AudioDeviceId>,
    keys: HashMap<AudioDeviceId, String>,
    next_device_id: u32,
    permission: PlatformPermission,
    enumerations: Vec<JoinHandle<()>>,
    catalog_dirty: bool,
    last_device_poll: Instant,
    /// Revocations/failures detected in synchronous handlers, executed by
    /// the next async step.
    deferred_close: Vec<(AudioStreamId, TerminalReason)>,
}

impl std::fmt::Debug for AudioState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioState")
            .field("streams", &self.streams.len())
            .finish_non_exhaustive()
    }
}

impl AudioState {
    pub(crate) fn new(
        backend: Arc<dyn AudioBackend>,
        signal_tx: mpsc::Sender<AudioSignal>,
    ) -> Self {
        let permission = backend.capture_permission();
        Self {
            backend,
            signal_tx,
            streams: BTreeMap::new(),
            zombies: Vec::new(),
            catalog: None,
            key_map: HashMap::new(),
            keys: HashMap::new(),
            next_device_id: 1,
            permission,
            enumerations: Vec::new(),
            catalog_dirty: false,
            last_device_poll: Instant::now(),
            deferred_close: Vec::new(),
        }
    }

    pub(crate) fn statuses(&self) -> Vec<AudioStreamStatus> {
        self.streams.values().map(status_of).collect()
    }

    /// Facts, state and "terminal pending" for one stream (Phase 3).
    pub(crate) fn stream(
        &self,
        id: AudioStreamId,
    ) -> Option<(&StreamInfo, AudioStreamState, bool)> {
        self.streams
            .get(&id)
            .map(|s| (&s.info, s.state, s.terminal.is_some()))
    }

    /// Attach an analysis tap to a capture stream's worker (Phase 3).
    pub(crate) fn attach_tap(&self, id: AudioStreamId, tap: ce_audio::TapSender) -> CeResult<()> {
        self.streams
            .get(&id)
            .ok_or_else(|| unknown_stream(id))?
            .handle
            .attach_tap(tap)
    }
}

fn status_of(slot: &StreamSlot) -> AudioStreamStatus {
    AudioStreamStatus {
        info: slot.info.clone(),
        state: slot.state,
        counters: slot.counters,
        terminal: slot.terminal.clone(),
    }
}

fn no_backend() -> CeError {
    CeError::lifecycle(
        "audio.no_backend",
        "no audio backend is configured for this engine",
    )
}

fn unknown_stream(id: AudioStreamId) -> CeError {
    CeError::invalid_input(
        "audio.stream_not_found",
        "no open audio stream with this id",
    )
    .with_context("stream", id.to_string())
}

fn denial_error(denial: AuthorizationDenial) -> CeError {
    CeError::new(
        ErrorCategory::Unauthorized,
        "audio.capture_unauthorized",
        "capture is not authorized",
    )
    .with_context("denial", denial.as_str())
}

impl Core {
    fn audio_mut(&mut self) -> CeResult<&mut AudioState> {
        self.audio.as_mut().ok_or_else(no_backend)
    }

    fn emit_audio(&mut self, event: AudioEvent, session: Option<SessionId>) {
        self.emit(EventPayload::Audio(event), session);
    }

    // ---------------------------------------------------------------- state

    /// Apply a stream transition through the table; emits the event.
    fn set_stream_state(&mut self, id: AudioStreamId, to: AudioStreamState) -> bool {
        let Some(audio) = self.audio.as_mut() else {
            return false;
        };
        let Some(slot) = audio.streams.get_mut(&id) else {
            return false;
        };
        let from = slot.state;
        if !from.can_transition_to(to) {
            return false;
        }
        slot.state = to;
        let (direction, session) = (slot.info.direction, slot.info.session_id);
        self.log(
            Observation::log(Level::Info, "audio.stream_state_changed")
                .field("stream", id)
                .field("direction", direction.as_str())
                .field("from", from)
                .field("to", to),
        );
        self.emit_audio(
            AudioEvent::StreamStateChanged {
                stream: id,
                direction,
                from: Some(from),
                to,
            },
            Some(session),
        );
        true
    }

    fn stream_failed(&mut self, id: AudioStreamId, error: &CeError, terminal: bool) {
        let session = self
            .audio
            .as_ref()
            .and_then(|a| a.streams.get(&id))
            .map(|s| s.info.session_id);
        let report = error.report(&self.config.runtime.redaction);
        self.log(
            Observation::log(Level::Warn, "audio.stream_failed")
                .field("stream", id)
                .field("error", error)
                .field("terminal", terminal),
        );
        if terminal {
            Metrics::inc(&self.metrics.audio_streams_failed, 1);
            if let Some(slot) = self.audio.as_mut().and_then(|a| a.streams.get_mut(&id)) {
                slot.handle.fail();
                slot.terminal
                    .get_or_insert(TerminalReason::Failed(report.clone()));
            }
            self.set_stream_state(id, AudioStreamState::Failed);
        }
        self.emit_audio(
            AudioEvent::StreamFailed {
                stream: id,
                error: report,
                terminal,
            },
            session,
        );
        self.refresh_runtime_health();
    }

    /// Synchronous part of a revocation: stop data flow now; release the
    /// device at the next async step.
    fn revoke_now(&mut self, id: AudioStreamId, denial: AuthorizationDenial) {
        let Some(audio) = self.audio.as_mut() else {
            return;
        };
        let Some(slot) = audio.streams.get_mut(&id) else {
            return;
        };
        if slot.state == AudioStreamState::Closed
            || matches!(slot.terminal, Some(TerminalReason::Revoked(_)))
        {
            return;
        }
        slot.handle.revoke();
        slot.terminal = Some(TerminalReason::Revoked(denial));
        let session = slot.info.session_id;
        audio
            .deferred_close
            .push((id, TerminalReason::Revoked(denial)));
        Metrics::inc(&self.metrics.audio_capture_revoked, 1);
        self.log(
            Observation::log(Level::Info, "audio.capture_revoked")
                .field("stream", id)
                .field("denial", denial.as_str()),
        );
        self.emit_audio(
            AudioEvent::CaptureRevoked { stream: id, denial },
            Some(session),
        );
    }

    fn capture_authorization(&self, session: SessionId) -> CaptureAuthorization {
        let permission = self
            .audio
            .as_ref()
            .map_or(PlatformPermission::Unknown, |a| a.permission);
        let consent = self
            .sessions
            .get(&session)
            .map_or(ConsentState::Unknown, |s| {
                s.config.consent(ConsentPurpose::Capture)
            });
        evaluate_capture_authorization(
            permission,
            consent,
            self.clock.wall(),
            self.config.audio.consent_max_age(),
        )
    }

    /// Re-evaluate every capture stream; revoke those no longer authorized.
    fn enforce_capture_authorization(&mut self) {
        let Some(audio) = self.audio.as_ref() else {
            return;
        };
        let candidates: Vec<(AudioStreamId, SessionId)> = audio
            .streams
            .values()
            .filter(|s| {
                s.info.direction == AudioDirection::Capture && s.state != AudioStreamState::Closed
            })
            .map(|s| (s.info.id, s.info.session_id))
            .collect();
        for (id, session) in candidates {
            if let CaptureAuthorization::Denied(denial) = self.capture_authorization(session) {
                self.revoke_now(id, denial);
            }
        }
    }

    fn set_permission(&mut self, to: PlatformPermission) {
        let Some(audio) = self.audio.as_mut() else {
            return;
        };
        let from = audio.permission;
        if from == to {
            return;
        }
        audio.permission = to;
        self.emit_audio(AudioEvent::PermissionChanged { from, to }, None);
        self.enforce_capture_authorization();
    }

    // ---------------------------------------------------------------- signals

    /// Handle a worker report or backend notice (synchronous).
    pub(super) fn handle_audio_signal(&mut self, signal: AudioSignal) {
        match signal {
            AudioSignal::Backend(ce_audio::BackendNotice::DevicesChanged) => {
                if let Some(a) = self.audio.as_mut() {
                    a.catalog_dirty = true;
                }
            }
            AudioSignal::Backend(ce_audio::BackendNotice::PermissionChanged(p)) => {
                self.set_permission(p)
            }
            AudioSignal::Worker(report) => self.handle_worker_report(report),
        }
    }

    fn handle_worker_report(&mut self, report: WorkerReport) {
        match report {
            WorkerReport::Fault { stream, fault } => {
                let Some(slot) = self.audio.as_ref().and_then(|a| a.streams.get(&stream)) else {
                    return;
                };
                let session = slot.info.session_id;
                let direction = slot.info.direction;
                match fault {
                    StreamFault::DeviceDisconnected | StreamFault::StreamInvalidated => {
                        let reason = if fault == StreamFault::DeviceDisconnected {
                            InvalidationReason::DeviceDisconnected
                        } else {
                            InvalidationReason::StreamInvalidated
                        };
                        self.invalidate_stream(stream, reason, session);
                    }
                    StreamFault::PermissionRevoked if direction == AudioDirection::Capture => {
                        // The platform withdrew access: treat as a permission change.
                        self.set_permission(PlatformPermission::Denied);
                        self.revoke_now(stream, AuthorizationDenial::PermissionDenied);
                    }
                    StreamFault::PermissionRevoked => {
                        self.invalidate_stream(
                            stream,
                            InvalidationReason::StreamInvalidated,
                            session,
                        );
                    }
                    StreamFault::BackendError => {
                        let err = CeError::new(
                            ErrorCategory::Device,
                            "audio.backend_error",
                            "the audio backend reported a stream error",
                        );
                        self.stream_failed(stream, &err, true);
                    }
                }
            }
            WorkerReport::Rerouted { stream } => {
                let session = self
                    .audio
                    .as_ref()
                    .and_then(|a| a.streams.get(&stream))
                    .map(|s| s.info.session_id);
                self.emit_audio(AudioEvent::StreamRerouted { stream }, session);
            }
            WorkerReport::Pressure {
                stream,
                counters,
                discontinuity,
            } => {
                let Some(slot) = self.audio.as_mut().and_then(|a| a.streams.get_mut(&stream))
                else {
                    return;
                };
                let previous = std::mem::replace(&mut slot.counters, counters);
                let session = slot.info.session_id;
                if counters.ring_overflow_frames > previous.ring_overflow_frames {
                    self.emit_audio(
                        AudioEvent::Overflow {
                            stream,
                            stage: OverflowStage::DeviceRing,
                            lost_frames_total: counters.ring_overflow_frames,
                        },
                        Some(session),
                    );
                }
                if counters.queue_dropped_frames > previous.queue_dropped_frames {
                    self.emit_audio(
                        AudioEvent::Overflow {
                            stream,
                            stage: OverflowStage::DeliveryQueue,
                            lost_frames_total: counters.queue_dropped_frames,
                        },
                        Some(session),
                    );
                }
                if counters.underrun_frames > previous.underrun_frames {
                    self.emit_audio(
                        AudioEvent::Underflow {
                            stream,
                            missing_frames_total: counters.underrun_frames,
                        },
                        Some(session),
                    );
                }
                if let Some((sequence, d)) = discontinuity {
                    self.emit_audio(
                        AudioEvent::Discontinuity {
                            stream,
                            sequence,
                            kind: d.kind,
                            lost_frames: d.lost_frames,
                            total: counters.discontinuities,
                        },
                        Some(session),
                    );
                }
            }
            WorkerReport::Panicked { stream } => {
                let err = CeError::internal("audio.worker_panicked", "the stream worker panicked");
                self.stream_failed(stream, &err, true);
            }
        }
    }

    fn invalidate_stream(
        &mut self,
        id: AudioStreamId,
        reason: InvalidationReason,
        session: SessionId,
    ) {
        let Some(slot) = self.audio.as_mut().and_then(|a| a.streams.get_mut(&id)) else {
            return;
        };
        if !slot.state.can_transition_to(AudioStreamState::Invalidated) {
            return;
        }
        slot.handle.invalidate();
        slot.terminal
            .get_or_insert(TerminalReason::Invalidated(reason));
        Metrics::inc(&self.metrics.audio_streams_invalidated, 1);
        self.set_stream_state(id, AudioStreamState::Invalidated);
        self.emit_audio(
            AudioEvent::StreamInvalidated { stream: id, reason },
            Some(session),
        );
        self.refresh_runtime_health();
    }

    // ---------------------------------------------------------------- catalog

    /// Re-enumerate devices (bounded by the enumerate timeout), diff against
    /// the previous catalog and emit device events.
    pub(super) async fn refresh_catalog(&mut self) -> CeResult<DeviceCatalog> {
        let backend = self.audio_mut()?.backend.clone();
        let (tx, rx) = oneshot::channel();
        let task = tokio::task::spawn_blocking(move || {
            let _ = tx.send(backend.enumerate());
        });
        self.audio_mut()?.enumerations.push(task);
        let deadline = Instant::now() + self.config.audio.enumerate_timeout();
        let result = match self.await_reply(rx, deadline).await {
            Ok(r) => r,
            Err(WaitError::TimedOut) => Err(CeError::new(
                ErrorCategory::Timeout,
                "audio.enumeration_timeout",
                "device enumeration exceeded its timeout",
            )),
            Err(WaitError::ActorGone) => Err(CeError::internal(
                "audio.enumeration_panicked",
                "device enumeration failed",
            )),
        };
        if let Some(a) = self.audio.as_mut() {
            a.catalog_dirty = false;
            a.last_device_poll = Instant::now();
        }
        match result {
            Ok(enumeration) => self.apply_enumeration(enumeration),
            Err(e) => {
                let report = e.report(&self.config.runtime.redaction);
                self.emit_audio(AudioEvent::EnumerationFailed { error: report }, None);
                Err(e)
            }
        }
    }

    fn apply_enumeration(&mut self, enumeration: BackendEnumeration) -> CeResult<DeviceCatalog> {
        let policy = self.config.runtime.redaction;
        let audio = self.audio_mut()?;
        let mut problems: Vec<_> = enumeration
            .problems
            .iter()
            .map(|e| e.report(&policy))
            .collect();
        let mut devices = Vec::new();
        for d in enumeration.devices {
            let id = *audio.key_map.entry(d.key.clone()).or_insert_with(|| {
                let id = AudioDeviceId(audio.next_device_id);
                audio.next_device_id += 1;
                id
            });
            audio.keys.insert(id, d.key.clone());
            let mut skipped = d.skipped_formats;
            let mut clean = |ranges: Vec<SupportedFormatRange>| -> Vec<SupportedFormatRange> {
                ranges
                    .into_iter()
                    .filter_map(|r| {
                        let s = r.sanitized();
                        if s.is_none() {
                            skipped += 1;
                        }
                        s
                    })
                    .collect()
            };
            let input_formats = clean(d.input_formats);
            let output_formats = clean(d.output_formats);
            let supports_input = d.supports_input && !input_formats.is_empty();
            let supports_output = d.supports_output && !output_formats.is_empty();
            if !supports_input && !supports_output {
                problems.push(
                    CeError::new(
                        ErrorCategory::Device,
                        "audio.device_unusable",
                        "a device reported no usable formats and was skipped",
                    )
                    .with_context("device", id.to_string())
                    .report(&policy),
                );
                continue;
            }
            devices.push(AudioDeviceDescriptor {
                id,
                name: d.name,
                key_stability: d.key_stability,
                supports_input,
                supports_output,
                is_default_input: d.is_default_input && supports_input,
                is_default_output: d.is_default_output && supports_output,
                input_formats,
                output_formats,
                default_input_format: d.default_input_format.filter(|f| f.validate().is_ok()),
                default_output_format: d.default_output_format.filter(|f| f.validate().is_ok()),
                skipped_formats: skipped,
            });
        }
        let catalog = DeviceCatalog {
            backend: audio.backend.info(),
            default_input: devices.iter().find(|d| d.is_default_input).map(|d| d.id),
            default_output: devices.iter().find(|d| d.is_default_output).map(|d| d.id),
            devices,
            problems,
        };
        let previous = audio.catalog.replace(catalog.clone());
        self.diff_catalog(previous.as_ref(), &catalog);
        Ok(catalog)
    }

    fn diff_catalog(&mut self, previous: Option<&DeviceCatalog>, current: &DeviceCatalog) {
        let inputs = u32::try_from(current.inputs().count()).unwrap_or(u32::MAX);
        let outputs = u32::try_from(current.outputs().count()).unwrap_or(u32::MAX);
        self.emit_audio(AudioEvent::DevicesEnumerated { inputs, outputs }, None);
        let Some(previous) = previous else { return };
        for d in &current.devices {
            if previous.device(d.id).is_none() {
                self.emit_audio(AudioEvent::DeviceAdded { device: d.id }, None);
            }
        }
        let mut removed = Vec::new();
        for d in &previous.devices {
            if current.device(d.id).is_none() {
                removed.push(d.id);
                self.emit_audio(AudioEvent::DeviceRemoved { device: d.id }, None);
            }
        }
        for (direction, from, to) in [
            (
                AudioDirection::Capture,
                previous.default_input,
                current.default_input,
            ),
            (
                AudioDirection::Playback,
                previous.default_output,
                current.default_output,
            ),
        ] {
            if from != to {
                self.emit_audio(
                    AudioEvent::DefaultDeviceChanged {
                        direction,
                        from,
                        to,
                    },
                    None,
                );
            }
        }
        // Streams on removed devices are invalidated; streams opened on the
        // default whose default moved elsewhere are reported, not moved.
        let streams: Vec<(
            AudioStreamId,
            SessionId,
            AudioDirection,
            AudioDeviceId,
            bool,
            bool,
            AudioStreamState,
        )> = self
            .audio
            .as_ref()
            .map(|a| {
                a.streams
                    .values()
                    .map(|s| {
                        (
                            s.info.id,
                            s.info.session_id,
                            s.info.direction,
                            s.info.device,
                            s.info.opened_on_default,
                            s.divergence_reported,
                            s.state,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        for (id, session, direction, device, on_default, reported, state) in streams {
            if !state.holds_device() || state == AudioStreamState::Invalidated {
                continue;
            }
            if removed.contains(&device) {
                self.invalidate_stream(id, InvalidationReason::DeviceRemoved, session);
                continue;
            }
            let new_default = match direction {
                AudioDirection::Capture => current.default_input,
                AudioDirection::Playback => current.default_output,
            };
            if on_default && !reported && new_default != Some(device) {
                if let Some(slot) = self.audio.as_mut().and_then(|a| a.streams.get_mut(&id)) {
                    slot.divergence_reported = true;
                }
                self.emit_audio(
                    AudioEvent::StreamDefaultDeviceDiverged {
                        stream: id,
                        bound_to: device,
                        new_default,
                    },
                    Some(session),
                );
            }
        }
    }

    // ---------------------------------------------------------------- commands

    pub(super) async fn list_audio_devices(&mut self) -> CeResult<CommandOutcome> {
        let catalog = self.refresh_catalog().await?;
        Ok(CommandOutcome::AudioDevices { catalog })
    }

    pub(super) fn list_audio_streams(&mut self) -> CeResult<CommandOutcome> {
        let audio = self.audio_mut()?;
        Ok(CommandOutcome::AudioStreams {
            streams: audio.statuses(),
        })
    }

    pub(super) async fn update_consent(
        &mut self,
        session_id: SessionId,
        purpose: ConsentPurpose,
        state: ConsentState,
    ) -> CeResult<CommandOutcome> {
        let session = self.sessions.get_mut(&session_id).ok_or_else(|| {
            CeError::invalid_input("session.not_found", "no open session with this id")
                .with_context("session", session_id.to_string())
        })?;
        session.config.consents.insert(purpose, state);
        let info = session.clone();
        self.log(
            Observation::log(Level::Info, "session.consent_updated")
                .field("session", session_id)
                .field("purpose", format!("{purpose:?}"))
                .field("granted", state.is_granted()),
        );
        if self.audio.is_some() && purpose == ConsentPurpose::Capture {
            self.enforce_capture_authorization();
            self.process_deferred_audio().await;
        }
        Ok(CommandOutcome::ConsentUpdated { session: info })
    }

    pub(super) async fn open_audio(
        &mut self,
        session_id: SessionId,
        request: StreamRequest,
    ) -> CeResult<OpenedAudio> {
        request.validate()?;
        if self.audio.is_none() {
            return Err(no_backend());
        }
        if self.state != ce_core::lifecycle::RuntimeState::Running {
            return Err(CeError::lifecycle(
                "runtime.not_running",
                "audio streams need a running runtime",
            )
            .with_context("state", self.state.to_string()));
        }
        let session = self.sessions.get(&session_id).cloned().ok_or_else(|| {
            CeError::invalid_input("session.not_found", "no open session with this id")
                .with_context("session", session_id.to_string())
        })?;
        if session.state != SessionState::Active {
            return Err(CeError::lifecycle(
                "session.not_active",
                "audio streams need an active session",
            ));
        }
        let max = self.config.audio.max_streams as usize;
        let open_count = self
            .audio_mut()?
            .streams
            .values()
            .filter(|s| s.state != AudioStreamState::Closed)
            .count();
        if open_count >= max {
            return Err(CeError::new(
                ErrorCategory::QueueSaturated,
                "audio.stream_limit",
                "the maximum number of audio streams is open",
            )
            .with_context("max_streams", max.to_string()));
        }
        if request.direction == AudioDirection::Capture {
            // Refresh the platform permission before deciding.
            let fresh = self.audio_mut()?.backend.capture_permission();
            self.set_permission(fresh);
            if let CaptureAuthorization::Denied(denial) = self.capture_authorization(session_id) {
                Metrics::inc(&self.metrics.audio_capture_denied, 1);
                self.emit_audio(
                    AudioEvent::CaptureDenied {
                        stream: None,
                        denial,
                    },
                    Some(session_id),
                );
                return Err(denial_error(denial));
            }
        }

        let catalog = self.refresh_catalog().await?;
        let device = match request.device {
            DeviceSelector::Default => {
                let id = match request.direction {
                    AudioDirection::Capture => catalog.default_input,
                    AudioDirection::Playback => catalog.default_output,
                };
                id.and_then(|id| catalog.device(id)).ok_or_else(|| {
                    CeError::new(
                        ErrorCategory::Device,
                        "audio.no_default_device",
                        "there is no default device for this direction",
                    )
                    .with_context("direction", request.direction.as_str())
                })?
            }
            DeviceSelector::Device(id) => catalog.device(id).ok_or_else(|| {
                CeError::new(
                    ErrorCategory::Device,
                    "audio.device_not_found",
                    "the audio device is not available",
                )
                .with_context("device", id.to_string())
            })?,
        }
        .clone();
        let (formats, default_format, supported) = match request.direction {
            AudioDirection::Capture => (
                &device.input_formats,
                device.default_input_format,
                device.supports_input,
            ),
            AudioDirection::Playback => (
                &device.output_formats,
                device.default_output_format,
                device.supports_output,
            ),
        };
        if !supported {
            return Err(CeError::invalid_input(
                "audio.device_direction_unsupported",
                "the device does not support this direction",
            )
            .with_context("device", device.id.to_string())
            .with_context("direction", request.direction.as_str()));
        }
        let resolved = negotiate(
            &request.format,
            request.channel_mode,
            formats,
            default_format.as_ref(),
        )?;
        let rate = resolved.device.sample_rate;
        let audio_cfg = self.config.audio.clone();
        let block_frames = match request.direction {
            AudioDirection::Capture => ms_to_frames(audio_cfg.block_ms, rate),
            AudioDirection::Playback => ms_to_frames(audio_cfg.playback_max_write_ms, rate),
        };
        let ring_ms = match request.direction {
            AudioDirection::Capture => audio_cfg.capture_ring_ms,
            AudioDirection::Playback => audio_cfg.playback_ring_ms,
        };
        let ring_frames = (ms_to_frames(ring_ms, rate) as usize)
            .max(2 * ms_to_frames(audio_cfg.block_ms, rate) as usize);
        let queue_blocks = match request.direction {
            AudioDirection::Capture => audio_cfg.capture_queue_blocks.get(),
            AudioDirection::Playback => audio_cfg.playback_queue_blocks.get(),
        };
        let id = AudioStreamId::generate(self.ids.as_ref());
        let key = self.audio_mut()?.keys.get(&device.id).cloned();
        let info = StreamInfo {
            id,
            session_id,
            correlation_id: session.correlation_id,
            direction: request.direction,
            device: device.id,
            opened_on_default: request.device == DeviceSelector::Default,
            request,
            format: resolved,
            block_frames,
        };
        let open = OpenSpec {
            device_key: key,
            format: info.format.device,
            buffer_frames: info.format.requested_buffer_frames,
        };
        let (backend, signal_tx) = {
            let a = self.audio_mut()?;
            (a.backend.clone(), a.signal_tx.clone())
        };
        let (handle, mut endpoint) = spawn_stream(
            backend,
            StreamContext {
                info: info.clone(),
                open,
            },
            WorkerConfig {
                poll: Duration::from_millis(u64::from(audio_cfg.worker_poll_ms)),
                ring_frames,
                queue_blocks,
                overflow: audio_cfg.capture_queue_overflow,
                pressure_interval: Duration::from_millis(audio_cfg.pressure_report_interval_ms),
                clock: self.clock.clone(),
            },
            signal_tx,
        )?;
        let direction = info.direction;
        self.audio_mut()?.streams.insert(
            id,
            StreamSlot {
                info,
                state: AudioStreamState::Configured,
                handle,
                counters: StreamCounters::default(),
                terminal: None,
                divergence_reported: false,
            },
        );
        Metrics::inc(&self.metrics.audio_streams_opened, 1);
        self.emit_audio(
            AudioEvent::StreamStateChanged {
                stream: id,
                direction,
                from: None,
                to: AudioStreamState::Configured,
            },
            Some(session_id),
        );

        let (tx, rx) = oneshot::channel();
        self.send_worker(id, WorkerCommand::Open(tx))?;
        let deadline = Instant::now() + audio_cfg.open_timeout();
        match self.await_reply(rx, deadline).await {
            Ok(Ok(actual)) => {
                let (format, device_id) = {
                    let slot = self.slot_mut(id)?;
                    slot.info.format.actual_buffer_frames = actual;
                    if let (Some(req), Some(act)) =
                        (slot.info.format.requested_buffer_frames, actual)
                    {
                        if req != act {
                            slot.info
                                .format
                                .adjustments
                                .push(FormatAdjustment::BufferFrames {
                                    requested: req,
                                    resolved: Some(act),
                                });
                        }
                    }
                    (slot.info.format.clone(), slot.info.device)
                };
                self.set_stream_state(id, AudioStreamState::Open);
                self.emit_audio(
                    AudioEvent::StreamFormatResolved {
                        stream: id,
                        device: device_id,
                        format,
                    },
                    Some(session_id),
                );
            }
            Ok(Err(e)) => {
                self.stream_failed(id, &e, true);
                self.close_stream(
                    id,
                    TerminalReason::Failed(e.report(&self.config.runtime.redaction)),
                )
                .await;
                return Err(e);
            }
            Err(wait) => {
                let e = match wait {
                    WaitError::TimedOut => CeError::new(
                        ErrorCategory::Timeout,
                        "audio.open_timeout",
                        "opening the device exceeded its timeout",
                    ),
                    WaitError::ActorGone => CeError::internal(
                        "audio.worker_panicked",
                        "the stream worker failed while opening",
                    ),
                };
                self.stream_failed(id, &e, true);
                self.close_stream(
                    id,
                    TerminalReason::Failed(e.report(&self.config.runtime.redaction)),
                )
                .await;
                return Err(e);
            }
        }
        // A permission change may have arrived while the device was opening.
        self.process_deferred_audio().await;
        let revoked = self
            .audio
            .as_ref()
            .and_then(|a| a.streams.get(&id))
            .is_none_or(|s| matches!(s.terminal, Some(TerminalReason::Revoked(_))));
        if revoked {
            let denial = match self.capture_authorization(session_id) {
                CaptureAuthorization::Denied(d) => d,
                CaptureAuthorization::Granted => AuthorizationDenial::PermissionUnknown,
            };
            return Err(denial_error(denial).with_context("stream", id.to_string()));
        }
        if let Some(slot) = self.audio.as_ref().and_then(|a| a.streams.get(&id)) {
            endpoint.set_info(slot.info.clone());
        }
        Ok(OpenedAudio { endpoint })
    }

    fn slot_mut(&mut self, id: AudioStreamId) -> CeResult<&mut StreamSlot> {
        self.audio_mut()?
            .streams
            .get_mut(&id)
            .ok_or_else(|| unknown_stream(id))
    }

    fn send_worker(&mut self, id: AudioStreamId, command: WorkerCommand) -> CeResult<()> {
        self.slot_mut(id)?.handle.send(command)
    }

    pub(super) async fn start_audio(&mut self, id: AudioStreamId) -> CeResult<CommandOutcome> {
        let (state, direction, session) = {
            let s = self.slot_mut(id)?;
            (s.state, s.info.direction, s.info.session_id)
        };
        if !matches!(state, AudioStreamState::Open | AudioStreamState::Stopped) {
            return Err(CeError::lifecycle(
                "audio.stream_not_startable",
                "only an open or stopped stream can start",
            )
            .with_context("state", state.to_string()));
        }
        if direction == AudioDirection::Capture {
            let fresh = self.audio_mut()?.backend.capture_permission();
            self.set_permission(fresh);
            if let CaptureAuthorization::Denied(denial) = self.capture_authorization(session) {
                Metrics::inc(&self.metrics.audio_capture_denied, 1);
                self.emit_audio(
                    AudioEvent::CaptureDenied {
                        stream: Some(id),
                        denial,
                    },
                    Some(session),
                );
                self.process_deferred_audio().await;
                return Err(denial_error(denial));
            }
        }
        let (tx, rx) = oneshot::channel();
        self.send_worker(id, WorkerCommand::Start(tx))?;
        let deadline = Instant::now() + self.config.audio.operation_timeout();
        match self.await_reply(rx, deadline).await {
            Ok(Ok(())) => {
                self.set_stream_state(id, AudioStreamState::Running);
            }
            Ok(Err(e)) => {
                // A start failure (e.g. a busy device) leaves the stream open
                // so the host may retry or close it.
                self.stream_failed(id, &e, false);
                return Err(e);
            }
            Err(wait) => {
                let e = op_wait_error(wait, "start");
                self.stream_failed(id, &e, true);
                return Err(e);
            }
        }
        // Authorization may have been withdrawn while starting.
        self.process_deferred_audio().await;
        self.stream_outcome(id)
    }

    pub(super) async fn stop_audio(&mut self, id: AudioStreamId) -> CeResult<CommandOutcome> {
        let state = self.slot_mut(id)?.state;
        if state != AudioStreamState::Running {
            return Err(CeError::lifecycle(
                "audio.stream_not_running",
                "only a running stream can stop",
            )
            .with_context("state", state.to_string()));
        }
        let (tx, rx) = oneshot::channel();
        self.send_worker(id, WorkerCommand::Stop(tx))?;
        let deadline = Instant::now() + self.config.audio.operation_timeout();
        match self.await_reply(rx, deadline).await {
            Ok(Ok(())) => {
                self.set_stream_state(id, AudioStreamState::Stopped);
            }
            Ok(Err(e)) => {
                // The device may still be running: fail safe.
                self.stream_failed(id, &e, true);
                return Err(e);
            }
            Err(wait) => {
                let e = op_wait_error(wait, "stop");
                self.stream_failed(id, &e, true);
                return Err(e);
            }
        }
        self.stream_outcome(id)
    }

    pub(super) async fn close_audio(&mut self, id: AudioStreamId) -> CeResult<CommandOutcome> {
        let state = self.slot_mut(id)?.state;
        if state == AudioStreamState::Closed {
            return Err(CeError::lifecycle(
                "audio.stream_closed",
                "the stream is already closed",
            ));
        }
        let status = self.close_stream(id, TerminalReason::Closed).await;
        status
            .map(|status| CommandOutcome::AudioStream { status })
            .ok_or_else(|| unknown_stream(id))
    }

    fn stream_outcome(&mut self, id: AudioStreamId) -> CeResult<CommandOutcome> {
        let slot = self.slot_mut(id)?;
        Ok(CommandOutcome::AudioStream {
            status: status_of(slot),
        })
    }

    /// Release a stream's device and remove it. Returns its final status.
    /// A worker that does not confirm in time is kept as a zombie and the
    /// stream is reported `Failed` until its thread exits.
    pub(super) async fn close_stream(
        &mut self,
        id: AudioStreamId,
        reason: TerminalReason,
    ) -> Option<AudioStreamStatus> {
        let (tx, rx) = oneshot::channel();
        let sent = {
            let slot = self.audio.as_mut()?.streams.get_mut(&id)?;
            if slot.state == AudioStreamState::Closed {
                return Some(status_of(slot));
            }
            slot.terminal.get_or_insert(reason);
            slot.handle.send(WorkerCommand::Close(tx))
        };
        let deadline = Instant::now() + self.config.audio.operation_timeout();
        let outcome = match sent {
            Ok(()) => self.await_reply(rx, deadline).await,
            Err(_) => Err(WaitError::ActorGone),
        };
        match outcome {
            Ok(counters) => {
                if let Some(slot) = self.audio.as_mut().and_then(|a| a.streams.get_mut(&id)) {
                    slot.counters = counters;
                }
                self.set_stream_state(id, AudioStreamState::Closed);
                let audio = self.audio.as_mut()?;
                let mut slot = audio.streams.remove(&id)?;
                let status = status_of(&slot);
                if slot.handle.join_if_finished().is_none() {
                    audio.zombies.push(slot.handle);
                }
                Some(status)
            }
            Err(wait) => {
                let alive = self
                    .audio
                    .as_ref()
                    .and_then(|a| a.streams.get(&id))
                    .is_some_and(|s| !s.handle.is_finished());
                if alive {
                    let e = op_wait_error(wait, "close");
                    self.stream_failed(id, &e, true);
                    let audio = self.audio.as_mut()?;
                    let slot = audio.streams.remove(&id)?;
                    let status = status_of(&slot);
                    audio.zombies.push(slot.handle);
                    Some(status)
                } else {
                    // The worker already exited (e.g. after a panic): the
                    // device was released when its stream was dropped.
                    self.set_stream_state(id, AudioStreamState::Closed);
                    let audio = self.audio.as_mut()?;
                    let mut slot = audio.streams.remove(&id)?;
                    let _ = slot.handle.join_if_finished();
                    Some(status_of(&slot))
                }
            }
        }
    }

    /// Execute closes requested by synchronous handlers, and refresh a dirty
    /// catalog.
    pub(super) async fn process_deferred_audio(&mut self) {
        loop {
            let next = self.audio.as_mut().and_then(|a| a.deferred_close.pop());
            let Some((id, reason)) = next else { break };
            self.close_stream(id, reason).await;
        }
        if self.audio.as_ref().is_some_and(|a| a.catalog_dirty) {
            let _ = self.refresh_catalog().await;
        }
    }

    /// Periodic housekeeping: platform permission, consent staleness, device
    /// polling, zombie reaping.
    pub(super) async fn audio_housekeeping(&mut self) {
        let Some(audio) = self.audio.as_mut() else {
            return;
        };
        audio.enumerations.retain(|t| !t.is_finished());
        audio.zombies.retain_mut(|z| z.join_if_finished().is_none());
        let fresh = audio.backend.capture_permission();
        let poll_due = self
            .config
            .audio
            .device_poll_interval_ms
            .is_some_and(|ms| audio.last_device_poll.elapsed() >= Duration::from_millis(ms));
        if poll_due {
            audio.catalog_dirty = true;
        }
        self.set_permission(fresh);
        self.enforce_capture_authorization();
        self.process_deferred_audio().await;
    }

    /// Close every stream of a session (session close).
    pub(super) async fn close_session_streams(&mut self, session: SessionId) -> u32 {
        let ids: Vec<AudioStreamId> = self
            .audio
            .as_ref()
            .map(|a| {
                a.streams
                    .values()
                    .filter(|s| s.info.session_id == session)
                    .map(|s| s.info.id)
                    .collect()
            })
            .unwrap_or_default();
        let mut closed = 0;
        for id in ids {
            if self
                .close_stream(id, TerminalReason::Closed)
                .await
                .is_some()
            {
                closed += 1;
            }
        }
        closed
    }

    /// Wait (bounded) for zombie worker threads during shutdown. Returns how
    /// many were still running at the deadline.
    pub(super) async fn drain_zombies(&mut self, deadline: Instant) -> usize {
        loop {
            let Some(audio) = self.audio.as_mut() else {
                return 0;
            };
            audio.zombies.retain_mut(|z| z.join_if_finished().is_none());
            audio.enumerations.retain(|t| !t.is_finished());
            if audio.zombies.is_empty() || Instant::now() >= deadline {
                return audio.zombies.len();
            }
            tokio::time::sleep(ZOMBIE_POLL).await;
        }
    }

    pub(super) fn audio_statuses(&self) -> Vec<AudioStreamStatus> {
        self.audio
            .as_ref()
            .map(AudioState::statuses)
            .unwrap_or_default()
    }
}

fn op_wait_error(wait: WaitError, op: &'static str) -> CeError {
    match wait {
        WaitError::TimedOut => CeError::new(
            ErrorCategory::Timeout,
            "audio.operation_timeout",
            "audio stream operation exceeded its timeout",
        )
        .with_context("operation", op),
        WaitError::ActorGone => CeError::internal(
            "audio.worker_gone",
            "the stream worker exited during an operation",
        )
        .with_context("operation", op),
    }
}
