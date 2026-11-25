# Streaming orchestration and timing (Phase 7)

Status: implemented in Phase 7 (see `docs/phases/phase-7-report.md`).
Decisions: ADR-0030 (pipeline graph), ADR-0031 (epochs and stale
rejection), ADR-0032 (timeline, causality, latency), ADR-0033 (control
plane scheduling).

## 1. What already existed (reconnaissance)

* **One control-plane task** (`runtime/core.rs`) owns all runtime state
  and assigns the global event `sequence` (ADR-0002, events.md).
* **Stage workers** run on their own supervised threads:
  * analysis (`ce-audio-intel`);
  * recognition (`ce-speech`);
  * semantics (`ce-semantic`);
  * translation (`ce-translate`).

  Each talks to the core through one bounded signal channel per stage
  type, shared by every stream: `Event(..)`, then `Ended { stream, .. }`.
* **Stage coupling is implicit**:
  * stages are started one by one with separate commands;
  * the core forwards transcripts to semantics and translation, and final
    observations to translation, if those workers happen to be running;
  * a recognition `Stopped` ends the downstream workers (`SourceEnded`);
  * `ActivityHints` segmentation is the only checked dependency (it needs
    a running analysis).
* **Timing contracts already exist** and are reused, not replaced
  (`ce-core::audio::frame`):
  * the per-stream **sample clock** (`frame_index`) is the authoritative
    timeline;
  * `timestamp` is CE monotonic time derived from the sample clock and a
    host anchor;
  * `host_time` and `device_time` are separate time bases that are never
    mixed;
  * discontinuities are explicit (`DeviceOverflow`, `DeliveryOverflow`,
    `StreamRestart`).

  Results carry `frame_range` and `time_range`: analysis `ResultMeta`,
  speech `SpeechMeta`, and translation `TranslationSource`. Semantics
  refers to the source revision instead.
* **Revision contracts**:
  * transcripts: `(stream, utterance, revision)`, statuses
    partial/final/cancelled, utterance numbers never reused (ADR-0021);
  * semantics: observations reference `SourceRef` (utterance, revision)
    and are retracted when they lose support (ADR-0025);
  * translation: results carry `sequence` per utterance, which is
    **worker-local** and restarts at 0 in a new worker run. `Started`
    resets consumer state, and `Withdrawn{through_sequence}` invalidates
    results (ADR-0029 and the Phase 6 audit).

### Gaps found

1. **No worker identity on signals.** Signals carry only the stream. A
   signal still queued from a stopped or abandoned worker is attributed
   to whatever worker currently runs on that stream:
   * its `Ended` removes the *new* worker's entry;
   * its events are published after the core already reported that
     worker `Stopped`.

   Spec §9 and §11 require prior-generation results to be rejected.
2. **No explicit graph.** Nothing validates that semantics or translation
   have a recognition to feed them. A required stage can silently depend
   on one that is absent. Nothing defines what happens when an optional
   stage fails.
3. **No causal or timeline checks.** Nothing verifies, across stages,
   that derived outputs reference a known source revision, that frame
   and time ranges are well formed, or that the sample-clock mapping is
   continuous.
4. **No end-to-end latency or deadline measure.** Only per-stage
   internals exist (for example translation `queued_ms` and
   `adapter_ms`).
5. **Data arms of the control loop are `biased`.** A stage channel that
   never empties starves the arms polled after it (spec §10 forbids
   unjustified starvation).

## 2. Model

```
capture stream ──► Analysis ──(hints)──► Recognition ──► Semantics
                         (required by ActivityHints)  │          │
                                                       │          ▼ (optional hint input)
                                                       └──────► Translation
```

A **pipeline** is the orchestration unit: one capture stream plus a
validated set of stages. Each stage has a kind, a requirement and an
optional configuration.

* **Kinds**: `analysis`, `recognition`, `semantics`, `translation`.
* **Requirement**: `required` or `optional`.
* **Configuration**: when absent, the engine default is used.

Edges come from the stage contracts, not from list order.

| Stage | Hard dependency | Soft input |
|---|---|---|
| analysis | the capture stream | — |
| recognition | the capture stream; analysis if `segmentation = activity_hints` | — (hints matter only under `activity_hints`, where analysis is a hard dependency) |
| semantics | recognition | — |
| translation | recognition | semantics, if `context.use_semantic` |

### Validation before activation

The pipeline is checked in three steps (`ce-orchestrate::graph`), and
nothing starts if any check fails.

