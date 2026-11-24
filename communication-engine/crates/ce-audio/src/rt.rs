//! The real-time handoff between device callbacks and the stream worker.
//!
//! A device callback may touch only:
//!
//! * [`CaptureEndpoint::push`] / [`PlaybackEndpoint::pull`] — copy samples
//!   through a pre-allocated lock-free SPSC ring (`rtrb`), converting the
//!   sample format on the way;
//! * [`FaultReporter`] methods — set atomic flags.
//!
//! Neither blocks, locks, allocates, logs nor performs I/O. Their cost is
//! O(samples) arithmetic plus a few atomic operations. All rings are sized
//! when the stream is created.
//!
//! ## Overflow policy at the device boundary
//!
//! * Capture: when the ring is full the callback **drops the newest frames**
//!   (it cannot wait). The count is accumulated atomically and a gap marker
//!   records where in the stream the loss happened, so the worker can place a
//!   `DeviceOverflow` discontinuity exactly.
//! * Playback: when the ring is empty the callback **outputs silence**. If
//!   data was pending upstream at the time, the silence is counted as an
//!   underrun; otherwise it is idle silence.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering::*};
use std::time::Duration;

use rtrb::{Consumer, Producer, RingBuffer};

use crate::convert::RawSample;

/// Capacity of the gap and timing marker rings.
pub(crate) const MARKER_CAPACITY: usize = 64;

/// A stream fault reported by the backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum StreamFault {
    /// The device is gone.
    DeviceDisconnected = 1,
    /// The stream must be rebuilt (e.g. format changed underneath).
    StreamInvalidated = 2,
    /// The platform revoked capture permission.
    PermissionRevoked = 3,
    /// An unclassified backend error that stops the stream.
    BackendError = 4,
}

impl StreamFault {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(StreamFault::DeviceDisconnected),
            2 => Some(StreamFault::StreamInvalidated),
            3 => Some(StreamFault::PermissionRevoked),
            4 => Some(StreamFault::BackendError),
            _ => None,
        }
    }
}

/// Terminal reason visible to readers/writers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Terminal {
    None = 0,
    Closed = 1,
    Revoked = 2,
    Invalidated = 3,
    Failed = 4,
}

impl Terminal {
    pub(crate) fn from_u8(v: u8) -> Self {
        match v {
            1 => Terminal::Closed,
            2 => Terminal::Revoked,
            3 => Terminal::Invalidated,
            4 => Terminal::Failed,
            _ => Terminal::None,
        }
    }
}

/// State shared between callback, worker, data plane and runtime.
#[derive(Debug)]
pub(crate) struct StreamShared {
    /// Capture: callbacks may enqueue. Playback: callbacks may output data.
    pub(crate) gate_open: AtomicBool,
    pub(crate) callbacks: AtomicU64,
    pub(crate) ring_overflow_frames: AtomicU64,
    pub(crate) underrun_frames: AtomicU64,
    pub(crate) silence_frames: AtomicU64,
    pub(crate) played_frames: AtomicU64,
    pub(crate) backend_xruns: AtomicU64,
    fault: AtomicU8,
    pub(crate) rerouted: AtomicBool,
    /// Playback: data has been pending upstream (worker sets each poll).
    pub(crate) upstream_pending: AtomicBool,
    terminal: AtomicU8,
}

impl StreamShared {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            gate_open: AtomicBool::new(true),
            callbacks: AtomicU64::new(0),
            ring_overflow_frames: AtomicU64::new(0),
            underrun_frames: AtomicU64::new(0),
            silence_frames: AtomicU64::new(0),
            played_frames: AtomicU64::new(0),
            backend_xruns: AtomicU64::new(0),
            fault: AtomicU8::new(0),
            rerouted: AtomicBool::new(false),
            upstream_pending: AtomicBool::new(false),
            terminal: AtomicU8::new(0),
        })
    }

    /// Record the first fault only.
    pub(crate) fn set_fault(&self, fault: StreamFault) {
        let _ = self.fault.compare_exchange(0, fault as u8, AcqRel, Acquire);
    }

    pub(crate) fn fault(&self) -> Option<StreamFault> {
        StreamFault::from_u8(self.fault.load(Acquire))
    }

    /// Set the terminal reason if none is set yet.
    pub(crate) fn set_terminal(&self, t: Terminal) {
        let _ = self.terminal.compare_exchange(0, t as u8, AcqRel, Acquire);
    }

    pub(crate) fn terminal(&self) -> Terminal {
        Terminal::from_u8(self.terminal.load(Acquire))
    }
}

/// Lets a backend report faults from any thread (e.g. a separate error
/// callback). Wait-free.
#[derive(Clone, Debug)]
pub struct FaultReporter {
    shared: Arc<StreamShared>,
}

impl FaultReporter {
    /// Report a fault. Only the first fault of a stream is kept.
    pub fn fault(&self, fault: StreamFault) {
        self.shared.set_fault(fault);
    }

