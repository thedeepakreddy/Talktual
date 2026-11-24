# Audio intelligence (Phase 3)

Signal analysis on top of the Phase 2 audio foundation: voice activity,
candidate segments, endpoint candidates, silence windows, input quality,
a small set of signal conditions, adaptive policy and resource degradation.
Decisions: ADR-0014 (architecture), ADR-0015 (detector), ADR-0016 (results
and units), ADR-0017 (privacy), ADR-0018 (resources).

Out of scope (later phases): speech recognition, language detection,
diarization, semantics, translation, TTS, voice cloning, advanced DSP
(noise suppression, echo cancellation), UI.

## Data flow

```
device callback ─▶ ring ─▶ capture worker ─┬─▶ host queue ─▶ CaptureReader (host)
                                           └─▶ analysis tap (bounded, try_send; drops counted)
                                                   │
                                     analysis worker thread (one per analysed stream)
                                                   │  StreamAnalyzer
             ┌──────────── Framer: blocks → hops (10/20/30 ms) + gaps, features ─────────────┐
             │   detector (VoiceActivityDetector) → Stabilizer (hysteresis) → Segmenter     │
             │   QualityAnalyzer → ConditionTracker → AdaptivePolicy;  ResourceGovernor      │
             └────────────────────────────────────────────────────────────────────────────────┘
                                                   │ IntelSignal (bounded channel)
                                        runtime core (owner) ─▶ EventPayload::Analysis
                                                              ─▶ RuntimeHealth::analyses, metrics
```

## Using it

```rust,ignore
let reader = engine.open_capture(session, StreamRequest::capture()).await?;
engine.start_audio(reader.info().id).await?;
let status = engine.start_analysis(reader.info().id, None).await?;   // default config
// events: EventPayload::Analysis(AnalysisEvent::…) on the stream's session
let status = engine.analysis_status(reader.info().id).await?;
let last = engine.stop_analysis(reader.info().id).await?;
// a custom detector (adapter):
engine.start_analysis_with(id, Some(cfg), Box::new(MyDetector::new())).await?;
```

Serializable commands: `StartAnalysis { stream_id, config }`,
`StopAnalysis`, `QueryAnalysis` → `CommandOutcome::Analysis { status }`.
Errors: `analysis.not_capture`, `analysis.already_running`,
`analysis.stream_ended`, `analysis.not_found`, `analysis.unavailable`,
`config.analysis_invalid`, `audio.unknown_stream`.

## Events (`AnalysisEvent`, tag `analysis_event`)

| Event | When | Key fields |
|---|---|---|
| `started` | analysis began | algorithm (id, version, config fingerprint), capabilities |
| `activity_changed` | stabilized activity changed | state, mean score (None under low confidence), decision latency |
| `segment_started` / `segment_ended` | candidate segment | segment no., continuation, duration, close reason, mean score |
| `endpoint_candidate` | pause reached a tier | silence ms, tier, confidence (Low/Medium/High) |
| `silence_window` | inactive stretch ≥ `min_silence_report_ms` ended | duration, mean level |
| `quality_changed` | usability/reasons changed (2 windows) | `QualityMetrics` |
| `condition_changed` | indicator on/off | condition, evidence (value, threshold, unit) |
| `policy_changed` | adaptive decision applied/reverted | decision |
| `degraded_mode_changed` | resource level changed | from, to, load, backlog |
| `discontinuity` | gap, analysis drop, format change | context |
| `failed` / `stopped` | worker failed / analysis ended | redacted error / reason, counters |

Every result event carries `ResultMeta` (ADR-0016).

## Configuration (`EngineConfig::analysis`, or per call)

| Field | Default | Meaning |
|---|---|---|
| `vad` | `spectral` | `spectral` or `energy` |
| `hop_ms` | 10 | 10, 20 or 30 |
| `activity.profile` | `balanced` | `responsive` (0.55/0.40, 20/150 ms), `balanced` (0.60/0.40, 40/240 ms), `conservative` (0.70/0.45, 80/400 ms), `custom` |
| `segment.close_silence_ms` | 700 | pause that closes a segment |
| `segment.endpoint_tiers_ms` | [250, 500] | endpoint candidate tiers (ascending, < close) |
| `segment.max_segment_ms` | 30000 | forced split (continuation) |
| `segment.min_silence_report_ms` | 200 | smallest silence window reported |
| `quality.window_ms` / `clip_level` | 1000 / 0.999 | quality window, clipped-sample level |
| `conditions.*` | 2000 ms, −50 dBFS, −6 dBFS, 0.001 | silence, noise floor, overload, clipping thresholds |
| `adaptive_policy` | true | input-driven decisions |
| `budget.queue_blocks` | 50 | tap capacity |
| `budget.max_load` / `degrade_after` / `recover_after` | 0.5 / 5 / 100 | resource hysteresis |
| `budget.stall_timeout_ms` | 2000 | stall detection |
| `budget.degrade` | true | allow degraded modes |

`AnalysisConfig::validate` rejects anything outside documented ranges;
`fingerprint()` identifies the effective configuration in results.

## Capabilities (`AnalysisCapabilities`)

Detector descriptor (id, version, score semantics, supported rates 8–96 kHz,
determinism, look-ahead 0 ms, warm-up 30 hops), quality on/off, the
condition list, and `overlap_detection: false` (not supported, ADR-0017).

## Formats

Any Phase 2 delivered format: 8–96 kHz, 1–N channels (8 tracked
individually for imbalance), any block size (hops are independent of
block boundaries). Spectral features run on a ~16 kHz decimated mono mix,
so their cost does not grow with the device rate.

## Failure handling

| Fault | Behaviour |
|---|---|
| Detector/analysis panic | `failed(analysis.worker_panicked)` + `stopped(failed)`; capture continues; a new analysis can start |
| Stall | `failed(analysis.stalled)` + `stopped(failed)` after `stall_timeout_ms`; thread abandoned and reaped |
| Overload | tap drops (analysis copy only), then `Reduced` → `Shedding` |
| Discontinuity | segment closes (`discontinuity`), activity ends, state resets, one `discontinuity` event |
| Malformed samples | NaN/∞ replaced by 0 and counted (`NonFinite` quality reason); partial frames ignored |
| Unsupported format | `failed(analysis.unsupported_format)` |
| Stream closed / revoked / invalidated / failed | `stopped` with that reason; revocation discards queued audio |
| Session close / shutdown | `stopped(stream_closed)` / `stopped(shutdown)` before streams close |

## Evaluation and replay

`ce_audio_intel::eval` builds deterministic scenarios from the TTS fixtures
(`fixtures/speech`, PROVENANCE.md) under noise, music, clipping, level and
format variants, replays them through `StreamAnalyzer` and scores frame
P/R/F1, false activations, missed speech, onset/offset error and latency,
fragmentation/merges, endpoint hits/false/delay, quality reasons and
real-time factor. `cargo run --release -p ce-audio-intel --example
evaluate -- test|dev|formats|tune`. Results: `docs/phases/phase-3-evaluation.md`.
