//! Typed, versioned event envelopes.
//!
//! ## Ordering
//!
//! The runtime assigns every dispatched event a `sequence` number from a
//! single counter, in dispatch order. Every observer (subscribers and module
//! mailboxes) receives events in strictly increasing `sequence` order. An
//! observer can *miss* events (bounded queues, see
//! `docs/architecture/events.md`) but never sees them reordered or duplicated.
//! Gaps are always made visible: a lagging subscriber receives a `Lagged`
//! error, and dropped mailbox deliveries produce `QueueSaturated` events.
//!
//! ## Versioning
//!
//! [`EventEnvelope::schema_version`] identifies the envelope and payload
//! schema. Adding payload variants or optional fields is a compatible change
//! and keeps the version; anything else increments it.
//! [`EventEnvelope::check_schema`] rejects envelopes from an unsupported
//! version, and unknown payload types fail to deserialize — both map to the
//! `Serialization` error category.
//!
//! ## Privacy
//!
//! Every envelope declares the [`DataClass`] of its payload. Runtime events
//! carry operational data only. Module events declare their own class, and
//! the runtime never logs payload values above the configured redaction
//! policy.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::command::ShutdownReport;
use crate::error::{CeError, ErrorCategory, ErrorReport};
use crate::health::{HealthStatus, HealthSubject};
use crate::ids::{CapabilityId, CorrelationId, EventId, EventName, ModuleId, SessionId};
use crate::lifecycle::{ModuleState, RuntimeState};
use crate::privacy::DataClass;
use crate::resource::QueueKind;
use crate::session::SessionState;
use crate::time::{MonotonicTime, WallTime};
use crate::value::Value;
use crate::version::Version;

/// Current envelope schema version.
pub const EVENT_SCHEMA_VERSION: u16 = 1;

/// Who produced an event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "module")]
pub enum EventSource {
    /// The runtime.
    Runtime,
    /// A module.
    Module(ModuleId),
}

/// Why the runtime is stopping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The host requested shutdown.
    Requested,
    /// The engine handle was dropped without an explicit shutdown.
    HandleDropped,
    /// Start failed and the host is cleaning up.
    StartFailed,
}

/// A module-defined event. Provisional: Phase 1 module payloads are
/// structured values; typed module payload schemas are a later-phase decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModuleEvent {
    /// Namespaced event name.
    pub name: EventName,
    /// Version of this event's attribute schema, owned by the module.
    pub version: u16,
    /// Attributes.
    pub attributes: BTreeMap<String, Value>,
}

/// Event payloads. Non-exhaustive: new variants are compatible additions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
#[non_exhaustive]
pub enum EventPayload {
    /// The runtime began starting modules.
    RuntimeStarting,
    /// All modules are running.
    RuntimeStarted {
        /// Number of running modules.
        modules: u32,
    },
    /// The runtime began shutting down.
    RuntimeStopping {
        /// Why.
        reason: StopReason,
    },
    /// All modules stopped; the runtime is about to complete shutdown.
    RuntimeStopped,
    /// Final event of a runtime. Nothing follows it.
    ShutdownCompleted {
        /// Summary of the shutdown.
        report: ShutdownReport,
    },
    /// The runtime state changed.
    RuntimeStateChanged {
        /// Previous state.
        from: RuntimeState,
        /// New state.
        to: RuntimeState,
    },
    /// A module was accepted by the registry.
    ModuleRegistered {
        /// Module.
        module: ModuleId,
        /// Module version.
        version: Version,
    },
    /// A module changed lifecycle state.
    ModuleStateChanged {
        /// Module.
        module: ModuleId,
        /// Previous state.
        from: ModuleState,
        /// New state.
        to: ModuleState,
    },
    /// A capability was registered.
    CapabilityRegistered {
        /// Capability.
        capability: CapabilityId,
        /// Capability version.
        version: Version,
        /// Providing module.
        provider: ModuleId,
    },
    /// A session was created (the envelope's `session_id` identifies it).
    SessionCreated,
    /// A session changed state.
    SessionStateChanged {
        /// Previous state.
        from: SessionState,
        /// New state.
        to: SessionState,
    },
    /// Health of the runtime or a module changed.
    HealthChanged {
        /// Whose health.
        subject: HealthSubject,
        /// Previous health.
        from: HealthStatus,
        /// New health.
        to: HealthStatus,
    },
    /// An error was surfaced (already redacted).
    RuntimeError {
        /// Module involved, if any.
        module: Option<ModuleId>,
        /// The redacted error.
        error: ErrorReport,
    },
    /// A bounded queue was saturated. Coalesced: published once per
    /// saturation episode, with the running total of dropped items.
    QueueSaturated {
        /// Which queue.
        queue: QueueKind,
        /// Its capacity.
        capacity: u32,
        /// Items dropped or rejected so far for this queue.
        dropped_total: u64,
    },
    /// A module-defined event.
    Module(ModuleEvent),
    /// An audio device, stream or authorization event (Phase 2).
    Audio(crate::audio::AudioEvent),
    /// An audio-intelligence event (Phase 3).
    Analysis(crate::intel::AnalysisEvent),
    /// A speech-recognition or language event (Phase 4).
    Speech(crate::speech::SpeechEvent),
}

