# Real-time boundary

Phase 1 contains **no real-time code** and makes **no latency, jitter,
battery or device claims**. It defines the boundary future phases must
respect.

## Two execution domains

| | Control plane (Phase 1) | Real-time media plane (Phase 2+) |
| --- | --- | --- |
| Runs on | Tokio tasks (core, module actors, supervised tasks) | platform audio callback threads (e.g. CoreAudio, AAudio/Oboe, WASAPI; possibly via CPAL — a candidate, not a decision) |
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

## Why the control plane is not real-time

Tokio schedules cooperatively; a task can be delayed by other tasks, timers
and the OS scheduler, and `tokio::time` has millisecond granularity. That is
appropriate for orchestration and unacceptable for audio. Treating "async" as
"real-time" is listed in the risk register (spec §18).
