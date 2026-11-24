# Phase 5 report — Semantic & Contextual Intelligence

Branch `deepak/gallant-hamilton-wegi2i`. The Phase 5 baseline is `31c3173`
(end of Phase 4). Date: 2026-11-24.

Related documents:
* specification: the Phase 5 PDF and the execution supplement;
* traceability: [`phase-5-ledger.md`](phase-5-ledger.md);
* evaluation: [`phase-5-evaluation.md`](phase-5-evaluation.md);
* alternatives: [`phase-5-research.md`](phase-5-research.md);
* architecture: [`../architecture/semantic.md`](../architecture/semantic.md);
* decisions: ADR-0025, ADR-0026;
* handoff: [`phase-5-handoff.md`](phase-5-handoff.md).

## 1. Executive summary

* **Delivered.** CE now has a semantic layer that consumes Phase 4
  transcript revisions. It produces typed, versioned observations:
  * entities, with normalized values only where justified;
  * relations;
  * reference resolutions (resolved, ambiguous or unresolved);
  * optional communicative acts, with abstention.

  Each observation links to the exact revision and character span it came
  from. It states its epistemic kind, uncertainty (a rule tier, never a
  probability), evidence and provenance.
* **Streaming.**
  * Stale, duplicate and late inputs are rejected.
  * Ids stay stable across revisions; observations stabilize and finalize
    with the transcript and are retracted when the final amends it.
  * Corrections cascade to dependent resolutions.
* **Context.** Scoped, bounded (items, bytes, tokens, TTL, utterance
  distance), per stream and ephemeral.
* **Domain profiles** are validated, fingerprinted data.
* **Execution.** A supervised, bounded, coalescing worker never blocks
  capture or the control plane.
* **Integration.** The runtime publishes `EventPayload::Semantic`. The
  facade has start, stop, reset-context, confirm and status calls.
* **Practical capability, end to end.** The deterministic English rule
  adapter extracts numbers, quantities, money, percentages, times, dates,
  relative days, weekdays, durations, digit sequences and profile terms.
  It also resolves references and labels acts.
  * It runs through the runtime, the Talktual bridge and the Engine Lab.
  * It was shown with the real sherpa-onnx model on synthetic speech.
* **Held-out replay results** on a small synthetic corpus, scored once
  after the rules were frozen:

  | Split | Entity exact F1 | Normalized values | Reference accuracy | Act macro-F1 |
  |---|---|---|---|---|
  | test | 0.907 | 34/34 | 0.733 | 0.929 |
  | asr (real recognizer output) | 0.500 (3 of 5 gold entities were lost by the ASR) | — | — | 0.810 |

  These describe this corpus only.
* **Not done; approval or a decision is needed** (§12):
  * ellipsis resolution;
  * optional profile schemas;
  * provider adapters (ML NER, hosted LLM);
  * name and location normalization.
* **Gates.**
  * CE: 318 tests (322 with all features), and 318 on Rust 1.85.0.
  * fmt, clippy `-D warnings`, docs `-D warnings`, 5 cross targets and
    `cargo deny` pass.
  * The bridge and sherpa gates pass, real-model tests included.
  * Talktual: tsc passes; 549 unit and 60 server tests pass; the build
    succeeds; the browser e2e passes 5 of 5.

  Phase 15 remains the certification gate.

## 2. Architecture

```text
Phase 4 SpeechEvent::Transcript ─▶ runtime core ─push (non-blocking)─▶ ce-semantic worker (per stream)
                                                                        │ bounded queue, coalescing, supervision
                                                                        ▼
                                                     SemanticProcessor ─▶ SemanticAdapter (rules | scripted | future)
                                                       revision checks   ◀─ drafts
                                                       contract checks, stable ids, status from revision, diff
                                                       ContextManager (scoped, bounded, expiring)
                                                                        │
                                       EventPayload::Semantic (topic Semantic, stream's session) ◀─ core
```

* **Contracts** are in `ce-core::semantic`, with no dependency.
* **Logic** is in the new crate `ce-semantic`, which depends on `ce-core`
  only.
* **Orchestration** is in `runtime/core/semantic.rs`.
* **Privacy class.** Observations are `Content`; everything else is
  `Operational`.
