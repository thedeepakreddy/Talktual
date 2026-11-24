# Phase 4 report — Speech recognition & language intelligence

Date: 2026-11-24 · Branch: `deepak/gallant-hamilton-wegi2i` (on top of
`main` = `9e6d4fe`, Phases 1–3). Verified on Linux x86_64 (cloud
container, 4 vCPU Intel Xeon @ 2.30 GHz, **no audio hardware**), Rust
1.97.0 and 1.85.0 (MSRV).

**Read this first.**

* **Real ASR baseline (approved, optional).** The sherpa-onnx streaming
  Zipformer English INT8 model runs behind the unchanged adapter boundary
  in a separately built crate, `adapters/ce-asr-sherpa` (ADR-0023).
  Genuine model results are in [phase-4-real-asr.md](phase-4-real-asr.md).
  They come from **synthetic TTS speech in replay** (held-out clean
  pipeline WER 0.121, 8/66 words), because real speech corpora are not
  approved; accuracy on human speech is unknown.
* The scripted adapter (tests) and the oracle reference adapter (pipeline
  evaluation) are kept unchanged. Every number in this report is labelled
  **real model**, **oracle**, **synthetic**, **replay**, **fake backend**
  or **compile-only**.
* **No audio hardware was available or tested.** Runtime evidence uses the
  test kit's fake backend; evaluation uses synthetic TTS speech in replay.
* Language identification is real (whichlang) but evaluated on **authored
  text**, not on ASR output.

## 1. Baseline and repository state

* `main` = `9e6d4fe` (Phases 1–3 merged via PRs #8 and #9). Work is on
  `deepak/gallant-hamilton-wegi2i`, which started from `main`, then the
  previous session's WIP commit `ad7fa28` (speech contracts, multi-
  subscriber tap, ce-speech core), then this session's commits. No user
  changes were found or overwritten; nothing was force-pushed.
