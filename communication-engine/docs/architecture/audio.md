# Audio foundation (Phase 2)

Decisions: ADR-0010 (backend), ADR-0011 (stream architecture), ADR-0012
(normalization), ADR-0013 (authorization gate).

## 1. Layers

```
host ──► Engine facade: audio_devices · update_consent · open_capture/open_playback
                         start_audio · stop_audio · close_audio · audio_streams
           │ (commands; one sequence of events)
           ▼
runtime core (Tokio) — owns stream state, device catalog, authorization gate
           │ WorkerCommand (bounded)            ▲ WorkerReport / BackendNotice (bounded)
           ▼                                    │
stream worker thread (ce-audio, one per stream) — owns BackendStream
   drains/fills ring · assembles blocks · timestamps · overflow policy
           │ CaptureReader / PlaybackWriter (bounded queues) ──► host
           ▼
device callback (backend thread) — CaptureEndpoint::push / PlaybackEndpoint::pull
   pre-allocated rtrb SPSC rings + atomics only
           ▲
AudioBackend adapter (cpal, fake, …) — library/platform specifics only
```

| Crate | Audio content |
| --- | --- |
| `ce-core::audio` | typed contracts: formats, devices, requests, lifecycle table, negotiation, frame metadata, authorization, events, config. No library, no runtime. |
| `ce-audio` | `AudioBackend` trait, real-time endpoints, conversion, workers, readers/writers; `cpal` adapter (feature). |
| `communication-engine` | orchestration in the core, facade methods. |
| `ce-testkit::fake_audio` | scriptable fake backend with fault injection. |

## 2. Contracts

* **Device catalog** — `DeviceCatalog { backend, devices, default_input,
  default_output, problems }`. Each `AudioDeviceDescriptor` has an opaque
  `AudioDeviceId` (`dev_N`), the backend key's stability, input/output
  support, sanitized format ranges, default formats and a count of skipped
  formats. Names are personal data: available to hosts, never in events or
  logs.
* **Unstable identifiers** — backend keys map to CE ids for the runtime's
  lifetime; a re-added key gets its old id; ids are never reused. Opening a
  stale id fails with `audio.device_not_found`.
* **Request → resolved** — `StreamRequest { direction, device,
  format: FormatPreference, channel_mode }` →
  `ResolvedFormat { requested, device, delivered, requested_buffer_frames,
  actual_buffer_frames, adjustments }`. `negotiate()` is pure and documented
  (rate first, then channels, then format; values clamped into the chosen
  range); the actual buffer size is filled in after open.
* **Frames** — `AudioBlock { meta: AudioFrameMeta, samples: Vec<f32> }`,
  meta carrying stream/session/correlation ids, `sequence`, `frame_index`,
  frame count, rate, channels, layout, format, `timestamp`, `host_time`,
  `device_time` and an optional `Discontinuity`.

## 3. Stream lifecycle

`Configured → Open → Running ⇄ Stopped`, any of these → `Invalidated` /
`Failed` / `Closed`; `Invalidated|Failed → Closed`. Exhaustively tested
table (`AudioStreamState::can_transition_to`). Double stop, double close,
operations on closed (stale) ids and restart after invalidation are
rejected with typed errors and no side effects. Every transition emits
`AudioEvent::StreamStateChanged` with the session's correlation id.

| Operation | Pre-state | Success | Failure |
| --- | --- | --- | --- |
| open | — | Configured → Open (+`StreamFormatResolved`) | error/timeout/panic → Failed → Closed, error returned |
| start | Open, Stopped | Running | device error: stays, non-terminal `StreamFailed`; timeout → Failed |
| stop | Running | Stopped (partial block flushed) | Failed (fail-safe: device may still run) |
| close | any but Closed | Closed, device released, callbacks quiesced | worker unresponsive → Failed, thread kept as zombie until it exits |

Session close and runtime shutdown close all streams first.

## 4. Clock policy and discontinuities

* `frame_index` — sample-clock position since first start, including lost
  frames: the authoritative timeline.
* `timestamp` — CE monotonic ns of the block's first frame, derived from the
  sample clock and anchored at the host time when data was first seen after
  each start (anchor accuracy ≈ poll interval + one device buffer; relative
  timing exact).
* `host_time` — CE monotonic ns at assembly; compare with `timestamp` to see
  device/host drift (never assumed equal).
* `device_time` — backend stream clock, separate time base, when available.
* Discontinuities flag the block immediately after a gap:
  `DeviceOverflow` (callback dropped frames; placed at the exact frame via
  gap markers, `frame_index` advances by `lost_frames`), `DeliveryOverflow`
  (blocks dropped from the delivery queue), `StreamRestart` (stop/start; real
  time elapsed, `frame_index` does not advance). Blocks are always contiguous
  internally: a gap ends the current block early.

## 5. Buffers, ownership and overflow policy

