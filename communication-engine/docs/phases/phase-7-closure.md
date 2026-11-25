# Phase 7 closure — Streaming Orchestration & Timing Intelligence

* **Status: CLOSED.**
* **Final verdict: ACCEPT WITH LIMITATIONS.**
* **Date**: 2026-11-25.
* **Branch**: `deepak/gallant-hamilton-wegi2i`, integrated into `main`
  by a normal merge. The integration report gives the PR and merge
  commit.
* **This is not production certification.** Phase 15 remains the
  certification gate.
* **Phase 8 has not started.**

> **Independence caveat.** The implementation, both audits and all
> corrections were performed by the same agent. The compensating
> evidence is: gates re-run on clean checkouts, deterministic
> reproductions written to falsify the corrections' claims, and proof
> that the final correction tests fail without the fixes.

## 1. History

| Step | Commit | Result |
|---|---|---|
| Baseline (main) | `8c92318` | Phase 6 closed |
| Implementation | `8418622` | Phase 7 orchestration and timing |
| Audit | `cf250b8` | REQUIRES CORRECTIONS: 1 HIGH, 3 MEDIUM, 8 LOW, 3 INFO |
| Corrections | `3b856eb` | AUD-01 to AUD-15 dispositioned |
| Verification audit | `3ec74ae` | REQUIRES CORRECTIONS: VA-01, VA-02 (reproduced); VA-03 informational |
| Final corrections and closure | this closure commit | VA-01 and VA-02 corrected; VA-03 accepted |

## 2. Findings closed

| Finding | Final status |
|---|---|
| AUD-01 (HIGH) abandoned-worker growth | **Corrected**: the cap covers all four stages (VA-01 completed analysis) |
| AUD-02 restart validation | **Corrected** |
| AUD-03 stale-output rejection end to end | **Verified with stated scope** (test hook through real senders) |
| AUD-04 saturation | **Partial, accepted**: stage-level saturation and forwarder backpressure verified |
| AUD-05, 06, 08, 10 | **Corrected** |
| AUD-09 findings limiter | **Corrected**; its metrics side effect corrected by VA-02 |
| AUD-07, AUD-14 | **Corrected in code; code review only** |
| AUD-11 Talktual bridge compile | **Open, accepted** (needs approval of a native download) |
| AUD-12, AUD-13 | **Accepted** (documented) |
| AUD-15 documentation | **Corrected** (including the notes for VA-01 and VA-02) |
| VA-01 analysis abandoned workers uncounted | **Corrected** |
| VA-02 engine metrics capped with events | **Corrected** |
| VA-03 `test-hooks` compiled by `--all-features` | **Accepted**: off by default, runtime opt-in, `#[doc(hidden)]`, documented "never enable in products" |

## 3. Accepted limitations

1. **Natural stale-output race unforced**: a worker thread emitting late
   during a real restart race was never forced. Rejection is proven
   through the replaced worker's real sender (test hook); 60 rapid
   restarts produced no natural late output.
2. **Main-loop fairness under saturation**: not demonstrated end to end
   with a saturated core channel. `next_data` fairness is unit-tested,
   and stage-level saturation is verified (SCH-2, EVL-4 and EVL-6
   PARTIAL).
3. **AUD-07** (discarding workers whose start failed) is **code review
   only**.
4. **AUD-14** (bounded module-exit drain) is **code review only**.
5. **The Talktual bridge** (`adapters/talktual-bridge`) was **not
   compiled**: the native sherpa-onnx download is unapproved. By
   inspection it uses no pipelines and matches events non-exhaustively.
   Talktual's own checks pass, but they mock the bridge.
6. **No real hardware validation**: no audio device, mobile or desktop
   device. Cross-platform evidence is compile-only.
7. **Real-world speech and acoustic evaluation is limited**: all Phase 7
   evidence is synthetic, scripted, replayed or fake-clock. There is no
   real-clock end-to-end latency (EVL-9 PARTIAL).
8. **No adapter contract-version comparison** (CMP-4 PARTIAL), and no
   long wall-clock run (EVL-7, TST-10 PARTIAL).
9. **Timeline-anomaly metric**: its completeness beyond 32 shares the
   VA-02 mechanism, but no runtime test produces more than 32 anomalies.
10. **Phase 7 closure is not Phase 15 production certification.**

## 4. Verification evidence (final corrected code; all executed, output observed)

### Working tree, after a full `cargo clean`

A stale-artifact incident and its resolution are described in §6.

| Gate | Command | Result |
|---|---|---|
| All CE gates | `scripts/verify.sh` | **all gates passed**, exit 0. Covers fmt, clippy (all features, `-D warnings`), tests, rustdoc (`-D warnings`), smoke examples, the Phase 3–7 gates (observer overhead mean 119 ns, under its 5 µs guard), cross targets (iOS, Android, Windows MSVC, macOS, wasm32; none skipped) and `cargo deny` (duplicate-crate warnings only, as at baseline). |
| Tests, default features | `verify.sh` step | **412 passed, 0 failed** |
| Tests, all features (includes `test-hooks`) | `verify.sh` step | **419 passed, 0 failed** |
| MSRV | `cargo +1.85.0 test --workspace --all-targets` | **412 passed, 0 failed** |
| MSRV with hooks | `cargo +1.85.0 test -p communication-engine --features test-hooks --test stale_output_runtime` | **3 passed** |
| Repeated lifecycle and concurrency | `orchestration_runtime` (11), `orchestration_corrections` (10), `stale_output_runtime` (3), ×5 | **5/5 runs, all passed** |
| Audit reproductions R1–R7 | `--test orchestration_audit -- --ignored` | **7/7 passed**. R1: cap 16 holds. R6: 126 = 126. R7: 3 analysis workers counted, 3 refused. |
| Talktual | `npx tsc --noEmit`, `npm test`, `npm run test:server`, `npm run build` | pass; **549**; **60**; pass |
| Talktual bridge | — | **not run** (approval) |

### Clean checkout of the closure commit

See the integration report (§5) for these results, together with the
post-merge results on `main`.

## 5. Acceptance criteria (spec §19) at closure

| Criterion | Status |
|---|---|
| Mandatory requirements implemented or dispositioned | met (ledger) |
| Invalid graphs fail before activation | met (also on restart) |
| Temporal provenance and causal context preserved, or loss represented | met |
| Revisions and lifecycle prevent stale or duplicate downstream state | met; the natural race is unforced (limitation 1) |
| Queues, concurrency, state and resources bounded | met; abandoned workers capped on every stage |
| Backpressure and deadlines deterministic and observable | met at stage level; core-channel saturation partial (limitation 2) |
| Cancellation, reset, stop, restart and shutdown tested | met |
| Failure isolation and degraded operation explicit | met |
| Multi-stream state isolated | met |
| Diagnostics privacy-reviewed | met (operational only) |
| All verification gates pass | met |
| Phase 1–6 regressions pass | met |
| Talktual checks pass | met; bridge not compiled (limitation 5) |
| No unapproved downloads, external transmission, paid APIs or deployment | met |
| Documentation, ledger, evidence and Git state complete | met |
| Independent audit and closure | met with the independence caveat |

## 6. Process note: a stale build artifact

During the final verification, one `ce-semantic` evaluation test failed
with `No such file or directory`.

* **Cause**: a test binary had been compiled earlier in a since-removed
  verification worktree that shared the build target directory. The
  binary carried that worktree's `CARGO_MANIFEST_DIR`, and Cargo reused
  it.
* **Resolution**: after `cargo clean`, every gate passed.
* **Not a code defect.** The clean-checkout verification of the closure
  commit uses an isolated target directory to avoid it.
