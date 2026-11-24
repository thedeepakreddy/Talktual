# Phase 2 — Reconnaissance, baseline and plan

Date: 2026-11-24 · Branch: `deepak/gallant-hamilton-wegi2i`, restarted from
`main` @ `0ad2dc6` (Phase 1 merged as PR #8 plus one later Talktual UI
commit). Working tree clean before work began.

## Findings

* **CE Phase 1** (`communication-engine/`): `ce-core`, `communication-engine`,
  `ce-testkit`; ADRs 0001–0009; `scripts/verify.sh`; path-filtered CI.
  The real-time boundary (realtime.md, ADR-0008) and consent representation
  (privacy.md) were designed for this phase.
* **Talktual audio** is browser-only: `src/lib/audio.ts` (getUserMedia, one
  AudioContext, mic epochs, device-lost listeners, `micPermission()`),
  `src/lib/textToSpeech.ts` (Web Audio PCM schedule). No native code.
* **Unresolved platform decision**: DR-8.5-1 (web-first vs. thin native iOS
  shell vs. hybrid) is *Proposed*, pending device tests. CE must therefore be
  usable from iOS, Android and desktop hosts and must not assume the browser.
* **Environment / hardware**: Ubuntu 24.04 container, Intel Xeon 2.1 GHz,
  4 vCPU, 16 GB. **No `/dev/snd`, no sound card, no audio server, no attached
  user computer.** ALSA runtime present; `libasound2-dev` installed for the
  build. ALSA exposes a virtual `null` PCM (capture = zeros, playback =
  discard) and a `default` PCM that fails (no card).
* **Build targets available**: x86_64 Linux (run); aarch64 iOS, aarch64
  Android, x86_64 Windows, aarch64 macOS, wasm32 (compile-check only).

## Baseline (before any Phase 2 change)

| Check | Result |
| --- | --- |
| CE `scripts/verify.sh` | all gates pass; 196 test results (97 default + 99 all-features), smoke PASS, cargo-deny ok |
| Talktual `npm run lint` | pass |
| Talktual `npm test` | 537 passed |
| Talktual `npm run test:server` | 60 passed |
| Talktual `npm run build` | pass |

## Backend research

See ADR-0010 for the comparison (cpal, per-platform crates, cubeb,
PortAudio, miniaudio, tinyaudio). cpal 0.18.2 source was read for: device
ids, supported config ranges, buffer sizes, error kinds, stream clock
timestamps, threading (`Stream` is `Send`), hot-plug (none) and permission
APIs (none). A throw-away probe confirmed the ALSA host, the `null` device's
reported ranges (1 Hz–4.29 GHz, i8…f64, 1–N channels) and the `default`
device failure mode.

## Plan (spec §11 order)

1. ADRs 0010–0013. 2. `ce-core::audio` contracts. 3. Authorization gate.
4. `ce-audio`: backend trait, rt endpoints, conversion, workers, cpal
adapter; fake backend in `ce-testkit`. 5. Lifecycle + negotiation in the
runtime core. 6. Framing, timestamps, overflow, discontinuities.
7. Device/route invalidation and recovery boundaries. 8. Smoke example,
integration and fault tests, ALSA-null run, probe. 9. Docs, gates,
Talktual regression, report.
