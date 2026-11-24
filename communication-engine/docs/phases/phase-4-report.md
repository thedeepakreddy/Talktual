# Phase 4 report — Speech recognition & language intelligence

Date: 2026-11-24 · Branch: `deepak/gallant-hamilton-wegi2i` (on top of
`main` = `9e6d4fe`, Phases 1–3). Verified on Linux x86_64 (cloud
container, 4 vCPU Intel Xeon @ 2.30 GHz, **no audio hardware**), Rust
1.97.0 and 1.85.0 (MSRV).

**Read this first.**

* **No real ASR model is integrated.** Every real option needs a model
  download (and most a native runtime); that needs your approval, which
  this phase did not have. CE therefore does **not** yet recognize speech.
  The full pipeline is built and evaluated with a deterministic
  **scripted** adapter (tests) and an **oracle reference** adapter
  (evaluation). No recognition accuracy, model latency, model RTF or model
  memory is claimed anywhere.
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
| [0020](../architecture/adr/0020-asr-and-language-id-selection.md) | ASR: whisper.cpp, sherpa-onnx, vosk, candle, ort compared (license, maintenance, streaming, model size, native deps, MSRV); **real ASR blocked pending approval**; recommended first adapter sherpa-onnx streaming zipformer. LID: whatlang, whichlang, lingua compared; **whichlang 0.1.1** selected on dev (0.95 vs 0.81 accuracy), MIT, zero dependencies |
| [0021](../architecture/adr/0021-transcript-revision-semantics.md) | Gapless revision numbers, snapshot + delta, append-only commits until the final, only finals amend (flagged), stale/duplicate rejection, hint-based delimitation, exact timestamps, no uncalibrated probabilities |
| [0022](../architecture/adr/0022-language-observations.md) | Utterance / stream / (reserved) span scopes never conflated; provenance and fingerprints; abstention (`InsufficientEvidence` below 10 letters, `LowConfidence`, `Ambiguous`, `Unsupported`); code-switching not claimed; language of text, never of the person |

Dependency added: `whichlang 0.1.1` (MIT, no transitive dependencies).
`cargo deny check` passes (section 10). No other third-party dependency.

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

Files changed relative to `main` (54 files, ≈ 11 000 lines added, 70
removed): `ce-core/src/speech/{config,mod,model}.rs` (new), `ce-core`
`command.rs`, `config.rs`, `event.rs`, `health.rs`, `lib.rs`; `ce-audio`
`tap.rs`, `worker.rs`, `lib.rs` (multi-subscriber tap); `ce-audio-intel`
`eval.rs` (utterance spans); `crates/ce-speech/**` (new);
`communication-engine` `src/lib.rs` (facade, builder, re-export),
`src/runtime/core.rs`, `core/speech.rs` (new), `core/intel.rs`,
`core/audio.rs`, `metrics.rs`, `Cargo.toml`, `examples/speech_smoke.rs`
(new), `tests/speech_recognition.rs` (new), `tests/common/mod.rs`,
`tests/audio_faults.rs` (timeout only); workspace `Cargo.toml`/`Cargo.lock`;
`fixtures/speech/manifest.json` (new), `fixtures/language/{lid_corpus.json,
PROVENANCE.md}` (new); `scripts/verify.sh`; docs (ADR-0019…0022,
`speech.md`, `events.md`, `privacy.md`, `phase-4-evaluation.md`, this
report, README, ARCHITECTURE).

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
stream-absolute).

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

Labels: **synthetic replay, oracle** unless stated.