* **Locality policy.** Remote and local-service adapters are refused by
  default, and locality is disclosed.

Full design: `semantic.md`. Decisions: ADR-0025 (layer and contracts) and
ADR-0026 (rule baseline, evaluation, payload tag).

## 3. Files changed (since `31c3173`)

**New**
* `crates/ce-core/src/semantic/{mod,config,model,profile}.rs`: contracts,
  config, profiles.
* `crates/ce-semantic/`:
  * `src/adapter.rs`, `src/text.rs`, `src/rules.rs`, `src/scripted.rs`;
  * `src/context.rs`, `src/processor.rs`, `src/worker.rs`;
  * `src/eval.rs`, `src/metrics.rs`, `src/lib.rs`;
  * `examples/evaluate_semantic.rs`;
  * `tests/{processor,worker,evaluation}.rs`.
* `crates/communication-engine/src/runtime/core/semantic.rs` and
  `tests/semantic_runtime.rs`.
* `fixtures/semantic/{corpus.jsonl,PROVENANCE.md,profiles/home.json}`.
* `docs/architecture/semantic.md`, ADR-0025, ADR-0026.
* `docs/phases/phase-5-{ledger,research,evaluation,report,handoff}.md`.
* Talktual: `src/engineLab/__fixtures__/bridge-semantic-session.jsonl`.

**Modified (additive)**

| Area | Files | Change |
|---|---|---|
| ce-core | `event.rs` | `Semantic` payload and topic |
| ce-core | `command.rs` | 6 commands, 2 outcomes |
| ce-core | `config.rs` | `semantic` settings |
| ce-core | `health.rs` | `semantics` statuses and 3 metrics, all `serde(default)` |
| ce-core | `lib.rs` | module |
| runtime | `runtime/core.rs` | state, select branches, dispatch, close and shutdown order, health |
| runtime | `runtime/core/speech.rs` | forwards transcripts and the stop: 6 lines |
| runtime | `runtime/metrics.rs` | counters |
| facade | `lib.rs` | API, builder, prelude |
| workspace | `Cargo.toml` and lockfiles | new crate; no new third-party crate |
| scripts | `scripts/verify.sh` | semantic gate, cross targets |
| bridge | `src/{lib,protocol}.rs`, `tests/bridge.rs`, `README.md` | optional `semantics` |
| docs | `events.md`, `privacy.md`, `talktual-integration.md`, `README.md`, `ARCHITECTURE.md` | Phase 5 |
| Talktual | `src/engineLab/{labState,useEngineLab}.ts`, `labState.test.tsx`, `src/pages/EnginePage.tsx`, `e2e/ce-bridge.mjs`, `README.md` | Engine Lab Phase 5 |

## 4. Requirements ledger

See [`phase-5-ledger.md`](phase-5-ledger.md): 130 rows across §1–15 of the
PDF.

| Status | Rows |
|---|---|
| VERIFIED | 126 |
| DEFERRED | 4: OBJ-5b (provider adapter), ENT-3b (names and locations), REF-1b (ellipsis), PRO-2b (profile schemas) |
| IMPLEMENTED, NOT STARTED, IN PROGRESS, BLOCKED | none |

## 5. Requirement-to-evidence map (summary)