* **Reproduced at the start of Phase 4 (previous session, on `9e6d4fe`)**:
  `scripts/verify.sh` default-feature tests 224 passed; the all-features run
  aborted once on an intermittent Phase 2 test
  (`audio_faults::a_panic_in_the_backend_is_contained_and_reported` got
  `audio.open_timeout` instead of `audio.worker_panicked`; 28 re-runs did
  not reproduce it; root cause not proven — the worker's open reply races
  the panic hook's backtrace symbolization against a 1 s deadline). That
  test now uses a 10 s open timeout (comment in the test); nothing was
  skipped. Talktual: tsc pass, 537 client tests, 60 server tests, build
  pass.
* **Reproduced at the start of this session (on `ad7fa28`)**:
  `cargo test --workspace --all-targets` → **238 passed, 0 failed**,
  matching the handoff.
* Handoff notes: [phase-4-handoff.md](phase-4-handoff.md) (kept for history;
  superseded by this report).

## 2. Research and ADR decisions

| ADR | Decision |
|---|---|
| [0019](../architecture/adr/0019-speech-architecture.md) | One new crate `ce-speech`; adapter boundary with cheap discovery and worker-thread loading; deterministic per-stream recognizer shared by live and replay; one supervised thread per recognition fed by its own bounded tap and a bounded hint queue; core-owned orchestration |
| [0020](../architecture/adr/0020-asr-and-language-id-selection.md) | ASR: whisper.cpp, sherpa-onnx, vosk, candle, ort compared (license, maintenance, streaming, model size, native deps, MSRV); recommended first adapter sherpa-onnx streaming zipformer, **later approved and integrated** (ADR-0023). LID: whatlang, whichlang, lingua compared; **whichlang 0.1.1** selected on dev (0.95 vs 0.81 accuracy), MIT, zero dependencies |
| [0021](../architecture/adr/0021-transcript-revision-semantics.md) | Gapless revision numbers, snapshot + delta, append-only commits until the final, only finals amend (flagged), stale/duplicate rejection, hint-based delimitation, exact timestamps, no uncalibrated probabilities |
| [0023](../architecture/adr/0023-real-asr-baseline-sherpa.md) | Approved baseline `sherpa-onnx-streaming-zipformer-en-2023-06-26` INT8 (Apache-2.0) via `sherpa-onnx` 2.0.3 in an optional crate **outside the workspace** (native libraries, Rust ≥ 1.89); only the INT8 files are extracted; `lead_in_ms` 500 and `preroll_ms` 1000 chosen on dev with the real model |
| [0022](../architecture/adr/0022-language-observations.md) | Utterance / stream / (reserved) span scopes never conflated; provenance and fingerprints; abstention (`InsufficientEvidence` below 10 letters, `LowConfidence`, `Ambiguous`, `Unsupported`); code-switching not claimed; language of text, never of the person |

Dependency added to the workspace: `whichlang 0.1.1` (MIT, no transitive
dependencies). The optional adapter crate adds `sherpa-onnx` /
`sherpa-onnx-sys` 2.0.3 (Apache-2.0) with its own `deny.toml`. That policy
is CE's plus permissive licenses used only by the sys crate's build script.
`cargo deny check` passes for both (section 10).

## 3. Architecture, contracts, dataflow and lifecycle

Full description: [speech.md](../architecture/speech.md).

* **Contracts (`ce-core::speech`)**: `RecognitionConfig` (segmentation,
  chunking, pre-roll, partial cadence, `StabilityPolicy`,
  `FinalizationPolicy`, `LanguageHint`, `LanguageIdConfig`, word timings,
  `RecognitionBudget`; `validate()` with one error code; `fingerprint()`),
  `SpeechConfig` (engine-wide concurrency), `AsrCapabilities`, `ModelInfo`,
  `RecognizerRef`, `SpeechMeta`, `TranscriptRevision` (+ `TranscriptText`,
  `TranscriptDelta`, `WordTiming`, `Score`), `LanguageObservation`
  (+ scope, decision, candidates, source, abstention), `SpeechCapabilities`,
  `RecognitionStatus` / `RecognitionCounters` / `Percentiles`, and
  `SpeechEvent` (12 kinds, privacy class per kind). Wired into
  `EventPayload::Speech` / `EventTopic::Speech`, five commands
  (`Start/Stop/Reset/QueryRecognition`, `ListRecognizers`), two outcomes,
  `EngineConfig::speech`, `MetricsSnapshot` and `RuntimeHealth`.
* **Adapter boundary (`ce-speech::adapter`)**: `AsrFactory`
  (`capabilities()` cheap, `create()` cheap) and `AsrAdapter` (`load`,
  `begin`, `accept`, `finalize`, `reset`, optional `position_hint`);
  `sanitize()` drops invalid text, scores and timings and never invents.
* **Dataflow**: capture worker → dedicated bounded tap (multi-subscriber,
  ≤ 4 per stream, each dropping independently) → recognition thread
  (downmix, resample, pre-roll, hints, utterances, adapter, commit policy,
  language) → bounded channel → core → event bus. Phase 3 events reach the
  recognition through the core with `try_send`.
* **Lifecycle**: register (builder) → discover (no load) → start (checks,
  limits, spawn, attach tap) → load on the worker (`ModelReady`; load
  timeout) → run → reset → stop / stream end / revocation / failure /
  stall / shutdown → `Stopped`. Session close and shutdown stop
  recognitions before analyses and streams.

## 4. Implementation and files changed

New crate `crates/ce-speech` (`forbid(unsafe_code)`):

| Module | Responsibility |
|---|---|
| `adapter` | adapter traits, `LoadOutcome`, `AdapterHypothesis`/`AdapterWord`, `sanitize` |
| `fake` | `ScriptedFactory` (scripts, endpoints, malformed output, error / always-error / panic / stall / finalize-error faults, load delay / failure / panic, shared-model counters); `ReferenceFactory` (oracle with deterministic truncation / rewrite / substitution noise) |
| `transcript` | `UtteranceText` (stable-prefix commits), `delta`/`apply`, `TranscriptAssembler` |
| `language` | `LanguageIdentifier`, `WhichlangIdentifier`, `ScriptedIdentifier`, `observe_utterance`, `StreamLanguage`, `fingerprint`, `letters` |
| `resample` | `downmix`, `Resampler` (anti-aliased, exact position mapping) |
| `recognizer` | `StreamRecognizer`, `RecognizerSetup` |
| `worker` | `spawn_recognition`, `RecognitionHandle`, `SpeechSignal` |
| `eval` | manifests, scenarios, replay, metrics, LID evaluation, digest |
| `metrics` | bounded nearest-rank percentiles |

Optional crate `adapters/ce-asr-sherpa` (outside the workspace;
`forbid(unsafe_code)` in its own code):

| Item | Responsibility |
|---|---|
| `src/lib.rs` | `SherpaConfig` (approved INT8 preset, endpoint rules, threads, tail padding), `SherpaFactory` (honest capabilities, shared model), adapter over sherpa-onnx online streams |
| `examples/evaluate_real.rs` | resources, isolated decoding, partials, dev lead-in sweep, pipeline under all conditions, commit policies, formats/drops/throttling — through the unchanged `ce_speech::eval` |
| `tests/model.rs` | real-model tests (`#[ignore]` unless the model is provisioned): fixture recognition, model sharing, determinism, language refusal; the real model inside the `Engine` at 48 kHz stereo on the fake backend |
| `scripts/fetch-model.sh`, `scripts/verify.sh`, `deny.toml`, `README.md` | checksum-verified provisioning (INT8 files only), the adapter's gates, license policy, usage |

Files changed relative to `main` (54 files, ≈ 11 000 lines added, 70
removed): `ce-core/src/speech/{config,mod,model}.rs` (new), `ce-core`
`command.rs`, `config.rs`, `event.rs`, `health.rs`, `lib.rs`; `ce-audio`
`tap.rs`, `worker.rs`, `lib.rs` (multi-subscriber tap); `ce-audio-intel`
`eval.rs` (utterance spans); `crates/ce-speech/**` (new);
`communication-engine` `src/lib.rs` (facade, builder, re-export),
`src/runtime/core.rs`, `core/speech.rs` (new), `core/intel.rs`,
`core/audio.rs`, `metrics.rs`, `Cargo.toml`, `examples/speech_smoke.rs`
(new), `tests/speech_recognition.rs` (new), `tests/common/mod.rs`,
`tests/audio_faults.rs` (timeout only); workspace `Cargo.toml` (now also
`exclude = ["adapters/ce-asr-sherpa"]`) and `Cargo.lock`;
`adapters/ce-asr-sherpa/**` (new);
`fixtures/speech/manifest.json` (new), `fixtures/language/{lid_corpus.json,
PROVENANCE.md}` (new); `scripts/verify.sh`; docs (ADR-0019…0022,
ADR-0023, `speech.md`, `events.md`, `privacy.md`, `phase-4-evaluation.md`,
`phase-4-real-asr.md`, this report, README, ARCHITECTURE).

## 5. Streaming transcript and revision semantics

(ADR-0021.) Per utterance: revisions 0…n without gaps; each carries the
full snapshot and a delta (`keep_committed_chars`, `append_committed`,
`provisional`); the last is `Final` or `Cancelled`. Committed text grows
only from a prefix stable for 3 hypotheses or 800 ms of audio, minus one
held-back word; it never shrinks before the final; contradictions are
counted, not applied; the final may amend and then sets `commit_amended`.
Partials every ≥ 200 ms of audio (none when throttled). The consumer
assembler rejects stale, duplicate, after-end and foreign revisions and
falls back to snapshots after a gap. Frames/times map exactly from adapter
samples; words appear only when declared and valid; scores only with
declared semantics. Verified by unit tests (`transcript`), sequence tests
(`tests/recognizer.rs`: partial → correction → stable → final, delta
reconstruction, exact frames at 16 kHz mono and 48 kHz stereo with an
offset start) and the runtime tests.

**Defects found by the evaluation and fixed**: (1) an activity onset
arriving inside an open utterance (a pause shorter than the segment-close
silence) blocked the following segment end, merging utterances up to the
20 s maximum (dev clean had 2 finals for 4 references; now 4/4); (2) after
a recognizer-only gap the rest of an active utterance was lost until the
next onset (now it resumes at the first audio after the gap; 3 × 100 ms
drops: oracle WER 0.015 instead of 0.73); (3) the oracle's position hint
was relative to the post-gap anchor (an evaluation-adapter defect; now
stream-absolute). (4) Found only with the **real model**: utterances began
exactly at the Phase 3 onset, which fires after soft word beginnings, so
initial words were lost ("TURN LEFT" → "LEFT"; clean test WER 0.288).
Utterances now start `lead_in_ms` (500, chosen on dev) before the onset,
served from a 1000 ms pre-roll; the frame range includes it (clean test
WER 0.121).

## 6. Language intelligence: capabilities and limitations

* **Capabilities**: utterance-scope observations on each final
  (whichlang, 16 languages, no score); stream-scope aggregate with its own
  events; declared language via `Fixed`; restriction via `Expected`;
  recognizer-reported language used only if the adapter declares it and no
  text identifier is configured; replaceable identifier on the builder.
* **Measured (held-out test, authored text)**: accuracy on decided items
  0.984 at 0.73 coverage (long 1.00 / 0.96, medium 1.00 / 0.77, short
  0.92 / 0.46). Mixed items: 7 of 8 abstain `Ambiguous`.
* **Limitations**: no scores (no calibrated confidence of any kind);
  short utterances usually abstain; no span-level code-switching
  (`code_switching = false`, scope `Span` reserved); only 7 of the 16
  languages evaluated; text from real ASR will be noisier. Nothing about
  the speaker is inferred.

## 7. Evaluation corpus provenance and methodology

[phase-4-evaluation.md](phase-4-evaluation.md) has the complete method and
tables. Speech: 10 Flite TTS utterances (`fixtures/speech`, PROVENANCE.md;
synthetic, no human voice), references = exact TTS input
(`manifest.json`), dev 4 / test 6. Language: 134 authored sentences
(`fixtures/language/PROVENANCE.md`), dev 42 / test 92. Dev was used for
every choice; test only reported; regression thresholds sit below the
reported test values. Mild LID selection leakage is disclosed. Metrics:
normalized WER/CER (stream level), utterance mapping (missed / split /
merged), revision counts, rate and visible corrections, commit
amendments, first-partial / first-commit / final delays in audio time,
start error, utterance language decisions, RTF; LID accuracy, coverage,
abstention by reason, confusion and mixed outcomes.

## 8. Metrics, actual results and honest limitations

### 8.1 Real model (sherpa-onnx Zipformer INT8) — REAL MODEL on SYNTHETIC speech, REPLAY

Held-out test split (6 TTS utterances, 66 words); defaults
(`ActivityHints`, `lead_in_ms` 500). Full tables, dev results, the lead-in
sweep and partial traces: [phase-4-real-asr.md](phase-4-real-asr.md).

| Scenario (test) | Finals / refs | Missed / split / merged | **WER** (edits/words) | **CER** | First partial p50 / p95 | Final after speech end p50 / p95 |
|---|---|---|---|---|---|---|
| isolated utterances (model alone, no pipeline) | 6 / 6 | – | 0.167 (11/66) | 0.078 | – | – |
| clean | 6 / 6 | 0 / 0 / 0 | **0.121 (8/66)** | 0.044 | 750 / 1000 ms | 770 / 790 ms |
| white 20 dB | 6 / 6 | 0 / 0 / 0 | 0.288 (19/66) | 0.136 | 760 / 1040 ms | 630 / 690 ms |
| pink 10 dB | 6 / 6 | 0 / 0 / 0 | 0.591 (39/66) | 0.305 | 780 / 1050 ms | 980 / 1310 ms |
| white 10 dB | 6 / 6 | 1 / 1 / 0 | 0.652 (43/66) | 0.462 | 780 / 1040 ms | 540 / 680 ms |
| white 5 dB | 3 / 6 | 3 / 0 / 0 | 0.894 (59/66) | 0.675 | 740 / 750 ms | 530 / 670 ms |
| music 10 dB | 4 / 6 | 2 / 0 / 3 | 0.773 (51/66) | 0.509 | 1650 / 1650 ms | 1710 / 1710 ms |
| clipped +20 dB | 6 / 6 | 0 / 0 / 0 | 0.439 (29/66) | 0.263 | 750 / 1050 ms | 810 / 910 ms |
| quiet −30 dB | 8 / 6 | 0 / 2 / 0 | 0.591 (39/66) | 0.521 | 1010 / 1060 ms | 260 / 680 ms |
| 48 kHz stereo / 44.1 kHz 512-frame / 8 kHz | 6 / 6 | 0 / 0 / 0 | 0.167 / 0.136 / 0.227 | 0.056 / 0.041 / 0.086 | 730–762 / 1005–1018 ms | 750–778 / 780–801 ms |
| 3 × 100 ms recognizer drops | 8 / 6 | 0 / 2 / 0 | 0.242 (16/66) | 0.121 | 750 / 1000 ms | 360 / 790 ms |
| throttled (no partials) | 6 / 6 | 0 / 0 / 0 | 0.121 (8/66) | 0.044 | none (by design) | 770 / 790 ms |
| model endpointing only (no analysis) | 4 / 6 | 2 / 0 / 4 | 0.182 (12/66) | 0.107 | – | – |

* **Partials**: ≈ 2.2 revisions per second of speech, 10.5 per utterance.
  The last word appears as a fragment and is completed ("BO" → "BOX").
  Committed text trails the hypothesis by about one word. 2.7 visible
  corrections per utterance; **0 finals amended committed text** (dev and
  test). First committed text p50 1300 ms after speech start.
* **Errors** (clean test): walk → WALKED, blocks → BOX, speak → PEOPLE,
  learning → EARNING, "bridge is" → BRIDGES, today → TO DAY. Noise degrades
  the model strongly (white 20 dB more than doubles WER). At ≤ 10 dB SNR
  and with music, Phase 3 segmentation fails as well.
* **Lead-in fix** (found with the real model, tuned on dev): before it,
  utterance-initial words were lost and clean test WER was 0.288 (19/66).
* **Language** (whichlang on real transcripts, clean test): 5 utterances
  `en`, 1 abstention (the one-word "No").
* Against the targets proposed before the model existed: final ≤ 1000 ms
  p95 after speech end — met (790 ms); zero merged/split on clean and
  20 dB — met; amendments ≤ 5 % — met (0 %); model RTF ≤ 0.3 — met
  (0.06–0.09); first partial ≤ 500 ms p95 — **not met** (1000 ms; the
  model emits its first non-empty hypothesis after ≈ 0.6–0.8 s of speech).

### 8.2 Pipeline with the oracle — ORACLE, SYNTHETIC, REPLAY (not ASR quality)

| Scenario (test split) | Finals / refs | Missed / split / merged | Oracle WER | First partial p50 / p95 | Final after speech end p50 / p95 |
|---|---|---|---|---|---|
| clean, activity hints | 6 / 6 | 0 / 0 / 0 | 0.000 | 300 / 400 ms | 770 / 790 ms |
| white 20 dB | 6 / 6 | 0 / 0 / 0 | 0.000 | 320 / 440 ms | 630 / 690 ms |
| white 10 dB | 6 / 6 | 1 / 1 / 0 | 0.197 | 340 / 840 ms | 540 / 680 ms |
| white 5 dB | 3 / 6 | 3 / 0 / 0 | 0.348 | 340 / 350 ms | 530 / 670 ms |
| pink 10 dB | 6 / 6 | 0 / 0 / 0 | 0.000 | 320 / 450 ms | 980 / 1310 ms |
| music 10 dB | 2 / 6 | 4 / 0 / 2 | 0.000 | – | – |
| 48 kHz stereo / 44.1 kHz 512-frame / 8 kHz | 6 / 6 | 0 / 0 / 0 | 0.000 | 300–310 / 407–418 ms | 750–778 / 780–801 ms |
| 3 × 100 ms recognizer drops | 8 / 6 | 0 / 2 / 0 | 0.015 | 300 / 400 ms | 360 / 790 ms |
| throttled | 6 / 6 | 0 / 0 / 0 | 0.000 | none (by design) | 770 / 790 ms |

* The pipeline loses nothing on clean, moderately noisy, clipped, quiet and
  re-formatted input. Losses at white ≤ 10 dB and with music belong to the
  **Phase 3 detector** (the same failure modes appear in the Phase 3
  report). Oracle utterances start ≈ 500 ms before the labelled speech:
  that is the lead-in.
* Final latency (≈ 0.6–1.0 s after speech end, oracle and real model
  alike) is dominated by Phase 3's default segment-close silence plus
  `trailing_ms`. `on_endpoint_tier` can finalize earlier; it is implemented
  and unit-tested but not tuned.
* Commit policy: the default (3 hyp / 800 ms / hold 1) was kept after dev
  comparisons with the oracle noise model and with the real model; it gave
  0 amendments in both.
* **Not measured**:
  * accuracy on human speech (corpora not approved);
  * punctuation and casing (the model has neither);
  * word-timestamp timing error (not exposed by the adapter);
  * other platforms and hardware.

## 9. Resource, latency and robustness evidence

* **Overhead (replay)**: recognizer RTF ≈ 0.0001 (16 kHz) to 0.0015
  (44.1 kHz, resampling) with the oracle — CE's own cost; Phase 3 analyzer
  ≈ 0.005.
* **Live (fake backend, release)**: `speech_smoke` — 2 real-time capture
  streams, 1 analysis, 2 recognitions for 5 s: 0.29 s CPU in total
  (≈ 6 % of one core), peak RSS 12.7 MiB; capture delivered 487 blocks per
  stream while recognizing.
* **Bounded memory**: tap, hint queue, pre-roll ring, ≤ 64 boundaries and
  ≤ 64 deferred hints, bounded latency reservoirs, ≤ 16 finished statuses;
  tested over 10 minutes of audio with an onset every 2 s and a 1000-hint
  flood (`memory_stays_bounded_over_long_streams_and_hint_floods`).
* **Robustness (runtime tests, fake backend)**: adapter panic contained
  (other stream continues, capture unaffected); load failure, load panic
  and load timeout reported as `LoadFailed` with codes; stall detected
  (`speech.stalled`) while capture keeps flowing; overload with an 8-block
  tap: capture of 100 blocks not delayed, drops reported
  (`QueuePressure`) and handled as a discontinuity; revocation cancels and
  discards; stream close finalizes (`StreamEnd`); reset; session close and
  shutdown order; 4 concurrent streams sharing one loaded model; 20
  start/stop cycles without leaks (all adapters released, statuses
  bounded); engine-wide and per-adapter concurrency limits.
* **Determinism**: replay digests identical across runs (oracle and real
  model with 1 thread).
* **Real model resources** (REAL MODEL, this machine, 1 ONNX thread):
  * load ≈ 2.0 s (1964 ms in the final run; 2.6–2.9 s in earlier runs);
    a shared reuse takes < 10 µs;
  * peak RSS 23 → 152 MB after loading (+≈ 129 MB); evaluation process
    peak 199 MB;
  * recognizer RTF 0.06–0.09 (model + CE) for one stream;
  * 4 concurrent streams sharing one model: 2.20 s wall for 4 × 27.9 s of
    audio;
  * inside the `Engine` (fake backend, 48 kHz stereo): capture never
    delayed over 3 s of audio.

## 10. Tests and regression results

All results below were run on the final tree, after the real-ASR
integration and the `lead_in_ms` / `preroll_ms` default changes (no audio
hardware; Linux x86_64). The Talktual gates were re-run as well; no file
outside `communication-engine/` changed.

| Gate | Result |
|---|---|
| `scripts/verify.sh` (fmt, clippy `-D warnings` all targets + all features, tests, docs `-D warnings`, smoke examples, Phase 3 and Phase 4 optimized regression gates, cross-target checks, `cargo deny`) | **exit 0, "all gates passed"** |
| Tests, default features (`cargo test --workspace --all-targets`) | **271 passed, 0 failed** (baseline at session start: 238) |
| Tests, all features incl. doctests and the cpal adapter on ALSA's null device | **275 passed, 0 failed** |
| Smoke examples | `runtime_smoke`, `audio_smoke`, `analysis_smoke`, `speech_smoke`: PASS |
| Phase 4 held-out regression gate (`ce-speech/tests/evaluation.rs`, release) | pass |
| MSRV: `cargo +1.85.0 check --workspace --all-targets --all-features` | pass |
| `cargo deny check` (cargo-deny 0.22.0) | advisories ok, bans ok, licenses ok, sources ok |
| Talktual `npx tsc --noEmit` | pass |
| Talktual `npm test` | **537 passed** (30 files) |
| Talktual `npm run test:server` | **60 passed, 0 failed** |
| Talktual `npm run build` | pass (existing chunk-size warning only) |
| **Optional adapter** `adapters/ce-asr-sherpa/scripts/verify.sh` (fmt, clippy `-D warnings`, tests, real-model tests, `cargo deny`, resources) | **exit 0**: 1 unit test, **2 real-model tests passed** (fixture recognition, model sharing, determinism, language refusal; real model inside the `Engine` at 48 kHz stereo); advisories, bans, licenses and sources ok |
| Adapter with Rust 1.85 | refused by design (`aes 0.9.4`, a build dependency of `sherpa-onnx-sys`, requires 1.89); the adapter declares `rust-version = "1.89"` and lives outside the workspace |

New Phase 4 tests: ce-core speech contracts (validation, serialization,
fingerprints, tags, events); ce-audio multi-subscriber tap; ce-speech unit
tests (sanitize, scripted/oracle adapters, transcript commits/deltas/
assembler, language identification and stream aggregate, resampler,
percentiles, evaluation metrics and manifests — 17); 12 recognizer sequence
tests (partial → revision → stable → final, exact timestamps through
resampling, Phase 3 hints with backdated starts and deferred hints,
fallback and recovery, gaps / capture discontinuity / format change,
adapter errors and repeated errors, finalize errors, reset / stop / stream
end / revocation, max duration and continuation, throttling and queue
pressure, malformed output, utterance vs stream language, bounded memory
and hint floods); 2 evaluation gate tests; 9 runtime integration tests
(lifecycle, start errors and limits, Phase 3 end-to-end, panic
containment, load failure / panic / timeout, stall, revocation / close /
reset / session close / shutdown, multi-stream and 20 start/stop cycles,
overload). The speech integration suite was run 3 times consecutively
without failure. Phase 1–3 suites: all pass, unchanged except the Phase 2
timeout noted in section 1.

## 11. Hardware and cross-platform evidence

* **Hardware: none.** No microphone, speaker or audio device was used; no
  device, latency or battery claim is made.
* **Linux x86_64**: built and tested (section 10).
* **Cross-target (compile-only)**: `cargo check` of `ce-core`, `ce-audio`,
  `ce-audio-intel`, `ce-speech` and `communication-engine` (all features)
  for `aarch64-apple-ios`, `aarch64-linux-android`,
  `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`, and of the four
  library crates for `wasm32-unknown-unknown`: all pass (via
  `verify.sh`). Compiling is not running: nothing was executed on those
  platforms.
* `ce-speech` is pure Rust with no native dependency. The optional
  sherpa-onnx adapter was built, tested and evaluated on **Linux x86_64
  only**. sherpa-onnx ships native libraries for macOS, Windows, Linux
  aarch64, Android and iOS, but none were built or tested here. The CE
  workspace's cross-target checks do not include the adapter.

## 12. Deviations, risks and unresolved issues

* **Real ASR evaluated on synthetic speech only.** Real speech corpora are
  not approved, and the sample recordings inside the model archive were
  deliberately not extracted. Accuracy on human speech is unknown; the
  test set is 66 words.
* **Optional adapter outside the workspace** (ADR-0023). It needs Rust
  ≥ 1.89 and native libraries, so it has its own gates and is not part of
  CE's CI or cross-target checks. It is Linux x86_64 only so far. Its
  provisioning downloads about 334 MB (model archive 310 MB, native
  libraries 24 MB) and keeps about 95 MB.
* **Defaults changed by real-model evidence**: `lead_in_ms` 0 → 500 and
  `preroll_ms` 500 → 1000. Both were chosen on dev; the dev sweep is
  non-monotonic on only 4 utterances.
* **First partial p95 is 1000 ms** with the real model, above the 500 ms
  target proposed earlier (the model emits its first word after
  ≈ 0.6–0.8 s of speech).
* **Noise robustness of the model is weak** on synthetic noise (white
  20 dB: WER 0.288). Phase 3 segmentation still fails at ≤ 10 dB SNR and
  with music.
* **Case/punctuation WER and word-timestamp timing error** are not
  measurable: the model has no punctuation or casing, and the adapter
  exposes no word timings (word end times would have to be invented).
* **Code-switching**: not supported (contract reserved), based on the
  evidence.
* **Hint/audio skew**: the effective lead-in shrinks when a Phase 3 hint
  reaches the recognizer more than `preroll_ms − lead_in_ms` (500 ms)
  late. This was not observed in tests.
* **Intermittent Phase 2 test** (section 1): mitigated with a longer
  timeout; root cause not proven.
* **LID corpus** is small and authored; mild selection leakage is
  disclosed.
* Worker threads stuck inside an adapter's `load()` or `accept()` cannot
  be killed. They are detached, send nothing, and are joined (bounded) at
  shutdown.

## 13. Acceptance checklist

| Item (spec §14) | Status |
|---|---|
| Baseline and user changes preserved; Phase 1–3 and Talktual gates recorded | Done (sections 1, 10) |
| Research/ADRs and dependency/license review completed | Done (ADR-0019…0022; `cargo deny` pass) |
| Modular ASR adapter and deterministic fake exist | Done (adapter traits; scripted + oracle; real sherpa-onnx baseline as an optional adapter) |
| Revisioned streaming transcript, stable/committed text, finalization and timestamps implemented/tested | Done |
| Gap, reset, discontinuity, degraded context and stale revision handling verified | Done (unit, sequence, runtime, replay) |
| Language-ID capability scoped, uncertainty-aware and evidence-backed | Done (text; evaluated on authored corpus) |
| Bounded work, supervision, cancellation, concurrency and fault containment tested | Done |
| Replay and evaluation with provenance, held-out methodology and quality/latency/resource/robustness results | Done: oracle pipeline, LID, and the **real model** (WER/CER, latency, partials, resources, robustness) — on **synthetic speech**; human-speech evaluation awaits corpus approval |
| Unit, integration, fault, replay, stress, regression, docs and dependency gates pass or limitations documented | See section 10 |
| Talktual regressions pass; no unsupported hardware or quality claims | Done |
| Report, ADRs, examples and readiness review completed | Done (`speech_smoke`, `evaluate_speech`) |

## 14. Phase 5 readiness assessment

**Recommendation: ready for Phase 5 work that consumes the Phase 4
contracts, with one remaining condition before any claim about real
conversations: evaluate on human speech.**

* The contracts Phase 5 builds on are implemented, documented and tested,
  and now exercised by a real streaming recognizer:
  * utterance identity, revision and finalization semantics;
  * timestamps aligned with Phase 3;
  * language observations with abstention;
  * privacy classes and lifecycle events.
* The real baseline works end to end: clean synthetic speech gives WER
  0.121, finals ≈ 0.8 s after speech end, RTF < 0.1 on one CPU thread and
  about 130 MB of model memory. It is weak in noise. Phase 5 can be
  developed against it, and against the scripted and oracle adapters for
  deterministic tests.
* Before Phase 5 results are judged on real conversations:
  1. approve a consented or properly licensed real-speech set (e.g.
     LibriSpeech test-clean, CC BY 4.0) and run `evaluate_real`
     unchanged on it;
  2. re-check `lead_in_ms` and the commit policy on its dev split;
  3. build and verify the adapter on the target platforms;
  4. validate on real audio hardware (outstanding since Phases 2–3).