impl EventPayload {
    /// Stable type name, safe to log.
    pub fn kind(&self) -> &'static str {
        match self {
            EventPayload::RuntimeStarting => "runtime_starting",
            EventPayload::RuntimeStarted { .. } => "runtime_started",
            EventPayload::RuntimeStopping { .. } => "runtime_stopping",
            EventPayload::RuntimeStopped => "runtime_stopped",
            EventPayload::ShutdownCompleted { .. } => "shutdown_completed",
            EventPayload::RuntimeStateChanged { .. } => "runtime_state_changed",
            EventPayload::ModuleRegistered { .. } => "module_registered",
            EventPayload::ModuleStateChanged { .. } => "module_state_changed",
            EventPayload::CapabilityRegistered { .. } => "capability_registered",
            EventPayload::SessionCreated => "session_created",
            EventPayload::SessionStateChanged { .. } => "session_state_changed",
            EventPayload::HealthChanged { .. } => "health_changed",
            EventPayload::RuntimeError { .. } => "runtime_error",
            EventPayload::QueueSaturated { .. } => "queue_saturated",
            EventPayload::Module(_) => "module",
            EventPayload::Audio(_) => "audio",
            EventPayload::Analysis(_) => "analysis",
            EventPayload::Speech(_) => "speech",
        }
    }

    /// The topic used for subscription filtering.
    pub fn topic(&self) -> EventTopic {
        match self {
            EventPayload::RuntimeStarting
            | EventPayload::RuntimeStarted { .. }
            | EventPayload::RuntimeStopping { .. }
            | EventPayload::RuntimeStopped
            | EventPayload::ShutdownCompleted { .. }
            | EventPayload::RuntimeStateChanged { .. } => EventTopic::Runtime,
            EventPayload::ModuleRegistered { .. }
            | EventPayload::ModuleStateChanged { .. }
            | EventPayload::CapabilityRegistered { .. } => EventTopic::Modules,
            EventPayload::SessionCreated | EventPayload::SessionStateChanged { .. } => {
                EventTopic::Sessions
            }
            EventPayload::HealthChanged { .. } => EventTopic::Health,
            EventPayload::RuntimeError { .. } | EventPayload::QueueSaturated { .. } => {
                EventTopic::Diagnostics
            }
            EventPayload::Module(_) => EventTopic::ModuleEvents,
            EventPayload::Audio(_) => EventTopic::Audio,
            EventPayload::Analysis(_) => EventTopic::Analysis,
            EventPayload::Speech(_) => EventTopic::Speech,
        }
    }
}

/// Coarse event groups modules subscribe to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventTopic {
    /// Runtime lifecycle.
    Runtime,
    /// Module registration and lifecycle.
    Modules,
    /// Session lifecycle.
    Sessions,
    /// Health changes.
    Health,
    /// Errors and saturation.
    Diagnostics,
    /// Events emitted by other modules.
    ModuleEvents,
    /// Audio device, stream and authorization events (Phase 2).
    Audio,
    /// Audio-intelligence events (Phase 3).
    Analysis,
    /// Speech-recognition and language events (Phase 4).
    Speech,
}

/// The envelope every event travels in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// Envelope schema version.
    pub schema_version: u16,
    /// Unique event id.
    pub id: EventId,
    /// Position in the runtime's total order (starts at 1).
    pub sequence: u64,
    /// Wall-clock time at dispatch (informational).
    pub emitted_at: WallTime,
    /// Monotonic time at dispatch.
    pub monotonic: MonotonicTime,
    /// Producer.
    pub source: EventSource,
    /// Session the event belongs to, if any.
    pub session_id: Option<SessionId>,
    /// Correlation id of the operation that caused it, if any.
    pub correlation_id: Option<CorrelationId>,
    /// The event that directly caused this one, if any.
    pub causation_id: Option<EventId>,
    /// Sensitivity of the payload.
    pub privacy: DataClass,
    /// Payload.
    pub payload: EventPayload,
}

impl EventEnvelope {
    /// Reject envelopes whose schema version this build does not understand.
    pub fn check_schema(&self) -> Result<(), CeError> {
        if self.schema_version == EVENT_SCHEMA_VERSION {
            Ok(())
        } else {
            Err(CeError::new(
                ErrorCategory::Serialization,
                "event.unsupported_schema",
                "unsupported event schema version",
            )
            .with_context("found", self.schema_version.to_string())
            .with_context("supported", EVENT_SCHEMA_VERSION.to_string()))
        }
    }

