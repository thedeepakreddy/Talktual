//! The host-facing data plane: bounded queues between stream workers and
//! hosts.
//!
//! These queues use a short `std::sync::Mutex` critical section (push/pop of
//! a `VecDeque`). They are **never** touched by device callbacks — only by
//! the worker thread and the host — so a lock here cannot stall audio I/O.
//!
//! ## Buffer ownership
//!
//! A capture block is copied out of the device ring by the worker into a
//! freshly allocated `Vec<f32>` owned by the [`AudioBlock`]; ownership moves
//! to the host on `recv`. The ring slot is reused immediately, so nothing a
//! host holds aliases device or backend memory. Playback writes move the
//! host's `Vec<f32>` into the queue; the worker copies it into the ring and
//! drops it.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use ce_core::audio::{
    AudioBlock, DeliveryOverflowPolicy, Discontinuity, DiscontinuityKind, StreamInfo,
};
use ce_core::{CeError, ErrorCategory};
use tokio::sync::Notify;

use crate::rt::{StreamShared, Terminal};

/// Why a reader or writer can no longer be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// The stream was closed (after all queued blocks were received).
    Closed,
    /// Capture authorization was lost; queued blocks were discarded.
    Revoked,
    /// The device was lost or the stream invalidated (after all blocks
    /// captured before that were received).
    Invalidated,
    /// The stream's worker failed.
    Failed,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn terminal_error(shared: &StreamShared) -> ReadError {
    match shared.terminal() {
        Terminal::Revoked => ReadError::Revoked,
        Terminal::Invalidated => ReadError::Invalidated,
        Terminal::Failed => ReadError::Failed,
        Terminal::Closed | Terminal::None => ReadError::Closed,
    }
}

/// Outcome of pushing a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PushOutcome {
    Queued,
    /// The new block was discarded (frames lost).
    DroppedNewest(u64),
    /// The oldest queued block was evicted (frames lost); the new block was
    /// queued.
    EvictedOldest(u64),
}

#[derive(Debug)]
struct BlockQueueInner {
    blocks: VecDeque<AudioBlock>,
    closed: bool,
    /// Loss carried to the next block when the queue has capacity 1.
    carry: Option<Discontinuity>,
}

/// Bounded capture block queue (worker → host).
#[derive(Debug)]
pub(crate) struct BlockQueue {
    inner: Mutex<BlockQueueInner>,
    notify: Notify,
    capacity: usize,
}

