# ADR-0011 — Audio streams: core-owned state, per-stream worker threads, lock-free handoff

Status: Accepted (Phase 2) · Date: 2026-11-24

## Decision
Three execution domains with one-way responsibilities:

1. **Device callback** (backend thread): touches only `CaptureEndpoint::push`
   / `PlaybackEndpoint::pull` and `FaultReporter` — pre-allocated `rtrb`
   SPSC rings and atomics. No locks, allocation, logging or I/O.
2. **Stream worker** (one OS thread per stream, owned by `ce-audio`): owns
   the backend stream (thread-bound handles are fine), executes
   open/start/stop/close, drains/fills the ring every `worker_poll_ms`,
   assembles blocks, applies overflow policy, reports faults/pressure.
   Runs under `catch_unwind`.
3. **Runtime core** (Tokio control plane, Phase 1): sole owner of stream
   state, the device catalog and the authorization gate; emits all audio
   events in the runtime's single sequence; waits on workers with deadlines
   while draining ingress, so worker reports can never deadlock it.

Hosts receive data through bounded queues (`CaptureReader`,
`PlaybackWriter`) that never touch the callback.

## Why plain threads for workers
Device APIs block (open can take 100s of ms) and some backend stream handles
must stay on their creating thread. Tokio's blocking pool offers neither
thread affinity nor per-stream supervision. A worker per stream is bounded by
`max_streams` (default 8).

## Why not do conversion/assembly in the callback
Format conversion to `f32` *is* done in the callback (O(n), branch-light,
allocation-free); block assembly, channel downmix, timestamping, queueing and
policy are not, to keep the callback minimal and predictable.

## Shutdown and cleanup
Close = gate closed → backend stream dropped (adapter guarantees callback
quiescence) → data plane ended → counters returned → thread joined if
finished, else kept as a zombie and joined later; shutdown waits for zombies
up to its deadline and reports `clean = false` if any remain.

## Consequences
* One extra copy (ring → block `Vec`) per capture block; justified to
  decouple host lifetimes from device memory.
* Poll-driven draining adds up to `worker_poll_ms` (default 2 ms) of
  latency; acceptable for Phase 2, to be measured on real devices.
