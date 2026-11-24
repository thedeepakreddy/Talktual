//! Analysis tap: a bounded, non-blocking copy of a capture stream's blocks
//! for in-engine consumers (Phase 3 audio intelligence).
//!
//! The capture worker thread (never the device callback) offers each block
//! it delivers to the host queue to the tap as well, with `try_send` on a
//! bounded channel. A full tap **drops the copy** and counts it; capture
//! and host delivery are never delayed. Drops show up to the consumer as
//! block-sequence gaps.
//!
//! When the stream ends the sender is dropped; the receiver drains what was
//! queued and then reports the end reason. On revocation the reason is set
//! first, so consumers can discard queued audio instead of analysing it.

use std::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed};
use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64};
use std::sync::mpsc::{
    Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError, sync_channel,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ce_core::audio::AudioBlock;

use crate::rt::{StreamFault, StreamShared, Terminal};

/// Why the tap ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TapEnd {
    /// The stream was closed (or the tap detached).
    Closed,
    /// Capture authorization was withdrawn: discard queued audio.
    Revoked,
    /// The stream was invalidated (device lost).
    Invalidated,
    /// The stream failed.
    Failed,
}

impl TapEnd {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(TapEnd::Closed),
            2 => Some(TapEnd::Revoked),
            3 => Some(TapEnd::Invalidated),
            4 => Some(TapEnd::Failed),
            _ => None,
        }
    }

    fn code(self) -> u8 {
        match self {
            TapEnd::Closed => 1,
            TapEnd::Revoked => 2,
            TapEnd::Invalidated => 3,
            TapEnd::Failed => 4,
        }
    }

    /// The reason when the capture worker ends the data plane. The worker
    /// can drain a faulted stream before the runtime has marked it
    /// terminal, so a missing terminal state is resolved from the fault.
    pub(crate) fn at_close(shared: &StreamShared) -> Self {
        match (shared.terminal(), shared.fault()) {
            (
                Terminal::None,
                Some(StreamFault::DeviceDisconnected | StreamFault::StreamInvalidated),
            ) => TapEnd::Invalidated,
            (Terminal::None, Some(StreamFault::PermissionRevoked)) => TapEnd::Revoked,
            (t, _) => TapEnd::from_terminal(t),
        }
    }

    pub(crate) fn from_terminal(t: Terminal) -> Self {
        match t {
            Terminal::None | Terminal::Failed => TapEnd::Failed,
            Terminal::Closed => TapEnd::Closed,
            Terminal::Revoked => TapEnd::Revoked,
            Terminal::Invalidated => TapEnd::Invalidated,
        }
    }
}

#[derive(Debug, Default)]
struct TapShared {
    offered: AtomicU64,
    dropped: AtomicU64,
    queued: AtomicU32,
    end: AtomicU8,
}

impl TapShared {
    fn set_end(&self, end: TapEnd) {
        let _ = self.end.compare_exchange(0, end.code(), AcqRel, Acquire);
    }
}

/// Producer side (owned by the capture worker once attached).
#[derive(Debug)]
pub struct TapSender {
    tx: SyncSender<AudioBlock>,
    shared: Arc<TapShared>,
}

impl TapSender {
    /// Offer a copy of `block` without blocking (the capture worker does
    /// this for every delivered block; replay and tests may too). A full tap
    /// drops the copy and counts it. Returns false when the receiver is gone.
    pub fn offer(&self, block: &AudioBlock) -> bool {
        self.shared.offered.fetch_add(1, Relaxed);
        match self.tx.try_send(block.clone()) {
            Ok(()) => {
                self.shared.queued.fetch_add(1, Relaxed);
                true
            }
            Err(TrySendError::Full(_)) => {
                self.shared.dropped.fetch_add(1, Relaxed);
                true
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }

    /// End the tap with `end` (dropping the sender).
    pub fn end(self, end: TapEnd) {
        self.shared.set_end(end);
    }
}

/// Why a receive returned nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapRecvError {
    /// Nothing arrived in time.
    Timeout,
    /// The stream ended and the queue is drained (or must be discarded).
    Ended(TapEnd),
}

/// Consumer side.
#[derive(Debug)]
pub struct TapReceiver {
    rx: Receiver<AudioBlock>,
    shared: Arc<TapShared>,
}

impl TapReceiver {
    /// Next block, waiting up to `timeout`. After a revocation queued
    /// blocks are discarded and `Ended(Revoked)` is returned at once.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<AudioBlock, TapRecvError> {
        if self.end_reason() == Some(TapEnd::Revoked) {
            while self.rx.try_recv().is_ok() {
                self.shared.queued.fetch_sub(1, Relaxed);
            }
            return Err(TapRecvError::Ended(TapEnd::Revoked));
        }
        match self.rx.recv_timeout(timeout) {
            Ok(b) => {
                self.shared.queued.fetch_sub(1, Relaxed);
                Ok(b)
            }
            Err(RecvTimeoutError::Timeout) => Err(TapRecvError::Timeout),
            Err(RecvTimeoutError::Disconnected) => Err(TapRecvError::Ended(
                self.end_reason().unwrap_or(TapEnd::Closed),
            )),
        }
    }

