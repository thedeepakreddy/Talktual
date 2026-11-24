# Events, commands and bounded queues

## Envelope

```text
EventEnvelope {
  schema_version: u16      // EVENT_SCHEMA_VERSION = 1
  id: EventId              // unique
  sequence: u64            // total order within one runtime, starts at 1
  emitted_at: WallTime     // UTC ns, informational, may jump
  monotonic: MonotonicTime // ns since the runtime clock origin, never decreases
  source: Runtime | Module(ModuleId)
  session_id, correlation_id, causation_id: Option<…>
  privacy: DataClass       // classification of the payload
  payload: EventPayload    // {"type": "...", "data": {...}}
}
```

Payloads are a closed, typed, `#[non_exhaustive]` enum: `RuntimeStarting`,
`RuntimeStarted`, `RuntimeStopping`, `RuntimeStopped`, `ShutdownCompleted`,
`RuntimeStateChanged`, `ModuleRegistered`, `ModuleStateChanged`,
`CapabilityRegistered`, `SessionCreated`, `SessionStateChanged`,
`HealthChanged`, `RuntimeError`, `QueueSaturated`, `Audio(AudioEvent)`
(Phase 2, topic `Audio`), `Analysis(AnalysisEvent)` (Phase 3, topic
`Analysis`, see [audio-intelligence.md](audio-intelligence.md)),
`Speech(SpeechEvent)` (Phase 4, topic `Speech`, see [speech.md](speech.md);
the envelope's `privacy` is `Content` for transcripts and `Personal` for
language observations), `Semantic(SemanticEvent)` (Phase 5, topic
`Semantic`, see [semantic.md](semantic.md); `Content` for observations,
`Operational` for lifecycle, context and rejection events) and
`Module(ModuleEvent)`.

**Module events** (provisional) carry a namespaced `EventName`, a module-owned
attribute schema version and structured attributes. Modules submit a
`ModuleEventDraft`; the runtime assigns sequence, timestamps and source, so a
module cannot forge runtime events. Typed per-module payload schemas are a
later-phase decision.

## Versioning

Adding a payload variant or an optional field is compatible and keeps
`schema_version`. Anything else increments it. `EventEnvelope::check_schema`
rejects other versions; unknown payload types fail to deserialize. Both map to
the `Serialization` error category. Every event observed in the integration
tests is round-tripped through JSON.

## Ordering guarantees

* The core assigns `sequence` from one counter in dispatch order.
* Every observer — each external subscription and each module mailbox —
  receives events in strictly increasing `sequence` order. No duplicates.
* Observers may **miss** events under saturation, but never silently (below).
* Events caused by one command are dispatched while that command is processed,
  so they are contiguous in the order relative to that command's other events.
* Module events are ordered by the time the core dequeues them from ingress.
* Timestamps never define order; `sequence` does.

## Correlation

* Each command carries a `CorrelationId` (fresh, or caller-supplied via
  `Engine::execute_correlated`). Runtime events dispatched while handling it
  carry it.
* A session's `correlation_id` is the correlation id of the command that
  created it; session-scoped runtime events carry the session id and that
  correlation id.
* `ModuleEventDraft::caused_by(&event)` propagates session and correlation and
  sets `causation_id`, building a causal chain.

## Commands

`Command` is a serializable enum: `Start`, `CreateSession`, `CloseSession`,
`ListCapabilities`, `QueryHealth`, `Shutdown`. `Command::validate` checks
arguments; the runtime checks state. Commands are processed one at a time by
the core, so a long `Start` (bounded by module operation timeouts) delays
later commands — which is what the command admission policy is for. Module
registration is a separate facade call because it carries code (a factory).

## Bounded queues and overflow policies

| Queue | Capacity (default) | When full | Visibility |
| --- | --- | --- | --- |
| Commands (host → core) | `command_queue_capacity` (64) | `command_admission`: `Reject` immediately, or `WaitUpTo{timeout_ms}` (default 500 ms) then reject | `QueueSaturated` error (retryable); `commands_rejected` metric; coalesced `QueueSaturated{Command}` event |
| Event ingress (modules → core) | `event_ingress_capacity` (256) | `emit` returns `QueueSaturated` immediately (never blocks a module) | metric `events_rejected_ingress`; coalesced `QueueSaturated{EventIngress}` event |
| Subscriber (core → host) | `event_broadcast_capacity` (1024) per subscriber | oldest events overwritten for that subscriber only | next `recv` returns `Lagged{missed}`; metric; `QueueSaturated{Subscriber}` event |
| Module mailbox (core → module) | `module_mailbox_capacity` (128) | `module_mailbox_overflow`: `DropNewest` or `FailModule` | metric `events_dropped_mailbox`; `QueueSaturated{ModuleMailbox}` once per saturation episode, or module `Failed` |
| Module task spawns | `module_task_queue_capacity` (16), at most `max_tasks_per_module` (32) live tasks | `spawn` returns `QueueSaturated` | error to the module |
| Module control | 4 (internal) | lifecycle ops are serialized, so it cannot fill | — |

The dispatcher **never blocks** on a subscriber or module: one slow consumer
cannot stall the others. Every capacity is a `QueueCapacity` (1..=65 536), so
"unbounded" cannot be configured. Saturation events are coalesced so that
reporting saturation cannot itself flood the system: a mailbox reports once
per episode (reset when a delivery succeeds); ingress and command saturation
are reported by the core when it next runs.

Tests: `tests/backpressure.rs` covers each row above.
