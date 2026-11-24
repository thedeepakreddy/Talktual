# ADR-0002 — One control-plane async runtime: Tokio, single-owner core

Status: Accepted (Phase 1) · Date: 2026-11-24

## Options considered
| Option | Pros | Cons |
| --- | --- | --- |
| **Tokio** | de-facto standard; mature bounded mpsc/broadcast/oneshot/Notify, JoinSet, paused-time testing; works on iOS/Android/desktop; MIT | heavier than minimal executors; timers unavailable on `wasm32-unknown-unknown` |
| smol / async-executor | small | smaller ecosystem; fewer primitives (bounded broadcast, JoinSet); no paused-time testing |
| Executor-agnostic core (own primitives) | maximum portability | re-implementing channels, timers and supervision — a large, risky surface |
| Threads + std channels | no async | no cooperative cancellation/timeouts; poor fit for I/O-bound adapters |

## Decision
* **Tokio**, features `rt`, `sync`, `time`, `macros` only. CE **runs on the
  host's Tokio runtime** (`Handle::try_current`) and never creates one —
  so there is never more than one runtime.
* `ce-core` stays executor-free: module services (`emit`, `spawn`,
  `cancelled`) are behind the object-safe `ModuleHost` trait, implemented by
  the runtime.
* **Single-owner core task** owns all runtime state; **one actor task per
  module** owns the instance; module background work runs in the actor's
  `JoinSet`. Cancellation uses `tokio_util::sync::CancellationToken` (root →
  module children).
* All queues bounded with explicit policies (events.md).

## Consequences
* No locks on runtime state; one total event order.
* Commands are serialized; a long `start` delays other commands (bounded by
  timeouts; admission policy makes it explicit).
* Browser embedding of the runtime needs a future spike (wasm timers, RNG).
* This runtime is control-plane only (ADR-0008).
