# Phase 2 Performance Report

All results are from this environment (4-vCPU shared container, Node 22.23.5, headless Chromium 157). Treat absolute numbers as indicative; the before/after comparisons, made with the same scripts on the same machine, are the useful part. **Targets are not claimed as achieved on real devices.**

## 1. End-to-end turn latency (emulated)

**Method** (`e2e/latency.mjs`): two Chromium contexts, production build, real server, P2P data channel ("Direct"), 30 turns per run. The speech recognizer is scripted and the translation provider is a 0 ms stub, so this measures **client pipeline + transport + server overhead only**: no real recognition, provider or TTS latency.

| Metric | Baseline (Phase 1, `89ef81a`) run 1 / run 2 | Phase 2 run 1 / run 2 |
| --- | --- | --- |
| Finalized phrase → listener speech starts, p50 | 15 / 15 ms | 15 / 16 ms |
| … p95 | 28 / 19 ms | 31 / 30 ms |
| … max | 29 / 28 ms | 32 / 33 ms |
| Button release → listener speech starts, p50 | 12 / 12 ms | 11 / 13 ms |
| Press → recognizer started (new) | n/a (always on) | 1–2 ms p50 *(fake recognizer — real start-up NOT measured)* |
| Release → recognizer stopped (new) | n/a (never stopped) | **452 ms p50, max 459 ms** (450 ms tail + flush) |

**Reading:** the engine layer adds no measurable latency at p50; p95 is a few ms higher (likely the extra snapshot/event work per segment, within run-to-run noise). The release→stop figure bounds how long audio can keep flowing after release.

**Unmeasured, and the most important open number:** real push-to-talk **first-word latency**. Phase 1 pre-armed the recognizer; Phase 2 starts it on press (P1). Real Chrome/Safari recognizers take time to start, which can clip the first syllable. This must be measured on devices (release plan stage 1). If it's material, mitigations that keep the privacy contract include a short visual "get ready" cue, or starting the recognizer on pointer-*down* intent (already the case) with a brief pre-roll from a local buffer (Phase 3 work).

## 2. Server (`e2e/perf.mjs`, Phase 1 server vs Phase 2 server, same run)

| Metric | Phase 1 server | Phase 2 server |
| --- | --- | --- |
| Startup → first `/health` | 144.7 ms | 133.8 ms |
| Idle RSS | 57.3 MB | 56.5 MB |
| `GET /health` p50 / p95 / p99 | 0.59 / 0.97 / 2.46 ms | 0.70 / 1.03 / 3.05 ms |
| `POST /api/translate` (passthrough) p50 / p95 | 0.61 / 0.97 ms | 0.72 / 1.03 ms |
| WS relay round trip p50 / p95 / p99 (n=2900) | 0.13 / 0.20 / 0.39 ms | 0.13 / 0.24 / 0.44 ms |
| 1000 join/leave cycles | 1396 ms (1.40 ms/cycle), RSS 111.7→112.9 MB | 1515 ms (1.51 ms/cycle), RSS 111.4→111.8 MB |
| 250 concurrent pairs | RSS 112.9→112.7 MB | RSS 111.8→111.9 MB |

The seat/resume logic adds ~0.1 ms per join/leave cycle. No RSS growth trend at this resolution. Sessions return to 0 after calls (E2E assertion).

## 3. Engine soak (`session.longrun.test.ts`, jsdom, fake adapters)

20,000 inbound segments (each played to completion) + 2,000 outbound segments over 200 pushes through one session:

| Check | Result |
| --- | --- |
| Inbound processing | 437 ms total (≈22 µs/segment) |
| Outbound (translate → send, serialized) | 62 ms for 2,000 |
| Transcript | capped at 500 |
| Diagnostics ring / metric samples | ≤ 300 / ≤ 200 each |
| Playback queue after drain | 0 |
| Event listeners | unchanged (no leak) |
| Recognizer instances | 1 reused across 200 pushes |

## 4. Buffer bounds (verified by tests)

| Buffer | Bound | Test |
| --- | --- | --- |
| Playback queue | 12 waiting; oldest waiting dropped with `dropped` status | playback.test |
| PCM scheduled ahead of the audio clock | ≤ 10 s (+1 chunk); reader waits | playback.test (removing the loop fails the test) |
| Transport outbox | 20 segments; partials never queued; latest hello only | webrtc.test |
| Server-STT single recording | 60 s | speechToText.test |
| Received-id dedupe | 500 ids (a resend older than that would be accepted — documented limit) | core + longrun tests |
| Transcript / diagnostics | 500 / 300 events, 200 samples per metric | longrun |
| Translation caches | 500 client / 2000 server (Phase 1) | Phase 1 tests |

## 5. Cancellation timing
* Barge-in: `cancelAll()` is synchronous; queued segments never start afterwards; the in-flight TTS request is aborted (tests).
* Translation: aborted on leave; a timeout is not retried (≤ 10 s per stalled segment on the serialized path, as in Phase 1); network errors retry once after 300 ms.
* Reconnect: backoff 0.5, 1, 2, 4, 8, 8 s (≈23.5 s of attempts) against a 20 s server seat hold.

## 6. Bundle
575.05 kB → 610.08 kB JS (+35 kB; gzip 196.9 → 207.2 kB) for the engine, adapters and consent UI. Code-splitting the QR scanner (Phase 1 recommendation) is still the cheapest win.

## 7. Caveats
Single machine, small run counts (2 × 30 turns), fake speech/TTS, localhost network. No mobile CPU, no real networks, no hours-long real-browser sessions. Re-run `node e2e/latency.mjs . 30` and `node e2e/perf.mjs` on target hardware for comparable numbers.