| Scenario (test split) | Finals / refs | Missed / split / merged | Oracle WER | First partial p50 / p95 | Final after speech end p50 / p95 |
|---|---|---|---|---|---|
| clean, activity hints | 6 / 6 | 0 / 0 / 0 | 0.000 | 300 / 400 ms | 770 / 790 ms |
| white 20 dB | 6 / 6 | 0 / 0 / 0 | 0.000 | 320 / 440 ms | 630 / 690 ms |
| white 10 dB | 6 / 6 | 1 / 1 / 0 | 0.258 | 340 / 960 ms | 540 / 680 ms |
| white 5 dB | 3 / 6 | 3 / 0 / 0 | 0.348 | 340 / 350 ms | 530 / 670 ms |
| pink 10 dB | 6 / 6 | 0 / 0 / 0 | 0.000 | 310 / 450 ms | 980 / 1310 ms |
| music 10 dB | 2 / 6 | 4 / 0 / 2 | 0.000 | – | – |
| 48 kHz stereo / 44.1 kHz 512-frame / 8 kHz | 6 / 6 | 0 / 0 / 0 | 0.000 | 300–310 / 407–418 ms | 750–778 / 780–801 ms |
| 3 × 100 ms recognizer drops | 8 / 6 | 0 / 2 / 0 | 0.015 | 300 / 400 ms | 360 / 790 ms |
| throttled | 6 / 6 | 0 / 0 / 0 | 0.000 | none (by design) | 770 / 790 ms |

* The pipeline loses nothing on clean, moderately noisy, clipped, quiet and
  re-formatted input; losses at white ≤ 10 dB and with music are the
  **Phase 3 detector's** (same failure modes in the Phase 3 report).
* Final latency (≈ 0.6–1.0 s after speech end) is dominated by Phase 3's
  default segment-close silence plus `trailing_ms`; `on_endpoint_tier` can
  finalize earlier (implemented and unit-tested, not tuned).
* Commit policy (dev, with synthetic revision noise): default 3 hyp /
  800 ms / hold 1 kept — 0 amendments, 3.75 visible corrections per
  utterance, first commit p50 870 ms; 2 hypotheses commit 200 ms earlier
  with the same corrections on 4 dev utterances; re-tune with a real
  adapter. Test: 0 amendments.
* **Not measured**: any real-ASR quality (WER/CER, punctuation, casing),
  model latency, RTF, CPU/RAM or warm-up; timing error of word timestamps
  (no adapter with real word timings); language ID on ASR output.
* **Proposed targets** for the first real adapter are in the evaluation
  document (from this evidence; not results).

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
* **Determinism**: replay digests identical across runs.

## 10. Tests and regression results

All results below were run in this session on the final tree (no audio
hardware; Linux x86_64).

| Gate | Result |
|---|---|
| `scripts/verify.sh` (fmt, clippy `-D warnings` all targets + all features, tests, docs `-D warnings`, smoke examples, Phase 3 and Phase 4 optimized regression gates, cross-target checks, `cargo deny`) | **exit 0, "all gates passed"** |
| Tests, default features (`cargo test --workspace --all-targets`) | **270 passed, 0 failed** (baseline at session start: 238) |
| Tests, all features incl. doctests and the cpal adapter on ALSA's null device | **274 passed, 0 failed** |
| Smoke examples | `runtime_smoke`, `audio_smoke`, `analysis_smoke`, `speech_smoke`: PASS |
| Phase 4 held-out regression gate (`ce-speech/tests/evaluation.rs`, release) | pass |
| MSRV: `cargo +1.85.0 check --workspace --all-targets --all-features` | pass |
| `cargo deny check` (cargo-deny 0.22.0) | advisories ok, bans ok, licenses ok, sources ok |
| Talktual `npx tsc --noEmit` | pass |
| Talktual `npm test` | **537 passed** (30 files) |
| Talktual `npm run test:server` | **60 passed, 0 failed** |
| Talktual `npm run build` | pass (existing chunk-size warning only) |

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
* `ce-speech` is pure Rust with no native dependency; a real adapter
  (sherpa-onnx / whisper.cpp) would add native builds per platform, to be
  verified when approved.

## 12. Deviations, risks and unresolved issues

