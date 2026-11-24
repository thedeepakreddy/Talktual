# Phase 7 — timestamp, clock-domain and timeline contracts; configuration guide

Related: [architecture](../architecture/orchestration.md) · ADR-0032 ·
[evaluation](phase-7-evaluation.md).

## 1. Clock domains (reused from Phases 1–4; nothing is redefined)

| Quantity | Origin | Domain and units | Precision | Used for | Never used for |
|---|---|---|---|---|---|
| `frame_index` and `frame_range` | audio worker (sample clock) | frames since the stream first started, **including lost frames**; per stream | exact (1 frame) | authoritative stream timeline, ordering within a stream, media time | comparing streams |
| media time (Phase 7) | `ce-orchestrate::timeline` | `frames / delivered sample_rate`, in ns; per stream | exact up to ns floor rounding | normalized timeline, durations in audio | wall-clock claims |
| `timestamp` and `time_range` | producers (audio worker; Phase 3 analyzer; Phase 4 recognizer) | CE monotonic ns since the runtime clock origin, derived as `anchor_time + (frame − anchor_frame) / rate` | bounded by the anchor's error (capture poll interval + one device buffer, Phase 2) | latency to output; correlation with core events | ordering across a re-anchor; device-clock claims |
| `host_time` | audio worker | CE monotonic ns when a block was assembled | host scheduling | drift diagnostics (Phase 2) | — |
| `device_time` | backend | the backend's own stream clock (a different time base) | backend-defined | — (never mixed) | anything in CE arithmetic |
| envelope `monotonic` | core (`Clock::monotonic`) | CE monotonic ns at dispatch | clock resolution | `now` for latency and deadlines | ordering (the envelope `sequence` orders events) |
| envelope `emitted_at` | core (`Clock::wall`) | UTC wall ns | can jump | humans, external correlation | ordering, durations, deadlines |
| deadlines and stop bounds | runtime timer and `Instant` | monotonic | timer resolution | stop/await bounds; observer overhead | — |

* **Capture vs arrival vs processing time**: `time_range` is capture time
  (anchor-derived). The envelope `monotonic` is the time the core
  processed and published the result. Worker queue and processing times
  are reported separately where a stage measures them (translation
  `queued_ms` and `adapter_ms`).
* **Transformations**: the only Phase 7 transformation is media time
  (exact). Reported times are copied unchanged into the normalized span
  (`NormalizedSpan.reported`).

## 2. Normalization rules (ADR-0032)

1. Media time is exact and always available.
2. Reported monotonic times are **kept**. The observed offset
   `time_range.start − media(start)` is tracked **per stage** (each
   producer anchors itself).
3. An offset change of more than one frame plus 1 µs (rounding slack) is
   a **re-anchor**. It opens a new segment, is reported
   (`timeline{kind: re_anchor}`) and counted, and is never smoothed or
   interpolated.
4. `time_range == (0, 0)` while frames advance means the producer had no
   anchor. Timing is **unknown**: the span keeps media time only, no
   latency is claimed for it, and the finding is reported.
5. **Integrity**: end before start (`malformed_range`) and a reported
   duration that disagrees with `frames / rate` by more than one frame
   (`duration_mismatch`) are reported.
6. **Ordering**: a new unit (utterance, segment) that starts earlier on
   the sample clock than the previous unit of the same stage is
   `out_of_order`. This uses causal identifiers, not arrival order. A
   later revision of the same unit may move its start.
7. **Gaps**: audio discontinuities (`DeviceOverflow`, `DeliveryOverflow`
   with lost frames, and `StreamRestart`, which has no frame advance) are
   recorded as `gap`, with lost frames summed. A stream restart shows up
   on the outputs as a re-anchor.
8. **Uncertainty** of the monotonic mapping is `unknown`. CE does not
   assume device and host clocks are synchronized, and normalization is
   not synchronization (risk register). A bounded value
   (`TimingUncertainty::Bounded`) exists for a future audio-layer bound.
   Nothing reports one yet.
9. **Source or device changes**: a device change invalidates the stream
   (Phase 2 `StreamInvalidated`). Stages end with `SourceEnded` and the
   pipeline ends with `source_ended`; a new stream is a new timeline.

## 3. Revision, duplicate, stale and sequence policy

| Situation | Classification | Action |
|---|---|---|
| Transcript revision newer than the latest of its utterance | applied | — |
| Same revision again | `duplicate_revision` | reported (protocol violation) |
| Older revision | `stale_revision` | reported |
| Any revision after a final or cancellation | `after_final` | reported |
| Revision of an utterance older than the bounded window | evicted | counted; not called a violation |
| Derived output whose source is the latest revision | current | — |
| Derived output from an older revision (allowed; replaced downstream per Phases 5–6) | superseded | counted |
| Derived output referring to a revision never published | `unknown_source` | reported |
| Translation `Started` | new worker run | per-utterance sequence view cleared |
| Translation sequence not above the last one in the run | `sequence_regression` | reported |
| Translation sequence skipping numbers in the run | `sequence_gap` | reported |
| Any output of an earlier worker generation | stale output | **dropped**: counted, reported once per generation, never published |

Findings are published **after** the stage event they concern. Within a
worker generation, payloads are never altered or dropped (ADR-0032).

## 4. Configuration and compatibility guide

* **Engine settings**, `EngineConfig.orchestration`:
  * `max_pipelines`: 1..=64, default 8;
  * `observer_utterances`: 16..=4096, default 256.
* **`PipelineSpec`**:
  * `stages`: 1..=8 `StageSpec { kind, requirement, config? }`. A missing
    `config` uses the engine default for that stage.
  * `stop`: `drain` (default) or `cancel`.
  * `deadlines`: `transcript_final_ms`, `semantic_final_ms` and
    `translation_final_ms`, each optional, 1..=600000.
* **Dependency rules** (validated before anything starts):
  * recognition needs analysis when `segmentation = activity_hints`
    (**the engine default**, so a default recognition stage needs an
    analysis stage, or a `continuous` configuration);
  * semantics and translation need recognition;
  * a required stage cannot depend on an optional one;
  * translation with `context.use_semantic` and no semantics stage gets
    the note `translation.hints_unavailable`.
* **Pre-flight** checks that:
  * the stream exists, is a capture stream and is live;
  * adapters resolve (recognition adapter, semantic adapter, translation
    adapters including privacy and the remote policy);
  * no worker of the stage already runs on the stream outside a pipeline
    (`orchestration.stage_already_running`);
  * capacity is available.
* **Compatibility not checked**: adapter contract versions are not
  compared. Only one contract version exists, so this is PARTIAL in the
  ledger.
* **Fingerprint**: FNV-1a over `pipeline.v1`, the kinds, requirements and
  configuration fingerprints in execution order, the stop mode and the
  deadlines. It contains no secrets and does not depend on declaration
  order.
* **Error codes**:
  * `orchestration.empty_pipeline`, `config_mismatch`, `duplicate_stage`,
    `missing_dependency`, `required_depends_on_optional` and `cycle`;
  * `orchestration.unavailable`, `not_capture`, `stream_ended`,
    `already_running`, `capacity_exhausted`, `stage_already_running` and
    `not_found`;
  * at run time: `orchestration.required_stage_failed`, `stage_failed`,
    `stage_stalled`, `stop_timed_out` and `dependency_unavailable`;
  * `config.orchestration_invalid`.