    /// Next block without waiting.
    pub fn try_recv(&self) -> Result<AudioBlock, TapRecvError> {
        if self.end_reason() == Some(TapEnd::Revoked) {
            return self.recv_timeout(Duration::ZERO);
        }
        match self.rx.try_recv() {
            Ok(b) => {
                self.shared.queued.fetch_sub(1, Relaxed);
                Ok(b)
            }
            Err(TryRecvError::Empty) => Err(TapRecvError::Timeout),
            Err(TryRecvError::Disconnected) => Err(TapRecvError::Ended(
                self.end_reason().unwrap_or(TapEnd::Closed),
            )),
        }
    }

    /// Blocks waiting in the tap.
    pub fn backlog(&self) -> u32 {
        self.shared.queued.load(Relaxed)
    }

    /// Blocks dropped because the tap was full.
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Relaxed)
    }

    /// Blocks offered by the capture worker.
    pub fn offered(&self) -> u64 {
        self.shared.offered.load(Relaxed)
    }

    /// Why the stream ended, once it has.
    pub fn end_reason(&self) -> Option<TapEnd> {
        TapEnd::from_u8(self.shared.end.load(Acquire))
    }

    /// Mark the tap ended from the consumer's side (e.g. the runtime knows
    /// the stream is being revoked before the worker does).
    pub fn mark_end(&self, end: TapEnd) {
        self.shared.set_end(end);
    }
}

/// A tap with room for `capacity` blocks (≥ 1).
pub fn analysis_tap(capacity: usize) -> (TapSender, TapReceiver) {
    let (tx, rx) = sync_channel(capacity.max(1));
    let shared = Arc::new(TapShared::default());
    (
        TapSender {
            tx,
            shared: shared.clone(),
        },
        TapReceiver { rx, shared },
    )
}

/// Most taps one capture stream can feed at once (analysis, recognition,
/// and headroom). Each tap is bounded and drops independently.
pub const MAX_TAPS: usize = 4;

/// Identifies one attached tap on its stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TapId(pub(crate) u64);

#[derive(Debug, Default)]
struct Taps {
    next: u64,
    taps: Vec<(TapId, TapSender)>,
}

/// The worker-side slot holding the attached taps (at most [`MAX_TAPS`]).
#[derive(Debug, Default)]
pub(crate) struct TapSlot {
    inner: Mutex<Taps>,
}

impl TapSlot {
    /// Attach; `Err(tap)` when [`MAX_TAPS`] are already attached.
    pub(crate) fn attach(&self, tap: TapSender) -> Result<TapId, TapSender> {
        let mut g = self.lock();
        if g.taps.len() >= MAX_TAPS {
            return Err(tap);
        }
        g.next += 1;
        let id = TapId(g.next);
        g.taps.push((id, tap));
        Ok(id)
    }

    /// End one tap.
    pub(crate) fn detach_one(&self, id: TapId, end: TapEnd) {
        let mut g = self.lock();
        if let Some(i) = g.taps.iter().position(|(t, _)| *t == id) {
            let (_, tap) = g.taps.remove(i);
            tap.end(end);
        }
    }

