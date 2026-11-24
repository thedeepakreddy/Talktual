# ADR-0014 — Audio intelligence architecture

Status: Accepted (Phase 3) · Date: 2026-11-24

## Context
Phase 3 adds signal analysis (activity, segmentation, quality, conditions)
on top of the Phase 2 audio foundation. It must consume audio only through
Phase 2 interfaces, never delay capture, be replaceable and testable, and
publish results through the existing event system.

## Decision
* **New crate `ce-audio-intel`** (depends on `ce-core` and `ce-audio` only;
  `forbid(unsafe_code)`; no new third-party dependencies). Contracts
  (`AnalysisConfig`, `AnalysisEvent`, `AnalysisStatus`, …) live in
  `ce-core::intel` so hosts and other modules can use them without the
  implementation.
* **Data source: a bounded analysis tap** (`ce_audio::tap`) in the capture
  worker thread. Each block delivered to the host queue is also offered,
  with `try_send`, to the tap. A full tap drops the *analysis copy* and
  counts it; drops surface downstream as block-sequence gaps
  (`DiscontinuityContext::AnalysisDrop`). The device callback is untouched;
  host delivery is never delayed. The host's single-consumer
  `CaptureReader` is unaffected.
* **Pipeline** (single-threaded, deterministic per stream):
  `Framer` (blocks → 10/20/30 ms hops + gaps, features) → detector
  (`VoiceActivityDetector` trait) → `Stabilizer` (hysteresis) →
  `Segmenter` (segments, endpoint candidates, silence windows), with
  `QualityAnalyzer`, `ConditionTracker`, `AdaptivePolicy` and
  `ResourceGovernor` alongside — assembled by `StreamAnalyzer`.
* **One supervised worker thread per analysed stream**: `catch_unwind`
  around the body (panic → `Failed` + `Stopped(Failed)`), a heartbeat for
  stall detection by the core's housekeeping tick, a stop flag checked
  between blocks, per-block load measurement for the resource governor.
* **The runtime core owns analyses** (`runtime/core/intel.rs`): commands
  `StartAnalysis` / `StopAnalysis` / `QueryAnalysis`, a non-serializable
  `start_analysis_with(detector)` for adapters, event publication as
  `EventPayload::Analysis` in the stream's session with the runtime's single
  sequence, health (`RuntimeHealth::analyses`) and metrics. Session close and
  shutdown stop analyses first, with explicit reasons, bounded by the audio
  operation timeout.
* **Stream end reaches the analysis through the tap**, which carries the
  exact reason (closed, revoked, invalidated, failed). Revocation is marked
  before the queue drains so queued audio is discarded, not analysed.
* **DSP is hand-written** (radix-2 FFT, Hann window, decimation, levels):
  one transform size per stream, tested against a direct DFT; avoids a
  dependency for ~100 lines.

## Alternatives rejected
* *Second `CaptureReader` / tee in the host queue*: the reader is
  single-consumer and its overflow policy belongs to the host.
* *Analysis inside the capture worker*: any slowness would delay capture.
* *Analysis as a Phase 1 module consuming events*: audio is not an event
  (biometric data never enters the event bus).
* *Async tasks instead of threads*: CPU-bound work would block the shared
  Tokio runtime; a thread per stream is bounded by the stream count.

## Consequences
Analysis can fall behind, fail or stall without affecting capture, other
streams or the control plane (tested). The cost is one thread and one
bounded queue (default 50 blocks ≈ 0.5 s) per analysed stream.
