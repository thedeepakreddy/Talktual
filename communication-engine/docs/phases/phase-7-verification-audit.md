# Phase 7 verification audit of the authorized corrections

* **Audited commit**: `3b856eb` (`Phase 7 audit corrections`) on
  `deepak/gallant-hamilton-wegi2i`.
* **History**: `8c92318` (main) → `8418622` (implementation) → `cf250b8`
  (audit) → `3b856eb` (corrections). Linear; verified with
  `git merge-base --is-ancestor`.
* **Date**: 2026-11-25. **Environment**: Linux x86_64 cloud container,
  Rust 1.97.0 and 1.85.0. No hardware, real speech or real model.

> **Independence: limited.** This audit was performed by the same agent
> that wrote the Phase 7 implementation, the original audit and the
> corrections. As compensation:
> * every gate was run on a **clean worktree of `3b856eb`**;
> * every finding below was **reproduced or inspected in the code**, not
>   taken from the correction report;
> * two new reproductions (VA-R6, VA-R7) were written specifically to
>   test the corrections' claims.
>
> A reviewer who did not write these changes should still confirm the
> verdict.

**Verdict: REQUIRES CORRECTIONS.** See §6. The two required corrections
are small and well scoped.

## 1. Git state and integrity

| Item | Value |
|---|---|
| `HEAD` = `origin/deepak/gallant-hamilton-wegi2i` | `3b856eb2a674adc6abeb7e20101ce671c4c2f269` |
| `origin/main` | `8c92318` |
| Commits ahead of main | 3 (linear) |
| Correction diff (`cf250b8..3b856eb`) | 28 files, +2,063 / −177 |
| Diff outside `communication-engine/` since main | none (Talktual sources unchanged) |
| Working tree at audit start | clean |

Audit additions:
* this report;
* two `#[ignore]`d reproductions (VA-R6, VA-R7) in
  `tests/orchestration_audit.rs`.

No implementation file was changed.

## 2. Gates on a clean worktree of `3b856eb` (all executed; output observed)

| Gate | Result |
|---|---|
| `scripts/verify.sh` | **all gates passed**, exit 0. Covers fmt, clippy (all features, `-D warnings`), tests, rustdoc (`-D warnings`), smoke examples, the Phase 3–7 gates including the observer-overhead threshold, cross targets (iOS, Android, Windows MSVC, macOS, wasm32; **none skipped**) and `cargo deny` (duplicate-crate warnings only, as at baseline). |
| Tests, default features | **409 passed, 0 failed** |
| Tests, all features (includes `test-hooks`) | **415 passed, 0 failed** |
| MSRV 1.85.0 `--workspace --all-targets` | **409 passed, 0 failed** |
| MSRV 1.85.0 `--features test-hooks --test stale_output_runtime` | **2 passed** |
| Repeated: `orchestration_runtime` (11), `orchestration_corrections` (7), `stale_output_runtime` (2) | **5/5 runs, all passed** |
| Audit reproductions R1–R5 (`--ignored`) | 5/5 passed. R1: growth 1→16, then 4 refusals. R2: `Degraded`. R3: 0 stale outputs in 60 restarts. R4: status p50 0.27 ms, p95 0.41 ms (debug build). R5: pipeline kept `Running`. |
| `observer_overhead` (release) ×3 | means **116, 118, 120 ns/event** (pre-built inputs); construction 363–395 ns/event (excluded) |
| Talktual (main tree; its Talktual files are identical to `3b856eb`) | `tsc` pass; `npm test` **549 passed**; `npm run test:server` **60 passed**; `npm run build` pass |
| Talktual in the clean worktree | **not valid as evidence**. `tsc` ran before dependency install, and `test:server` failed with `Cannot find module 'express'` because `server/` dependencies are not installed in a fresh worktree. Environment artefacts, not regressions; the main tree is the result of record. |
| Talktual bridge compile | **not run**: needs an unapproved native sherpa-onnx download (AUD-11) |

## 3. Evidence matrix

