# Phase 7 evaluation — streaming orchestration & timing

**Evidence provenance.** Every Phase 7 result below is **synthetic,
scripted or replayed**:
* fake audio backend and scripted ASR;
* the real Phase 3 analyzer and Phase 6 processor on Phase 3 fixtures or
  scripted inputs;
* scripted translation adapters with injected faults.

There is **no real speech, no real model, no audio hardware and no
mobile/desktop device** evidence. Runtime tests use a **manual test
clock**, so their latency values are not real-time measurements.
Observer-overhead figures use the real monotonic clock, in a release
build, in this container:

| Environment | Value |
|---|---|
| Machine | Linux x86_64 cloud container, 4 vCPU |
| Rust | 1.97.0; MSRV 1.85.0 also run |
| Date | 2026-11-24 |

## 1. What is evaluated, and how

| Area (spec §16) | Method | Evidence |
|---|---|---|
| Graph validation | unit tests on rule pipelines and synthetic DAGs (cycles, self-loops, ties); runtime rejection before activation | `ce-orchestrate` `graph` (6 tests); R `invalid_pipelines_fail_before_any_stage_starts` |
| Causal correctness | the real Phase 6 processor's events replayed into the observer; contract violations injected | O `a_clean_stream…`, `revision_contract_violations…`, `translation_started_resets…` |
| Real producers vs the checks (false-positive control) | real Phase 4 recognizer and Phase 5/6 workers in the runtime; the real Phase 3 analyzer on its dev fixtures (clean, pink 10 dB, clipped 20 dB) | R end-to-end: 0 violations, 0 anomalies; O `the_real_analyzer_satisfies_the_timeline_checks`: 0 findings |
| Temporal mapping, uncertainty, discontinuities | scripted anchored spans; re-anchor; unknown timing; malformed and mismatched ranges; out-of-order; gaps | `timeline` (3 tests); O `timeline_reanchors_unknown_timing…` |
| Revision, stale, duplicate and restart correctness | reconcile tests; generation admission; runtime restart (new epoch and generations; translation sequences restart at 0 with no violation) | `reconcile` (3), `epoch` (1); R `restart_and_reset…`, `a_required_failure…restart recovers` |
| Multi-stream isolation | two pipelines; one fails (panicking adapter), the other completes | R `pipelines_on_two_streams_are_isolated` |
| Fairness | a channel kept non-empty cannot starve another | `core.rs` `scheduling_tests` |
| Cancellation, deadlines, lifecycle, recovery | drain/cancel outcomes; hung stage reported `timed_out`; deadline misses typed; required failure then restart | R tests |
| Backpressure and saturation (audit AUD-04) | one stream's translation blocked during sustained two-stream load; per-worker forwarder filled | `aud04_a_saturated_stage…`; `tagged_sender_is_bounded…` | stage level and forwarder verified; **PARTIAL**: core-channel saturation not forced |
| Abandoned-worker bound (audit AUD-01) | hung adapters abandoned repeatedly; concurrent starts at the cap; recovery | `aud01_*` (3); audit R1 | the cap holds (16 by default), with typed refusal and recovery |
| Stale-output rejection end to end (audit AUD-03) | late signals sent through replaced and abandoned workers' real senders (feature `test-hooks`) | `stale_output_runtime` (2) | rejected, reported, never published; current ones published |
| Fault injection, repeated lifecycle | panics, hangs, missing adapters; 20 start/stop rounds with 10 restarts | R tests |
| Bounds | reconcile window; reservoirs (512); forwarders return to 0; at most 16 finished pipelines remembered | `latency`, O `state_is_bounded…`, R `repeated_lifecycles…` |
| Latency | the observer's own overhead (real clock); audio-to-output plumbing on the manual clock | §3 |

## 2. Metric definitions

| Metric | Definition | Clock | Notes |
|---|---|---|---|
| `transcript_final` | envelope-time `now` minus the final revision's `time_range.end` | CE monotonic | audio-end to final, as seen by the core; includes ASR, queueing and the core |
| `semantic_final` | `now` at `utterance_finalized` minus the utterance's latest `time_range.end` | CE monotonic | — |
| `translation_final` | `now` at the final translation minus `source.time_range.end` | CE monotonic | — |
| `translation_queue`, `translation_adapter` | the worker-reported `queued_ms` and `adapter_ms` | worker `Instant` | queue wait is kept separate from processing |
| `overhead` | time spent in the runtime's observer call per event (clock reads, lookup, observer, finding limiting) | `std::time::Instant` | orchestration overhead (see §3) |
| Counters | `PipelineStatus.counters` (including `suppressed_findings`, AUD-09); `MetricsSnapshot` `pipelines_started`, `pipelines_failed`, `stale_outputs_rejected`, `causal_violations`, `timeline_anomalies`, `deadline_misses`, `starts_refused_abandoned` and the gauges `worker_forwarders` and `abandoned_workers` | — | — |

