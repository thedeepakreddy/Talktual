//! A CE audio backend whose "device" is fed by the host: PCM that arrives
//! over the network (here: from Talktual's browser microphone) is pushed
//! into the capture endpoint exactly as a device callback would push it.
//!
//! The device (`talktual-browser-mic`) is mono, signed 16-bit, 8–96 kHz.
//! CE's normal capture path then applies: authorization gate, format
//! negotiation, bounded real-time handoff, timestamps, taps for Phase 3
//! analysis and Phase 4 recognition.
//!
//! Permission: the browser holds the microphone permission and only sends
//! audio while the user is talking, so this backend reports `Granted`; CE's
//! own consent gate (`ConsentPurpose::Capture`) still applies per session.
//!
//! Quiescence: pushes happen under the stream's slot mutex; dropping the
//! stream takes the endpoint out of the slot, so no push can follow.

use std::sync::{Arc, Mutex, MutexGuard};

use ce_audio::{
    AudioBackend, BackendDevice, BackendEnumeration, BackendStream, CaptureEndpoint, OpenSpec,
    PlaybackEndpoint,
};
use ce_core::audio::{
    BackendInfo, BufferSizeRange, IdStability, PlatformPermission, SampleFormat, StreamFormat,
    SupportedFormatRange,
};
use ce_core::{CeError, ErrorCategory};

/// Key of the single virtual input device.
pub const DEVICE_KEY: &str = "talktual-browser-mic";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Debug, Default)]
struct SlotState {
    endpoint: Option<CaptureEndpoint>,
    running: bool,
    pushed_frames: u64,
}

/// Where pushed audio goes: one per opened capture stream.
#[derive(Debug, Clone, Default)]
pub struct PushSlot {
    state: Arc<Mutex<SlotState>>,
}

impl PushSlot {
    /// Push interleaved i16 samples (mono). Returns false when the stream is
    /// not running or already closed (the audio is dropped, not queued).
    pub fn push_i16(&self, samples: &[i16]) -> bool {
        let mut s = lock(&self.state);
        if !s.running {
            return false;
        }
        let Some(ep) = s.endpoint.as_mut() else {
            return false;
        };
        ep.push(samples, None);
        s.pushed_frames += samples.len() as u64;
        true
    }

    /// Frames accepted so far.
    pub fn pushed_frames(&self) -> u64 {
        lock(&self.state).pushed_frames
    }
}

/// The backend.
#[derive(Debug, Clone, Default)]
pub struct PushBackend {
    last: Arc<Mutex<Option<PushSlot>>>,
}

impl PushBackend {
    /// New backend.
    pub fn new() -> Self {
        Self::default()
    }

    /// The slot of the most recently opened capture stream (taken). Callers
    /// serialize open + take so the slot belongs to their stream.
    pub fn take_last_slot(&self) -> Option<PushSlot> {
        lock(&self.last).take()
    }
}

fn device() -> BackendDevice {
    BackendDevice {
        key: DEVICE_KEY.into(),
        name: "Talktual browser microphone (network)".into(),
        key_stability: IdStability::Stable,
        supports_input: true,
        supports_output: false,
        is_default_input: true,
        is_default_output: false,
        input_formats: vec![SupportedFormatRange {
            channels: 1,
            min_sample_rate: 8_000,
            max_sample_rate: 96_000,
            sample_format: SampleFormat::I16,
            buffer: BufferSizeRange::Range {
                min: 32,
                max: 8_192,
            },
        }],
        output_formats: Vec::new(),
        default_input_format: StreamFormat::new(48_000, 1, SampleFormat::I16).ok(),
        default_output_format: None,
        skipped_formats: 0,
    }
}

struct PushStream {
    slot: PushSlot,
    buffer: Option<u32>,
}

impl BackendStream for PushStream {
    fn start(&mut self) -> Result<(), CeError> {
        lock(&self.slot.state).running = true;
        Ok(())
    }

    fn stop(&mut self) -> Result<(), CeError> {
        lock(&self.slot.state).running = false;
        Ok(())
    }

    fn buffer_frames(&self) -> Option<u32> {
        self.buffer
    }
}

impl Drop for PushStream {
    fn drop(&mut self) {
        let mut s = lock(&self.slot.state);
        s.running = false;
        s.endpoint = None;
    }
}

impl AudioBackend for PushBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "talktual-push".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            hotplug_notifications: false,
            permission_query: false,
        }
    }

    fn enumerate(&self) -> Result<BackendEnumeration, CeError> {
        Ok(BackendEnumeration {
            devices: vec![device()],
            problems: Vec::new(),
        })
    }

    fn capture_permission(&self) -> PlatformPermission {
        PlatformPermission::Granted
    }

    fn open_capture(
        &self,
        spec: &OpenSpec,
        endpoint: CaptureEndpoint,
    ) -> Result<Box<dyn BackendStream>, CeError> {
        if spec.format.channels != 1 || spec.format.sample_format != SampleFormat::I16 {
            return Err(CeError::new(
                ErrorCategory::Device,
                "bridge.format_unsupported",
                "the network microphone delivers mono 16-bit audio only",
            ));
        }
        let slot = PushSlot::default();
        lock(&slot.state).endpoint = Some(endpoint);
        *lock(&self.last) = Some(slot.clone());
        Ok(Box::new(PushStream {
            slot,
            buffer: spec.buffer_frames,
        }))
    }

    fn open_playback(
        &self,
        _spec: &OpenSpec,
        _endpoint: PlaybackEndpoint,
    ) -> Result<Box<dyn BackendStream>, CeError> {
        Err(CeError::new(
            ErrorCategory::Device,
            "bridge.no_playback",
            "the bridge has no playback device",
        ))
    }
}