    /// Stable payload type name.
    pub fn kind(&self) -> &'static str {
        self.payload.kind()
    }
}

/// What a module submits to emit an event. The runtime fills in sequence,
/// timestamps and source; a module cannot forge runtime events.
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleEventDraft {
    /// Event.
    pub event: ModuleEvent,
    /// Sensitivity of the attributes.
    pub privacy: DataClass,
    /// Session, if session-scoped. Must be an open session.
    pub session_id: Option<SessionId>,
    /// Correlation id to propagate.
    pub correlation_id: Option<CorrelationId>,
    /// Event that caused this one.
    pub causation_id: Option<EventId>,
}

impl ModuleEventDraft {
    /// A draft with no session or correlation.
    pub fn new(name: EventName, privacy: DataClass) -> Self {
        Self {
            event: ModuleEvent {
                name,
                version: 1,
                attributes: BTreeMap::new(),
            },
            privacy,
            session_id: None,
            correlation_id: None,
            causation_id: None,
        }
    }

    /// Add an attribute.
    pub fn attribute(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.event.attributes.insert(key.into(), value.into());
        self
    }

    /// Mark the draft as caused by `cause`, inheriting its session and
    /// correlation.
    pub fn caused_by(mut self, cause: &EventEnvelope) -> Self {
        self.session_id = cause.session_id;
        self.correlation_id = cause.correlation_id;
        self.causation_id = Some(cause.id);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn envelope(payload: EventPayload) -> EventEnvelope {
        EventEnvelope {
            schema_version: EVENT_SCHEMA_VERSION,
            id: EventId::from_uuid(Uuid::from_u128(1)),
            sequence: 7,
            emitted_at: WallTime::from_unix_nanos(10),
            monotonic: MonotonicTime::from_nanos(5),
            source: EventSource::Runtime,
            session_id: Some(SessionId::from_uuid(Uuid::from_u128(2))),
            correlation_id: Some(CorrelationId::from_uuid(Uuid::from_u128(3))),
            causation_id: None,
            privacy: DataClass::Operational,
            payload,
        }
    }

    #[test]
    fn envelopes_round_trip_with_explicit_type_tags() {
        let env = envelope(EventPayload::SessionStateChanged {
            from: SessionState::Created,
            to: SessionState::Active,
        });
        let json = serde_json::to_value(&env).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["payload"]["type"], "session_state_changed");
        assert_eq!(json["payload"]["data"]["to"], "active");
        assert_eq!(json["privacy"], "operational");
        let back: EventEnvelope = serde_json::from_value(json).unwrap();
        assert_eq!(back, env);
        back.check_schema().unwrap();
    }

    #[test]
    fn unsupported_versions_and_unknown_types_are_rejected() {
        let mut env = envelope(EventPayload::RuntimeStopped);
        env.schema_version = 2;
        let err = env.check_schema().unwrap_err();
        assert_eq!(err.category(), ErrorCategory::Serialization);
        let mut json = serde_json::to_value(envelope(EventPayload::RuntimeStopped)).unwrap();
        json["payload"]["type"] = "teleported".into();
        assert!(serde_json::from_value::<EventEnvelope>(json).is_err());
    }

    #[test]
    fn module_drafts_inherit_correlation_from_their_cause() {
        let cause = envelope(EventPayload::SessionCreated);
        let draft = ModuleEventDraft::new(
            EventName::new("ce.test.ack").unwrap(),
            DataClass::Operational,
        )
        .attribute("n", 1i64)
        .caused_by(&cause);
        assert_eq!(draft.session_id, cause.session_id);
        assert_eq!(draft.correlation_id, cause.correlation_id);
        assert_eq!(draft.causation_id, Some(cause.id));
        assert_eq!(draft.event.attributes["n"], Value::Integer(1));
    }

    #[test]
    fn every_payload_has_a_kind_and_topic() {
        let m = ModuleId::new("ce.test.m").unwrap();
        let cases = [
            (EventPayload::RuntimeStarting, EventTopic::Runtime),
            (
                EventPayload::ModuleStateChanged {
                    module: m.clone(),
                    from: ModuleState::Ready,
                    to: ModuleState::Running,
                },
                EventTopic::Modules,
            ),
            (EventPayload::SessionCreated, EventTopic::Sessions),
            (
                EventPayload::QueueSaturated {
                    queue: QueueKind::Command,
                    capacity: 1,
                    dropped_total: 1,
                },
                EventTopic::Diagnostics,
            ),
            (
                EventPayload::HealthChanged {
                    subject: HealthSubject::Module(m),
                    from: HealthStatus::Unknown,
                    to: HealthStatus::Healthy,
                },
                EventTopic::Health,
            ),
        ];
        for (payload, topic) in cases {
            assert_eq!(payload.topic(), topic);
            assert!(!payload.kind().is_empty());
        }
    }
}