* **Structure** (spec §6, §11):
  * no empty pipeline and no duplicate kind;
  * every hard dependency is present;
  * a `required` stage never hard-depends on an `optional` one (otherwise
    an optional failure would hide the loss of a required function);
  * per-stage configuration validates.
* **Topology**: a generic DAG check with deterministic Kahn ordering
  (ties broken by declaration order) rejects cycles. The fixed kinds
  cannot form one, but the check runs on every graph and is tested on
  synthetic graphs.
* **Pre-flight compatibility** (runtime): the stream exists, is a
  capture stream and is live; adapters for recognition, semantics and
  translation resolve against the registered providers; pipeline
  capacity is available.

A missing soft input is not an error. It is recorded as a **note** in
the validated graph and the status (for example
`translation.hints_unavailable`).

**Fingerprint**: FNV over a versioned canonical rendering of the
validated graph (kinds, requirements, configuration fingerprints, policy).
It contains no secrets.

**No hot reconfiguration**: a changed spec applies at `restart` (a safe
boundary).

## 3. Execution and lifecycle

Pipeline states: `starting → running ⇄ degraded → stopping → stopped`,
and `starting|running|degraded → failed`. Transitions are validated.

* **Start**: stages start in topological order. A required stage that
  fails to start rolls back the started stages in reverse order and
  returns the error; the pipeline ends `failed`. An optional stage that
  fails to start leaves the pipeline `degraded`, with a typed reason.
* **Runtime failure** (worker `Failed`, a stall, an unexpected end): an
  optional stage makes the pipeline `degraded`. A required stage makes
  it `failed` and stops the remaining stages (cancel). Each outcome is
  an event, never a silent change.
* **Stop**:
  * `drain` (default) stops upstream first: recognition ends, and the
    downstream workers finish the finals they already have (bounded by
    the module operation timeout).
  * `cancel` stops downstream first and discards pending work.
  * Each stage reports `clean`, `timed_out` (abandoned) or `not_running`.
* **Reset**: reset recognition, semantic context and translation in
  dependency order, and increment `reset_epoch`. Identifiers and
  correlation are kept: utterance numbers are never reused, and
  translation sequences continue (Phase 6 D2).
* **Restart**: the replacement (stored or given) is fully checked
  first. That covers the specification, stream, graph, abandoned-worker
  cap and per-stage pre-flight. If any check fails, the running pipeline
  is left untouched (audit AUD-02). Then the pipeline is stopped and
  started with a new `epoch`. Every stage worker gets a new generation.
  A start-time failure of a valid replacement rolls back, and is reported
  with per-stage outcomes (`pipeline_stopped{reason: failed}`, AUD-06).
* **Optional stage stopped directly**: while the pipeline runs, it
  degrades the pipeline (`stage_degraded`, code `orchestration.stage_ended`,
  AUD-05).
* **Stop deadline**: one shared deadline bounds a whole stop (AUD-13). A
  slow upstream stop can leave downstream stages reported `timed_out`.
* **Source end**: when the stream ends and every stage has ended, the
  pipeline is `stopped` with reason `source_ended`.

### Epochs and stale rejection (ADR-0031)

* Every stage worker start gets a fresh **generation** (u64, engine-wide
  monotonic).
* The runtime gives each worker its own small bounded channel. A
  forwarding task tags each signal with that generation into the stage's
  core channel. Order is preserved, backpressure is preserved, and the
  task ends with the worker (`worker_forwarders` gauge).
* The core admits a signal only if the stream's running entry has the
  same generation. Anything else is a **stale output**: it is counted,
  logged and reported (`orchestration.stale_output_rejected`), and never
  published.
* Pipelines also carry `epoch` (start or restart) and `reset_epoch`.
  Both appear in every orchestration event and in the status.

## 4. Observer: correlation, causality, timeline, latency

Each pipeline has a bounded **observer** (`ce-orchestrate`, pure).

The core feeds it every admitted stage event of the pipeline stream,
after admission and before publication. The observer **annotates and
counts**; it does not rewrite payloads. Within a generation, workers are
the authority on their own ordering, and dropping, for example, a
translation result would break Phase 6's gapless-sequence contract. Only
stale-generation signals are dropped (§3).

* **Transcript reconciliation** (bounded table of 256 utterances): each
  revision is classified as `applied`, `duplicate`, `stale` (older
  revision), `after_final` or `evicted`. Every class except `applied` is
  a protocol violation of ADR-0021 and is reported.