* **Real ASR baseline not implemented** (spec §11 "if permitted"):
  blocked pending your approval of a model download. Everything else is
  built so that adding it is one adapter.
* **Case/punctuation-sensitive WER** implemented but not reported (no
  adapter produces punctuation/casing).
* **Word-timestamp timing error** not measured (no adapter with real word
  timings; the scripted adapter's are synthetic).
* **Code-switching**: not supported (contract reserved), by evidence.
* **Commit policy** chosen on oracle noise; must be re-tuned with real
  model revisions.
* **Phase 3 dependencies**: music and very low SNR defeat the activity
  detector, and hence activity-delimited recognition. `Continuous`
  delimitation needs an adapter with endpointing.
* **Hint/audio skew**: in the live runtime analysis and recognition run on
  separate threads; a Phase 3 onset older than `preroll_ms` (500 ms) when
  it reaches the recognizer clips the utterance start. Not observed in
  tests; the pre-roll is configurable up to 2 s.
* **Intermittent Phase 2 test** (section 1): mitigated with a longer
  timeout, root cause not proven.
* **LID corpus**: small and authored; mild selection leakage disclosed.
* Stopped/abandoned worker threads in a stuck adapter `load()` or
  `accept()` cannot be killed (Rust threads); they are detached, send
  nothing, and are joined (bounded) at shutdown — a non-cooperative real
  adapter could keep a thread alive until process exit.

## 13. Acceptance checklist

| Item (spec §14) | Status |
|---|---|
| Baseline and user changes preserved; Phase 1–3 and Talktual gates recorded | Done (sections 1, 10) |
| Research/ADRs and dependency/license review completed | Done (ADR-0019…0022; `cargo deny` pass) |
| Modular ASR adapter and deterministic fake exist | Done (adapter traits; scripted + oracle) |
| Revisioned streaming transcript, stable/committed text, finalization and timestamps implemented/tested | Done |
| Gap, reset, discontinuity, degraded context and stale revision handling verified | Done (unit, sequence, runtime, replay) |
| Language-ID capability scoped, uncertainty-aware and evidence-backed | Done (text; evaluated on authored corpus) |
| Bounded work, supervision, cancellation, concurrency and fault containment tested | Done |
| Replay and evaluation with provenance, held-out methodology and quality/latency/resource/robustness results | Done for the **pipeline (oracle)** and LID; **real-ASR quality blocked** (no model) |
| Unit, integration, fault, replay, stress, regression, docs and dependency gates pass or limitations documented | See section 10 |
| Talktual regressions pass; no unsupported hardware or quality claims | Done |
| Report, ADRs, examples and readiness review completed | Done (`speech_smoke`, `evaluate_speech`) |

## 14. Phase 5 readiness assessment

**Recommendation: ready for Phase 5 work that consumes the Phase 4
contracts (transcript revisions, finals, language observations) — with
one gating condition: a real ASR adapter must be approved, integrated and
evaluated before any Phase 5 result can be judged on real speech.**

* The contracts Phase 5 would build on (utterance identity, revision and
  finalization semantics, timestamps aligned with Phase 3, language
  observations with abstention, privacy classes, lifecycle events) are
  implemented, documented and tested; Phase 5 can be developed and tested
  against the scripted and oracle adapters today.
* Before Phase 5 claims anything about real conversations: approve a model
  download (recommended: sherpa-onnx streaming zipformer, English int8,
  ≈ 20–70 MB; alternative whisper.cpp `tiny.en`, ≈ 31–75 MB), add it as an
  optional feature, run the existing evaluation unchanged (it already
  reports WER/CER, revision churn, latencies and RTF per adapter), re-tune
  the commit policy on dev, and obtain consented real-speech recordings
  (or a properly licensed public set such as LibriSpeech, CC BY 4.0 — also
  a download needing approval).
* Hardware validation (real microphones, real-time scheduling) remains
  outstanding from Phases 2–3 and is unaffected by Phase 4.
