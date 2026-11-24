//! `cpal` backend adapter (feature `cpal`). Selected in ADR-0010.
//!
//! Maps cpal devices, configs, streams and errors onto CE contracts. All cpal
//! types stay inside this module.
//!
//! * Device keys are cpal [`cpal::DeviceId`] strings (`host:identifier`);
//!   cpal documents them as intended to be stable across restarts, which CE
//!   reports as [`IdStability::Stable`] (a backend claim, not verified by CE).
//! * cpal has no device-list change notifications, so `hotplug_notifications`
//!   is false and the runtime detects changes by re-enumerating.
//! * cpal cannot query platform capture permission. On Linux/BSD (ALSA) there
//!   is no permission model, so the adapter reports `NotRequiredByPlatform`;
//!   on every other platform it reports `Unknown`, which **denies capture**
//!   until the host supplies the real state via
//!   [`crate::PermissionOverride`].
//! * The data callback calls only [`CaptureEndpoint::push`] /
//!   [`PlaybackEndpoint::pull`]; the error callback only [`FaultReporter`].
//! * Callback quiescence relies on cpal: dropping a cpal `Stream` stops the
//!   device and joins/stops the callback before returning (verified for the
//!   ALSA host by `tests/cpal_alsa_null.rs`; other hosts per cpal's
//!   implementation, not verified here).

use std::str::FromStr;
use std::time::Duration;

use ce_core::audio::{
    BackendInfo, BufferSizeRange, IdStability, PlatformPermission, SampleFormat, StreamFormat,
    SupportedFormatRange,
};
use ce_core::{CeError, ErrorCategory, Recoverability};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::backend::{AudioBackend, BackendDevice, BackendEnumeration, BackendStream, OpenSpec};
use crate::convert::RawSample;
use crate::rt::{CaptureEndpoint, FaultReporter, PlaybackEndpoint, StreamFault};

/// Timeout cpal is asked to honour when building a stream.
const BUILD_TIMEOUT: Duration = Duration::from_secs(5);

/// The cpal adapter, bound to one cpal host (ALSA, CoreAudio, WASAPI,
/// AAudio, …).
#[derive(Debug, Clone, Copy)]
pub struct CpalBackend {
    host_id: cpal::HostId,
}

impl CpalBackend {
    /// Use the platform's default cpal host.
    pub fn default_host() -> Self {
        Self {
            host_id: cpal::default_host().id(),
        }
    }

    /// Use a specific cpal host.
    pub fn with_host(host_id: cpal::HostId) -> Self {
        Self { host_id }
    }

    fn host(&self) -> Result<cpal::Host, CeError> {
        cpal::host_from_id(self.host_id).map_err(|e| map_error(&e, "audio.host_unavailable"))
    }
}

fn map_sample_format(f: cpal::SampleFormat) -> Option<SampleFormat> {
    Some(match f {
        cpal::SampleFormat::I8 => SampleFormat::I8,
        cpal::SampleFormat::U8 => SampleFormat::U8,
        cpal::SampleFormat::I16 => SampleFormat::I16,
        cpal::SampleFormat::U16 => SampleFormat::U16,
        cpal::SampleFormat::I32 => SampleFormat::I32,
        cpal::SampleFormat::U32 => SampleFormat::U32,
        cpal::SampleFormat::F32 => SampleFormat::F32,
        cpal::SampleFormat::F64 => SampleFormat::F64,
        _ => return None,
    })
}

/// Map a cpal error to a CE error. Messages from cpal are not copied into
/// the CE message (they can contain device names); the kind is kept.
fn map_error(e: &cpal::Error, code: &'static str) -> CeError {
    use cpal::ErrorKind as K;
    let (category, recoverability) = match e.kind() {
        K::PermissionDenied => (ErrorCategory::Unauthorized, Recoverability::Recoverable),
        K::DeviceBusy | K::ResourceExhausted => (ErrorCategory::Device, Recoverability::Retryable),
        K::UnsupportedConfig | K::UnsupportedOperation | K::InvalidInput => {
            (ErrorCategory::Device, Recoverability::Recoverable)
        }
        K::DeviceNotAvailable | K::HostUnavailable | K::StreamInvalidated | K::DeviceChanged => {
            (ErrorCategory::Device, Recoverability::Recoverable)
        }
        _ => (ErrorCategory::Device, Recoverability::Unknown),
    };
    CeError::new(category, code, "audio backend operation failed")
        .with_context("backend_error_kind", format!("{:?}", e.kind()))
        .with_recoverability(recoverability)
}

