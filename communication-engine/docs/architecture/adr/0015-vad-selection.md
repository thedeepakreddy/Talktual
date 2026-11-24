# ADR-0015 — Voice-activity detector selection

Status: Accepted (Phase 3) · Date: 2026-11-24

## Decision
* Default detector: **`ce.vad.spectral` v1.0.0**, CE-native. Adaptive noise
  floor (fast down, 3 dB/s up) → SNR evidence, plus periodicity (window-
  corrected autocorrelation, 70–400 Hz), speech-band energy ratio
  (300–3400 Hz of 50–8000 Hz), spectral flatness (300–4000 Hz) and level
  modulation (p10–p90 range over 500 ms), combined by a fixed logistic model
  (`SpectralWeights::V1`, weights chosen by grid search on the development
  split only). Scores are **heuristic confidences**, not probabilities.
* Alternative and degraded-mode fallback: **`ce.vad.energy`** (adaptive
  floor SNR only).
* Test fake: **`ScriptedVad`** (scripted scores, delay and panic injection).
* **Replaceable**: `VoiceActivityDetector` is public; adapters may declare
  `needs_audio()` to receive each hop's mono samples (for external or
  neural detectors) and are started with `Engine::start_analysis_with`.

## Evidence
Benchmark on the held-out test split through CE's own adapter trait,
stabilizer and scoring (TTS fixtures; see `docs/phases/phase-3-research.md`
for the full table). Frame F1:

| condition | ce.vad.spectral | ce.vad.energy | webrtc-vad (aggr.) | earshot |
|---|---|---|---|---|
| clean | 0.983 | 0.988 | 0.976 | 0.971 |
| white 20 dB | 0.975 | 0.949 | 0.976 | 0.966 |
| white 10 dB | 0.790 | 0.769 | **0.961** | 0.908 |
| white 5 dB | 0.746 | 0.255 | 0.782 (always on) | **0.888** |
| pink 10 dB | **0.923** | 0.769 | 0.791 (always on) | 0.896 |
| music 10 dB | 0.782 (always on) | **0.807** | 0.787 (always on) | 0.783 (always on) |
| clipped +20 dB | 0.946 | 0.948 | 0.953 | **0.966** |
| quiet −30 dB | 0.964 | 0.974 | **0.978** | 0.752 |

"Always on" = one region spanning everything (precision ≈ 0.64): F1 hides
it; the merge and endpoint columns in the research report show it.

## Why not a third-party detector as default
* **webrtc-vad 0.4** (MIT, C via FFI): binary output only, `!Send` state,
  16 kHz-family rates only, and it collapsed to always-on at ≤ 5 dB white
  and 10 dB pink. Rejected.
* **earshot 1.2.2** (MIT/Apache-2.0, pure Rust, 40 KB embedded weights):
  the most noise-robust here, but 16 kHz / 256-sample frames only, uses
  `unsafe` internally, MSRV 1.87 (> CE's 1.85), and lost most activity on
  quiet input. **Recommended as a future optional adapter** (the trait
  already supports it); not adopted now.
* **Silero / TEN VAD** need ONNX Runtime (native binary download) — not
  authorized in this phase; not evaluated.
* **nnnoiseless** (RNNoise port) is a denoiser with a by-product VAD at
  48 kHz; not evaluated beyond review.

The native detector is competitive on clean, quiet and moderate-noise input,
has no dependency or unsafe code, supports 8–96 kHz and 1–N channels,
is deterministic and is tunable in-house.

## Known weaknesses (documented, not hidden)
Sustained music is reported as activity by every detector tested
(including CE's). Low-SNR white noise (≤ 10 dB) costs CE's detectors
recall. TTS fixtures overstate real-world performance.