| Path | Bound (default) | Policy when full | Visibility |
| --- | --- | --- | --- |
| Capture ring (callback → worker) | `capture_ring_ms` = 250 ms of device frames | drop newest frames in the callback | `ring_overflow_frames`, `Overflow{DeviceRing}`, `DeviceOverflow` flag |
| Capture queue (worker → host) | `capture_queue_blocks` = 64 blocks | `DropOldest` (default) or `DropNewest` | `queue_dropped_*`, `Overflow{DeliveryQueue}`, `DeliveryOverflow` flag, sequence gap |
| Playback queue (host → worker) | `playback_queue_blocks` = 64 writes of ≤ `playback_max_write_ms` | `write` waits (backpressure); `try_write` rejects `QueueSaturated` | error |
| Playback ring (worker → callback) | `playback_ring_ms` = 250 ms | callback outputs silence; counted as underrun only if data was pending upstream | `underrun_frames`, `Underflow` |
| Marker rings (gaps, timing) | 64 entries | gap: carried to next marker; timing: skipped | — |
| Worker command channel | 4 | `audio.worker_busy` | error |
| Audio signal channel (workers/backend → core) | 256 | faults block (core always drains); pressure/notices skipped | — |

Memory per stream is bounded by `ring_frames × channels × 4 B` plus
`queue_blocks × block_frames × channels × 4 B` (e.g. 48 kHz stereo,
defaults: 96 KB ring + 245 KB queue). Capture blocks are fresh `Vec<f32>`
owned by the host after `recv`; nothing a host holds aliases device memory.
Playback writes are moved into the queue and copied into the ring.

Pressure events are coalesced: at most one report per
`pressure_report_interval_ms` per stream.

## 6. Device and route changes

* **Notified** (`BackendNotice::DevicesChanged`) or **polled**
  (`device_poll_interval_ms`) changes trigger re-enumeration and a diff:
  `DeviceAdded`, `DeviceRemoved`, `DefaultDeviceChanged`.
* A stream whose device disappears (backend fault or catalog diff) becomes
  **Invalidated**: data captured before the loss is delivered, then readers
  get `Invalidated`; the host must close it. **No silent restart.**
* A backend that reroutes a default-device stream (cpal `DeviceChanged`)
  produces `StreamRerouted`; the stream keeps running.
* If the default changes while a stream opened on the default stays bound to
  the old device, `StreamDefaultDeviceDiverged` reports it once; CE does not
  move the stream.

## 7. Failure handling

| Failure | Result |
| --- | --- |
| backend `DeviceNotAvailable` / `StreamInvalidated` | Invalidated |
| backend `PermissionDenied` on capture | permission → Denied, capture revoked |
| other backend stream error | Failed (terminal) |
| `Xrun` | counted (`backend_xruns`) |
| open error / timeout / worker panic | Failed → Closed; error returned; panic contained by `catch_unwind` |
| start error | non-terminal; stream stays Open/Stopped |
| stop error | Failed (fail-safe) |
| enumeration failure / timeout | typed error, `EnumerationFailed`, previous catalog kept |
| invalid backend descriptors | ranges sanitized or skipped (counted); unusable devices listed in `problems` |

Audio streams that are Invalidated or Failed degrade runtime health.

## 8. Observability and privacy

Events: device enumeration/changes, permission changes, stream lifecycle,
resolved formats, reroutes/divergence, overflow/underflow/discontinuity,
failures, capture denied/revoked — all `operational`. Logs record state
transitions and errors with operational fields only. Metrics:
`audio_streams_opened`, `audio_capture_denied`, `audio_capture_revoked`,
`audio_streams_invalidated`, `audio_streams_failed`, plus per-stream
counters in `audio_streams()` and `RuntimeHealth`. **Audio content is never
logged, put in events or persisted.**

## 8a. Analysis tap (Phase 3)

`WorkerHandle::attach_tap` gives a capture stream a bounded, non-blocking
copy of every delivered block for in-engine analysis (`ce_audio::tap`). The
worker thread offers each block with `try_send`; a full tap drops the copy
and counts it, never delaying the host queue. When the stream ends the tap
ends with the exact reason (closed, revoked, invalidated, failed; a faulted
stream drained before the runtime marks it resolves the reason from the
fault). On revocation the reason is set first so queued copies are
discarded. See [audio-intelligence.md](audio-intelligence.md).

## 9. Platform matrix (Phase 2)

| Target | cpal host | Built | Run here | Capture permission source |
| --- | --- | --- | --- | --- |
| Linux x86_64 | ALSA | yes | yes — ALSA `null` PCM only (virtual, no hardware) | `NotRequiredByPlatform` |
| macOS aarch64 | CoreAudio | `cargo check` | no | host via `PermissionOverride` (else denied) |
| iOS aarch64 | CoreAudio (AVAudioSession) | `cargo check` | no | host via `PermissionOverride` (else denied) |
| Android aarch64 | AAudio | `cargo check` | no | host via `PermissionOverride` (else denied) |
| Windows x86_64 | WASAPI | `cargo check` | no | host via `PermissionOverride` (else denied) |
| Browser (wasm32) | — | contracts and ce-audio compile; runtime does not (Phase 1) | no | n/a |