fn ranges(
    configs: impl Iterator<Item = cpal::SupportedStreamConfigRange>,
    skipped: &mut u32,
) -> Vec<SupportedFormatRange> {
    let mut out = Vec::new();
    for c in configs {
        let Some(sample_format) = map_sample_format(c.sample_format()) else {
            *skipped += 1;
            continue;
        };
        let buffer = match c.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => BufferSizeRange::Range {
                min: *min,
                max: *max,
            },
            cpal::SupportedBufferSize::Unknown => BufferSizeRange::Unknown,
        };
        out.push(SupportedFormatRange {
            channels: c.channels(),
            min_sample_rate: c.min_sample_rate(),
            max_sample_rate: c.max_sample_rate(),
            sample_format,
            buffer,
        });
    }
    out
}

fn default_format(c: Result<cpal::SupportedStreamConfig, cpal::Error>) -> Option<StreamFormat> {
    let c = c.ok()?;
    StreamFormat::new(
        c.sample_rate(),
        c.channels(),
        map_sample_format(c.sample_format())?,
    )
    .ok()
}

fn device_key(d: &cpal::Device) -> Option<String> {
    d.id().ok().map(|id| id.to_string())
}

impl AudioBackend for CpalBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: format!("cpal/{}", self.host_id.name()),
            version: "0.18".to_owned(),
            hotplug_notifications: false,
            permission_query: false,
        }
    }

    fn enumerate(&self) -> Result<BackendEnumeration, CeError> {
        let host = self.host()?;
        let default_in = host.default_input_device().and_then(|d| device_key(&d));
        let default_out = host.default_output_device().and_then(|d| device_key(&d));
        let devices = host
            .devices()
            .map_err(|e| map_error(&e, "audio.enumeration_failed"))?;
        let mut out = BackendEnumeration::default();
        for d in devices {
            let Some(key) = device_key(&d) else {
                out.problems.push(CeError::new(
                    ErrorCategory::Device,
                    "audio.device_without_id",
                    "a device without an identifier was skipped",
                ));
                continue;
            };
            let name = d
                .description()
                .map(|x| x.name().to_owned())
                .unwrap_or_else(|_| key.clone());
            let mut skipped = 0;
            let supports_input = d.supports_input();
            let supports_output = d.supports_output();
            let input_formats = if supports_input {
                d.supported_input_configs()
                    .map(|c| ranges(c, &mut skipped))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let output_formats = if supports_output {
                d.supported_output_configs()
                    .map(|c| ranges(c, &mut skipped))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            out.devices.push(BackendDevice {
                is_default_input: default_in.as_deref() == Some(key.as_str()),
                is_default_output: default_out.as_deref() == Some(key.as_str()),
                default_input_format: if supports_input {
                    default_format(d.default_input_config())
                } else {
                    None
                },
                default_output_format: if supports_output {
                    default_format(d.default_output_config())
                } else {
                    None
                },
                key,
                name,
                key_stability: IdStability::Stable,
                supports_input,
                supports_output,
                input_formats,
                output_formats,
                skipped_formats: skipped,
            });
        }
        Ok(out)
    }

    fn capture_permission(&self) -> PlatformPermission {
        if cfg!(any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "netbsd"
        )) {
            PlatformPermission::NotRequiredByPlatform
        } else {
            PlatformPermission::Unknown
        }
    }

    fn open_capture(
        &self,
        spec: &OpenSpec,
        endpoint: CaptureEndpoint,
    ) -> Result<Box<dyn BackendStream>, CeError> {
        let device = self.find(spec, true)?;
        let config = stream_config(spec);
        let reporter = endpoint.fault_reporter();
        let stream = match spec.format.sample_format {
            SampleFormat::I8 => build_capture::<i8>(&device, config, endpoint, reporter),
            SampleFormat::U8 => build_capture::<u8>(&device, config, endpoint, reporter),
            SampleFormat::I16 => build_capture::<i16>(&device, config, endpoint, reporter),
            SampleFormat::U16 => build_capture::<u16>(&device, config, endpoint, reporter),
            SampleFormat::I32 => build_capture::<i32>(&device, config, endpoint, reporter),
            SampleFormat::U32 => build_capture::<u32>(&device, config, endpoint, reporter),
            SampleFormat::F32 => build_capture::<f32>(&device, config, endpoint, reporter),
            SampleFormat::F64 => build_capture::<f64>(&device, config, endpoint, reporter),
        }
        .map_err(|e| map_error(&e, "audio.open_failed"))?;
        Ok(Box::new(CpalStream { stream }))
    }

    fn open_playback(
        &self,
        spec: &OpenSpec,
        endpoint: PlaybackEndpoint,
    ) -> Result<Box<dyn BackendStream>, CeError> {
        let device = self.find(spec, false)?;
        let config = stream_config(spec);
        let reporter = endpoint.fault_reporter();
        let stream = match spec.format.sample_format {
            SampleFormat::I8 => build_playback::<i8>(&device, config, endpoint, reporter),
            SampleFormat::U8 => build_playback::<u8>(&device, config, endpoint, reporter),
            SampleFormat::I16 => build_playback::<i16>(&device, config, endpoint, reporter),
            SampleFormat::U16 => build_playback::<u16>(&device, config, endpoint, reporter),
            SampleFormat::I32 => build_playback::<i32>(&device, config, endpoint, reporter),
            SampleFormat::U32 => build_playback::<u32>(&device, config, endpoint, reporter),
            SampleFormat::F32 => build_playback::<f32>(&device, config, endpoint, reporter),
            SampleFormat::F64 => build_playback::<f64>(&device, config, endpoint, reporter),
        }
        .map_err(|e| map_error(&e, "audio.open_failed"))?;
        Ok(Box::new(CpalStream { stream }))
    }
}

