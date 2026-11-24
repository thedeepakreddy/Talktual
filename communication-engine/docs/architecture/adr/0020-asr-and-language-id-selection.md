# ADR-0020 — ASR and language-identification selection

Status: Accepted (Phase 4) · Date: 2026-11-24 · Real ASR: the
recommended model was **approved and integrated** as an optional adapter
(ADR-0023)

## ASR candidates (crates.io data of 2026-11-24)

| Candidate | Version / updated | License | Streaming | Notes |
|---|---|---|---|---|
| whisper-rs (whisper.cpp) | 0.16.0 / 2026-03 | Unlicense (bindings), MIT (whisper.cpp) | No (sliding-window re-decode → revisions) | C++ build; GGML models tiny ≈ 75 MB, tiny.en-q5 ≈ 31 MB; 99 languages (multilingual models), reports language, word timestamps (token-level) |
| sherpa-onnx | 2.0.3 / 2026-11-22 | Apache-2.0 | Yes (streaming zipformer / paraformer) | Needs ONNX Runtime native libraries and a model download (≈ 20–70 MB int8); endpointing and timestamps |
| sherpa-rs | 0.6.8 / 2025-10 | MIT | via sherpa-onnx | thin wrapper |
| vosk | 0.3.1 / 2024-10 | MIT (crate), Apache-2.0 (Vosk) | Yes (partials) | needs the libvosk binary and a model (small-en ≈ 40 MB); crate unmaintained since 2024-10 |
| candle-transformers (Whisper) | 0.11.1 / 2026-10 | MIT / Apache-2.0 | No | pure Rust, heavy dependency tree, model download |
| ort | 2.0.0-rc.15 | MIT / Apache-2.0 | n/a | ONNX Runtime bindings; MSRV 1.92 > CE's 1.85 |

**Decision.** Every real option needs a model download (and most a native
runtime), which requires the user's approval. Before approval, none was
downloaded. After approval, the recommended sherpa-onnx streaming
Zipformer was integrated (ADR-0023; results in `phase-4-real-asr.md`). The pipeline is built
and evaluated with the **scripted** adapter (tests) and the **reference
oracle** adapter (evaluation; ADR-0019). Recommended first real adapter,
once approved: **sherpa-onnx streaming zipformer** (true streaming,
endpointing, timestamps, Apache-2.0, small int8 models) behind
`AsrAdapter`; alternative: whisper.cpp `tiny.en` (simpler build, but
windowed re-decoding produces heavier revision churn and higher latency).
Either is an optional feature so the default build gains no native
dependency.

## Text language identification candidates

Probed in a scratch crate outside the workspace on the authored corpus
(`fixtures/language`, see its PROVENANCE):

| Candidate | Version | License | Dependencies | Notes |
|---|---|---|---|---|
| whatlang | 0.18.0 | MIT | hashbrown | 69 languages, confidence score; poor on short text ("Hello" → Danish) |
| whichlang | 0.1.1 | MIT | none | 16 languages, logistic regression on character n-grams; **no score exposed** |
| lingua | 1.8.0 | Apache-2.0 | large | 3.3 MB crate plus large per-language models; not evaluated (size) |

Accuracy, single-language items, all decided (no abstention):

| split / length | whatlang | whatlang + allowlist | whichlang |
|---|---|---|---|
| dev short (n=14) | 0.64 | 0.71 | 0.86 |
| dev medium (n=24) | 0.79 | 0.83 | 1.00 |
| dev long (n=4) | 1.00 | 1.00 | 1.00 |
| dev all (n=42) | 0.76 | 0.81 | 0.95 |
| test short (n=28) | 0.29 | 0.46 | 0.89 |
| test medium (n=31) | 0.84 | 0.97 | 1.00 |
| test long (n=25) | 1.00 | 1.00 | 1.00 |
| test all (n=84) | 0.70 | 0.81 | 0.96 |

**Decision: whichlang 0.1.1** behind `LanguageIdentifier`
(`ce.lid.whichlang`), as a workspace dependency (MIT, zero transitive
dependencies, pure Rust, compiles for every CE target and on Rust 1.85).
It reports no score, so observations carry `score_semantics: None` and no
invented confidence. Selection is supported by dev alone (0.95 vs 0.81);
the probe also printed test numbers at the same time (mild selection
leakage, disclosed). Abstention thresholds were tuned on dev only
(ADR-0022).

## Consequences
CE has a working, honest language layer now and a clear, approved-on-request
path to real ASR. Until then, no statement about recognition accuracy,
real-time factor of a model, or model memory can be made.
