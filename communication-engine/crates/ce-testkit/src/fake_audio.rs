//! A scriptable fake audio backend.
//!
//! Exercises the same real-time endpoints as a real adapter, with two
//! driving modes:
//!
//! * [`FakeDriver::Manual`] — callbacks run only when a test calls
//!   [`FakeAudioBackend::tick`], on the test's thread: fully deterministic.
//! * [`FakeDriver::Realtime`] — each running stream gets a device thread that
//!   calls back every `period`, like a real device.
//!
//! Capture data is a deterministic per-frame signal ([`expected_sample`]),
//! so tests can verify conversion, ordering and exactly which frames were
//! lost. Playback output is recorded.
//!
//! Fault injection covers enumeration failures/delays/invalid responses,
//! open/start/stop failures, hangs and panics, disconnects, reroutes, xruns
//! and permission changes. This is test code: it uses locks and allocates in
//! its "callbacks", which real adapters must not.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use ce_audio::convert::RawSample;
use ce_audio::{
    AudioBackend, BackendDevice, BackendEnumeration, BackendNotice, BackendNotifier, BackendStream,
    CaptureEndpoint, FaultReporter, OpenSpec, PlaybackEndpoint, StreamFault,
};
use ce_core::audio::{
    BackendInfo, BufferSizeRange, IdStability, PlatformPermission, SampleFormat, StreamFormat,
    SupportedFormatRange,
};
use ce_core::{CeError, ErrorCategory};

/// How fake device callbacks are driven.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FakeDriver {
    /// Only [`FakeAudioBackend::tick`] runs callbacks.
    Manual,
    /// A device thread runs callbacks every `period`.
    Realtime {
        /// Callback period.
        period: Duration,
    },
}

/// The deterministic capture signal: frame `f`, channel `c`.
pub fn expected_sample(frame: u64, channel: usize) -> f32 {
    let base = ((frame % 200) as f32 - 100.0) / 128.0;
    if channel % 2 == 0 { base } else { -base }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// A device with sensible input and output ranges.
pub fn device(key: &str, input: bool, output: bool) -> BackendDevice {
    let ranges = |fmt| {
        vec![
            SupportedFormatRange {
                channels: 1,
                min_sample_rate: 8_000,
                max_sample_rate: 48_000,
                sample_format: fmt,
                buffer: BufferSizeRange::Range {
                    min: 32,
                    max: 4_096,
                },
            },
            SupportedFormatRange {
                channels: 2,
                min_sample_rate: 8_000,
                max_sample_rate: 48_000,
                sample_format: fmt,
                buffer: BufferSizeRange::Range {
                    min: 32,
                    max: 4_096,
                },
            },
        ]
    };
    let default = StreamFormat::new(48_000, if input { 1 } else { 2 }, SampleFormat::I16).ok();
    BackendDevice {
        key: key.to_owned(),
        name: format!("Fake {key}"),
        key_stability: IdStability::Session,
        supports_input: input,
        supports_output: output,
        is_default_input: false,
        is_default_output: false,
        input_formats: if input {
            ranges(SampleFormat::I16)
        } else {
            Vec::new()
        },
        output_formats: if output {
            ranges(SampleFormat::I16)
        } else {
            Vec::new()
        },
        default_input_format: if input { default } else { None },
        default_output_format: if output { default } else { None },
        skipped_formats: 0,
    }
}

/// Error injected into a fake operation.
#[derive(Clone, Debug)]
pub struct FakeError {
    /// Category.
    pub category: ErrorCategory,
    /// Code.
    pub code: &'static str,
}

impl FakeError {
    /// A device error with `code`.
    pub fn device(code: &'static str) -> Self {
        Self {
            category: ErrorCategory::Device,
            code,
        }
    }

    fn to_error(&self) -> CeError {
        CeError::new(self.category, self.code, "injected fake backend failure")
    }
}

#[derive(Debug, Default)]
struct Faults {
    fail_enumerations: u32,
    enumerate_delay: Duration,
    open_error: Option<FakeError>,
    open_delay: Duration,
    panic_on_open: bool,
    start_error: Option<FakeError>,
    stop_error: Option<FakeError>,
    hang_on_start: bool,
    actual_buffer_frames: Option<u32>,
}

struct StreamCtl {
    capture: Option<CaptureEndpoint>,
    playback: Option<PlaybackEndpoint>,
    running: bool,
    closed: bool,
    next_frame: u64,
    played: Vec<f32>,
}

/// Inspectable state of one fake stream.
pub struct FakeStreamState {
    /// Backend key opened (`None` = default).
    pub device_key: Option<String>,
    /// Device format opened.
    pub format: StreamFormat,
    /// Capture (`true`) or playback.
    pub capture: bool,
    reporter: FaultReporter,
    ctl: Mutex<StreamCtl>,
    callbacks_after_close: AtomicU64,
    driver_stop: Arc<AtomicBool>,
    driver: Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for FakeStreamState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeStreamState")
            .field("format", &self.format)
            .field("capture", &self.capture)
            .finish_non_exhaustive()
    }
}