* **Causal validation** of derived outputs:
  * A semantic observation or translation result must reference a
    revision the reconciler has seen. It is classified `current`,
    `superseded` (a newer revision exists, which is allowed but counted),
    `unknown` (a violation) or `evicted` (outside the bounded window,
    which is reported as such and not called a violation).
  * Translation results are checked against Phase 6 rules: in one worker
    run, the sequence per utterance strictly increases. `Started` resets
    the per-utterance view (worker-local sequences may restart at 0), and
    a duplicate or regressing sequence is a violation.
* **Timeline** (ADR-0032):
  * The sample clock is authoritative.
  * **Normalization** produces media time `frames / rate`, which is
    exact. Reported monotonic times are kept alongside; nothing is
    rewritten.
  * The observed mapping offset is `time_range.start − media(start)`.
    It is constant within one anchoring. A change larger than one frame
    is a **re-anchor** (for example a stream restart): a new timeline
    segment is opened and counted, and is never smoothed over.
  * Malformed ranges (end before start, or a duration disagreeing with
    `frames / rate` by more than one frame) are temporal-integrity
    violations.
  * A new utterance starting before the previous one on the same stage
    is **out of order**.
  * Audio discontinuities with lost frames are recorded as gaps.
  * Uncertainty of the monotonic mapping: `unknown` unless the audio
    layer reports a bound. CE never implies device and host clock
    synchronization.
* **Latency** (monotonic only; ADR-0032):
  * Audio-to-output latency: `now_monotonic − source time_range.end` for
    final transcripts, final semantic observations and final
    translations.
  * Translation queue wait and processing come from `TranslationTiming`.
  * Orchestration overhead is the time spent in the observer per event.
  * All of these are bounded reservoirs reporting p50, p95 and max.
* **Deadlines**: an optional budget per output (`transcript_final_ms`,
  `semantic_final_ms`, `translation_final_ms`). A miss is a typed,
  observable outcome (`deadline_missed`, with stage, utterance, latency
  and budget). A late result is still published, because Phase 4–6
  consumers rely on it. It never replaces newer state, which the
  revision rules already prevent. Deadlines never cancel work silently.

## 5. Scheduling and backpressure (ADR-0033)

* **Control priority**: the core polls shutdown and commands first
  (`biased`). Commands are user-rate and must not wait behind data.
* **Data arms are polled fairly**: ingress, module exits, the audio,
  analysis, speech, semantic and translation channels, and housekeeping
  sit in an **unbiased** inner select with a random start, so no data
  channel starves another.
* **Bounds**:
  * existing per-stage queues and policies are unchanged (partials are
    coalesced before finals are dropped; finals are never coalesced);
  * the per-worker forwarding channel holds 64 signals;
  * at most `orchestration.max_pipelines` pipelines (default 8);
  * observer tables are bounded (256 utterances; reservoirs of 512).
* **Retries**: none are added. Phase 6's bounded, explicit retry policy
  is unchanged.
* **Abandoned workers** (audit AUD-01):
  * A worker abandoned at a stop timeout or a stall keeps its OS thread
    (threads cannot be killed) and its forwarder until the thread exits.
  * The core counts these from the worker handles, live (a thread that
    exited no longer counts). No new stage worker starts while the count
    is at `orchestration.max_abandoned_workers`.
  * The resulting bound on abandoned threads is the cap − 1 plus the
    workers running when the cap was reached.
  * The gauges are `abandoned_workers` and `starts_refused_abandoned`.
* **Diagnostics** (AUD-09): at most 32 findings per kind and stage per
  epoch are published; the rest are counted.

## 6. Privacy

* Orchestration events and status carry identifiers, counts, kinds and
  durations only: never text, audio or language content. All of it is
  `Operational`.
* Observer state is in memory, bounded, and cleared when the pipeline
  ends. Nothing is persisted or transmitted.

## 7. Observability

* **Events** (`EventPayload::Orchestration`):
  * `pipeline_started`, `stage_started`, `stage_degraded`,
    `stage_ended`, `pipeline_reset`, `pipeline_stopped` and
    `pipeline_failed`;
  * `stale_output_rejected`, `causal_violation`, `timeline_anomaly` and
    `deadline_missed`.
* **Status**: `pipeline_status` and `RuntimeHealth.pipelines`.
* **Metrics**: pipelines started and failed, stale outputs rejected,
  causal violations, timeline anomalies and deadline misses.

## 8. Out of scope

* New models, adapters or algorithms.
* Phase 8–11 features, UI and persistence.
* Hot reconfiguration.
* Clock synchronization across devices.
* Preempting in-process adapters (an existing limitation).