    /// End every tap.
    pub(crate) fn detach(&self, end: TapEnd) {
        let taps = std::mem::take(&mut self.lock().taps);
        for (_, t) in taps {
            t.end(end);
        }
    }

    pub(crate) fn offer(&self, block: &AudioBlock) {
        let mut g = self.lock();
        // A tap whose receiver is gone detaches itself.
        g.taps.retain(|(_, t)| t.offer(block));
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().taps.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Taps> {
        // A poisoned slot only means a panic elsewhere while holding it; the
        // Option inside is still valid.
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ce_core::audio::{AudioFrameMeta, ChannelLayout, SampleFormat};
    use ce_core::ids::{AudioStreamId, CorrelationId, SessionId};
    use ce_core::time::MonotonicTime;
    use uuid::Uuid;

    fn block(seq: u64) -> AudioBlock {
        AudioBlock {
            meta: AudioFrameMeta {
                stream_id: AudioStreamId::from_uuid(Uuid::from_u128(1)),
                session_id: SessionId::from_uuid(Uuid::from_u128(2)),
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(3)),
                sequence: seq,
                frame_index: seq * 10,
                frames: 10,
                sample_rate: 16_000,
                channels: 1,
                layout: ChannelLayout::Mono,
                sample_format: SampleFormat::F32,
                timestamp: MonotonicTime::from_nanos(0),
                host_time: MonotonicTime::from_nanos(0),
                device_time: None,
                discontinuity: None,
            },
            samples: vec![0.0; 10],
        }
    }

    #[test]
    fn full_taps_drop_and_count_without_blocking() {
        let (tx, rx) = analysis_tap(2);
        let slot = TapSlot::default();
        slot.attach(tx).unwrap();
        for s in 0..5 {
            slot.offer(&block(s));
        }
        assert_eq!((rx.offered(), rx.dropped(), rx.backlog()), (5, 3, 2));
        assert_eq!(rx.try_recv().unwrap().meta.sequence, 0);
        assert_eq!(rx.backlog(), 1);
        slot.detach(TapEnd::Invalidated);
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(1))
                .unwrap()
                .meta
                .sequence,
            1,
            "queued blocks drain first"
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(1)),
            Err(TapRecvError::Ended(TapEnd::Invalidated))
        );
    }

    #[test]
    fn revocation_discards_queued_blocks_and_dropped_receivers_detach() {
        let (tx, rx) = analysis_tap(4);
        let slot = TapSlot::default();
        slot.attach(tx).unwrap();
        slot.offer(&block(0));
        slot.detach(TapEnd::Revoked);
        assert_eq!(rx.try_recv(), Err(TapRecvError::Ended(TapEnd::Revoked)));
        assert_eq!(rx.backlog(), 0);
        let (tx, rx) = analysis_tap(1);
        slot.attach(tx).unwrap();
        drop(rx);
        slot.offer(&block(1));
        assert_eq!(slot.len(), 0, "a gone receiver detaches the tap");
    }

    #[test]
    fn several_taps_are_independent_and_bounded() {
        let slot = TapSlot::default();
        let (fast_tx, fast) = analysis_tap(8);
        let (slow_tx, slow) = analysis_tap(1);
        let a = slot.attach(fast_tx).unwrap();
        slot.attach(slow_tx).unwrap();
        for s in 0..4 {
            slot.offer(&block(s));
        }
        assert_eq!(
            (fast.backlog(), fast.dropped()),
            (4, 0),
            "a slow tap does not affect a fast one"
        );
        assert_eq!((slow.backlog(), slow.dropped()), (1, 3));
        slot.detach_one(a, TapEnd::Closed);
        assert_eq!(slot.len(), 1);
        while fast.try_recv().is_ok() {}
        assert_eq!(fast.try_recv(), Err(TapRecvError::Ended(TapEnd::Closed)));
        for _ in 0..MAX_TAPS - 1 {
            slot.attach(analysis_tap(1).0).unwrap();
        }
        assert!(slot.attach(analysis_tap(1).0).is_err(), "at most MAX_TAPS");
        slot.detach(TapEnd::Revoked);
        assert_eq!(slow.try_recv(), Err(TapRecvError::Ended(TapEnd::Revoked)));
    }
}