fn merge(
    existing: Option<Discontinuity>,
    kind: DiscontinuityKind,
    lost: Option<u64>,
) -> Discontinuity {
    match existing {
        None => Discontinuity {
            kind,
            lost_frames: lost,
        },
        Some(d) => Discontinuity {
            kind,
            lost_frames: match (d.lost_frames, lost) {
                (None, None) => None,
                (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
            },
        },
    }
}

impl BlockQueue {
    pub(crate) fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(BlockQueueInner {
                blocks: VecDeque::with_capacity(capacity),
                closed: false,
                carry: None,
            }),
            notify: Notify::new(),
            capacity: capacity.max(1),
        })
    }

    pub(crate) fn push(
        &self,
        mut block: AudioBlock,
        policy: DeliveryOverflowPolicy,
    ) -> PushOutcome {
        let mut q = lock(&self.inner);
        if q.closed {
            return PushOutcome::DroppedNewest(u64::from(block.meta.frames));
        }
        if let Some(carry) = q.carry.take() {
            let kind = carry.kind;
            block.meta.discontinuity =
                Some(merge(block.meta.discontinuity, kind, carry.lost_frames));
        }
        let outcome = if q.blocks.len() < self.capacity {
            q.blocks.push_back(block);
            PushOutcome::Queued
        } else {
            match policy {
                DeliveryOverflowPolicy::DropNewest => {
                    let lost = u64::from(block.meta.frames)
                        + block
                            .meta
                            .discontinuity
                            .and_then(|d| d.lost_frames)
                            .unwrap_or(0);
                    q.carry = Some(merge(
                        q.carry,
                        DiscontinuityKind::DeliveryOverflow,
                        Some(lost),
                    ));
                    PushOutcome::DroppedNewest(u64::from(block.meta.frames))
                }
                DeliveryOverflowPolicy::DropOldest => {
                    let evicted = q.blocks.pop_front();
                    let frames = evicted.as_ref().map_or(0, |b| u64::from(b.meta.frames));
                    let carried = evicted
                        .and_then(|b| b.meta.discontinuity)
                        .and_then(|d| d.lost_frames)
                        .unwrap_or(0);
                    let target = q.blocks.front_mut().map(|b| &mut b.meta.discontinuity);
                    match target {
                        Some(d) => {
                            *d = Some(merge(
                                *d,
                                DiscontinuityKind::DeliveryOverflow,
                                Some(frames + carried),
                            ))
                        }
                        None => {
                            block.meta.discontinuity = Some(merge(
                                block.meta.discontinuity,
                                DiscontinuityKind::DeliveryOverflow,
                                Some(frames + carried),
                            ));
                        }
                    }
                    q.blocks.push_back(block);
                    PushOutcome::EvictedOldest(frames)
                }
            }
        };
        drop(q);
        self.notify.notify_one();
        outcome
    }

    /// Stop accepting; readers drain what is queued, then see the terminal.
    pub(crate) fn close(&self) {
        lock(&self.inner).closed = true;
        self.notify.notify_one();
    }

    /// Discard everything queued and close (revocation).
    pub(crate) fn purge_and_close(&self) {
        let mut q = lock(&self.inner);
        q.blocks.clear();
        q.closed = true;
        drop(q);
        self.notify.notify_one();
    }

    pub(crate) fn len(&self) -> usize {
        lock(&self.inner).blocks.len()
    }

    fn try_pop(&self) -> Result<Option<AudioBlock>, ()> {
        let mut q = lock(&self.inner);
        match q.blocks.pop_front() {
            Some(b) => Ok(Some(b)),
            None if q.closed => Err(()),
            None => Ok(None),
        }
    }
}

/// Receives captured audio blocks in sequence order.
#[derive(Debug)]
pub struct CaptureReader {
    info: StreamInfo,
    queue: Arc<BlockQueue>,
    shared: Arc<StreamShared>,
}

impl CaptureReader {
    pub(crate) fn new(info: StreamInfo, queue: Arc<BlockQueue>, shared: Arc<StreamShared>) -> Self {
        Self {
            info,
            queue,
            shared,
        }
    }

    /// The stream this reader belongs to.
    pub fn info(&self) -> &StreamInfo {
        &self.info
    }

    pub(crate) fn set_info(&mut self, info: StreamInfo) {
        self.info = info;
    }

    /// Next block, waiting if none is queued.
    pub async fn recv(&mut self) -> Result<AudioBlock, ReadError> {
        loop {
            match self.try_recv()? {
                Some(block) => return Ok(block),
                None => self.queue.notify.notified().await,
            }
        }
    }

    /// Next block if one is queued.
    pub fn try_recv(&mut self) -> Result<Option<AudioBlock>, ReadError> {
        if self.shared.terminal() == Terminal::Revoked {
            return Err(ReadError::Revoked);
        }
        match self.queue.try_pop() {
            Ok(block) => Ok(block),
            Err(()) => Err(terminal_error(&self.shared)),
        }
    }

    /// Blocks currently queued.
    pub fn queued(&self) -> usize {
        self.queue.len()
    }
}

#[derive(Debug)]
struct PlaybackQueueInner {
    items: VecDeque<(u64, Vec<f32>)>,
    closed: bool,
}

/// Bounded playback queue (host → worker).
#[derive(Debug)]
pub(crate) struct PlaybackQueue {
    inner: Mutex<PlaybackQueueInner>,
    space: Notify,
    capacity: usize,
}