| ID (original severity) | Method | Evidence | Result | Residual risk | Closure impact |
|---|---|---|---|---|---|
| **AUD-01 (HIGH)**: abandoned-worker bound | Code read; tests `aud01_*` ×3; R1; **new VA-R7** | **Correct for recognition, semantics and translation**: `abandon()` keeps the thread handle, the core pushes the handle to `abandoned`, and the count is `!is_finished()`, so it is exact. Admission is on the single control task, so concurrent requests are serialized, and the count can only fall as threads exit, so there is no admission race. The cap is checked in every stage's start and in pipeline pre-flight. R1, the concurrency test (12 simultaneous starts refused, no growth) and recovery all reproduce. **Not correct for analysis**: see VA-01. | **VERIFIED PARTIAL** | Hung analysis workers are neither counted nor capped. | **Blocks closure** (VA-01) |
| **AUD-02 (MEDIUM)**: restart validated first | Code read (`prepare_pipeline` before `stop_procedure`); tests `aud02_*` ×2; R5 | Six invalid classes: pipeline `Running`, same epoch, same generations, no `pipeline_stopped`. A valid restart works. A valid replacement that fails at start rolls back with a reported outcome. A cycle cannot be built from the fixed stage kinds (unit-tested on synthetic graphs). Capacity: the abandoned-cap refusal preserves the pipeline (`aud01_restart…`). | **VERIFIED CORRECTED** | The replacement is re-validated after the stop, because the stream or adapters can change during the stop's awaits. A failure there leaves the pipeline stopped and is reported (inherent; documented). | none |
| **AUD-03 (MEDIUM)**: stale rejection end to end | Code read of the hook; `stale_output_runtime` ×2 | Feature `test-hooks` is **off by default** (`default = []`). Retention also needs a runtime opt-in. The injected sender is the clone of the sender handed to `spawn_translation` for that generation. The tests show: positive control published; reset keeps the generation and is published; after restart the old generation is rejected (`current = new`) and never published; 20 mixed stale signals all rejected while the new worker publishes; an abandoned worker's signal is rejected (`current = none`). Synchronization is by events and counters (one bounded 2 ms poll on a counter). | **VERIFIED PARTIAL**: proves admission and rejection across the real forwarder, core channel and publication boundary. It does **not** prove a worker thread itself emitting late during a real restart (that race is unforced; R3: 0 in 60 restarts). | The natural race is unforced. The mechanism is the same path, so the residual risk is low. | Acceptable as a documented limitation |
| **AUD-04 (MEDIUM)**: saturation | `aud04_…`; `tagged_sender_is_bounded…` | Blocked stage under two-stream load: backlog in (0, 64], 0 inputs refused, coalescing confirmed after release, finals translated, the other stream translated, status < 1 s, forwarder count stable, clean shutdown. Forwarder: accepted ≤ 64 + 2, then full; delivered in order and tagged; ends with its sender. | **VERIFIED PARTIAL**: core-channel saturation and main-loop fairness under it are unit-level only (`next_data`). | A starvation bug that only appears under real core-channel saturation would not be caught. | Acceptable as a documented deferral (the ledger keeps it PARTIAL) |
| AUD-05 (LOW) | `aud05_…`; R2 | Direct stop of an optional stage gives `Degraded` with code `orchestration.stage_ended`; a `source_ended` end does not degrade. | **VERIFIED CORRECTED** | — | none |
| AUD-06 (LOW) | `aud02_aud06_…`; code | Per-stage rollback outcome set and published (`pipeline_stopped{failed}`). | **VERIFIED CORRECTED** | Only a clean rollback is tested; a timed-out rollback is inspected. | none |
| AUD-07 (LOW) | Code read (`discard_generation` at both attach-failure sites; checked before the stale report) | Logic is correct by inspection. | **NOT VERIFIED BY TEST** (inspection only; no fault hook) | Low | none |
| AUD-08 (LOW) | Replay `audio_gaps_are_stream_level`; code | Gaps have `stage: None`. | **VERIFIED CORRECTED** | — | none |
| AUD-09 (LOW) | Replay `repeated_findings…`; **new VA-R6** | Events are limited to 32 per kind and stage per epoch, and the pipeline counters stay complete. **But the engine-wide `MetricsSnapshot` counters now undercount** (VA-02). | **VERIFIED CORRECTED, with a REGRESSION** (VA-02) | Under-reported engine metrics | **Blocks closure** (VA-02) |
| AUD-10 (LOW) | Example code read; reproduced ×3 | The isolated observer (inputs pre-built) is 116–120 ns/event. Construction is timed apart. The runtime figure (the `overhead` reservoir) is a different quantity: release p50 1.0–1.2 µs, p95 2.6–3.0 µs (correction report; R4 here in debug: p50 3.7 µs, p95 8.8 µs). The gate threshold (mean < 5 µs) is a coarse guard and **does not cover the runtime figure**. | **VERIFIED CORRECTED** (methodology) | The regression guard covers only the isolated loop. | none |
| AUD-11 (INFO) | Docs | The bridge was not compiled (approval). Documented consistently. | **OPEN / ACCEPTED LIMITATION** | Additive enum variants and fields could still break a downstream exhaustive match; by inspection the bridge has none. | Not a blocker (approval matter) |
| AUD-12, AUD-13 (INFO) | Docs | Documented. | **ACCEPTED LIMITATION** | — | none |
| AUD-14 (LOW) | Code read (`for _ in 0..ingress_rx.len()`) | Bounded by the messages queued at entry. The drain does not wait on modules: it only dispatches what is queued, then `reap_actors` joins only finished actors. Shutdown semantics are unchanged (bounded per-module operation timeouts). | **NOT VERIFIED BY TEST** (inspection; Phase 1 tests pass) | Low | none |
| AUD-15 (LOW) | Cross-check of the documents | Mostly accurate. PARTIAL rows (SCH-2, EVL-4, EVL-6, EVL-9, …) remain PARTIAL. Synthetic and real evidence are distinguished, and no hardware or real-speech claim was found. **Overstatements**: the AUD-01 claims of accurate live accounting for every stage (audit §13, ledger SCH-1 and FAI-5, `orchestration.md` §5) are false for analysis (VA-01); "violation counters stay complete" is true for pipeline counters but not for `MetricsSnapshot` (VA-02). | **VERIFIED PARTIAL** | — | Fix together with VA-01 and VA-02 |