impl FakeStreamState {
    /// Whether callbacks are running.
    pub fn is_running(&self) -> bool {
        lock(&self.ctl).running
    }

    /// Whether the stream was dropped by CE (device released).
    pub fn is_closed(&self) -> bool {
        lock(&self.ctl).closed
    }

    /// Whether the endpoint is still held (false after close: quiescence).
    pub fn holds_endpoint(&self) -> bool {
        let c = lock(&self.ctl);
        c.capture.is_some() || c.playback.is_some()
    }

    /// Callback attempts made after close (must stay 0).
    pub fn callbacks_after_close(&self) -> u64 {
        self.callbacks_after_close.load(Ordering::SeqCst)
    }

    /// Recorded playback output (normalized), bounded to the last 1M samples.
    pub fn played(&self) -> Vec<f32> {
        lock(&self.ctl).played.clone()
    }

    /// Report a fault as the backend's error callback would.
    pub fn fault(&self, fault: StreamFault) {
        self.reporter.fault(fault);
    }

    /// Report that the backend rerouted this stream.
    pub fn reroute(&self) {
        self.reporter.rerouted();
    }

    /// Report a backend xrun.
    pub fn xrun(&self) {
        self.reporter.xrun();
    }

    /// Run one callback of `frames` frames now (any driver mode).
    pub fn callback(&self, frames: usize) {
        let mut c = lock(&self.ctl);
        if c.closed {
            self.callbacks_after_close.fetch_add(1, Ordering::SeqCst);
            return;
        }
        if !c.running {
            return;
        }
        let ch = usize::from(self.format.channels);
        let start = c.next_frame;
        c.next_frame += frames as u64;
        let fmt = self.format.sample_format;
        if let Some(ep) = c.capture.as_mut() {
            push_signal(ep, fmt, start, frames, ch);
        } else if c.playback.is_some() {
            let out = pull_signal(c.playback.as_mut().expect("checked"), fmt, frames * ch);
            c.played.extend(out);
            let len = c.played.len();
            if len > 1_000_000 {
                c.played.drain(..len - 1_000_000);
            }
        }
    }
}

fn signal<T: RawSample>(start: u64, frames: usize, ch: usize) -> Vec<T> {
    let mut v = Vec::with_capacity(frames * ch);
    for f in 0..frames {
        for c in 0..ch {
            v.push(T::from_f32(expected_sample(start + f as u64, c)));
        }
    }
    v
}

fn push_signal(ep: &mut CaptureEndpoint, fmt: SampleFormat, start: u64, frames: usize, ch: usize) {
    let t = Some(Duration::from_micros(start * 10));
    match fmt {
        SampleFormat::I8 => ep.push(&signal::<i8>(start, frames, ch), t),
        SampleFormat::U8 => ep.push(&signal::<u8>(start, frames, ch), t),
        SampleFormat::I16 => ep.push(&signal::<i16>(start, frames, ch), t),
        SampleFormat::U16 => ep.push(&signal::<u16>(start, frames, ch), t),
        SampleFormat::I32 => ep.push(&signal::<i32>(start, frames, ch), t),
        SampleFormat::U32 => ep.push(&signal::<u32>(start, frames, ch), t),
        SampleFormat::F32 => ep.push(&signal::<f32>(start, frames, ch), t),
        SampleFormat::F64 => ep.push(&signal::<f64>(start, frames, ch), t),
    }
}

fn pull<T: RawSample>(ep: &mut PlaybackEndpoint, n: usize) -> Vec<f32> {
    let mut buf = vec![T::SILENCE; n];
    ep.pull(&mut buf);
    buf.into_iter().map(RawSample::to_f32).collect()
}