impl PlaybackQueue {
    pub(crate) fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(PlaybackQueueInner {
                items: VecDeque::with_capacity(capacity),
                closed: false,
            }),
            space: Notify::new(),
            capacity: capacity.max(1),
        })
    }

    pub(crate) fn pop(&self) -> Option<(u64, Vec<f32>)> {
        let item = lock(&self.inner).items.pop_front();
        if item.is_some() {
            self.space.notify_one();
        }
        item
    }

    pub(crate) fn is_empty(&self) -> bool {
        lock(&self.inner).items.is_empty()
    }

    pub(crate) fn close(&self) {
        let mut q = lock(&self.inner);
        q.closed = true;
        q.items.clear();
        drop(q);
        self.space.notify_one();
    }

    fn try_push(&self, seq: u64, samples: Vec<f32>) -> Result<Result<(), Vec<f32>>, ()> {
        let mut q = lock(&self.inner);
        if q.closed {
            return Err(());
        }
        if q.items.len() >= self.capacity {
            return Ok(Err(samples));
        }
        q.items.push_back((seq, samples));
        Ok(Ok(()))
    }
}

/// Writes interleaved `f32` audio to a playback stream.
#[derive(Debug)]
pub struct PlaybackWriter {
    info: StreamInfo,
    queue: Arc<PlaybackQueue>,
    shared: Arc<StreamShared>,
    next_seq: u64,
}

impl PlaybackWriter {
    pub(crate) fn new(
        info: StreamInfo,
        queue: Arc<PlaybackQueue>,
        shared: Arc<StreamShared>,
    ) -> Self {
        Self {
            info,
            queue,
            shared,
            next_seq: 0,
        }
    }

    /// The stream this writer belongs to.
    pub fn info(&self) -> &StreamInfo {
        &self.info
    }

    pub(crate) fn set_info(&mut self, info: StreamInfo) {
        self.info = info;
    }

    fn validate(&self, samples: &[f32]) -> Result<(), CeError> {
        let ch = usize::from(self.info.format.delivered.channels);
        let max = self.info.block_frames as usize * ch;
        if samples.is_empty() || samples.len() % ch != 0 || samples.len() > max {
            return Err(CeError::invalid_input(
                "audio.invalid_playback_block",
                "playback data must be a non-empty whole number of frames within the write limit",
            )
            .with_context("samples", samples.len().to_string())
            .with_context("channels", ch.to_string())
            .with_context("max_samples", max.to_string()));
        }
        Ok(())
    }

    fn closed(&self) -> CeError {
        let reason = match terminal_error(&self.shared) {
            ReadError::Invalidated => "invalidated",
            ReadError::Failed => "failed",
            ReadError::Revoked => "revoked",
            ReadError::Closed => "closed",
        };
        CeError::new(
            ErrorCategory::Lifecycle,
            "audio.stream_closed",
            "the playback stream is no longer accepting data",
        )
        .with_context("reason", reason)
    }

    /// Queue samples, waiting while the queue is full (backpressure).
    /// Returns the write's sequence number.
    pub async fn write(&mut self, samples: Vec<f32>) -> Result<u64, CeError> {
        self.validate(&samples)?;
        let mut pending = samples;
        loop {
            match self.queue.try_push(self.next_seq, pending) {
                Err(()) => return Err(self.closed()),
                Ok(Ok(())) => {
                    self.next_seq += 1;
                    return Ok(self.next_seq - 1);
                }
                Ok(Err(back)) => {
                    pending = back;
                    self.queue.space.notified().await;
                }
            }
        }
    }