## 4. New findings of this verification audit

### VA-01 — MEDIUM (residual of AUD-01 HIGH) — abandoned **analysis** workers are not counted or capped

* **Location**:
  * `ce-audio-intel/src/worker.rs:167`: `AnalysisHandle::abandon()` calls
    `self.thread.take()`;
  * `communication-engine/src/runtime/core/intel.rs`: the stall path
    (around line 339) and the stop-timeout path (around line 418) abandon
    the handle without pushing it to `abandoned`.
* **Description**: a hung analysis worker's thread is detached and its
  handle dropped. `intel_abandoned_live()` cannot see it, and even a
  retained handle would report `is_finished() == true`, because the
  thread handle is gone. Its forwarder stays alive until the thread
  exits.
* **Reproduction**: `r7_abandoned_analysis_workers_versus_the_cap`, using
  `start_analysis_with` and a `ScriptedVad` with a 3 s per-hop delay.
  Over 20 rounds of start and stop: the stop timed out and the worker
  was abandoned **4 times**, then `audio.too_many_taps` refused the other
  16 starts (the cap did not). Final state: `abandoned_workers` gauge
  **1**, `worker_forwarders` **5**, thread delta **+5**, 0
  `abandoned_workers_exhausted` refusals.
* **Impact**:
  * The AUD-01 cap and gauge do not cover the analysis stage.
  * Growth is still bounded, by the audio layer's `MAX_TAPS = 4` per
    stream (an abandoned analysis keeps its tap) times the open capture
    streams. Those taps also block recognition on that stream. This is
    pre-existing behaviour that the correction did not fix.
  * The correction documents claim otherwise.
* **Correction**:
  * Keep the analysis thread handle on abandonment, so that `abandon()`
    stops and flags without detaching, as the other stages do.
  * Push the handle to `intel.abandoned` in the stall and stop-timeout
    paths.
  * Add a runtime test (VA-R7 as an assertion: the gauge counts hung
    analysis workers and the cap refuses at the limit).
  * Correct the documents.
* **Reproducible**: yes, deterministic. **Introduced by**: the AUD-01
  correction is incomplete. The detaching `abandon()` is pre-existing
  (Phase 3).

### VA-02 — LOW/MEDIUM (regression introduced by the AUD-09 correction) — engine-wide finding metrics undercount

* **Location**: `communication-engine/src/runtime/core/pipeline.rs`,
  `report_findings`. `causal_violations`, `timeline_anomalies` and
  `deadline_misses` in `MetricsSnapshot` are incremented per
  **published** finding, which the observer now limits to 32 per kind
  and stage per epoch.
