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
    pub(crate) analyses_started: AtomicU64,
    pub(crate) analysis_failures: AtomicU64,
    pub(crate) recognitions_started: AtomicU64,
    pub(crate) recognition_failures: AtomicU64,
    pub(crate) semantics_started: AtomicU64,
    pub(crate) semantic_failures: AtomicU64,
    pub(crate) semantic_inputs_dropped: AtomicU64,
    pub(crate) translations_started: AtomicU64,
    pub(crate) translation_failures: AtomicU64,
    pub(crate) translation_inputs_dropped: AtomicU64,
    pub(crate) pipelines_started: AtomicU64,
    pub(crate) pipelines_failed: AtomicU64,
    pub(crate) stale_outputs_rejected: AtomicU64,
    pub(crate) causal_violations: AtomicU64,
    pub(crate) timeline_anomalies: AtomicU64,
    pub(crate) deadline_misses: AtomicU64,
    /// Live per-worker signal forwarders (gauge).
    pub(crate) worker_forwarders: AtomicU64,
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
            analyses_started: self.analyses_started.load(Relaxed),
            analysis_failures: self.analysis_failures.load(Relaxed),
            recognitions_started: self.recognitions_started.load(Relaxed),
            recognition_failures: self.recognition_failures.load(Relaxed),
            semantics_started: self.semantics_started.load(Relaxed),
            semantic_failures: self.semantic_failures.load(Relaxed),
            semantic_inputs_dropped: self.semantic_inputs_dropped.load(Relaxed),
            translations_started: self.translations_started.load(Relaxed),
            translation_failures: self.translation_failures.load(Relaxed),
            translation_inputs_dropped: self.translation_inputs_dropped.load(Relaxed),
            pipelines_started: self.pipelines_started.load(Relaxed),
            pipelines_failed: self.pipelines_failed.load(Relaxed),
            stale_outputs_rejected: self.stale_outputs_rejected.load(Relaxed),
            causal_violations: self.causal_violations.load(Relaxed),
            timeline_anomalies: self.timeline_anomalies.load(Relaxed),
            deadline_misses: self.deadline_misses.load(Relaxed),
            worker_forwarders: self.worker_forwarders.load(Relaxed),
        }
    }
}
