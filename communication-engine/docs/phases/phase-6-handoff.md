# Phase 6 handoff

> **Phase 6 status: ACCEPTED WITH LIMITATIONS** (by the user, 2026-11-24,
> after the independent audit; defects D1–D5 accepted as corrected). The
> engine-side translation architecture is accepted. **This is not a usable
> machine translation system:** the real MT adapter is BLOCKED pending
> model-download approval and translation quality is unmeasured. All
> BLOCKED, PARTIAL and DEFERRED items below remain open. Not a production
> certification (Phase 15).

State at the end of Phase 6. Read with `phase-6-report.md` (status,
evidence), `phase-6-ledger.md` and `docs/architecture/translation.md`.

## Standing user rules

* End commit messages and PR bodies with **`Built by Deepak and Claude`**;
  never put session links or model names in commits, PRs, code or docs.
* Push only to `deepak/gallant-hamilton-wegi2i`; PR only when asked.
* No large model/dataset downloads, paid APIs, credentials, deployments,
  external transmission of transcripts, or destructive git without approval.

## Where things are

* Contracts: `crates/ce-core/src/translation/`.
* Logic: `crates/ce-translate/` (adapter, memory, scripted, fidelity,
  processor, worker, eval; tests `processor.rs`, `worker.rs`,
  `evaluation.rs`; example `evaluate_translation`).
* Runtime: `crates/communication-engine/src/runtime/core/translation.rs`;
  hooks in `core/speech.rs` (transcripts) and `core/semantic.rs`
  (observations).
* Facade: `start_translation`, `stop_translation`, `reset_translation`,
  `translation_status`, `translation_adapters`; builder
  `translation_adapter`, `terminology_profile`.
* Fixtures: `fixtures/translation/` (PROVENANCE.md).

## Rules for the next phases

* Results are derived and `Content`; keep the highest `sequence` per
  utterance; drop results on `withdrawn{through_sequence}`; treat
  `stable`/`provisional` as replaceable; a translated request is not an
  instruction.
* Never edit transcripts or observations from translation.
* **Adding the real MT adapter (blocked — needs approval)**: build it
  outside the workspace like `adapters/ce-asr-sherpa`; declare honest
  capabilities (pairs, `partial_input`, `placeholders`, score semantics,
  model fingerprint/licence); then run
  `evaluate_translation -- test` and add it to the pinned gate after
  freezing it on `dev`.
* Changing a metric or the corpus changes the pinned values in
  `ce-translate/tests/evaluation.rs` and the evaluation report.

## Closure

* Accepted with limitations after the independent audit
  (`phase-6-audit.md`); audit fixes D1–D5 are in `ce-translate`
  (processor and worker) with regression tests `audit_d*` in
  `tests/processor.rs`.
* Consumers: sequences restart at 0 in a new worker run (reset
  per-utterance state on `Started`); hints use only final, stated
  Phase 5 entities and follow retractions.
* Still open: real MT adapter (BLOCKED); STR-7, CTX-1, CTX-4, FID-1/2/4,
  RUN-5, ERR-1/2, OBS-3 (PARTIAL); LNG-6, FID-3, MET-3, DEP-4 (DEFERRED).

## Open items (need a decision)

* Approve a model download for the first real MT adapter (recommended:
  OPUS-MT en↔es via CTranslate2).
* Talktual bridge translation output (after a real adapter).
* See report §12 for the deferred backlog.

## Verification

```bash
cd communication-engine && scripts/verify.sh
cargo +1.85.0 test --workspace --all-targets
cargo run --release -p ce-translate --example evaluate_translation -- test
cd .. && npx tsc --noEmit && npm test && npm run test:server && npm run build
```