fn pull_signal(ep: &mut PlaybackEndpoint, fmt: SampleFormat, n: usize) -> Vec<f32> {
    match fmt {
        SampleFormat::I8 => pull::<i8>(ep, n),
        SampleFormat::U8 => pull::<u8>(ep, n),
        SampleFormat::I16 => pull::<i16>(ep, n),
        SampleFormat::U16 => pull::<u16>(ep, n),
        SampleFormat::I32 => pull::<i32>(ep, n),
        SampleFormat::U32 => pull::<u32>(ep, n),
        SampleFormat::F32 => pull::<f32>(ep, n),
        SampleFormat::F64 => pull::<f64>(ep, n),
    }
}

#[derive(Default)]
struct State {
    devices: Vec<BackendDevice>,
    default_input: Option<String>,
    default_output: Option<String>,
    permission: Option<PlatformPermission>,
    notifier: Option<BackendNotifier>,
    faults: Faults,
    streams: Vec<Arc<FakeStreamState>>,
    enumerations: u64,
}

/// The fake backend. Cloning shares state, so a test keeps a clone to
/// script and inspect while the engine owns another.
#[derive(Clone)]
pub struct FakeAudioBackend {
    state: Arc<Mutex<State>>,
    driver: FakeDriver,
}

impl std::fmt::Debug for FakeAudioBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeAudioBackend")
            .field("driver", &self.driver)
            .finish_non_exhaustive()
    }
}

impl FakeAudioBackend {
    /// A backend with a default mono microphone (`mic`) and stereo speaker
    /// (`speaker`), platform permission granted.
    pub fn new(driver: FakeDriver) -> Self {
        let b = Self::empty(driver);
        b.add_device(device("mic", true, false));
        b.add_device(device("speaker", false, true));
        b.set_default_input(Some("mic"));
        b.set_default_output(Some("speaker"));
        lock(&b.state).enumerations = 0;
        b
    }

    /// A backend with no devices.
    pub fn empty(driver: FakeDriver) -> Self {
        let b = Self {
            state: Arc::new(Mutex::new(State::default())),
            driver,
        };
        lock(&b.state).permission = Some(PlatformPermission::Granted);
        b
    }

    fn notify(&self, n: BackendNotice) {
        if let Some(notifier) = lock(&self.state).notifier.clone() {
            notifier.notify(n);
        }
    }

    /// Add (or replace) a device and notify.
    pub fn add_device(&self, device: BackendDevice) {
        {
            let mut s = lock(&self.state);
            s.devices.retain(|d| d.key != device.key);
            s.devices.push(device);
        }
        self.notify(BackendNotice::DevicesChanged);
    }

    /// Remove a device: its streams report `DeviceDisconnected`, then the
    /// backend notifies a device change.
    pub fn remove_device(&self, key: &str) {
        let affected: Vec<Arc<FakeStreamState>> = {
            let mut s = lock(&self.state);
            s.devices.retain(|d| d.key != key);
            let default_in = s.default_input.clone();
            let default_out = s.default_output.clone();
            if default_in.as_deref() == Some(key) {
                s.default_input = None;
            }
            if default_out.as_deref() == Some(key) {
                s.default_output = None;
            }
            s.streams
                .iter()
                .filter(|st| {
                    st.device_key.as_deref() == Some(key)
                        || (st.device_key.is_none()
                            && if st.capture {
                                default_in.as_deref() == Some(key)
                            } else {
                                default_out.as_deref() == Some(key)
                            })
                })
                .cloned()
                .collect()
        };
        for st in affected {
            st.fault(StreamFault::DeviceDisconnected);
        }
        self.notify(BackendNotice::DevicesChanged);
    }

    /// Set the default input device and notify.
    pub fn set_default_input(&self, key: Option<&str>) {
        lock(&self.state).default_input = key.map(str::to_owned);
        self.notify(BackendNotice::DevicesChanged);
    }

    /// Set the default output device and notify.
    pub fn set_default_output(&self, key: Option<&str>) {
        lock(&self.state).default_output = key.map(str::to_owned);
        self.notify(BackendNotice::DevicesChanged);
    }

    /// Set the platform permission and notify.
    pub fn set_permission(&self, permission: PlatformPermission) {
        lock(&self.state).permission = Some(permission);
        self.notify(BackendNotice::PermissionChanged(permission));
    }

