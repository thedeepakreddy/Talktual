//! Lock-free runtime counters.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

use ce_core::health::MetricsSnapshot;

#[derive(Debug, Default)]
pub(crate) struct Metrics {
    pub(crate) commands_accepted: AtomicU64,
    pub(crate) commands_rejected: AtomicU64,
    pub(crate) events_dispatched: AtomicU64,
    pub(crate) events_rejected_ingress: AtomicU64,
    pub(crate) events_dropped_mailbox: AtomicU64,
    pub(crate) events_missed_by_subscribers: AtomicU64,
    pub(crate) module_failures: AtomicU64,
    pub(crate) sessions_created: AtomicU64,
    pub(crate) audio_streams_opened: AtomicU64,
    pub(crate) audio_capture_denied: AtomicU64,
    pub(crate) audio_capture_revoked: AtomicU64,
    pub(crate) audio_streams_invalidated: AtomicU64,
    pub(crate) audio_streams_failed: AtomicU64,
    /// Set by producers when a queue rejects; cleared by the core when it
    /// publishes the coalesced `QueueSaturated` event.
    pub(crate) ingress_saturated: AtomicBool,
    pub(crate) command_saturated: AtomicBool,
}

impl Metrics {
    pub(crate) fn inc(counter: &AtomicU64, by: u64) {
        counter.fetch_add(by, Relaxed);
    }

    pub(crate) fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            commands_accepted: self.commands_accepted.load(Relaxed),
            commands_rejected: self.commands_rejected.load(Relaxed),
            events_dispatched: self.events_dispatched.load(Relaxed),
            events_rejected_ingress: self.events_rejected_ingress.load(Relaxed),
            events_dropped_mailbox: self.events_dropped_mailbox.load(Relaxed),
            events_missed_by_subscribers: self.events_missed_by_subscribers.load(Relaxed),
            module_failures: self.module_failures.load(Relaxed),
            sessions_created: self.sessions_created.load(Relaxed),
            audio_streams_opened: self.audio_streams_opened.load(Relaxed),
            audio_capture_denied: self.audio_capture_denied.load(Relaxed),
            audio_capture_revoked: self.audio_capture_revoked.load(Relaxed),
            audio_streams_invalidated: self.audio_streams_invalidated.load(Relaxed),
            audio_streams_failed: self.audio_streams_failed.load(Relaxed),
        }
    }
}
