# Phase 7 report — Streaming Orchestration & Timing Intelligence

* **Branch**: `deepak/gallant-hamilton-wegi2i`.
* **Baseline**: `8c92318` (main with Phase 6 closed, PR #12).
* **Date**: 2026-11-24.
* **Environment**: Linux x86_64 cloud container, 4 vCPU, Rust 1.97.0;
  MSRV 1.85.0 also run.
* **No audio hardware, GPU, real speech or real model.**
* **Status: implemented; independent audit and closure pending (spec
  §23–24). Not a production certification (Phase 15).**

Related: [ledger](phase-7-ledger.md) · [evaluation](phase-7-evaluation.md)
· [timing and configuration](phase-7-timing.md) ·
[architecture](../architecture/orchestration.md) · ADR-0030 to ADR-0033 ·
[handoff](phase-7-handoff.md).

## 1. Baseline and repository state

* **Phase 6 closure confirmed**: PR #12 merged; `main` = `origin/main` =
  `8c92318`, which contains the audit fixes D1–D5. The working branch
  equalled main, the tree was clean, and there were no unexplained
  changes. Phase 6 semantics were confirmed from its handoff and code:
  translation `Started` resets consumer state, and sequences are
  worker-local and restart at 0 in a new run.
* **Baseline gates**, run on a separate clean worktree of `8c92318`, so
  no Phase 7 edit could contaminate them:
  * `scripts/verify.sh`: **all gates passed**;
  * tests: **361 default / 365 all features, 0 failed**.

  Talktual on the same state, from the Phase 6 closure run: tsc pass, 549
  unit tests, 60 server tests, build pass.
* `../docs/phase7*` belong to Talktual's own UI phases, which are
  unrelated to CE Phase 7.
* No pre-existing failure was found.

## 2. Reconnaissance findings (summary; details in `architecture/orchestration.md` §1)

* One control-plane task owns all state and the global event `sequence`.
* Stage workers (analysis, recognition, semantics, translation) run on
  supervised threads. Each stage type has one shared signal channel.
* Stage coupling was implicit: data was forwarded to whatever worker was
  running.
* Timing contracts were already rich, and are reused:
  * the sample clock is authoritative;
  * `time_range` is anchor-derived monotonic time;
  * `host_time` and `device_time` are separate;
  * discontinuities are explicit.
* **Gaps addressed**:
  1. Worker signals carried no generation. A stale `Ended` or event from
     a replaced or abandoned worker could be attributed to its successor:
     the core could remove the new worker's entry, or publish after a
     synthesized `Stopped`.
  2. No explicit or validated stage graph, and no defined
     optional/required failure semantics.
  3. No cross-stage causal or timeline checks.
  4. No end-to-end latency or deadlines.
  5. The control loop's `biased` select could starve later data arms.

## 3. Implementation summary

* **Contracts** (`ce-core::orchestration`, additive; 0 deletions in
  `ce-core`):
  * `PipelineSpec`, `StageSpec`, `StageConfig`, `Requirement`,
    `StopMode`, `DeadlinePolicy` and `OrchestrationSettings` (validated);
  * `PipelineState` (validated transitions), `StageState`, `StageEnd`,
    `StopOutcome`, `FaultClass` (spec §12 classes),
    `CausalKind`, `TimelineKind`, `TimingUncertainty`, `TimelineStatus`,
    `LatencyMetric` and `LatencySummary`;
  * `OrchestrationCounters`, `PipelineStatus` and `OrchestrationEvent`
    (11 kinds, all `Operational`);
  * commands `Start/Stop/Reset/Restart/QueryPipeline`; `RuntimeHealth.pipelines`;
    seven `MetricsSnapshot` counters, including the `worker_forwarders`
    gauge.
* **`ce-orchestrate`** (new, pure, ce-core only; checks on wasm32):
  * `graph`: dependency rules, generic Kahn toposort with cycle
    detection, required/optional rules, notes, fingerprint and stop
    order;
  * `epoch`: generations and admission;
  * `reconcile`: bounded revision table and source classification;
  * `timeline`: exact media time, offset tracking, re-anchor, unknown
    timing, integrity, ordering and gaps;
  * `latency`: reservoirs and typed deadlines;
  * `observer`, which combines the above.
* **Runtime**:
  * per-worker tagged channels with forwarders, and generation admission
    in all four stage handlers;
  * stage-end tracking on normal, failed, stalled and stop-timeout paths;
  * pipelines: pre-flight (stream, adapters, target, running stages,
    capacity); dependency-ordered start with rollback; optional
    degradation, including dependents; required failure stops the
    pipeline with cancel; `drain`/`cancel` stop with per-stage outcomes;
    reset; restart with a new epoch; source-end retirement;
    session-close and shutdown retirement;
  * observer hooks, with findings published after the stage event; audio
    gap hook;
  * an unbiased data select behind command priority, with ingress drained
    before reaping an exited module.
* **Facade**: `start_pipeline`, `stop_pipeline`, `reset_pipeline`,
  `restart_pipeline` and `pipeline_status`; prelude exports.
* **Not changed**:
  * the Phase 3–6 worker crates and adapters (no diff);
  * the Talktual bridge;
  * existing per-stage commands.

## 4. Architecture decisions

ADR-0030 (pipelines and graph), ADR-0031 (epochs and stale rejection),
ADR-0032 (timeline, causality, latency) and ADR-0033 (scheduling). There
are **two intended behaviour changes to existing paths**, both documented:
* outputs of a replaced or abandoned worker are no longer published;
* data arms of the control loop are polled without bias.

No approval-requiring decision arose: all contract changes are additive,
there are no new third-party dependencies, and no models.

## 5. Requirements ledger

`phase-7-ledger.md` has a row for every bullet of the specification.

| Status | Rows |
|---|---|
| PARTIAL | CAU-5 (the race is not reproduced end to end), CMP-4 (no adapter contract-version check), EVL-6 and TST-6 (no new saturation or burst test of the forwarder hop), EVL-7 and TST-10 (no long wall-clock run), EVL-9 (no real-clock end-to-end, real model or hardware), DOC-12 (audit and closure pending) |
| PENDING | ACC-18: independent audit and closure |
| VERIFIED | all others, with the evidence named per row |

## 6. Tests and exact results (all executed; output observed)

| Gate | Command | Result |
|---|---|---|
| All CE gates | `scripts/verify.sh` | **all gates passed** (exit 0). Covers fmt, clippy (all features, `-D warnings`), tests, rustdoc (`-D warnings`), smoke examples, the Phase 3–6 replay and evaluation gates, the new orchestration gate, cross-target checks (now including `ce-orchestrate`, plus wasm32) and `cargo deny`. `cargo deny` printed only duplicate-crate warnings, as at baseline. |
| Tests, default features | `verify.sh` step | **399 passed, 0 failed** (baseline 361) |
| Tests, all features | `verify.sh` step | **403 passed, 0 failed** (baseline 365) |
| MSRV | `cargo +1.85.0 test --workspace --all-targets` | **399 passed, 0 failed** |
| Talktual | `npx tsc --noEmit`, `npm test`, `npm run test:server`, `npm run build` | pass; **549** passed; **60** passed; pass. CE is used by Talktual only through the unchanged bridge, so Talktual is not affected; these were run anyway. |

**New tests (38)**:
* ce-core orchestration: 4;
* `ce-orchestrate` unit tests: 15;
* replay `tests/replay.rs`: 7, using the real Phase 6 processor and the
  real Phase 3 analyzer on its dev fixtures;
* runtime `tests/orchestration_runtime.rs`: 11;
* fairness `core.rs`: 1.

**Stability**: the orchestration runtime tests (9 at that time) and the
Phase 6 runtime tests were run **5× consecutively without failure**. The
later two runtime tests ran in every subsequent gate.

**Failures found and fixed during the phase** (none pre-existing):
* **Wrong rule**: a recognition "soft input" note duplicated a hard
  dependency. It was removed.
* **Unmet MSRV**: `div_ceil` needs a newer toolchain; it was replaced.
* **Stop outcomes**: stages that ended on their own during a drain were
  reported `not_running`. A snapshot of running stages at the start of
  the stop now makes them `clean`.
* **Wrong drain order**: drain stopped analysis first. Analysis, the hint
  provider, is now stopped last.
* **Test expectations, wrong**:
  * manual-clock latency arithmetic;
  * a final translation never has sequence 0 under full partials;
  * the recorder only sees new events.
* **Gates**: clippy findings (`print_stdout`, `explicit_counter_loop`,
  `drain_collect`, `should_implement_trait`, `while_let_loop`).
* **Pre-flight gap**: a missing translation target was caught only at
  start. Pre-flight now catches it.

## 7. Evaluation and metrics

See `phase-7-evaluation.md`. Highlights:
* Real Phase 4–6 producers in the runtime and the real Phase 3 analyzer
  on fixtures produced **0 causal violations and 0 timeline anomalies**.
  This is the false-positive control.
* Injected violations of every kind are reported.
* **Observer overhead**: about **149–152 ns per event**, an upper bound
  that includes input construction. Measured in release on the real
  clock over 4 invocations of 5 runs each; the final gate run gave 149 ns
  mean with SD 1 ns.
* End-to-end latency was **not** measured on a real clock, model or
  device.

## 8. Performance and resources

| Resource | Bound |
|---|---|
| Per-worker forwarding channel | 64 |
| Forwarders | one per worker; they return to 0 after repeated lifecycles (tested) |
| Pipelines | at most `max_pipelines` (8 by default) |
| Finished pipelines remembered | 16 |
| Observer | `observer_utterances` (256 by default) |
| Reservoirs | 512 samples each |
| Stale-report de-duplication | 64 entries |

Existing per-stage queues and policies are unchanged. No RTF, memory or
latency figures exist for real models or devices.

## 9. Known limitations and risks (risk register, spec §25)

1. All evidence is synthetic, scripted or replayed. Real speech, models
   and devices are untested. Cross-platform evidence is compile-only.
2. Timestamp normalization is not synchronization. The mapping
   uncertainty is reported as `unknown`.
3. Low observer overhead does not prove end-to-end latency.
4. **The stale-generation race is logic-tested, not reproduced end to
   end.** Abandoned workers already suppress most late sends; admission
   covers the remaining window.
5. A hung, abandoned worker keeps its forwarder task until its thread
   exits. This is bounded by the abandoned workers and visible in the
   gauge.
6. A required stage stopped directly through per-stage commands ends the
   pipeline. Mixing direct control with pipelines is discouraged.
7. Pipelines cannot take a custom voice-activity detector.
8. Adapter contract versions are not compared.
9. Speech give-up (stall or load timeout) does not end downstream
   workers by itself (pre-existing, Phase 4). In a pipeline, the required
   failure stops them.
10. Phase 15 remains the certification gate.

## 10. Scope and stop conditions

* No models, datasets, adapters or algorithms added.
* No Phase 8–11, UI, SDK, persistence, hosted API, billing or deployment.
* No external transmission and no paid services.
* No destructive git: the baseline used a scratch worktree, removed
  after use.
* **Phase 8 not started.**

## 11. Git

Commit on `deepak/gallant-hamilton-wegi2i` on top of `8c92318`. It ends
with "Built by Deepak and Claude". No PR has been opened: PRs are opened
only on request, after the audit.

## 12. Recommendation

**Ready for the independent audit.** Phase 7's engine-side orchestration
is implemented and verified with synthetic and replay evidence:
* graph validation;
* generations and epochs;
* lifecycle, degraded and failed operation;
* causal and timeline checks;
* latency and deadlines;
* fair scheduling;
* bounds and privacy.

The PARTIAL items in §5 should be weighed by the audit. Acceptance is
the user's decision after the audit and closure.
