# Phase 7 handoff

State after the Phase 7 implementation, its audit (`phase-7-audit.md`:
REQUIRES CORRECTIONS) and the authorized corrections (audit §13).
**The verification audit and closure have not happened yet.**

Read this with:
* `phase-7-report.md` (status and evidence);
* `phase-7-ledger.md`;
* `docs/architecture/orchestration.md`;
* `phase-7-timing.md`.

## Standing user rules

* End commit messages and PR bodies with **`Built by Deepak and Claude`**.
  Never put session links or model names in commits, PRs, code or docs.
* Push only to `deepak/gallant-hamilton-wegi2i`. Open a PR only when asked.
* No model or dataset downloads, paid APIs, credentials, deployments,
  external transmission of audio or transcripts, or destructive git
  without approval.
* Do not start Phase 8 until Phase 7 is audited and closed.

## Where things are

* Contracts: `crates/ce-core/src/orchestration/` (`PipelineSpec`,
  `StageSpec`, `PipelineStatus`, `OrchestrationEvent` and the rest).
  `EngineConfig.orchestration`; `MetricsSnapshot` counters;
  `RuntimeHealth.pipelines`.
* Pure logic: `crates/ce-orchestrate/`:
  * modules `graph`, `epoch`, `reconcile`, `timeline`, `latency` and
    `observer`;
  * replay tests in `tests/replay.rs` (including the real Phase 3
    analyzer);
  * example `observer_overhead`.
* Runtime:
  * `crates/communication-engine/src/runtime/core/pipeline.rs`:
    generation tagging, stale admission, pipelines and observer hooks;
  * generation admission and stage-end notes in `core/{intel,speech,semantic,translation}.rs`;
  * fair data select (`next_data`) in `core.rs`;
  * audio gap hook in `core/audio.rs`.
* Facade: `start_pipeline`, `stop_pipeline`, `reset_pipeline`,
  `restart_pipeline` and `pipeline_status`. The commands are
  `StartPipeline`, `StopPipeline`, `ResetPipeline`, `RestartPipeline` and
  `QueryPipeline`.
* Tests:
  * `crates/communication-engine/tests/orchestration_runtime.rs`;
  * `orchestration_corrections.rs`: the audit corrections;
  * `stale_output_runtime.rs`: needs `--features test-hooks`;
  * `orchestration_audit.rs`: `#[ignore]`d reproductions; run with
    `--ignored --nocapture`;
  * the fairness unit test in `core.rs` (`scheduling_tests`) and the
    forwarder test in `core/pipeline.rs`.
* Test-only feature `test-hooks`: retains translation worker senders for
  fault injection. Never enable it in products.
* Decisions: ADR-0030 to ADR-0033.

## Rules for consumers and the next phases

* `EventPayload::Orchestration` events are `Operational` and never carry
  content.
* **Worker generations**: an output of a replaced, stopped or abandoned
  worker is never published (`stale_output_rejected`). Consumers can rely
  on every published stage event belonging to the current worker of that
  stage and stream.
* **Translation (Phase 6 rules, unchanged)**: `Started` resets consumer
  state. Sequences are worker-local and restart at 0 in a new run (for
  example after a pipeline restart). Keep the highest sequence per
  utterance *within a run*.
* **Pipelines**:
  * the epoch is engine-wide monotonic;
  * `reset_epoch` counts resets within an epoch;
  * a restart is a new epoch with new generations;
  * utterance numbers restart at 0 with a new recognition worker (that
    is, on restart) and continue across a reset.
* **Findings** (`causal_violation`, `timeline`, `deadline_missed`) are
  published after the stage event they concern. They annotate; they do
  not change or retract the stage event.
* **A required stage stopped directly** (for example
  `stop_recognition` on a pipeline stream) ends the pipeline. An optional
  stage's failure degrades it.
* **Stop modes**:
  * `drain` stops recognition first and lets semantics and translation
    finish what they have (bounded by `audio.operation_timeout_ms`), with
    analysis last;
  * `cancel` stops downstream first.
* **Do not** put pipeline-managed stages under direct per-stage control
  and expect the pipeline not to react. Stopping an optional stage
  directly degrades the pipeline; stopping a required one ends it.
* **Abandoned-worker cap**: at `orchestration.max_abandoned_workers`
  live abandoned threads, every new stage worker start is refused
  (`orchestration.abandoned_workers_exhausted`) until hung threads exit.
  Watch the `abandoned_workers` gauge.
* **Restart is checked first**: an invalid replacement leaves the
  running pipeline untouched.
* **`timeline` events**: `stage` is optional (`null` for audio gaps).
  Findings are limited to 32 per kind and stage per epoch.

## Open items (need a decision or later work)

* A verification audit of the corrections, then closure (spec §23–24).
* **Still PARTIAL after the corrections**:
  * scheduler fairness under core-channel saturation (SCH-2, EVL-4,
    EVL-6);
  * the natural stale-output race, which the test hook substitutes for
    (CAU-5 scope);
  * AUD-07 and AUD-14 are corrected in code but have no dedicated test.
* The real MT adapter is still blocked (Phase 6). Pipelines carry it
  unchanged when it arrives.
* An audio-layer bound for timing uncertainty (currently `unknown`).
* Adapter contract-version compatibility checks (CMP-4 PARTIAL).
* An end-to-end reproduction of the stale-generation race is logic-tested
  only (see the report).
* Latency on real hardware, with a real model and the system clock, has
  not been measured (EVL-9 PARTIAL).
* The pipeline facade cannot take a custom voice-activity detector
  (`start_analysis_with`); pipelines use the configured detector.

## Verification

```bash
cd communication-engine && scripts/verify.sh
cargo +1.85.0 test --workspace --all-targets
cargo +1.85.0 test -p communication-engine --features test-hooks --test stale_output_runtime
cargo test -p communication-engine --test orchestration_audit -- --ignored --nocapture
cargo run --release -p ce-orchestrate --example observer_overhead
cd .. && npx tsc --noEmit && npm test && npm run test:server && npm run build
```
