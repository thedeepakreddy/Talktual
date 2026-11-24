# Real-time boundary

Phase 1 defined this boundary; **Phase 2 implements the first code on the
media side of it** (device callbacks through `ce-audio`'s endpoints; see
`audio.md`, ADR-0011). CE still makes **no latency, jitter, battery or
device claims**: the only backend run so far is ALSA's virtual `null` PCM.

## Two execution domains

| | Control plane (Phase 1) | Real-time media plane (Phase 2+) |
| --- | --- | --- |
| Runs on | Tokio tasks (core, module actors, supervised tasks) | platform audio callback threads (via the cpal adapter, ADR-0010), plus one CE stream worker thread per stream that drains/fills the ring |
| Work | lifecycle, commands, events, health, sessions | capture/playback buffers, DSP |
| May | allocate, await, log via the sink, use channels | only bounded, pre-allocated, wait-free operations |
| Must not | be used for per-sample/per-buffer audio work | block, lock contended mutexes, allocate unpredictably, log synchronously, call the observability sink, perform I/O or network calls, await |

## Rules for future audio callbacks

1. Exchange data with the control plane only through pre-allocated,
   fixed-capacity, lock-free single-producer/single-consumer structures.
2. Overflow/underflow is counted with atomics and reported by a control-plane
   task, never from the callback.
3. Buffers are sized and allocated before the stream starts.
4. No `CeError` construction, formatting or event emission inside the
   callback.
5. Callback threads are owned by the platform adapter; their lifecycle is
   mirrored into CE module lifecycle by the adapter's control-plane side.
6. Latency must be measured on real target devices before any claim is made.

## How Phase 2 implements the rules

| Rule | Implementation |
| --- | --- |
| 1. SPSC, pre-allocated, fixed capacity | `rtrb` rings sized at stream creation (`CaptureEndpoint`, `PlaybackEndpoint`) |
| 2. Overflow/underflow counted atomically, reported elsewhere | `StreamShared` atomics; the worker reports to the core; the core emits events |
| 3. Buffers allocated before start | rings and marker rings allocated in `spawn_stream` |
| 4. No errors/formatting/events in the callback | callbacks only convert and copy; faults set an atomic (`FaultReporter`) |
| 5. Callback threads owned by the adapter | cpal owns them; dropping the backend stream quiesces them before CE releases anything |
| 6. Measure before claiming | `examples/audio_probe.rs` + `docs/phases/phase-2-measurements.md`; no hardware measured yet |

The worker thread is *not* a real-time thread (it may allocate and lock the
non-real-time host queues) but it never blocks the callback: the only shared
structures are the wait-free rings and atomics.

## Why the control plane is not real-time

Tokio schedules cooperatively; a task can be delayed by other tasks, timers
and the OS scheduler, and `tokio::time` has millisecond granularity. That is
appropriate for orchestration and unacceptable for audio. Treating "async" as
"real-time" is listed in the risk register (spec §18).
