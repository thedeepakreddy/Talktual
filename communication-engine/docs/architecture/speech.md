# Speech recognition and language intelligence (Phase 4)

Decisions: ADR-0019 (architecture), ADR-0020 (ASR / LID selection),
ADR-0021 (transcript semantics), ADR-0022 (language observations).
Evaluation: [phase-4-evaluation.md](../phases/phase-4-evaluation.md).

**Status.** The full pipeline exists and is tested with a deterministic
scripted adapter and a reference **oracle** adapter. **No real ASR model is
integrated** (a model download needs approval), so CE does not yet
recognize speech; no recognition accuracy, model latency or model memory
is claimed.

## Components

```
ce-core::speech            contracts: RecognitionConfig (+ validate, fingerprint),
                           AsrCapabilities, TranscriptRevision, SpeechMeta,
                           LanguageObservation, SpeechEvent, RecognitionStatus
ce-speech::adapter         AsrFactory / AsrAdapter, sanitize()
ce-speech::fake            ScriptedFactory (tests, faults), ReferenceFactory (oracle)
ce-speech::transcript      UtteranceText (commits, deltas), TranscriptAssembler (consumer)
ce-speech::language        LanguageIdentifier, WhichlangIdentifier, observe_utterance,
                           StreamLanguage
ce-speech::resample        downmix, Resampler (exact position mapping)
ce-speech::recognizer      StreamRecognizer — deterministic per-stream state machine
ce-speech::worker          spawn_recognition, RecognitionHandle, SpeechSignal
ce-speech::eval            manifests, scenarios, replay, metrics, LID evaluation
communication-engine       runtime/core/speech.rs; facade start/stop/reset/status/recognizers;
                           EngineBuilder::asr_adapter / language_identifier
```

## Dataflow

```
device callback ─▶ capture worker ─┬─▶ host CaptureReader (unchanged)
                                   ├─▶ tap #1 (bounded, try_send) ─▶ analysis thread ─▶ AnalysisEvent ─┐
                                   └─▶ tap #2 (bounded, try_send) ─▶ recognition thread                │
                                                         ▲                 │                          │
                     core forwards Phase 3 hints ────────┘ (bounded,       │ SpeechSignal (bounded)    │
                     with try_send, drops counted)        try_send)        ▼                          ▼
                                                                    runtime core ─▶ EventPayload::Speech / ::Analysis
                                                                                    (bus; session-correlated;
                                                                                     privacy class per event)
```

Inside the recognition thread, per block: gap detection → downmix →
resample to the adapter rate → pre-roll ring → hints (deferred until their
audio has arrived) → utterance open/feed/finalize → chunks to the adapter
(`chunk_ms`, ×4 when throttled) → sanitize → commit policy → revision with
exact frames and times → final: text language observation → stream
aggregate.

## Lifecycle

1. **Register** adapters on the builder (`asr_adapter`); discovery
   (`Engine::recognizers`) never creates or loads anything.
2. **Start** (`start_recognition(stream, config)`): validate, resolve the
   adapter (`config.adapter` or the first registered), check the stream is
   a live capture, require a running analysis for `ActivityHints`, enforce
   concurrency limits, create the (unloaded) adapter, spawn the worker,
   attach its tap. `Started` is emitted.
3. **Load** on the worker thread; `ModelReady { load_ms, reused }`; the
   core fails it with `speech.load_timeout` after `load_timeout_ms`.
4. **Run**: `UtteranceStarted`, `Transcript` revisions, `LanguageObserved`
   (stream scope), `Reset`, `Discontinuity`, `QueuePressure`,
   `ModeChanged`, `HintsChanged`.
5. **Reset** (`reset_recognition`): open utterance `Cancelled(Reset)`,
   adapter reset, recognition continues with new audio.
6. **End**: `stop_recognition` (final `Stopped`), stream close / device
   loss / failure (final `StreamEnd`), revocation (cancel, queued audio
   discarded), adapter failure ×3, panic or stall (`Failed`), shutdown;
   then `Stopped { reason, counters }`. Session close and shutdown stop
   recognitions **before** analyses and streams.

## Configuration (`RecognitionConfig`, validated, fingerprinted)

| Field | Default | Meaning |
|---|---|---|
| `adapter` | first registered | adapter id |
| `segmentation` | `ActivityHints` | or `Continuous` |
| `chunk_ms` | 100 | audio per adapter call (10–1000) |
| `preroll_ms` | 500 | audio kept for backdated onsets (0–2000) |
| `partials` / `partial_interval_ms` | true / 200 | partial cadence in audio time |
| `stability` | 3 hyp / 800 ms / holdback 1 | commit policy |
| `finalization` | segment end on, endpoint tier off, adapter endpoint on, max 20 s, trailing 200 ms | |
| `language` | `Auto` | `Expected([..])` or `Fixed(tag)` |
| `language_id` | enabled, min 10 letters, min score 0.15, stream 2 utterances / 0.6 share, split check on | |
| `word_timings` | true | only if the adapter declares them |
| `budget` | tap 200 blocks, hints 256, max load 0.8, degrade after 10, recover after 100, stall 5 s, load 60 s | |

`SpeechConfig::max_concurrent_recognitions` (default 4, 1–64) limits the
engine; `AsrCapabilities::max_concurrent_streams` limits an adapter.

## Capabilities

`SpeechCapabilities` = the adapter's `AsrCapabilities` (streaming,
partials, revisions, word timestamps, punctuation, casing, languages,
language reports, endpointing, rate, maximum utterance, score semantics,
determinism, concurrency, load requirement) + the language identifier
(id, version, languages) + `code_switching: false`. CE never reports a
capability the adapter did not declare and strips undeclared output.

## Errors

`config.recognition_invalid`, `speech.unavailable` (no audio backend),
`speech.no_recognizer`, `speech.not_capture`, `speech.stream_ended`,
`speech.analysis_required`, `speech.already_running`,
`speech.capacity_exhausted`, `speech.not_found`; in events:
`speech.worker_panicked`, `speech.stalled`, `speech.load_timeout`,
`speech.adapter_failed` (three consecutive adapter errors), adapter load
errors.

## Privacy

Transcripts are `Content`, language observations `Personal`, everything
else `Operational` (envelope `privacy`). No audio, transcript text or
language decision is written to logs; observations carry ids, adapter,
config fingerprint and reasons only. Nothing is persisted. Language is a
property of text, never of the speaker.

## Writing a real adapter

Implement `AsrFactory::capabilities` (static, cheap, honest) and
`create`, and `AsrAdapter::{load, begin, accept, finalize, reset}`. Accept
mono f32 at your declared rate; return full hypotheses; positions in
utterance samples; set `endpoint` only if you declared endpointing. Do not
spawn threads you do not join, and make `load` safe to abandon (the runtime
may stop waiting for it).