* **Reservoirs**: the last 512 samples, nearest-rank p50 and p95, and the
  maximum, plus an all-time count.
* **No latency is claimed** for outputs with unknown timing (no anchor).
* **Accuracy**: latency is bounded by the anchor's accuracy (Phase 2):
  the capture poll interval plus one device buffer.

## 3. Results

### Observer overhead (release, real clock) — corrected after audit AUD-10

The first measurement (~149–152 ns per event, kept here for the record)
timed inline input construction together with the observer, and excluded
the runtime's wrapper around it. It was corrected as follows.

* **Isolated observer**: `observer_overhead` example. The inputs are
  pre-built, so only the observer calls are timed. 220,000 events per
  run, 5 runs per invocation.

  | Invocation | Per-run means (ns/event) | Mean |
  |---|---|---|
  | 1 | 121, 122, 122, 122, 122 | 122 |
  | 2 | 122, 126, 125, 123, 127 | 125 |
  | 3 | 136, 137, 128, 124, 121 | 129 |

  Input construction, timed separately and not included: 383–406
  ns/event (this includes cloning and storing the inputs). The gate
  asserts a coarse mean below 5 µs, as a regression guard only.
* **Runtime per-event observer cost**: the `overhead` reservoir, which
  includes the clock reads, the pipeline lookup and finding handling.
  Release build, wall clock, two streams with live workers, fake audio;
  audit test R4, 3 runs:

  | Run | p50 | p95 | max | Status round trip p50 / p95 |
  |---|---|---|---|---|
  | 1 | 1.2 µs | 3.0 µs | 27.7 µs | 0.121 / 0.203 ms |
  | 2 | 1.0 µs | 2.6 µs | 30.0 µs | 0.114 / 0.197 ms |
  | 3 | 1.2 µs | 2.7 µs | 53.5 µs | 0.117 / 0.189 ms |

  The runtime figure is roughly **10×** the isolated one, and it is the
  representative number for "orchestration overhead per event" in this
  container. Neither figure is end-to-end latency.

### Runtime (fake audio, scripted adapters, manual clock)

* **End to end**: recognition, semantics and translation with real
  workers. 0 causal violations, 0 timeline anomalies and 0 stale outputs
  were observed. Latency summaries are produced for `transcript_final`,
  `translation_final` and `overhead`. Values on the manual clock are not
  meaningful as wall latency.
* **Deadline**: the manual clock was advanced 3 s while a 200 ms
  scripted adapter worked. `deadline_missed{translation, utterance 0}` was
  reported with latency between 2,000 and 3,000 ms against a 100 ms
  budget, and the final translation was still published.
* **Stability**: the orchestration runtime tests and the Phase 6 runtime
  tests were run 5 times consecutively without failure (§ report).
* **Saturation (AUD-04)**: stream A's translation adapter was blocked
  during 2.4 s of audio on two streams.
  * A's backlog stayed between 1 and 64; one observed run showed 24.
  * No input was refused: superseded partials were coalesced and finals
    kept.
  * Stream B translated 34 results, 11 of them finals, meanwhile.
  * Status round trips stayed under the 1 s test bound.
  * The forwarder count was unchanged.
  * After release, A's finals were translated and coalescing was
    confirmed. Shutdown was clean.
* **Abandoned-worker cap (AUD-01)**, audit R1 after the correction:
  * forwarders and threads grow 1→16 across rounds 1–16;
  * rounds 17–20 are refused with `orchestration.abandoned_workers_exhausted`;
  * the gauge stays at 16.

  Before the correction, they grew without a bound.

### Not measured

* End-to-end latency with the system clock, real audio and a real model.
* Behaviour under a real device's clock drift.
* Mobile or desktop hardware.

These need hardware and model approval, and are recorded as PARTIAL or
NOT TESTED in the ledger.

## 4. Final corrections (verification audit VA-01, VA-02)

* **Analysis-stage abandoned workers**:
  * R7 (cap set to 3): 3 hung analysis workers abandoned; the gauge
    reads 3; there are 3 extra threads and 3 forwarders; the next 3
    starts are refused with `orchestration.abandoned_workers_exhausted`.
  * Before the correction: 4 abandoned, gauge 1, no cap refusal (the
    audio tap limit refused instead).
  * `va01_*`: analysis and translation share the cap; 12 simultaneous
    analysis starts at the cap are all refused with no worker spawned;
    capacity returns only after the threads exit.
* **Metric completeness**:
  * R6: 126 deadline misses, 94 suppressed events, engine metric 126
    (before: 32).
  * `va02_…`: exactly 32 events published; the engine metric equals the
    pipeline counter, and is cumulative across epochs.
  * Causal violations through `test-hooks`: 40 injections give ≥ 79
    violations, 32 events of the kind published, and a complete metric.
  * Timeline-anomaly metrics share the mechanism; no runtime test
    produces more than 32 of them.

All of this is synthetic (fake audio, scripted ASR, scripted adapters
and detectors, manual test clock).
