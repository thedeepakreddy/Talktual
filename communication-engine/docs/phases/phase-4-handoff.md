# Phase 4 handoff — Speech Recognition & Language Intelligence (HISTORICAL)

> **Superseded.** Phase 4 was completed from this handoff; see
> [phase-4-report.md](phase-4-report.md). Kept as a record of the
> mid-phase state.

Written 2026-11-24 when the previous session ran out of budget. Read this
whole file, then the Phase 4 PDF (authoritative), then continue. Nothing
here overrides the PDF.

## Standing rules from the user (keep following them)

* Commit/PR attribution: end commit messages and PR bodies with
  **`Built by Deepak and Claude`**. **Never** put Claude session links or
  model names in commits, PRs, code or docs.
* Work on branch `deepak/gallant-hamilton-wegi2i`; push only there. Do not
  push to `main`. Open a PR only when the user asks.
* No large model downloads, paid APIs, credential changes, deployments or
  destructive git operations without explicit approval.
* Never claim a test passed unless it was run; report hardware evidence
  only if hardware was tested (there is none in this environment).
* Never infer identity, emotion, health, nationality, ethnicity, location or
  other personal traits.

## Repository state at handoff

* `main` = `9e6d4fe` (Phases 1–3 merged via PRs #8 and #9).
* Branch `deepak/gallant-hamilton-wegi2i` was fast-forwarded to `9e6d4fe`,
  then one **WIP commit** with everything below was pushed on top.
* Workspace builds; `cargo clippy --workspace --all-targets -- -D warnings`
  is clean; see "Tests at handoff" for test results.

## Baseline (reproduced at the start of Phase 4, on `9e6d4fe`)

* `scripts/verify.sh`: exit 101. Default features: 224 passed, 0 failed.
  All-features run aborted on **one intermittent Phase 2 test**:
  `audio_faults::a_panic_in_the_backend_is_contained_and_reported` got
  `audio.open_timeout` instead of `audio.worker_panicked`.
  * Investigation: 28 re-runs (warm, freshly relinked, under 3× CPU load,
    with and without `RUST_BACKTRACE`) never reproduced it. Mechanism from
    the code: the worker's open reply is dropped only after the panic hook
    runs (symbolizing a backtrace with `RUST_BACKTRACE=1`), racing the test's
    1 s open deadline. Root cause **not proven**.
  * Change made (in the WIP commit): that test now uses
    `open_timeout_ms = 10_000` (comment explains why). Nothing skipped;
    timeout classification is covered by
    `open_timeouts_fail_and_the_late_worker_is_reaped`.
* Talktual: `npx tsc --noEmit` pass; `npm test` 537 passed; `npm run
  test:server` 60 passed; `npm run build` pass.

## Research done (record in ADRs; numbers are real)

ASR candidates (crates.io data from 2026-11-24):

| Candidate | Version / updated | License | Notes |
|---|---|---|---|
| whisper-rs (whisper.cpp) | 0.16.0 / 2026-03 | Unlicense (bindings), MIT (whisper.cpp) | C++ build; not natively streaming (sliding-window re-decode → revisions); GGML models: tiny ≈ 75 MB, tiny.en-q5 ≈ 31 MB |
| sherpa-onnx | 2.0.3 / 2026-11-22 | Apache-2.0 | true streaming (zipformer/paraformer), timestamps; needs ONNX Runtime native libs + model download (≈ 20–70 MB int8) |
| sherpa-rs | 0.6.8 / 2025-10 | MIT | wrapper over sherpa-onnx |
| vosk | 0.3.1 / 2024-10 | MIT (crate), Apache-2.0 (Vosk) | streaming partials; needs libvosk binary + model (small-en ≈ 40 MB); crate stale |
| candle-transformers (Whisper) | 0.11.1 / 2026-10 | MIT/Apache-2.0 | pure Rust, heavy dependency tree, model download |
| ort | 2.0.0-rc.15 | MIT/Apache-2.0 | ONNX Runtime bindings, MSRV 1.92 (> CE 1.85) |

**Decision:** every real ASR option needs a model download (and some need
native runtimes), which needs the user's approval. Real ASR baseline is
**blocked pending approval**. Recommended first real adapter:
sherpa-onnx streaming zipformer (true streaming, Apache-2.0) or whisper.cpp
tiny.en (simpler, but windowed). Implement the full pipeline with scripted
and reference (oracle) adapters; document the blocked status; ask the user
for approval in the final message.

Text language-ID candidates (probed in a scratch crate, not in CE):

| Candidate | Version | License | Deps | Notes |
|---|---|---|---|---|
| whatlang | 0.18.0 | MIT | hashbrown | 69 languages, confidence score; poor on short text ("Hello" → Danish) |
| whichlang | 0.1.1 | MIT | none | 16 languages (incl. en es fr de it pt nl), logistic regression; **no score exposed** (weights private) |
| lingua | 1.8.0 | Apache-2.0 | large | 3.3 MB crate + large per-language models; not evaluated |

Accuracy on the authored corpus (`fixtures/language/lid_corpus.json`,
single-language items):

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

**Decision:** adopt **whichlang 0.1.1** (already added as a workspace
dependency). Disclose in the ADR: the selection is supported by dev alone,
but the probe printed test numbers at the same time (mild selection
leakage); thresholds were tuned on dev only.

Abstention tuning on **dev only** (whichlang): `min_letters` 0/5 → 0.952
accuracy, 100 % coverage; 8 → 0.974 / 93 %; **10 → 1.000 / 86 %** (chosen;
now the default `LanguageIdConfig::min_letters = 10`); 12+ → 1.000 / ≤ 71 %.
A split-half consistency check (abstain as `Ambiguous` when both halves
have ≥ max(min_letters, 10) letters and disagree) had no effect on dev
single-language text; its purpose is mixed-language detection; evaluate
it on the held-out mixed items. whichlang gives no score →
`score_semantics: None`; never invent one.

Code-switching: 8 mixed items with character spans exist in the corpus.
Expectation: span-level detection is not supportable with these tools →
keep `LanguageScope::Span` reserved, `SpeechCapabilities::code_switching =
false`, report the mixed-item results (identified vs abstained-ambiguous).

## Implemented so far (in the WIP commit)

1. **`ce-core::speech`** (complete, 69 ce-core tests pass):
   * `config.rs`: `RecognitionConfig` (adapter, `SegmentationMode
     {ActivityHints, Continuous}`, chunk_ms, preroll_ms, partials,
     partial_interval_ms, `StabilityPolicy`, `FinalizationPolicy`,
     `LanguageHint {Auto, Expected, Fixed}`, `LanguageIdConfig`,
     word_timings, `RecognitionBudget`), `SpeechConfig {recognition,
     max_concurrent_recognitions}`, `validate()` (code
     `config.recognition_invalid`), `fingerprint()`.
   * `model.rs`: `LanguageTag` (validated BCP-47-ish), `AsrCapabilities`,
     `ModelInfo`, `RecognizerRef`, `RecognitionMode {Full, Throttled}`,
     `HintSource`, `SpeechMeta`, `RevisionStatus {Partial, Final,
     Cancelled}`, `FinalReason`, `TranscriptText`, `TranscriptDelta`
     (keep_committed_chars + append_committed + provisional),
     `WordTiming`, `Score`, `TranscriptRevision`, `LanguageScope {Utterance,
     Span (reserved), Stream}`, `LanguageSource`, `AbstainReason`,
     `LanguageDecision`, `LanguageCandidate`, `LanguageObservation`,
     `SpeechCapabilities`, `RecognitionEndReason`, `ResetReason`,
     `Percentiles`, `RecognitionCounters`, `RecognitionStatus`,
     `SpeechEvent` (12 kinds, `kind()`, `stream()`, `data_class()`:
     Transcript = `Content`, LanguageObserved = `Personal`, rest
     `Operational`).
   * Wired: `EventPayload::Speech`, `EventTopic::Speech`; commands
     `StartRecognition`, `StopRecognition`, `ResetRecognition`,
     `QueryRecognition`, `ListRecognizers`; outcomes `Recognition`,
     `Recognizers`; `EngineConfig::speech` (validated);
     `MetricsSnapshot::{recognitions_started, recognition_failures}`;
     `RuntimeHealth::recognitions`.
2. **`ce-audio::tap`**: now **multi-subscriber** (at most `MAX_TAPS = 4`,
   each bounded and dropping independently), `TapId`,
   `WorkerHandle::attach_tap -> Result<TapId>` (error
   `audio.too_many_taps`), `detach_tap(id)`. 18 ce-audio tests pass,
   including a new independence/bound test. Phase 3 runtime adapted.
3. **`ce-speech` crate** (new; deps: ce-core, ce-audio, ce-audio-intel,
   tokio sync, uuid, serde, serde_json, whichlang); 8 tests pass:
   * `adapter.rs`: `AsrAdapter` (load → begin → accept* → finalize /
     reset), `AsrFactory` (cheap `capabilities()`, `create()`),
     `AdapterHypothesis`, `AdapterWord` (utterance-relative samples),
     `LoadOutcome`, `sanitize()` (drops invalid scores, word timings and
     control characters; never invents).
   * `transcript.rs`: `UtteranceText` (stable-prefix commits by
     hypothesis-count or audio-time rule, holdback, append-only commits,
     contradictions counted, final may amend → `commit_amended`, cancel
     keeps commits, dedup), `delta()`/`apply()`, `TranscriptAssembler`
     (rejects stale, duplicate, after-final and wrong-utterance revisions;
     snapshot after gaps).
   * `resample.rs`: `downmix()`, `Resampler` (31-tap windowed-sinc FIR +
     linear interpolation; output n ↔ input n·from/to exactly, no group
     delay in the mapping).
   * `metrics.rs`: `Reservoir` (bounded, nearest-rank p50/p95/p99).
4. Runtime placeholders: `MetricsSnapshot` recognition fields are 0 and
   `RuntimeHealth::recognitions` is empty until the runtime is wired
   (marked with comments in `runtime/metrics.rs` and `runtime/core.rs`).
5. `fixtures/language/lid_corpus.json`: 134 authored items (7 languages ×
   6 dev + 12 test, length classes, plus 8 mixed test items with spans).
   **Still needs a `PROVENANCE.md`** (authored for CE, original text, no
   third-party sources, splits, label method = authoring language).

## Remaining work (in order)

1. `ce-speech/src/language.rs`: `LanguageIdentifier` trait (id, version,
   languages, score_semantics, detect); `WhichlangIdentifier` (map the 16
   whichlang languages to BCP-47: ar, zh, de, en, fr, hi, it, ja, ko, nl,
   pt, ru, es, sv, tr, vi); `ScriptedIdentifier`;
   `observe_utterance()` implementing Declared (Fixed hint), min_letters →
   `InsufficientEvidence`, min_score (only with scores), split-half →
   `Ambiguous`, Expected set → `Unsupported`; `StreamLanguage` aggregate
   (session_min_utterances, session_min_share, emit only on decision
   change). Never conflate utterance and stream scope.
2. `ce-speech/src/fake.rs`: `ScriptedAdapter` + factory (scripted
   hypotheses by utterance audio ms, endpoint flags, errors, panic, stall,
   load delay/failure/panic, shared-model reuse counter);
   `ReferenceAdapter` ("ce.asr.reference", an *oracle* for pipeline
   evaluation: reveals reference words in proportion to audio consumed,
   optional deterministic revision noise and substitution rate;
   `word_timestamps: false`). An absolute position hint is needed for the
   oracle: add a default trait method such as `fn position_hint(&mut self,
   absolute_start_sample: u64) {}`.
3. `ce-speech/src/recognizer.rs`: deterministic `StreamRecognizer`
   (push_block, push_hint, note_load, note_tap, reset, finish, status).
   Design decided:
   * Hints from Phase 3 `AnalysisEvent`s: ActivityChanged(Active) opens an
     utterance at the backdated boundary (pre-roll ring of adapter-rate
     samples); SegmentEnded(SilencePolicy) → end at the span end +
     `trailing_ms`; EndpointCandidate tier ≥ `on_endpoint_tier` → end;
     DegradedModeChanged(Shedding) or analysis Stopped → fall back to
     Continuous (`HintsChanged`); back on recovery.
   * Continuous: utterance from first audio; ends on adapter endpoint or
     `max_utterance_ms`.
   * Own gap detection (sequence, frame, discontinuity flag, format change)
     → final with `Discontinuity`, adapter reset, `Discontinuity` event;
     utterances never span a gap.
   * Partials rate-limited by `partial_interval_ms` (skip hypotheses
     between); none in `Throttled` (chunks ×4). Governor like Phase 3.
   * Adapter error → cancel (`Failure`) + reset + `Reset{AdapterError}`; 3
     consecutive → recognition `Failed`.
   * Finish: Stopped/Shutdown → final `Stopped`; stream ends → final
     `StreamEnd`; Revoked → cancel (discard queued audio).
   * Frame mapping: anchor (adapter index, stream frame, time) reset at
     gaps; frame = anchor + k·rate/adapter_rate. Never shift timestamps.
   * Latency in audio time: first partial (utterance start → input end),
     final delay (end hint → final); wall-clock processing per block.
4. `ce-speech/src/worker.rs`: one supervised thread per recognition
   (`catch_unwind`, heartbeat, stop/reset flags, load on the thread with
   load-started time so the core can enforce `load_timeout_ms`), tap
   receiver + bounded hint channel (`std::sync::mpsc::sync_channel`,
   `try_send`, drops counted), events to the core via a bounded tokio
   channel (`SpeechSignal`).
5. Runtime `runtime/core/speech.rs` + facade: factory registry on
   `EngineBuilder` (`asr_adapter(factory)`), `start_recognition`,
   `start_recognition_with` (optional), `stop_recognition`,
   `reset_recognition`, `recognition_status`, `recognizers`; requires a
   running analysis for `ActivityHints` (error `speech.analysis_required`);
   concurrency limit (`speech.capacity_exhausted`); forward Phase 3
   analysis events to the recognition's hint channel; emit speech events
   with `SpeechEvent::data_class()` (add a classified variant of
   `Core::emit`); session close / shutdown stop recognitions before
   analyses and streams; stall and load-timeout detection in housekeeping;
   fill the metrics and health placeholders.
6. `ce-speech/src/eval.rs` + example: manifests for the TTS fixtures (the
   reference transcript is the exact TTS input text, see
   `fixtures/speech/generate.sh`), WER/CER (normalized) plus a
   case/punctuation-sensitive variant, revision metrics (count, rate, edit
   distance), first/useful partial, commit and final delays, LID confusion,
   coverage and abstention, RTF, deterministic replay through the Phase 3
   analyzer plus the recognizer. Label every number as synthetic, replay or
   oracle; there is **no real-ASR quality result** (blocked).
7. Tests: unit (above modules), scripted sequences (partial → revision →
   stable → final, error, reset, gaps, cancellation), runtime integration
   with the fake audio backend, faults (panic, stall, load failure and
   timeout, cancellation, shutdown, saturation), multi-stream, stress and
   bounded memory, malformed adapter output, stale/duplicate revisions.
8. Docs: ADR-0019 (speech architecture), ADR-0020 (ASR and LID selection,
   with the tables above), ADR-0021 (transcript revision semantics),
   ADR-0022 (language observation scope/uncertainty/privacy),
   `docs/architecture/speech.md`, `fixtures/language/PROVENANCE.md`,
   `docs/phases/phase-4-report.md` with every section the user listed;
   update README, ARCHITECTURE and verify.sh (Phase 4 gates); run
   `scripts/verify.sh`, MSRV 1.85 (`cargo +1.85.0 check --workspace
   --all-targets --all-features`; avoid let-chains), cross-target checks,
   `cargo deny check`, Talktual gates; commit with "Built by Deepak and
   Claude" and push.

## Tests at handoff

`cargo test --workspace --all-targets`: **238 passed, 0 failed** (up from
224 at baseline: new ce-core speech, ce-audio tap and ce-speech tests).
`cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D
warnings` clean. `scripts/verify.sh`, all-features tests, MSRV, cross-target
and `cargo deny` were **not** re-run after the Phase 4 changes. Run them
before relying on the tree; in particular `whichlang` is new to
`cargo deny` and to the 1.85 MSRV check.
