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
| Backpressure | per-stage queues are unchanged (Phase 3–6 tests); forwarders are bounded at 64 | existing W/R tests | **PARTIAL**: no new sustained or burst load test of the forwarder hop |
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
| `overhead` | time spent in the observer per event | `std::time::Instant` | orchestration overhead |
| Counters | `PipelineStatus.counters`; `MetricsSnapshot` `pipelines_started`, `pipelines_failed`, `stale_outputs_rejected`, `causal_violations`, `timeline_anomalies`, `deadline_misses` and the `worker_forwarders` gauge | — | — |

* **Reservoirs**: the last 512 samples, nearest-rank p50 and p95, and the
  maximum, plus an all-time count.
* **No latency is claimed** for outputs with unknown timing (no anchor).
* **Accuracy**: latency is bounded by the anchor's accuracy (Phase 2):
  the capture poll interval plus one device buffer.

## 3. Results

### Observer overhead (release, real clock)

Workload: `observer_overhead` example. 20,000 utterances, each with 5
transcript revisions, 5 translation results and 1 finalization: 220,000
events per run, 5 runs per invocation, 3 invocations.

| Invocation | Per-run means (ns/event) | Mean | SD |
|---|---|---|---|
| 1 | 149, 150, 149, 150, 149 | 149 | 0 |
| 2 | 160, 150, 152, 148, 148 | 152 | 5 |
| 3 | 151, 150, 150, 149, 150 | 150 | 1 |

* The figure **includes constructing the synthetic inputs** (string
  allocation per revision), so it is an upper bound on the observer
  alone.
* State stayed at the bound: 256 utterances remembered.
* This is overhead per observed event, **not** end-to-end latency: low
  orchestration overhead does not prove end-to-end latency (risk
  register).

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

### Not measured

* End-to-end latency with the system clock, real audio and a real model.
* Behaviour under a real device's clock drift.
* Mobile or desktop hardware.

These need hardware and model approval, and are recorded as PARTIAL or
NOT TESTED in the ledger.