| Area | Main evidence |
|---|---|
| Contracts, schema, round trips, privacy class, validation | `ce-core` semantic unit tests (11): `observations_round_trip…`, `every_payload_kind_round_trips…`, `inputs_are_validated`, config and profile validation and fingerprints |
| Revisions: update, stabilize, finalize, retract, stable ids | T-proc `observations_update_stabilize_finalize_and_retract_with_stable_ids` |
| Stale, duplicate, after-final, revision gaps | T-proc `stale_duplicate_and_late_inputs_never_become_current` |
| Cancellation | T-proc `cancellation_retracts_everything_and_invalidates_context` |
| References: resolved, ambiguous, scoped | T-proc `references_resolve_only_to_permitted_compatible_context` |
| Context bounds, TTL, distance, items, bytes, tokens; admission | T-proc `context_is_bounded_by_distance_ttl_items_and_bytes` |
| Context scopes off/utterance | T-proc `context_scopes_off_and_utterance` |
| Correction cascade | T-proc `a_correction_invalidates_context_and_dependent_resolutions` |
| Isolation, reset, stop | T-proc `contexts_are_isolated…`; T-rt `streams_are_isolated_and_session_close_drains` |
| Malformed and oversized input | T-proc `invalid_inputs_are_rejected_and_finals_still_close` |
| Adapter contract enforcement (spans, mentions, evidence, confirmation, calibration, relations) | T-proc `contract_violations_by_adapters_are_dropped` |
| Output limits, min certainty, partials policy | T-proc `output_limits_min_certainty_and_partials_policy` |
| Adapter errors, deadline, repeated failure | T-proc `adapter_errors_deadlines_and_repeated_failure` |
| Locality policy, unsupported features | T-proc `locality_policy_and_unsupported_features_are_enforced`; T-rt `start_errors_policy_and_explicit_stop` |
| User confirmation | T-proc `only_the_host_confirms…`; T-rt; T-lab |
| Profiles never manufacture certainty | T-proc `domain_profiles_add_terms_without_manufacturing_certainty` |
| Act taxonomy subsets; non-English; contradictions kept | T-proc `act_taxonomy_subsets…`, `non_english_input…`, `contradictions_across_utterances…` |
| Deterministic replay | T-proc `replay_is_deterministic`; T-eval pinned scores |
| Coalescing, overload, non-blocking push | T-work `partials_are_coalesced…`, `overload_drops_oldest_partials…` |
| Panic and stall supervision | T-work `a_panicking_adapter…`, `a_hung_adapter_is_detected_as_stalled_and_abandoned` |
| Failure isolation | T-work `one_failing_worker_does_not_affect_another` |
| End to end through the runtime, privacy, linkage | T-rt `transcripts_become_linked_observations_end_to_end` |
| No content in logs, health or metrics | T-rt `logs_and_metrics_carry_no_semantic_content` |
| Bridge (scripted) | T-br `semantics_interpret_the_session_transcripts_when_asked` |
| Bridge (real model) | T-br `the_real_model_and_semantics_find_the_time_in_a_fixture` (ignored by default; run in `verify.sh`) |
| Engine Lab | T-lab (2 new tests); E2E "all five phases" |
| Quality, streaming, performance | `phase-5-evaluation.md` |

## 6. Tests and commands (all run on 2026-11-24)

| Command | Result |
|---|---|
| `communication-engine/scripts/verify.sh` (fmt, clippy `-D warnings` all features, tests default features, tests all features, docs `-D warnings`, smoke examples, Phase 3/4/5 evaluation gates, cross-target checks, `cargo deny`) | exit 0, "all gates passed" |
| `cargo +1.85.0 test --workspace --all-targets` | 318 passed, 0 failed |
| `cargo +1.85.0 check --workspace --all-targets --all-features` | ok (CI MSRV step) |
| `adapters/talktual-bridge/scripts/verify.sh` (with `CE_SHERPA_MODEL_DIR`, `SHERPA_ONNX_ARCHIVE_DIR`) | exit 0 |
| `adapters/ce-asr-sherpa/scripts/verify.sh` | exit 0 |
| `cargo run --release -p ce-semantic --example evaluate_semantic -- all --errors` | report in `phase-5-evaluation.md` |
| Talktual `npx tsc --noEmit` | ok |
| Talktual `npm test` | 32 files, 549 tests passed |
| Talktual `npm run test:server` | 60 passed, 0 failed |
| Talktual `npm run build` | built |
| Talktual `CE_SHERPA_MODEL_DIR=… node e2e/ce-bridge.mjs` | 5/5 passed (real model, Chromium fake mic, synthetic speech) |

## 7. Counts and results

| Gate | Baseline (`31c3173`) | Now | New |
|---|---|---|---|
| CE workspace tests, default features | 271 | 318 | +47: ce-core semantic 11, ce-semantic 32 (lib 6, processor 19, worker 6, evaluation 1), runtime 4 |
| CE workspace tests, all features | — | 322 | |
| MSRV 1.85.0 tests | 271 | 318 | |
| Bridge tests | 4 + 1 ignored | 5 + 2 ignored (both ignored tests run with the model: 2/2) | +1, +1 real-model |
| Sherpa adapter tests | 1 + 2 ignored | unchanged, all pass | |
| Talktual unit | 547 | 549 | +2 |
| Talktual server | 60 | 60 | |
| Talktual e2e (CE) | 5/5 | 5/5 (Engine Lab case extended to Phase 5) | |

