# ADR-0019 — Speech recognition architecture

Status: Accepted (Phase 4) · Date: 2026-11-24

## Context
Phase 4 adds streaming speech recognition and text language observations
on capture streams. It must consume the Phase 2 audio interfaces and the
Phase 3 analysis without ever delaying capture, playback, analysis or the
control plane, keep vendor/model APIs replaceable, and be reproducible in
replay. No real ASR model may be downloaded without approval.

## Decision
* **One new crate, `ce-speech`** (contracts stay in `ce-core::speech`):
  adapter boundary, transcript semantics, language identification,
  resampling, the per-stream recognizer, the supervised worker and the
  evaluation harness. It depends on `ce-core`, `ce-audio` (tap),
  `ce-audio-intel` (synthetic blocks and scenario building for replay),
  `tokio` (sync only), `serde`/`serde_json` (manifests) and `whichlang`.
* **Adapter boundary** (`AsrFactory` + `AsrAdapter`): discovery
  (`capabilities()`) is cheap and never loads a model; `create()` is cheap;
  `load()` (expensive, may reuse a shared model) runs **on the recognition
  worker thread**, never in an audio callback or on the control plane;
  utterances are `begin` → `accept`* → `finalize` or `reset`; unload is
  drop. Adapters return *full* hypotheses of the utterance so far with
  positions in utterance samples; CE derives commits, deltas, frames and
  times. Unsupported capabilities are declared `false`/`None`, and CE drops
  any output the adapter did not declare (scores without semantics, word
  timings, languages).
* **Deterministic core** (`StreamRecognizer`): one single-threaded state
  machine per stream fed with blocks and Phase 3 events; the same code runs
  in the live worker and in replay. Wall-clock time enters only through
  load time, processing time and the governor's load input.
* **Worker**: one supervised thread per recognized stream. Inputs: a
  dedicated bounded tap on the capture worker (multi-subscriber tap, at
  most 4 per stream, each dropping independently) and a bounded hint queue
  the core feeds with `try_send`. Output: a bounded Tokio channel. Panics
  are caught (`Failed` + `Stopped(Failed)`); a heartbeat and a published
  load start let the core detect stalls and load timeouts and abandon the
  worker (it then sends nothing more and is joined, bounded, at shutdown).
* **Runtime**: `runtime/core/speech.rs` owns recognitions: start (validates
  config, resolves the adapter, requires a running analysis for
  `ActivityHints`, enforces `max_concurrent_recognitions` and the adapter's
  `max_concurrent_streams`), stop, reset, query, list; forwards Phase 3
  events; publishes `EventPayload::Speech` with the event's privacy class;
  stops recognitions before analyses on session close and shutdown.
* **Phase 3 integration**: activity onsets, segment ends and endpoint
  candidates are *hints* (ADR-0021); degraded `Shedding` or a stopped
  analysis switches delimitation to `Continuous`, reported by
  `HintsChanged`.
* **Resource policy** mirrors ADR-0018: load = processing time / audio
  time; `Throttled` mode (partials off, chunks ×4) after `degrade_after`
  overruns, recovery after `recover_after` healthy blocks; tap drops are
  reported (`QueuePressure`) and become discontinuities.
* **Real ASR**: blocked pending approval of a model download (ADR-0020).
  Phase 4 ships the scripted adapter (tests) and the reference **oracle**
  adapter (pipeline evaluation only).

## Alternatives
* Recognition inside the Phase 3 analysis thread: couples failure domains
  and forces analysis to pay ASR latency. Rejected.
* Async tasks instead of threads: model inference is blocking CPU work and
  would starve the runtime's executor. Rejected.
* Running ASR as a `Module` (Phase 1 contract): modules have no data-plane
  access by design; recognition needs the tap. Rejected for Phase 4.

## Consequences
Recognition can fail, stall or fall behind without touching capture or
analysis; every such case is a typed event. Memory per recognition is
bounded (tap, hint queue, pre-roll ring, boundary and deferred-hint queues,
one utterance's text). A real adapter only implements two traits.