    /// Change the permission silently (as a platform without notifications
    /// would; CE notices at its next check).
    pub fn set_permission_silently(&self, permission: PlatformPermission) {
        lock(&self.state).permission = Some(permission);
    }

    /// Fail the next `n` enumerations.
    pub fn fail_enumerations(&self, n: u32) {
        lock(&self.state).faults.fail_enumerations = n;
    }

    /// Delay every enumeration.
    pub fn delay_enumeration(&self, d: Duration) {
        lock(&self.state).faults.enumerate_delay = d;
    }

    /// Fail every open with `e` (until cleared with `None`).
    pub fn fail_open(&self, e: Option<FakeError>) {
        lock(&self.state).faults.open_error = e;
    }

    /// Delay every open.
    pub fn delay_open(&self, d: Duration) {
        lock(&self.state).faults.open_delay = d;
    }

    /// Panic inside the next open (on the worker thread).
    pub fn panic_on_open(&self) {
        lock(&self.state).faults.panic_on_open = true;
    }

    /// Fail the next start with `e`.
    pub fn fail_next_start(&self, e: FakeError) {
        lock(&self.state).faults.start_error = Some(e);
    }

    /// Fail the next stop with `e`.
    pub fn fail_next_stop(&self, e: FakeError) {
        lock(&self.state).faults.stop_error = Some(e);
    }

    /// Make the next start block forever (until the stream is dropped).
    pub fn hang_next_start(&self) {
        lock(&self.state).faults.hang_on_start = true;
    }

    /// Report this buffer size after open instead of the requested one.
    pub fn report_buffer_frames(&self, frames: Option<u32>) {
        lock(&self.state).faults.actual_buffer_frames = frames;
    }

    /// Streams opened so far (including closed ones), in open order.
    pub fn streams(&self) -> Vec<Arc<FakeStreamState>> {
        lock(&self.state).streams.clone()
    }

    /// The most recently opened stream.
    pub fn last_stream(&self) -> Option<Arc<FakeStreamState>> {
        lock(&self.state).streams.last().cloned()
    }

    /// Enumerations performed.
    pub fn enumerations(&self) -> u64 {
        lock(&self.state).enumerations
    }

    /// Manual mode: run one callback of `frames` frames on every running
    /// stream.
    pub fn tick(&self, frames: usize) {
        for st in self.streams() {
            st.callback(frames);
        }
    }

    fn open(
        &self,
        spec: &OpenSpec,
        capture: bool,
        reporter: FaultReporter,
        ctl: StreamCtl,
    ) -> Result<Box<dyn BackendStream>, CeError> {
        let (delay, error, panic, devices, default) = {
            let mut s = lock(&self.state);
            let panic = std::mem::take(&mut s.faults.panic_on_open);
            let default = if capture {
                s.default_input.clone()
            } else {
                s.default_output.clone()
            };
            (
                s.faults.open_delay,
                s.faults.open_error.clone(),
                panic,
                s.devices.clone(),
                default,
            )
        };
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        if panic {
            panic!("injected panic in fake open");
        }
        if let Some(e) = error {
            return Err(e.to_error());
        }
        let key = spec.device_key.clone().or(default);
        let exists = key
            .as_ref()
            .is_some_and(|k| devices.iter().any(|d| &d.key == k));
        if !exists {
            return Err(CeError::new(
                ErrorCategory::Device,
                "audio.device_not_found",
                "fake device not present",
            ));
        }
        let state = Arc::new(FakeStreamState {
            device_key: spec.device_key.clone(),
            format: spec.format,
            capture,
            reporter,
            ctl: Mutex::new(ctl),
            callbacks_after_close: AtomicU64::new(0),
            driver_stop: Arc::new(AtomicBool::new(false)),
            driver: Mutex::new(None),
        });
        lock(&self.state).streams.push(state.clone());
        let buffer = lock(&self.state)
            .faults
            .actual_buffer_frames
            .or(spec.buffer_frames)
            .or(Some(256));
        Ok(Box::new(FakeStream {
            state,
            backend: self.clone(),
            buffer,
        }))
    }
}

struct FakeStream {
    state: Arc<FakeStreamState>,
    backend: FakeAudioBackend,
    buffer: Option<u32>,
}