There are no failures, and no test was skipped or disabled. The
`cargo deny` duplicate-crate warnings in the bridge (bzip2, getrandom, syn,
windows-sys) pre-date Phase 5.

## 8. Evaluation

Full tables, method, error analysis and raw output:
[`phase-5-evaluation.md`](phase-5-evaluation.md).

* **Process**:
  1. The corpus was committed (`e18acef`).
  2. The dev split was scored and its errors fixed with generic rules.
  3. The adapter was frozen (`b1e5a2d`).
  4. Test and asr were scored once. They are pinned in
     `tests/evaluation.rs`.
* **test** (45 utterances):
  * entities: exact P/R/F1 0.919 / 0.895 / 0.907; overlap F1 0.933;
    normalized values 34/34;
  * references: accuracy 0.733 (11/15); resolution precision 0.750;
    abstention rate 0.714;
  * acts: macro-F1 0.929; coverage 0.822; selective accuracy 0.946;
    abstention recall 0.857.
* **asr** (10 real recognizer hypotheses of synthetic speech): entity F1
  0.500, with 3 of 5 gold values lost by the recognizer; act macro-F1
  0.810.
* **Robustness.** Replay with duplicated and stale partials, or with
  finals only, gives identical finals in 39 of 39 dialogues.
* **Scope.** The corpus is small, synthetic and English, and was written
  by the authors of the rules. No generalization is claimed, and no score
  is calibrated.

## 9. Performance

Measured in a Linux x86_64 container (Xeon @ 2.30 GHz, 4 vCPU, 16 GB),
release build:
* processor time per input: p50 ≈ 0.006 ms, p95 ≤ 0.013 ms;
* single-thread replay throughput: about 100 000 inputs/s;
* 4 threads: about 330 000 inputs/s aggregate;
* evaluation-process peak RSS: 4–6 MB;
* output: p95 under 3 KB of event JSON per input.

There are no device, mobile or browser-side measurements. The semantic
work runs in the bridge, not in the browser. Live latency through the
runtime was not separately measured. In the real-model test, the final
observation arrives with the final transcript.

## 10. Platforms

* **Built and tested:** Linux x86_64 (stable 1.97 and 1.85.0).
* **Compile-checked** (`cargo check`, all features where applicable):
  aarch64-apple-ios, aarch64-linux-android, x86_64-pc-windows-msvc,
  aarch64-apple-darwin, and wasm32-unknown-unknown (ce-core and the
  libraries including ce-semantic).

  On wasm, `std::thread` compiles but would not run; the worker needs
  threads.
* No hardware was available, so nothing ran on a device.

## 11. Privacy and limitations

**Privacy.**
* Observations are `Content`; context and lifecycle events are
  `Operational` and carry no text.
* Logs, health and metrics contain no transcript or observation text
  (tested).
* Context is in memory, per stream, bounded, cleared at reset, stop,
  cancellation, session close and shutdown, and never persisted.
* Only the in-process adapter ships. Remote adapters are refused unless
  the host allows them, and locality is disclosed.
* There is no inference about the speaker: no identity, emotion, health,
  nationality or location.
* Nothing acts autonomously on an observation.

**Limitations.**
* English rules only; other languages yield nothing.
* No person, organization or location entities.
* No `dozen`, no negative numbers, and `pounds` stays ambiguous.
* Reference compatibility is by category class, so things that are not
  entities (a flight, an event) cannot be antecedents.
* A greeting before a polite request hides the request.
* Acts are cue-based hypotheses.
* Contradictions are kept, not resolved.
* Deadlines discard late results but cannot pre-empt an in-process call;
  stalls are detected and the worker abandoned.
* No session-level summary event exists beyond `stopped`.
* Evaluation latencies use a synthetic timeline.
* Talktual uses semantics only in the Engine Lab (diagnostics). The call
  flow and translation are unchanged by design (out of scope).

