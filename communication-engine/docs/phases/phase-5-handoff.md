# Phase 5 handoff

State at the end of Phase 5. Read with `phase-5-report.md` (status,
evidence) and `docs/architecture/semantic.md` (design).

## Where things are

* **Contracts**: `crates/ce-core/src/semantic/`.
* **Logic**: `crates/ce-semantic/`:
  * the adapter trait;
  * `ce.semantic.rules` 1.0.0 and the scripted adapter;
  * context, processor, worker;
  * evaluation.
* **Runtime**: `crates/communication-engine/src/runtime/core/semantic.rs`.
* **Facade**: `Engine::start_semantics(stream, Option<SemanticConfig>)`,
  `stop_semantics`, `reset_semantic_context`, `confirm_observation`,
  `semantic_status`, `semantic_adapters`. The builder has
  `semantic_adapter(...)` and `without_default_semantic_adapter()`.
* **Corpus and profile**: `fixtures/semantic/`. The gate is
  `crates/ce-semantic/tests/evaluation.rs` (pinned scores).
* **Talktual**: the bridge's `"semantics": true` and the Engine Lab
  Phase 5 panel.

## How to use it

```rust
let st = engine.start_semantics(stream, None).await?;   // rule adapter, session context
engine.start_recognition(stream, Some(cfg)).await?;     // order does not matter
// EventPayload::Semantic(SemanticEvent::Observation { change, observation })
engine.confirm_observation(stream, observation.id.clone()).await?; // after the user confirmed
```

Semantics end, after draining, when the stream's recognition stops.
Session close and shutdown drain them after recognitions.

## Rules for the next phases

* **Treat observations as hypotheses** unless `epistemic` is
  `user_confirmed_fact`. Check `status`: provisional observations may
  change or be retracted; `final` ones may still be retracted by a
  correction cascade.
* **Never use observation text in logs or metrics.** Observations are
  `Content`.
* **Translation (Phase 6)** may read final observations, for example to
  keep numbers and times consistent across languages. It must not edit
  transcripts or observations.
* **Changing a rule** changes the pinned scores:
  1. Re-evaluate on `dev` only.
  2. Re-record `tests/evaluation.rs` and `phase-5-evaluation.md`.
  3. Bump `RULES_VERSION`.
* **A new adapter** (ML NER, a hosted LLM):
  * build it outside the workspace, like `adapters/ce-asr-sherpa`;
  * declare locality and score semantics;
  * a `remote` adapter runs only with `policy.allow_remote`;
  * it needs the user's approval for downloads, paid use and credentials.

## Open items (need a decision)

* REF-1b: ellipsis resolution.
* PRO-2b: profile schemas.
* OBJ-5b: provider adapter.
* ENT-3b: names and locations.

See report §12.

## Verification

```bash
cd communication-engine && scripts/verify.sh            # all CE gates (Phases 1-5)
cargo +1.85.0 test --workspace --all-targets             # MSRV
cargo run --release -p ce-semantic --example evaluate_semantic -- all --errors
CE_SHERPA_MODEL_DIR=… SHERPA_ONNX_ARCHIVE_DIR=… adapters/talktual-bridge/scripts/verify.sh
cd .. && npx tsc --noEmit && npm test && npm run test:server && npm run build
CE_SHERPA_MODEL_DIR=… node e2e/ce-bridge.mjs
```
