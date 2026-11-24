# Phase 6 handoff

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