    /// Queue samples without waiting; fails with `QueueSaturated` if full.
    pub fn try_write(&mut self, samples: Vec<f32>) -> Result<u64, CeError> {
        self.validate(&samples)?;
        match self.queue.try_push(self.next_seq, samples) {
            Err(()) => Err(self.closed()),
            Ok(Ok(())) => {
                self.next_seq += 1;
                Ok(self.next_seq - 1)
            }
            Ok(Err(_)) => Err(CeError::new(
                ErrorCategory::QueueSaturated,
                "queue.saturated",
                "playback queue is full",
            )
            .with_context("capacity", self.queue.capacity.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ce_core::audio::{AudioFrameMeta, ChannelLayout, SampleFormat};
    use ce_core::ids::{AudioStreamId, CorrelationId, SessionId};
    use ce_core::time::MonotonicTime;
    use uuid_like::id;

    mod uuid_like {
        pub(super) fn id(n: u128) -> ce_core::ids::AudioStreamId {
            ce_core::ids::AudioStreamId::from_uuid(uuid::Uuid::from_u128(n))
        }
    }

    fn block(seq: u64, frames: u32) -> AudioBlock {
        AudioBlock {
            meta: AudioFrameMeta {
                stream_id: id(1),
                session_id: SessionId::from_uuid(uuid::Uuid::from_u128(2)),
                correlation_id: CorrelationId::from_uuid(uuid::Uuid::from_u128(3)),
                sequence: seq,
                frame_index: seq * u64::from(frames),
                frames,
                sample_rate: 48_000,
                channels: 1,
                layout: ChannelLayout::Mono,
                sample_format: SampleFormat::F32,
                timestamp: MonotonicTime::from_nanos(0),
                host_time: MonotonicTime::from_nanos(0),
                device_time: None,
                discontinuity: None,
            },
            samples: vec![0.0; frames as usize],
        }
    }

    #[test]
    fn drop_newest_keeps_the_prefix_and_flags_the_next_delivered_block() {
        let q = BlockQueue::new(2);
        assert_eq!(
            q.push(block(0, 10), DeliveryOverflowPolicy::DropNewest),
            PushOutcome::Queued
        );
        assert_eq!(
            q.push(block(1, 10), DeliveryOverflowPolicy::DropNewest),
            PushOutcome::Queued
        );
        assert_eq!(
            q.push(block(2, 10), DeliveryOverflowPolicy::DropNewest),
            PushOutcome::DroppedNewest(10)
        );
        assert_eq!(
            q.push(block(3, 10), DeliveryOverflowPolicy::DropNewest),
            PushOutcome::DroppedNewest(10)
        );
        assert_eq!(q.try_pop().unwrap().unwrap().meta.sequence, 0);
        assert_eq!(q.try_pop().unwrap().unwrap().meta.sequence, 1);
        q.push(block(4, 10), DeliveryOverflowPolicy::DropNewest);
        let b = q.try_pop().unwrap().unwrap();
        assert_eq!(b.meta.sequence, 4);
        assert_eq!(
            b.meta.discontinuity,
            Some(Discontinuity {
                kind: DiscontinuityKind::DeliveryOverflow,
                lost_frames: Some(20)
            })
        );
        let _ = AudioStreamId::from_uuid(uuid::Uuid::nil());
    }

    #[test]
    fn drop_oldest_keeps_the_newest_and_flags_the_block_after_the_gap() {
        let q = BlockQueue::new(2);
        for s in 0..4 {
            q.push(block(s, 10), DeliveryOverflowPolicy::DropOldest);
        }
        let b2 = q.try_pop().unwrap().unwrap();
        assert_eq!(b2.meta.sequence, 2);
        assert_eq!(
            b2.meta.discontinuity,
            Some(Discontinuity {
                kind: DiscontinuityKind::DeliveryOverflow,
                lost_frames: Some(20)
            })
        );
        assert_eq!(q.try_pop().unwrap().unwrap().meta.discontinuity, None);
        // Capacity 1: the loss lands on the incoming block.
        let q = BlockQueue::new(1);
        q.push(block(0, 5), DeliveryOverflowPolicy::DropOldest);
        q.push(block(1, 5), DeliveryOverflowPolicy::DropOldest);
        assert_eq!(
            q.try_pop()
                .unwrap()
                .unwrap()
                .meta
                .discontinuity
                .unwrap()
                .lost_frames,
            Some(5)
        );
    }

    #[test]
    fn closing_drains_then_ends_and_purging_discards() {
        let q = BlockQueue::new(4);
        q.push(block(0, 1), DeliveryOverflowPolicy::DropOldest);
        q.close();
        assert!(q.try_pop().unwrap().is_some());
        assert!(q.try_pop().is_err());
        assert_eq!(
            q.push(block(1, 1), DeliveryOverflowPolicy::DropOldest),
            PushOutcome::DroppedNewest(1)
        );
        let q = BlockQueue::new(4);
        q.push(block(0, 1), DeliveryOverflowPolicy::DropOldest);
        q.purge_and_close();
        assert!(q.try_pop().is_err());
    }

    #[test]
    fn memory_is_bounded_by_capacity() {
        let q = BlockQueue::new(3);
        for s in 0..1_000 {
            q.push(block(s, 480), DeliveryOverflowPolicy::DropOldest);
            assert!(q.len() <= 3);
        }
    }
}
