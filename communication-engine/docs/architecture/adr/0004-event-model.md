# ADR-0004 — Typed, versioned event envelope with a single sequence

Status: Accepted (Phase 1) · Date: 2026-11-24

## Decision
* One `EventEnvelope` type with id, `sequence`, wall + monotonic timestamps,
  source, session/correlation/causation ids, privacy class and a typed,
  `#[non_exhaustive]`, adjacently tagged payload enum. `schema_version = 1`.
* Runtime events are a closed enum; module events are `Module(ModuleEvent)`
  with a namespaced name, module-owned version and structured attributes
  (provisional — avoids a generic type parameter across the whole runtime in
  Phase 1, while keeping names validated and payloads classified).
* Ordering: one counter in the single-owner core; every observer receives
  increasing sequences; loss is always reported (`Lagged`, `QueueSaturated`).
* Delivery: `tokio::sync::broadcast` for external subscribers (per-subscriber
  bounded, drop-oldest with lag count); bounded mpsc mailboxes for modules
  (drop-newest or fail-module), never blocking the dispatcher.
* Events are `serde`-serializable for future transports.

## Rejected
* Stringly-typed topics/payloads (spec §7).
* Blocking backpressure from subscribers into the dispatcher (one slow
  consumer would stall the engine).