* **Reproduction**: `r6_engine_metrics_vs_pipeline_counters…`. A 1 ms
  transcript deadline with the test clock advanced 600 s gives pipeline
  counters `deadline_misses` **125** (`suppressed_findings` 93), but
  engine metrics `deadline_misses` **32**.
* **Impact**: `MetricsSnapshot` documents these as violations "observed",
  but they silently cap at 32 per kind per pipeline epoch. Dashboards
  would under-report persistent problems, which is exactly the case the
  limiter exists for.
* **Correction**: increment the engine metrics from the observer's
  counter deltas, not from published findings, or count before
  limiting. Make VA-R6 an assertion that the two agree. Document the
  distinction between events and counters.
* **Reproducible**: yes. **Introduced by**: the AUD-09 correction.

### VA-03 — INFORMATIONAL — `test-hooks` is compiled by `--all-features`

* `verify.sh`'s all-features steps (tests, clippy, docs, cross targets)
  compile the injection API, which is `#[doc(hidden)]`, public, and
  needs a runtime opt-in (`test_retain_translation_senders`).
* A product built with `--all-features` would expose an API that can
  inject synthetic translation results. The risk is low (an explicit,
  documented test feature). It could be hardened with a
  `compile_error!` outside `cfg(test)` or debug builds, or with a
  feature name that is never part of release profiles.
* With retention enabled, the retained senders are never released.
  Test-only.

### Checked and not found

The corrections were also checked for the following, with no issue
found:
* **Worker lifecycle and accounting races**: the count only decreases
  on thread exit, and admission runs on the single control task.
* **Deadlocks**: no new locks were added; the forwarder awaits only the
  core channel.
* **Bounded-queue violations**: the forwarder test passes.
* **Stale-output publication**: none was observed.
* **Shutdown and restart regressions**: none; Phase 1–6 tests and the
  gates pass.
* **Configuration compatibility**: the new fields use `serde(default)`;
  `deny_unknown_fields` only rejects unknown keys.
* **Cross-platform**: all cross-target checks compile; the only new
  OS-specific code is `/proc` in an ignored audit test.

## 5. Residual limitations (if closed after VA-01 and VA-02)

* The natural stale-output race is unforced (AUD-03 scope).
* Core-channel saturation and main-loop fairness are unit-level only
  (AUD-04; SCH-2, EVL-4, EVL-6 PARTIAL).
* AUD-07 and AUD-14 are verified by inspection only.
* The Talktual bridge was not compiled (approval).
* All evidence is synthetic, scripted, replayed or fake-clock. There is
  no real-clock end-to-end latency, no real model and no hardware
  (EVL-9).
* Phase 15 remains the production certification gate. Closure of
  Phase 7 is not certification.

## 6. Verdict and correction plan

**REQUIRES CORRECTIONS.** Phase 7 is not yet eligible for closure:
* the AUD-01 (HIGH) correction does not cover the analysis stage, and
  its documentation claims it does (VA-01, reproduced);
* the AUD-09 correction introduced a metrics regression (VA-02,
  reproduced).

Everything else is verified corrected, or partial with limitations that
are acceptable for closure once documented as above.

**Correction plan** (not implemented; needs separate authorization):
1. **VA-01**:
   * make `AnalysisHandle::abandon()` stop and flag without detaching
     the thread, a minimal Phase 3 crate change;
   * push abandoned analysis handles to `intel.abandoned` in the stall
     and stop-timeout paths;
   * turn VA-R7 into an asserting test: the gauge counts hung analysis
     workers, the cap refuses with `orchestration.abandoned_workers_exhausted`
     at the limit, and recovery happens after exit.
2. **VA-02**:
   * derive `MetricsSnapshot` finding metrics from observer counter
     deltas;
   * turn VA-R6 into an asserting test (engine metric = pipeline
     counter).
3. **Documents**:
   * correct the audit §13, the ledger (SCH-1, FAI-5, OBS-8),
     `orchestration.md` §5 and the evaluation claims about accounting
     and counters;
   * record VA-01 to VA-03;
   * keep the AUD-03, AUD-04, AUD-07, AUD-11 and AUD-14 classifications
     as above.
4. **Optional, VA-03**: harden or document the `test-hooks` exposure.
5. **Re-run**: `verify.sh`, MSRV (with and without `test-hooks`), the
   repeated Phase 7 tests, the audit reproductions and the Talktual
   checks.