## 12. Deferred / blocked, with approval requests

| ID | Item | Why it is not done | What is needed |
|---|---|---|---|
| REF-1b | Ellipsis resolution (§6.2) | Not implemented. A sound design needs a contract for elliptical fragments (e.g. "and three eggs" reusing the previous request's predicate). The current contract has no such observation, and adding one is a scope and design addition. | Your decision: approve a design increment (a new `ellipsis` reference kind resolved to the previous utterance's act, evaluated on dev/test), or accept it as deferred |
| PRO-2b | Optional schemas in domain profiles (§6.5) | No extraction would consume them yet. Adding unused configuration would not meet the completion rules. | Your decision: approve per-category attribute schemas together with extraction support, or defer |
| OBJ-5b | External/provider adapter (§1) | Paid service, credentials and remote processing of transcripts need approval. The contract and locality policy are ready. | Approval of a provider, credentials and a data-processing basis |
| ENT-3b | Name and location normalization (§6.1) | Rules are unreliable on upper-case, unpunctuated ASR text. ML NER (GLiNER/ONNX) needs a model download of hundreds of MB and Rust ≥ 1.92 (an out-of-workspace adapter). | Approval of the model download |

Nothing is BLOCKED.

## 13. Regressions

None found. All Phase 1–4 gates pass, including:
* the Phase 3 and Phase 4 evaluation gates in `verify.sh`;
* the real-model bridge and sherpa tests;
* the unchanged v1 bridge protocol for clients that do not request
  semantics;
* the Talktual suites and e2e.

Breaking changes to semantic types made inside Phase 5 (before any
consumer):
* the observation payload tag is `type` (it was `kind`, which clashed
  with fields inside relation and reference payloads; found by a new
  round-trip test);
* `Relation::kind` became `Relation::relation`.

## 14. Git status

* Branch `deepak/gallant-hamilton-wegi2i`. Phase 5 commits are
  `1b7edb1` … `283e373`, followed by the report and ledger commit.
* All are pushed to `origin/deepak/gallant-hamilton-wegi2i`, which
  updates PR #10.
* The working tree is clean after the final commit, with no untracked
  files.
* Commit messages end with "Built by Deepak and Claude".

## 15. Acceptance assessment and independent review

**Review.** The final code was re-read against the PDF text, section by
section (`pdftotext` of the uploaded PDF), separately from the ledger.
Gaps found in that pass were fixed and re-tested:

| Gap | Fix |
|---|---|
| no context token limit (§7) | `max_tokens` added and tested |
| configurable taxonomy (§6.3) | `act_labels` subset added and tested |
| multilingual robustness untested (§9) | non-English test added |
| contradiction tests (§9) | contradiction test added |
| output size and concurrency not reported (§9) | the evaluator reports both |
| a payload tag clash that broke deserialization of relation and reference observations | fixed; round-trip test added |

Two gaps could not be closed without a scope decision: ellipsis (§6.2)
and profile schemas (§6.5). They are reported in §12, not hidden.

**§14 criteria.**

| Criterion | Assessment |
|---|---|
| Modular, provider-agnostic; core independent of model SDKs | **met** |
| Typed, versioned, evidence-linked outputs with uncertainty and provenance | **met** |
| Immutable transcript; outputs reference revision/span | **met** (tested against published revisions) |
| Context bounded, isolated, resettable, expiry-aware, ephemeral by default | **met** |
| Streaming revision/finalization, stale input, cancellation, overload verified | **met** |
| Deterministic adapter tests without external services | **met** |
| ≥ 1 practical capability end to end; others gated | **met** (entity extraction, references and acts end to end with the real model on synthetic speech; provider adapters gated) |
| Reproducible methodology and results; explicit limitations; no inflated claims | **met** |
| Existing feasible regression gates pass; gaps documented | **met** |
| Phase report and handoff; Phase 15 remains the certification gate | **met** |

**Overall.** The §14 acceptance criteria are met with recorded evidence.
Phase 5 is **not declared fully complete**: two in-scope capabilities
(ellipsis resolution and optional profile schemas) are deferred pending
your decision, and two items need approvals (provider adapters, ML NER
for names and locations). Phase 15 remains the certification gate.