    /// Report a backend-level over/underrun (a glitch the backend saw).
    pub fn xrun(&self) {
        self.shared.backend_xruns.fetch_add(1, Relaxed);
    }

    /// Report that the backend rerouted a default-device stream.
    pub fn rerouted(&self) {
        self.shared.rerouted.store(true, Release);
    }
}

/// Where the capture callback dropped frames: `lost` frames are missing
/// immediately after `at` successfully written frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GapMarker {
    pub(crate) at: u64,
    pub(crate) lost: u64,
}

/// Backend stream-clock time of the frame at written position `pos`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TimingMarker {
    pub(crate) pos: u64,
    pub(crate) device_nanos: u64,
}

/// The capture side handed to a backend's device callback.
#[derive(Debug)]
pub struct CaptureEndpoint {
    samples: Producer<f32>,
    gaps: Producer<GapMarker>,
    timing: Producer<TimingMarker>,
    shared: Arc<StreamShared>,
    channels: usize,
    written: u64,
    unrecorded_lost: u64,
}

/// The worker's side of a capture endpoint.
#[derive(Debug)]
pub(crate) struct CaptureDrain {
    pub(crate) samples: Consumer<f32>,
    pub(crate) gaps: Consumer<GapMarker>,
    pub(crate) timing: Consumer<TimingMarker>,
}

impl CaptureEndpoint {
    /// Create an endpoint and its worker-side drain. `ring_frames` × `channels`
    /// samples are allocated here, once.
    pub(crate) fn new(
        channels: u16,
        ring_frames: usize,
        shared: Arc<StreamShared>,
    ) -> (Self, CaptureDrain) {
        let channels = usize::from(channels.max(1));
        let (sp, sc) = RingBuffer::new(ring_frames.max(1) * channels);
        let (gp, gc) = RingBuffer::new(MARKER_CAPACITY);
        let (tp, tc) = RingBuffer::new(MARKER_CAPACITY);
        (
            Self {
                samples: sp,
                gaps: gp,
                timing: tp,
                shared,
                channels,
                written: 0,
                unrecorded_lost: 0,
            },
            CaptureDrain {
                samples: sc,
                gaps: gc,
                timing: tc,
            },
        )
    }

    /// Channels per frame the backend must deliver.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// A fault reporter for the backend's error path.
    pub fn fault_reporter(&self) -> FaultReporter {
        FaultReporter {
            shared: self.shared.clone(),
        }
    }

    /// Enqueue interleaved native samples from a device callback.
    ///
    /// Whole frames only; a trailing partial frame is ignored. When the ring
    /// cannot hold everything, the newest frames are dropped and counted.
    /// `device_time` is the backend's stream-clock time of the first frame,
    /// if known.
    pub fn push<S: RawSample>(&mut self, data: &[S], device_time: Option<Duration>) {
        self.shared.callbacks.fetch_add(1, Relaxed);
        let ch = self.channels;
        let frames = data.len() / ch;
        if frames == 0 || !self.shared.gate_open.load(Acquire) {
            return;
        }
        if let Some(t) = device_time {
            let device_nanos = u64::try_from(t.as_nanos()).unwrap_or(u64::MAX);
            let _ = self.timing.push(TimingMarker {
                pos: self.written,
                device_nanos,
            });
        }
        let fit = frames.min(self.samples.slots() / ch);
        if fit > 0 {
            if let Ok(chunk) = self.samples.write_chunk_uninit(fit * ch) {
                chunk.fill_from_iter(data[..fit * ch].iter().map(|s| s.to_f32()));
                self.written += fit as u64;
            }
        }
        let lost = (frames - fit) as u64;
        if lost > 0 {
            self.shared.ring_overflow_frames.fetch_add(lost, Relaxed);
            let lost_total = lost + self.unrecorded_lost;
            match self.gaps.push(GapMarker {
                at: self.written,
                lost: lost_total,
            }) {
                Ok(()) => self.unrecorded_lost = 0,
                // Marker ring full: carry the loss to the next marker.
                Err(_) => self.unrecorded_lost = lost_total,
            }
        }
    }
}

/// The playback side handed to a backend's device callback.
#[derive(Debug)]
pub struct PlaybackEndpoint {
    samples: Consumer<f32>,
    shared: Arc<StreamShared>,
    channels: usize,
}

impl PlaybackEndpoint {
    /// Create an endpoint and the worker-side producer.
    pub(crate) fn new(
        channels: u16,
        ring_frames: usize,
        shared: Arc<StreamShared>,
    ) -> (Self, Producer<f32>) {
        let channels = usize::from(channels.max(1));
        let (p, c) = RingBuffer::new(ring_frames.max(1) * channels);
        (
            Self {
                samples: c,
                shared,
                channels,
            },
            p,
        )
    }

