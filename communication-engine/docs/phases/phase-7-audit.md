# Phase 7 audit — Streaming Orchestration & Timing Intelligence

* **Audited commit**: `8418622` on `deepak/gallant-hamilton-wegi2i`.
* **Baseline**: `main` = `origin/main` = `8c92318`.
* **Date**: 2026-11-24.
* **Environment**: Linux x86_64 cloud container, 4 vCPU, Rust 1.97.0 and
  1.85.0.
* **No audio hardware, real speech or real model.**

> **Independence caveat.** This audit was performed by the same agent
> that implemented Phase 7. To compensate, it relies on reproductions and
> on gates re-run on a clean checkout, not on the implementation report's
> claims. A reviewer independent of the implementation should still
> review it.

**Recommendation: REQUIRES CORRECTIONS.** See §11.

## 1. Scope

All Phase 7 areas A–N of the audit directive:
* graph and validation;
* lifecycle;
* generations;
* failure isolation;
* scheduling;
* timing;
* bounds and backpressure;
* concurrency;
* multi-stream behaviour;
* observability;
* compatibility;
* performance;
* toolchains;
* architecture.

Out of scope: real hardware, models and devices (not available). Not
treated as defects unless a specific defect is shown.

## 2. Git state (verified)

| Item | Value |
|---|---|
| `HEAD` | `8418622` (`Phase 7: streaming orchestration and timing intelligence`) |
| `origin/deepak/gallant-hamilton-wegi2i` | `8418622` |
| `main` / `origin/main` | `8c92318` (merge of PR #12) |
| Commits ahead of main | 1 |
| Commit diff against main | 46 files, +7,096 / −67 |
| Working tree at audit start | clean |
| PRs | none opened; Phase 8 not started |

Audit additions (no implementation file changed):
* `crates/communication-engine/tests/orchestration_audit.rs`: five
  reproductions, `#[ignore]`d so the gates are unaffected;
* this report.

## 3. Methodology

1. **Code read** of every Phase 7 file:
   * contracts;
   * `ce-orchestrate`;
   * `runtime/core/pipeline.rs`;
   * the generation and stage-end edits in the four stage modules;
   * the `core.rs` loop.

   Also the relevant Phase 2–6 worker abandonment paths, and the
   Talktual bridge's event handling.
2. **Diff review** against `8c92318`:
   * `ce-core` changes are additive only (0 deleted lines);
   * the Phase 3–6 worker crates and adapters are unchanged;
   * the only new package in `Cargo.lock` is `ce-orchestrate`.
3. **Gates re-run** on a **clean worktree of `8418622`** (removed after
   use), plus MSRV, Talktual, and repeated runs of the runtime tests.
4. **Reproductions** of suspected defects with the existing harness:
   fake audio, scripted ASR, scripted translation faults, and wall-clock
   measurement where stated.

## 4. Independent verification results (observed)

| Gate | Result |
|---|---|
| `scripts/verify.sh` on a clean worktree of `8418622` | **all gates passed**, exit 0. Covers fmt, clippy, tests, rustdoc, smoke examples, the Phase 3–7 gates, cross targets (none skipped) and `cargo deny`. |
| Tests, default / all features | **399 / 403 passed, 0 failed** |
| MSRV `cargo +1.85.0 test --workspace --all-targets` | **399 passed, 0 failed** |
| `orchestration_runtime` repeated | **3/3 runs, 11/11 passed** each (this audit); the implementer reported 5 earlier |
| Talktual `tsc`, `npm test`, `npm run test:server`, `npm run build` | pass, **549**, **60**, pass |
| Talktual bridge (`adapters/talktual-bridge`) | **not compiled**: its `ce-asr-sherpa` dependency downloads native sherpa-onnx and ONNX Runtime libraries, which needs approval (see AUD-11) |

A first `verify.sh` run in the main tree stopped at `fmt` on the audit's
own unformatted reproduction file. That was not an implementation
failure. It was fixed, and the clean-worktree run above is the result
of record.

## 5. Findings

Severity scale: BLOCKER, HIGH, MEDIUM, LOW, INFORMATIONAL. "New" means
introduced by Phase 7.

### AUD-01 — HIGH — abandoned workers accumulate OS threads and forwarder tasks without a bound

* **Location**:
  * `runtime/core/pipeline.rs`, `tagged_sender` (forwarder per worker);
  * the abandoned lists in `runtime/core/{intel,speech,semantic,translation}.rs`
    (`abandoned.push`, with no cap);
  * the claims in `phase-7-report.md` §8, ADR-0031, and ledger SCH-1,
    FAI-5 and GOL-5.
* **Description**:
  * A worker whose adapter never returns is abandoned at stop or stall.
    Its OS thread cannot be killed, and it stays in `abandoned` until it
    exits.
  * Phase 7 adds one forwarder task per worker, which lives as long as
    that thread.
  * Nothing caps the number of abandoned workers. Each pipeline stop,
    restart or failure involving a hung stage adds one thread and one
    task.
  * The documentation calls this "bounded by the abandoned workers",
    which is not a bound.
* **Reproduction**: `orchestration_audit::r1_hung_workers_accumulate_threads_and_forwarders`.
  The adapter hangs 120 s per call; 8 rounds of start, feed and
  cancel-stop:

  | Round | Translation stop | `worker_forwarders` | Thread delta |
  |---|---|---|---|
  | 1 | timed_out | 1 | +1 |
  | 4 | timed_out | 4 | +4 |
  | 8 | timed_out | 8 | +8 |

* **Impact**: with a misbehaving adapter, repeated lifecycle operations
  grow threads, tasks and memory linearly. This is spec §10 and §12
  ("bound … task/thread creation", "prevent memory/task leaks") and audit
  area G. The `stop_timed_out` outcome is reported honestly; the
  resources are not bounded.
* **Correction**:
  * a per-stage (or engine-wide) cap on abandoned workers; at the cap,
    refuse new starts with a typed `Overload` error and report it;
  * expose the abandoned count in status;
  * correct the report, ADR-0031 and the ledger claims.
* **Reproducible**: yes, deterministic.
* **New**: partly. Thread retention is pre-existing (Phases 3–6
  abandonment). The forwarder tasks are new, and pipelines make the
  lifecycle easy to repeat.

### AUD-02 — MEDIUM — restart tears the running pipeline down before validating the new graph

* **Location**: `runtime/core/pipeline.rs`, `restart_pipeline`. Only
  `spec.validate()` (structural) runs before `stop_procedure`. Graph
  validation and pre-flight run afterwards, inside `start_pipeline`.
* **Description**: a restart with an invalid replacement spec stops a
  healthy pipeline and then fails. The stream is left with no pipeline.
  This violates "validate dependencies/configuration before startup"
  (spec §6, §11, §14) for the restart path.
* **Reproduction**: `r5_restart_with_an_invalid_graph_stops_the_running_pipeline_first`.
  Observed: error `orchestration.missing_dependency`; afterwards the
  pipeline is `stopped (ended: restart)` and recognition is not running.
* **Impact**: avoidable loss of a working pipeline because of a
  configuration mistake.
* **Correction**: resolve and validate the graph (and run the pre-flight
  checks that do not depend on the current stages) before stopping. On
  failure, keep the running pipeline.
* **Reproducible**: yes. **New**: yes.

### AUD-03 — MEDIUM — stale-output protection is not verified end to end; the motivating gap was not demonstrated

* **Location**:
  * generation admission in the four stage handlers and
    `pipeline.rs::admit_signal`;
  * `architecture/orchestration.md` §1 "Gaps found" item 1; ADR-0031.
* **Description**:
  * Admission is unit-tested (`epoch`) and inspected.
  * No test publishes or rejects an actual stale output through the
    runtime.
  * The race it guards against is a signal in flight while its worker
    is replaced. It is narrow, because abandoned workers already check a
    flag before every send.
  * The architecture document states the pre-Phase-7 hazard as fact. It
    was never reproduced.
* **Reproduction attempt**: `r3_attempt_stale_output_race_by_rapid_restart`.
  60 restarts while a 2 ms adapter was busy with full partials. Observed
  `stale_outputs_rejected = 0`: the race was not triggered, and the
  admission path did not fire end to end.
* **Impact**: the protection is plausible but unproven at the system
  level, and the ledger's CAU-5 PARTIAL is accurate. The documentation
  wording overstates the original hazard.
* **Correction**: add a deterministic test seam (for example, a
  test-only hook that injects a tagged signal of an old generation into
  a stage channel) to prove rejection end to end. Reword the gap as a
  latent race.
* **Reproducible**: the race, no. The evidence gap, yes. **New**: yes
  (the claim).

### AUD-04 — MEDIUM — fairness under sustained saturation is not verified end to end

* **Location**:
  * `runtime/core.rs` `next_data` and the `scheduling_tests` unit test;
  * ledger SCH-2 and EVL-4.
* **Description**: the only fairness evidence is a unit test of
  `next_data` with two channels. No test keeps a stage channel saturated
  in the running engine and measures another stage's or stream's
  progress, or command latency under that saturation.
* **Evidence collected**: `r4_command_latency_under_load`, wall clock,
  two streams, full-partial translation, a 1,500-block burst. 509 status
  round trips: **p50 0.297 ms, p95 0.538 ms, max 1.362 ms**. Both streams
  had 225 revisions and 225 derived outputs, 0 violations and 0 drops.
  This shows command responsiveness under a burst, not under saturation.
* **Impact**: the requirement is met by design (random start of the
  inner select) but is not demonstrated in the system. SCH-2 and EVL-4
  are recorded as VERIFIED.
* **Correction**: add a saturation test (one stage flooding, another
  stage or stream making progress within a bound), or re-mark SCH-2 and
  EVL-4 as PARTIAL.
* **Reproducible**: n/a (missing test). **New**: yes.

### AUD-05 — LOW — an optional stage that ends normally leaves the pipeline reported as `running`

* **Location**: `pipeline.rs::note_stage_end`. A normal end of an optional
  stage only sets `StageState::Ended`.
* **Reproduction**: `r2_optional_stage_stopped_directly_is_not_reported_as_degraded`.
  Optional translation stopped with `stop_translation`. Observed: pipeline
  `Running`, translation stage `Ended`, 0 `stage_degraded` events.
* **Impact**: misleading status. The pipeline runs without a stage while
  claiming `running`.
* **Correction**: mark the pipeline `degraded` (reason
  `orchestration.stage_ended`) when an optional stage ends while the
  pipeline is not stopping.
* **Reproducible**: yes. **New**: yes.

### AUD-06 — LOW — rollback and stop errors are discarded; rollback outcomes are not reported

* **Location**:
  * `pipeline.rs::stop_stage` (`let _ = match kind …`);
  * the rollback loop in `start_pipeline`.
* **Description**: during rollback, per-stage stop results and timeouts
  are not collected. `PipelineFailed` carries no rollback outcomes. A
  stage that hangs during rollback is visible only through `StageEnded`
  and logs.
* **Evidence**: inspection. Rollback is tested
  (`a_required_stage_failing_at_start…`), but only with stages that stop
  cleanly.
* **Correction**: collect the `StopOutcome` per rolled-back stage and
  report it, for example as a `PipelineStopped{reason: failed, outcomes}`
  after `PipelineFailed`.
* **Reproducible**: by inspection. **New**: yes.

### AUD-07 — LOW — workers cleaned up after a failed start are reported as "stale outputs"

* **Location**: the attach-failure paths in `core/intel.rs` (start: tap
  attach failure, `abandoned.push(handle)`) and `core/speech.rs`
  (likewise).
* **Description**: these workers never became current. Their `Started`,
  `Stopped` and `Ended` signals are now rejected and reported as
  `stale_output_rejected`, which misclassifies a start failure.
  Previously these signals were published.
* **Evidence**: inspection. There is no fault hook to force a tap attach
  failure.
* **Correction**: drop the signals of never-admitted generations quietly
  (counted separately), or abandon the worker before it can send.
* **Reproducible**: not reproduced. **New**: yes.

### AUD-08 — LOW — audio gaps are attributed to the recognition stage

* **Location**: `ce-orchestrate/src/observer.rs::on_audio_gap`
  (`stage: StageKind::Recognition`).
* **Description**: a gap is an audio-stream fact. It is labelled
  recognition even in pipelines without recognition (for example
  analysis only).
* **Correction**: a stream-level attribution (an `Option<StageKind>` or a
  dedicated kind), documented.
* **Reproducible**: by inspection. **New**: yes.

### AUD-09 — LOW — diagnostic events are not rate-limited

* **Location**: `pipeline.rs::report_findings`.
* **Description**: every finding becomes one event. A producer that
  persistently violates a rule, for example a duration mismatch on
  every revision, emits one diagnostic per stage event onto the
  broadcast bus. The volume is bounded by the producer's event rate, not
  by a diagnostic budget, and can push subscribers into lag. Stale
  outputs are de-duplicated per generation; findings are not.
* **Evidence**: inspection; not reproduced. The real producers emitted
  zero findings in all runs.
* **Correction**: coalesce repeated findings per (stream, stage, kind)
  within a window, with counters carrying the totals.
* **New**: yes.

### AUD-10 — LOW — the ~150 ns overhead figure is not representative of runtime orchestration cost

* **Location**:
  * `ce-orchestrate/examples/observer_overhead.rs`;
  * `phase-7-evaluation.md` §3; report §7.
* **Methodology issues**:
  1. The timed loop includes building the inputs: string allocations in
     `revision_for`, `LanguageTag::parse` and the result structures. The
     observer's own share is unknown.
  2. The runtime wrapper is excluded: two monotonic clock reads, a map
     lookup, two `Instant` reads, and the findings `Vec`.
  3. Event dispatch and publication are excluded.
  4. One workload shape, with no findings path.
  5. Single-threaded, warm caches, one container.
  6. The gate prints but does not enforce a threshold, so a regression
     would pass.
* **Results reproduced**: 149 ns mean, SD 1 ns (clean-worktree gate run),
  consistent with the report.
* **Impact**: the number is a valid measurement of *that loop*, labelled
  an upper bound for the observer. It must not be read as
  per-event orchestration overhead in the engine. It is not a real-time
  claim.
* **Correction**: measure construction separately and subtract it; report
  the runtime `overhead` reservoir from a wall-clock runtime run; add a
  regression threshold.
* **Reproducible**: yes. **New**: yes.

### AUD-11 — INFORMATIONAL — Talktual integration is not exercised against Phase 7

* **Location**: `adapters/talktual-bridge`; Talktual
  `src/lib/ceSpeech.ts` and its tests.
* **Description**:
  * The bridge could not be compiled: native sherpa-onnx download needs
    approval.
  * By inspection, it matches event payloads with patterns and
    fallbacks, does not construct `MetricsSnapshot` or `RuntimeHealth`,
    and does not use pipelines, so the additive enum variants and fields
    should compile.
  * Talktual's tests mock the bridge socket and do not run the CE
    runtime.
  * Phase 7 changes cross-channel event interleaving (ADR-0033). The
    bridge combines speech and semantic `Stopped` with flags (order
    independent), but activity and transcript interleaving may differ
    from before.
* **Correction**: with approval, build the bridge and run its tests
  against `8418622`.
* **New**: n/a (verification gap).

### AUD-12 — INFORMATIONAL — cross-channel interleaving is nondeterministic run to run

* `next_data` uses a random start, by design (ADR-0033). Runtime tests
  rely on causal predicates and passed 3 + 5 repeated runs.
* Bit-exact replay determinism exists for the pure observer only
  (`state_is_bounded_and_replay_is_deterministic`).
* Acceptable, but tests asserting cross-channel order would be flaky.
  None was found.

### AUD-13 — INFORMATIONAL — drain uses one shared deadline across stages

* `pipeline.rs::stop_procedure` computes a single deadline.
* A slow recognition stop consumes the drain budget of the downstream
  stages, which are then reported `timed_out`. The bound is respected,
  but the attribution can mislead.

### AUD-14 — LOW — the module-exit arm drains ingress with an unbounded loop

* **Location**: `runtime/core.rs::handle_data`,
  `DataMsg::Exit` (`while let Ok(…) = self.ingress_rx.try_recv()`).
* **Description**: if module producers keep the ingress channel
  non-empty, the loop can hold the control task. This is theoretical:
  producers share the runtime, and ingress is bounded.
* **Correction**: drain at most the number of messages present at entry.
* **Evidence**: inspection. **New**: yes.

### AUD-15 — LOW — ledger and report claims inconsistent with findings

* **SCH-1** ("bound … tasks and threads") and **FAI-5** ("prevent
  memory/task leaks"), both VERIFIED, and report §8 "forwarders return to
  0 after repeated lifecycles (tested)": true only for workers that exit.
  Contradicted by AUD-01.
* **SCH-2** and **EVL-4** VERIFIED: AUD-04.
* **CAU-5** PARTIAL and **CMP-4** PARTIAL: accurate.
* **Correction**: amend them with the audit corrections.

## 6. Reproductions summary (observed output)

| Id | Test (`orchestration_audit.rs`, `--ignored`) | Observed |
|---|---|---|
| R1 | `r1_hung_workers_accumulate_threads_and_forwarders` | forwarders 1→8, threads +1→+8 over 8 rounds; translation stop `timed_out` each round |
| R2 | `r2_optional_stage_stopped_directly…` | pipeline `Running`, stage `Ended`, 0 degraded events |
| R3 | `r3_attempt_stale_output_race_by_rapid_restart` | 60 restarts, 61 pipelines started, **0 stale outputs**: race not reproduced |
| R4 | `r4_command_latency_under_load` | status round trip p50 0.297 ms, p95 0.538 ms, max 1.362 ms; 0 violations, anomalies or drops on either stream |
| R5 | `r5_restart_with_an_invalid_graph…` | `orchestration.missing_dependency`; pipeline left `Stopped (restart)`; recognition stopped |

## 7. Area-by-area assessment

| Area | Assessment |
|---|---|
| A. Graph and validation | Rules correct. Deterministic order; cycle detection tested on synthetic graphs (the fixed kinds cannot cycle). Errors are typed and actionable. Pre-flight covers stream, adapters, target, already-running stages and capacity. Gaps: restart validates late (AUD-02); worker capacity is caught only at start, but rollback covers it (tested). |
| B. Lifecycle | Start order, rollback, drain and cancel, reset, restart and shutdown retirement are implemented and tested. Gaps: rollback outcomes (AUD-06); restart (AUD-02); leaks with hung workers (AUD-01). |
| C. Generations | Engine-wide monotonic generation per worker start; restart gives new generations (tested); reset keeps workers (correct). Stale rejection is not proven end to end (AUD-03); start-failure misclassification (AUD-07). |
| D. Failure isolation | Optional failures degrade at start and at run time; required failures stop with cancel; recovery by restart; two-stream isolation (tested). Downstream stages of a failed recognition get no further input; Phase 5–6 withdrawal semantics apply. Gap: AUD-05. |
| E. Scheduling | Command priority with an unbiased data select is sound by design. Responsiveness under a burst was measured (R4). Saturation was not tested (AUD-04). Nondeterminism: AUD-12. |
| F. Timing | Clock domains are documented; media time is exact; reported times are never modified (payloads untouched, verified by code read and by the end-to-end translation-text test). Real producers (Phase 3 analyzer on fixtures; Phase 4–6 workers in the runtime) gave 0 false positives. Injected violations are detected. Deadlines are typed. Latency is real only for observer overhead; the runtime uses a manual clock. Gap: AUD-08. |
| G. Bounds | Observer tables, reservoirs, pipelines, the finished list, stale de-duplication and forwarder channels (64) are bounded, and backpressure is preserved through the forwarder. **Abandoned workers are unbounded (AUD-01).** Diagnostics are not rate-limited (AUD-09). |
| H. Concurrency | A single control task serializes commands. Pending finishes are epoch-checked (restarted or stopped pipelines skip them). No locks were added. No race found beyond AUD-03's latent one. |
| I. Multi-stream | State is keyed per stream; generations are admitted per (stage, stream); one stream's failure is isolated (tested). Resource accounting is engine-wide (`max_pipelines`); abandoned workers are not accounted (AUD-01). |
| J. Observability | Status, stage states, epochs, generations, outcomes and counters are exposed. Misleading cases: AUD-05, AUD-07, AUD-08. Queue occupancy of the forwarders is not exposed (only the forwarder count). |
| K. Compatibility | Additive contracts. Two intended behaviour changes (stale drop, fair select) are documented. Phase 1–6 tests and gates pass. Translation `Started` and sequence semantics are unchanged and checked. The bridge was not compiled (AUD-11). |
| L. Performance | §8. |
| M. Toolchains | MSRV 1.85 tests pass; fmt, clippy (`-D warnings`), rustdoc, cross targets (iOS, Android, Windows MSVC, macOS, wasm32 for `ce-orchestrate`) and `cargo deny` pass. Platform-sensitive new code: `std::time::Instant` in the runtime observer wrapper (runtime only; not compiled for wasm32); `/proc` is used only in the audit test. None found to behave differently. |
| N. Architecture | §10. |

## 8. Performance evidence assessment

| Claim | Kind | Assessment |
|---|---|---|
| Observer ~149–152 ns per event | measured, real clock, synthetic replay | valid for the example loop only; see AUD-10 |
| Runtime latency summaries | fake (manual) clock | not real-time; no claim may be drawn |
| Deadline miss at 2–3 s | fake clock, scripted | verifies the mechanism, not latency |
| Command round trip p95 0.54 ms under a two-stream burst (this audit) | measured, real clock, fake audio, scripted adapters | indicative of control-plane responsiveness in this container only |
| End-to-end latency, sustained operation, memory over time | **untested** | — |

## 9. Resource safety assessment

* **Bounded**:
  * forwarder channel capacity (64; backpressure preserved);
  * per-stage worker queues (unchanged);
  * `max_pipelines`;
  * observer state;
  * reservoirs;
  * the finished pipeline list (16);
  * stale de-duplication (64).
* **Unbounded**:
  * **abandoned threads and forwarder tasks under repeated hangs**
    (AUD-01, reproduced);
  * diagnostic event volume tied to the producer rate (AUD-09).
* **Not observed**: any other growth path. Normal workers' forwarders
  returned to 0 (implementation test; R4).

## 10. Architecture assessment

The pipeline abstraction is a sound foundation:
* a pure, wasm-checkable logic crate kept separate from runtime effects;
* contract-derived edges;
* explicit required and optional semantics;
* epochs and generations;
* an annotate-don't-mutate observer;
* fair scheduling behind command priority.

The fixed set of stage kinds is deliberate (ADR-0030). It will need
extending, but that is not a blocker.

Concrete limitations relevant to later phases:
* the abandoned-worker bound (AUD-01) must exist before longer-running
  or multi-session deployments;
* restart should be transactional (AUD-02);
* pipelines cannot take a custom detector;
* mixing direct per-stage commands with pipelines has edge cases
  (AUD-05).

No speculative refactoring is recommended.

## 11. Risk assessment and recommendation

| Severity | Findings |
|---|---|
| BLOCKER | none |
| HIGH | AUD-01 |
| MEDIUM | AUD-02, AUD-03, AUD-04 |
| LOW | AUD-05, AUD-06, AUD-07, AUD-08, AUD-09, AUD-10, AUD-14, AUD-15 |
| INFORMATIONAL | AUD-11, AUD-12, AUD-13 |

**Recommendation: REQUIRES CORRECTIONS.**

All gates pass and the core design is sound. However:
* a reproducible unbounded thread and task growth path (AUD-01)
  contradicts the phase's resource claims;
* restart is not validated before teardown (AUD-02);
* two required verifications, end-to-end stale rejection and saturation
  fairness, are missing (AUD-03, AUD-04).

**Minimum corrections for acceptance**:
* AUD-01: cap and report abandoned workers;
* AUD-02: validate before teardown;
* AUD-03: a test seam proving rejection end to end, plus a wording fix;
* AUD-04: a saturation test, or re-mark as PARTIAL;
* AUD-15: ledger and report corrections.

The LOW items can be fixed or explicitly dispositioned. Hardware, model
and real-clock latency evidence remain open limitations, not defects.

## 12. Limitations of this audit

* Self-audit (see the caveat at the top).
* The bridge was not compiled.
* No hardware, real speech or model.
* The race in AUD-03 was attempted but not forced.
* Saturation was not reproduced.
* Single container.