impl CpalBackend {
    fn find(&self, spec: &OpenSpec, input: bool) -> Result<cpal::Device, CeError> {
        let host = self.host()?;
        let found = match &spec.device_key {
            Some(key) => cpal::DeviceId::from_str(key)
                .ok()
                .and_then(|id| host.device_by_id(&id)),
            None if input => host.default_input_device(),
            None => host.default_output_device(),
        };
        found.ok_or_else(|| {
            CeError::new(
                ErrorCategory::Device,
                "audio.device_not_found",
                "the audio device is not available",
            )
        })
    }
}

fn stream_config(spec: &OpenSpec) -> cpal::StreamConfig {
    cpal::StreamConfig {
        channels: spec.format.channels,
        sample_rate: spec.format.sample_rate,
        buffer_size: spec
            .buffer_frames
            .map_or(cpal::BufferSize::Default, cpal::BufferSize::Fixed),
    }
}

fn error_callback(reporter: FaultReporter) -> impl FnMut(cpal::Error) + Send + 'static {
    move |e: cpal::Error| match e.kind() {
        cpal::ErrorKind::DeviceNotAvailable => reporter.fault(StreamFault::DeviceDisconnected),
        cpal::ErrorKind::StreamInvalidated => reporter.fault(StreamFault::StreamInvalidated),
        cpal::ErrorKind::PermissionDenied => reporter.fault(StreamFault::PermissionRevoked),
        cpal::ErrorKind::DeviceChanged => reporter.rerouted(),
        cpal::ErrorKind::Xrun => reporter.xrun(),
        cpal::ErrorKind::RealtimeDenied => {}
        _ => reporter.fault(StreamFault::BackendError),
    }
}

fn build_capture<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut endpoint: CaptureEndpoint,
    reporter: FaultReporter,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample + RawSample,
{
    device.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], info: &cpal::InputCallbackInfo| {
            let t = info.timestamp().capture.as_nanos();
            let device_time = u64::try_from(t).ok().map(Duration::from_nanos);
            endpoint.push(data, device_time);
        },
        error_callback(reporter),
        Some(BUILD_TIMEOUT),
    )
}

fn build_playback<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut endpoint: PlaybackEndpoint,
    reporter: FaultReporter,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample + RawSample,
{
    device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], _info: &cpal::OutputCallbackInfo| endpoint.pull(data),
        error_callback(reporter),
        Some(BUILD_TIMEOUT),
    )
}

struct CpalStream {
    stream: cpal::Stream,
}

impl BackendStream for CpalStream {
    fn start(&mut self) -> Result<(), CeError> {
        self.stream
            .play()
            .map_err(|e| map_error(&e, "audio.start_failed"))
    }

    fn stop(&mut self) -> Result<(), CeError> {
        self.stream
            .pause()
            .map_err(|e| map_error(&e, "audio.stop_failed"))
    }

    fn buffer_frames(&self) -> Option<u32> {
        self.stream.buffer_size().ok()
    }
}