    /// Channels per frame the backend expects.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// A fault reporter for the backend's error path.
    pub fn fault_reporter(&self) -> FaultReporter {
        FaultReporter {
            shared: self.shared.clone(),
        }
    }

    /// Fill a device buffer with interleaved native samples. Missing data is
    /// replaced by silence.
    pub fn pull<S: RawSample>(&mut self, out: &mut [S]) {
        self.shared.callbacks.fetch_add(1, Relaxed);
        let ch = self.channels;
        let frames = out.len() / ch;
        let mut filled = 0;
        if self.shared.gate_open.load(Acquire) {
            let fit = frames.min(self.samples.slots() / ch);
            if fit > 0 {
                if let Ok(chunk) = self.samples.read_chunk(fit * ch) {
                    let (a, b) = chunk.as_slices();
                    for (o, s) in out.iter_mut().zip(a.iter().chain(b.iter())) {
                        *o = S::from_f32(*s);
                    }
                    chunk.commit_all();
                    filled = fit;
                }
            }
        }
        for o in &mut out[filled * ch..] {
            *o = S::SILENCE;
        }
        self.shared.played_frames.fetch_add(filled as u64, Relaxed);
        let missing = (frames - filled) as u64;
        if missing > 0 {
            if self.shared.upstream_pending.load(Acquire) && self.shared.gate_open.load(Acquire) {
                self.shared.underrun_frames.fetch_add(missing, Relaxed);
            } else {
                self.shared.silence_frames.fetch_add(missing, Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_push_converts_and_drops_newest_with_a_gap_marker() {
        let shared = StreamShared::new();
        let (mut ep, mut drain) = CaptureEndpoint::new(2, 4, shared.clone());
        ep.push::<i16>(&[16_384, -16_384, 0, 0], Some(Duration::from_millis(5)));
        assert_eq!(drain.samples.slots(), 4);
        let chunk = drain.samples.read_chunk(4).unwrap();
        let (a, _) = chunk.as_slices();
        assert_eq!(a, &[0.5, -0.5, 0.0, 0.0]);
        chunk.commit_all();
        // 3 frames into a ring with room for 4 total, then 3 more: 2 dropped.
        ep.push::<i16>(&[1; 6], None);
        ep.push::<i16>(&[2; 6], None);
        assert_eq!(shared.ring_overflow_frames.load(Relaxed), 2);
        assert_eq!(drain.gaps.pop().unwrap(), GapMarker { at: 6, lost: 2 });
        assert_eq!(
            drain.timing.pop().unwrap(),
            TimingMarker {
                pos: 0,
                device_nanos: 5_000_000
            }
        );
        // A trailing partial frame is ignored; callbacks are counted.
        ep.push::<i16>(&[3], None);
        assert_eq!(shared.callbacks.load(Relaxed), 4);
    }

    #[test]
    fn closed_gate_discards_without_counting_overflow() {
        let shared = StreamShared::new();
        let (mut ep, drain) = CaptureEndpoint::new(1, 8, shared.clone());
        shared.gate_open.store(false, Release);
        ep.push::<f32>(&[0.1; 4], None);
        assert_eq!(drain.samples.slots(), 0);
        assert_eq!(shared.ring_overflow_frames.load(Relaxed), 0);
    }

    #[test]
    fn playback_pull_converts_fills_silence_and_classifies_missing_frames() {
        let shared = StreamShared::new();
        let (mut ep, mut prod) = PlaybackEndpoint::new(1, 8, shared.clone());
        prod.push(0.5).unwrap();
        let mut out = [7u8; 3];
        ep.pull(&mut out);
        assert_eq!(out, [192, 128, 128]);
        assert_eq!(
            shared.silence_frames.load(Relaxed),
            2,
            "nothing pending upstream: idle"
        );
        shared.upstream_pending.store(true, Release);
        let mut out = [1i16; 2];
        ep.pull(&mut out);
        assert_eq!(out, [0, 0]);
        assert_eq!(
            shared.underrun_frames.load(Relaxed),
            2,
            "data pending upstream: underrun"
        );
        assert_eq!(shared.played_frames.load(Relaxed), 1);
    }

    #[test]
    fn only_the_first_fault_and_terminal_reason_are_kept() {
        let shared = StreamShared::new();
        let (ep, _drain) = CaptureEndpoint::new(1, 4, shared.clone());
        let r = ep.fault_reporter();
        r.fault(StreamFault::DeviceDisconnected);
        r.fault(StreamFault::BackendError);
        r.xrun();
        r.rerouted();
        assert_eq!(shared.fault(), Some(StreamFault::DeviceDisconnected));
        assert_eq!(shared.backend_xruns.load(Relaxed), 1);
        assert!(shared.rerouted.load(Acquire));
        shared.set_terminal(Terminal::Revoked);
        shared.set_terminal(Terminal::Closed);
        assert_eq!(shared.terminal(), Terminal::Revoked);
    }
}
