# ADR-0012 — Normalization contract: interleaved f32 at the device rate

Status: Accepted (Phase 2) · Date: 2026-11-24

## Decision
CE-facing audio (capture blocks, playback writes) is **interleaved `f32`,
nominal range [-1, 1], at the device's resolved sample rate**, with the
device's channel count or (capture, opt-in) a plain-mean mono downmix.
No resampling in Phase 2.

## Rationale
* **Phase 3/4 needs** (VAD, ASR, translation pipelines) universally accept
  float PCM; most want mono 16 kHz. Mono is provided by downmix. 16 kHz is
  provided by *negotiation* — CE asks the device for the requested rate and
  reports exactly what it got — rather than by a resampler whose quality and
  latency trade-offs need Phase 3 measurement to choose.
* **Backend coverage**: every backend format CE supports converts to `f32`
  cheaply in the callback; integer formats round-trip losslessly for 8/16
  bit; 32-bit sources keep 24 bits of precision (≤ 128 LSB error), inherent to
  `f32`.
* **Measured cost**: conversion is O(samples) arithmetic; on ALSA's null PCM
  the whole pipeline sustained >100× real time on one core (not a hardware
  measurement; see phase-2-measurements.md).

## Rules
* Requested vs. resolved: every deviation is listed in
  `ResolvedFormat::adjustments`; the actual backend buffer size is recorded
  after open.
* No silent format changes: a backend format change invalidates the stream.
* Float output to devices is clamped to [-1, 1]; non-finite samples become
  silence; integer output rounds half away from zero and clips.

## Deferred
Resampling (and its quality/latency choice) — Phase 3, if real devices
cannot provide the rates ASR needs.