impl BackendStream for FakeStream {
    fn start(&mut self) -> Result<(), CeError> {
        let (error, hang) = {
            let mut s = lock(&self.backend.state);
            (
                s.faults.start_error.take(),
                std::mem::take(&mut s.faults.hang_on_start),
            )
        };
        if hang {
            loop {
                std::thread::sleep(Duration::from_millis(5));
                if self.state.driver_stop.load(Ordering::SeqCst) {
                    return Err(CeError::new(
                        ErrorCategory::Device,
                        "fake.hang_released",
                        "hang released",
                    ));
                }
            }
        }
        if let Some(e) = error {
            return Err(e.to_error());
        }
        lock(&self.state.ctl).running = true;
        if let FakeDriver::Realtime { period } = self.backend.driver {
            let mut driver = lock(&self.state.driver);
            if driver.is_none() {
                self.state.driver_stop.store(false, Ordering::SeqCst);
                let st = self.state.clone();
                let frames = (u64::from(self.state.format.sample_rate) * period.as_micros() as u64
                    / 1_000_000)
                    .max(1) as usize;
                *driver = Some(std::thread::spawn(move || {
                    while !st.driver_stop.load(Ordering::SeqCst) {
                        st.callback(frames);
                        std::thread::sleep(period);
                    }
                }));
            }
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), CeError> {
        if let Some(e) = lock(&self.backend.state).faults.stop_error.take() {
            return Err(e.to_error());
        }
        lock(&self.state.ctl).running = false;
        Ok(())
    }

    fn buffer_frames(&self) -> Option<u32> {
        self.buffer
    }
}

impl Drop for FakeStream {
    /// Quiescence: stop and join the driver thread, then release the
    /// endpoint, before returning.
    fn drop(&mut self) {
        self.state.driver_stop.store(true, Ordering::SeqCst);
        if let Some(t) = lock(&self.state.driver).take() {
            let _ = t.join();
        }
        let mut c = lock(&self.state.ctl);
        c.running = false;
        c.closed = true;
        c.capture = None;
        c.playback = None;
    }
}

impl AudioBackend for FakeAudioBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "fake".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            hotplug_notifications: true,
            permission_query: true,
        }
    }

    fn enumerate(&self) -> Result<BackendEnumeration, CeError> {
        let (delay, fail, mut devices, din, dout) = {
            let mut s = lock(&self.state);
            s.enumerations += 1;
            let fail = s.faults.fail_enumerations > 0;
            if fail {
                s.faults.fail_enumerations -= 1;
            }
            (
                s.faults.enumerate_delay,
                fail,
                s.devices.clone(),
                s.default_input.clone(),
                s.default_output.clone(),
            )
        };
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        if fail {
            return Err(CeError::new(
                ErrorCategory::Device,
                "fake.enumeration_failed",
                "injected enumeration failure",
            ));
        }
        for d in &mut devices {
            d.is_default_input = din.as_deref() == Some(d.key.as_str());
            d.is_default_output = dout.as_deref() == Some(d.key.as_str());
        }
        Ok(BackendEnumeration {
            devices,
            problems: Vec::new(),
        })
    }

    fn capture_permission(&self) -> PlatformPermission {
        lock(&self.state)
            .permission
            .unwrap_or(PlatformPermission::Unknown)
    }

    fn set_notifier(&self, notifier: BackendNotifier) {
        lock(&self.state).notifier = Some(notifier);
    }

    fn open_capture(
        &self,
        spec: &OpenSpec,
        endpoint: CaptureEndpoint,
    ) -> Result<Box<dyn BackendStream>, CeError> {
        let reporter = endpoint.fault_reporter();
        self.open(
            spec,
            true,
            reporter,
            StreamCtl {
                capture: Some(endpoint),
                playback: None,
                running: false,
                closed: false,
                next_frame: 0,
                played: Vec::new(),
            },
        )
    }

    fn open_playback(
        &self,
        spec: &OpenSpec,
        endpoint: PlaybackEndpoint,
    ) -> Result<Box<dyn BackendStream>, CeError> {
        let reporter = endpoint.fault_reporter();
        self.open(
            spec,
            false,
            reporter,
            StreamCtl {
                capture: None,
                playback: Some(endpoint),
                running: false,
                closed: false,
                next_frame: 0,
                played: Vec::new(),
            },
        )
    }
}
