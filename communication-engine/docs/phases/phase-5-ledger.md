# Phase 5 requirements ledger — Semantic & Contextual Intelligence

Source: *Communication Engine — Phase 5, Semantic & Contextual Intelligence —
Engineering Specification* (PDF, authoritative) and the user's execution
supplement. Every bullet of the PDF has its own ID.

**Status values.**
* NOT STARTED.
* IN PROGRESS.
* IMPLEMENTED: the code exists and works, but its verification is
  incomplete.
* VERIFIED: the evidence is recorded below.
* BLOCKED.
* DEFERRED: not done, with the reason; an approval request where scope is
  involved.

**Evidence labels.**
* *test*: an automated test that was run.
* *scripted*: a scripted ASR or semantic adapter.
* *replay*: the corpus replayed through the real processor.
* *real model*: the sherpa-onnx model on synthetic TTS speech.
* *browser*: the Chromium e2e test.

The final review against the PDF is in `phase-5-report.md` §15.

**Location legend.**

| Short | Path |
|---|---|
| `core` | `crates/ce-core/src/semantic/` (`model.rs`, `config.rs`, `profile.rs`) |
| `sem` | `crates/ce-semantic/src/` |
| `rt` | `crates/communication-engine/src/runtime/core/semantic.rs` |
| `fac` | `crates/communication-engine/src/lib.rs` |
| `fx` | `fixtures/semantic/` |
| `arch` | `docs/architecture/semantic.md` |

**Test legend.**

| Short | Test |
|---|---|
| T-core | unit tests in `core` |
| T-proc | `crates/ce-semantic/tests/processor.rs` |
| T-work | `crates/ce-semantic/tests/worker.rs` |
| T-eval | `crates/ce-semantic/tests/evaluation.rs` |
| T-rt | `crates/communication-engine/tests/semantic_runtime.rs` |
| T-br | `adapters/talktual-bridge/tests/bridge.rs` |
| T-lab | `src/engineLab/labState.test.tsx` (Talktual) |
| E2E | `e2e/ce-bridge.mjs` (Talktual) |

## 1. Objectives

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| OBJ-1 | Semantic processing consumes Phase 4 transcript events and produces typed, versioned observations | arch, ADR-0025 | `sem/lib.rs` `input_from_revision`; `rt` `forward_transcript`; `core/model.rs` | T-rt `transcripts_become_linked_observations_end_to_end`; T-br real-model test | VERIFIED | scripted ASR, plus real model on synthetic speech |
| OBJ-2 | Progressive interpretation, utterance/session finalization, correction handling | arch "Streaming" | `sem/processor.rs` | T-proc update/stabilize/finalize/retract, correction cascade; T-rt session close drains | VERIFIED | "session finalization" = `stopped` after drain (`source_ended`/`shutdown`); no separate session-summary event |
| OBJ-3 | Entities, relations, attributes, reference resolution, optional acts; no universal fixed taxonomy | arch "rules" | `sem/rules.rs`; contract categories are strings, act taxonomy id/version | T-proc, T-eval, replay | VERIFIED | ellipsis: see REF-1b |
| OBJ-4 | Uncertainty, evidence, provenance, temporal and transcript linkage; never silently turn hypotheses into facts | ADR-0025 | `core/model.rs` (`Epistemic`, `Uncertainty`, `Evidence`, `Provenance`, `SourceRef`); processor rejects `UserConfirmedFact` drafts | T-core; T-proc `contract_violations…`, `only_the_host_confirms…` | VERIFIED | |
| OBJ-5a | Deterministic and local adapters behind capability contracts | ADR-0025 | `sem/adapter.rs`; `RulesFactory`; `ScriptedFactory` | T-proc, T-work, T-rt | VERIFIED | both in process |
| OBJ-5b | External/provider adapter behind the same contract | research §Candidates | contract and `Locality::Remote` policy only | T-proc and T-rt policy tests (scripted remote) | DEFERRED | a real provider needs approval (paid, credentials, remote data); ML NER needs a model download |
| OBJ-6 | Context management: scope, isolation, bounds, expiry, privacy | arch "Context" | `sem/context.rs` | T-proc context tests; T-rt isolation | VERIFIED | |
| OBJ-7 | Extensible domain profiles; generic core domain-neutral | arch "Domain profiles" | `core/profile.rs`; `fx/profiles/home.json` | T-core profile tests; T-proc profile test; replay | VERIFIED | |
| OBJ-8 | Replay evaluation, instrumentation, gates, tests, documented limitations | ADR-0026 | `sem/eval.rs`, `examples/evaluate_semantic.rs`, `scripts/verify.sh` | T-eval; `phase-5-evaluation.md` | VERIFIED | |

## 2. Scope and boundaries

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| SCO-1 | In-scope items delivered | arch | see the rows below | — | VERIFIED | except REF-1b and PRO-2b (deferred) |
| SCO-2 | Out of scope not implemented: translation, orchestration redesign, TTS, diarization, acoustics, product UI, persistence, deployment, autonomous actions, broad redesign | report §11 | — | code review: none added; the Engine Lab panel is a diagnostics page | VERIFIED | Talktual's call flow is unchanged |
| SCO-3 | Do not modify prior phases unnecessarily | report §3 | additive only: new `EventPayload`/`EventTopic` variants, commands, health fields (`serde(default)`); 2 lines in the speech signal handler | full regression gates | VERIFIED | |
| SCO-4 | Core not coupled to an LLM or provider | ADR-0025 | no new dependency in `ce-core`/`ce-semantic` | `Cargo.toml`; `cargo deny` | VERIFIED | |
| SCO-5 | Semantic outputs not treated as ground truth | arch | epistemic status, `calibrated: false`, confirmation only by the host | T-proc | VERIFIED | |
| SCO-6 | Do not invent context | arch "Context" | admission: committed and ≥ medium only; no defaults | T-proc `context_is_bounded…` (weak/provisional excluded) | VERIFIED | |
| SCO-7 | Do not overwrite transcripts | ADR-0025 | the processor enforces mention == text at span; inputs are snapshots | T-proc `contract_violations…`; T-rt span check against published revisions | VERIFIED | |
| SCO-8 | Do not persist audio or personal content by default | privacy.md | nothing is written; context in memory and cleared | T-rt `logs_and_metrics_carry_no_semantic_content`; code review | VERIFIED | |
| SCO-9 | No heavyweight model downloads without approval | research | none | — | VERIFIED | |

## 3. Reconnaissance and safeguards

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| REC-1 | Inspect Git state; preserve user changes | research §Baseline | — | branch clean at `31c3173`, PR #10 | VERIFIED | |
| REC-2 | Read Phase 1–4 reports, ADRs, contracts, events, tests, scripts | research | — | research §"What Phase 5 consumes" | VERIFIED | |
| REC-3 | Confirm baseline and PR state | research §Baseline | — | 271 CE tests; Talktual 547/60 reproduced before changes | VERIFIED | |
| REC-4 | MSRV, crate boundaries, features, platforms, conventions, redaction, sequencing | research §Baseline | — | MSRV 1.85 kept (`cargo +1.85.0 test`) | VERIFIED | |
| REC-5 | Research methods/providers: licences, costs, privacy, offline, latency, memory, concurrency, evaluation | `phase-5-research.md` | — | candidate table (crates.io metadata 2026-11-24) | VERIFIED | |
| REC-6 | No large downloads, datasets, paid calls, credentials, deployments, destructive actions without approval | — | — | none performed | VERIFIED | |
| REC-7 | Document alternatives first; no provider SDK in core | research, ADR-0025 | — | research committed with the contracts, before the crate | VERIFIED | |

## 4. Architecture principles

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| ARC-1 | Provider-agnostic typed core; adapters isolated | ADR-0025 | `core` has no logic; adapters behind `SemanticAdapter` | — | VERIFIED | |
| ARC-2 | Capability discovery, versioned metadata, validated configs | arch | `SemanticCapabilities`; `Engine::semantic_adapters`; `SemanticConfig::validate`/`fingerprint` | T-core config tests; T-rt discovery | VERIFIED | not in the Phase 1 module capability registry (same pattern as Phase 4 recognizers) |
| ARC-3 | Supervised, cancellable, bounded workers isolated from control plane and audio callbacks | arch "Bounded execution" | `sem/worker.rs` (thread, `catch_unwind`, heartbeat, non-blocking push) | T-work (panic, stall, stop, push never blocks) | VERIFIED | |
| ARC-4 | Bounded context, queues, output, concurrency, deadlines; explicit overload/cancellation | arch | `SemanticBudget`, `ContextConfig`, `SemanticSettings::max_concurrent` | T-work overload; T-proc limits; T-rt capacity | VERIFIED | a deadline discards late output; in-process calls are not pre-empted (a stall is detected and abandoned) |
| ARC-5 | Transcript immutable; outputs reference revisions and spans | ADR-0025 | `SourceRef` | T-rt; T-proc | VERIFIED | |
| ARC-6 | Types for observation / hypothesis / resolved interpretation / user-confirmed fact | `core/model.rs` `Epistemic` | — | T-proc | VERIFIED | |
| ARC-7 | Stable ids, revisions, session/utterance linkage, timestamps/spans, fingerprints, provenance | arch | ids per utterance; `version`; envelope `session_id`; `produced_at`; provenance | T-proc stable ids; T-rt session ids | VERIFIED | session linkage through the event envelope |
| ARC-8 | Deterministic tests need no network or credentials | — | — | all CE tests offline | VERIFIED | |

## 5. Contracts

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| CON-1 | SemanticInput: session/stream/utterance, revision, text/span, language, recognition confidence, timing, context snapshot, policy | `core/model.rs` | `SemanticInput`; the context snapshot is `ContextView`, passed with the input; the policy is enforced by the processor | T-core `inputs_are_validated` | VERIFIED | session comes from the envelope; context and policy are separate arguments by design (an adapter cannot widen them) |
| CON-2 | SemanticObservation: id/schema version, source revision/span, type, payload, status, uncertainty, provenance, timestamps, adapter fingerprint | `core/model.rs` | `SemanticObservation` | T-core round trips | VERIFIED | |
| CON-3 | Entity: category, normalized only when justified, raw mention/span, attributes, uncertainty, provenance | `core/model.rs` | `Entity` | T-core; replay normalized checks | VERIFIED | |
| CON-4 | Relation: typed links with evidence and uncertainty | `core/model.rs` | `Relation { relation, from, to }` | T-core `every_payload_kind_round_trips…`; T-proc refers_to | VERIFIED | a tag clash found and fixed (payload tag `type`) |
| CON-5 | ReferenceResolution: mention, candidates, supporting context, uncertainty, unresolved state | `core/model.rs` | `ReferenceResolution` | T-proc references | VERIFIED | |
| CON-6 | CommunicativeAct: optional extensible taxonomy, evidence, uncertainty, schema version | `core/model.rs` | `CommunicativeAct` | T-proc acts; replay | VERIFIED | |
| CON-7 | ContextItem: scoped, typed, bounded, provenance, expiry, status, policy classification, source linkage | `core/model.rs` | `ContextItem` (`data_class: Content`) | T-proc context | VERIFIED | |
| CON-8 | DomainProfile: versioned terminology, aliases, context hints, schemas, evaluation identity; validated, bounded | `core/profile.rs` | everything except schemas | T-core profile tests | VERIFIED | schemas: see PRO-2b |
| CON-9 | SemanticEvent: output/update/finalize/error/degraded/cancel/context lifecycle | `core/model.rs` | `SemanticEvent` | T-core; T-proc; T-rt | VERIFIED | |

## 6. Capabilities

### 6.1 Entity and structured extraction

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| ENT-1 | Entities and attributes with original mentions and spans | arch | `sem/rules.rs` | replay (test exact F1 0.907); T-rt span check | VERIFIED | |
| ENT-2 | Domain-neutral categories and extensions; no industry hard-coding | arch | built-ins plus `profile:*` | profile test; corpus profile dialogues | VERIFIED | |
| ENT-3a | Normalize dates, quantities, times, money … only when supported | arch | `NormalizedValue`; am/pm unknown stays unknown; weekdays never become dates | replay normalized 34/34 (test) | VERIFIED | |
| ENT-3b | Normalize names and locations | research | — | — | DEFERRED | no reliable rule method on upper-case unpunctuated ASR; needs ML NER (model download, approval) |
| ENT-4 | Normalization separate from mention; never invent missing fields | `core/model.rs` | `Date` with optional parts; `resolved_date` only with a host reference date | T-core; T-proc | VERIFIED | |

### 6.2 Contextual interpretation and reference resolution

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| REF-1a | Resolve pronouns and conversational references only with permitted scoped context | arch "References" | `rules.rs` reference forms; `ContextManager::view` | T-proc; replay (test accuracy 0.733) | VERIFIED | |
| REF-1b | Resolve **ellipsis** | — | — | — | DEFERRED — approval requested | no contract or rule for elliptical fragments; a proper design (predicate copying from earlier acts) is a scope addition, see report §12 |
| REF-2 | Candidates or unresolved instead of forced resolution | arch | `Resolution::{Ambiguous, Unresolved}` | T-proc ambiguity; replay confusion | VERIFIED | |
| REF-3 | No cross-session leakage; explicit scope | arch | per-stream manager; `ContextScopeMode` | T-proc isolation; T-rt streams | VERIFIED | |
| REF-4 | Reset, expiry, correction, invalidation | arch | `clear`, TTL, invalidation cascade | T-proc; T-rt reset | VERIFIED | |

### 6.3 Optional intent / communicative acts

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| ACT-1 | Optional request/question/correction/confirmation/refusal/acknowledgement/statement taxonomy | arch | `ce.acts` 1.0.0 | replay (test macro-F1 0.929) | VERIFIED | |
| ACT-2 | Taxonomy configurable/versioned; not mandatory | arch | `acts` on/off; `act_labels` subset; taxonomy id@version in capabilities | T-proc `act_taxonomy_subsets…` | VERIFIED | |
| ACT-3 | Evaluate ambiguous, non-classifiable, out-of-taxonomy examples | ADR-0026 | corpus gold `null`; greetings; ambiguous cues | replay abstention recall 0.857 (test) | VERIFIED | |

### 6.4 Streaming / revision

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| STR-1 | Provisional and committed revisions; stale never current | arch | processor revision checks | T-proc `stale_duplicate…`; robustness replay 15/15 | VERIFIED | |
| STR-2 | Update, stabilize, retract on transcript changes | arch | diff | T-proc | VERIFIED | |
| STR-3 | Ordering, deduplication, late arrival, finalization defined | arch | — | T-proc; T-work | VERIFIED | |
| STR-4 | Bound speculative work; cancel/restart when justified | arch | coalescing; `partials` off; deadline; stop discards | T-work coalescing; T-proc partials | VERIFIED | an in-flight call is not interrupted |

### 6.5 Domain profiles

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| PRO-1 | Profiles are data, not industry logic | arch | JSON, `deny_unknown_fields` | T-core | VERIFIED | |
| PRO-2a | Terminology, aliases, context hints | `core/profile.rs` | `terms`, `aliases`, `hints` | T-proc profile test | VERIFIED | |
| PRO-2b | Optional **schemas** in profiles | — | — | — | DEFERRED — approval requested | no extraction would consume them yet; adding unused config would not meet the completion rules (report §12) |
| PRO-3 | Version, fingerprint, validate, bound, evaluate | `core/profile.rs` | limits module; `fingerprint`; `evaluation` id | T-core; replay with profile | VERIFIED | |
| PRO-4 | Never override evidence or manufacture certainty | arch | the adapter decides certainty; shared forms → low hypothesis | T-proc profile test | VERIFIED | |

### 6.6 Uncertainty / evidence

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| UNC-1 | Document confidence semantics; no cross-provider comparison | arch | tiers; `score_semantics`; mismatches rejected | T-proc | VERIFIED | |
| UNC-2 | Evidence to spans/context; explicit vs inferred | `core/model.rs` `EvidenceKind` | — | T-proc reference evidence | VERIFIED | |
| UNC-3 | Abstention/unresolved and low-confidence filtering | arch | `min_certainty`; abstentions | T-proc `output_limits_min_certainty…` | VERIFIED | |
| UNC-4 | No calibration claims without calibration tests | arch | `calibrated: false`; calibrated claims rejected | T-proc | VERIFIED | |

## 7. Context lifecycle and privacy

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| CTX-1 | Utterance/session scope; longer scope only if approved | arch | `Off`/`Utterance`/`Session` | T-proc `context_scopes…` | VERIFIED | |
| CTX-2 | Ephemeral by default; no cross-session reuse | privacy.md | — | T-proc; T-rt | VERIFIED | |
| CTX-3 | TTL, item/byte/token limits, eviction, clear/reset, teardown | arch | `ttl_ms`, `max_items`, `max_bytes`, `max_tokens`; `clear` | T-proc bounds; T-rt reset | VERIFIED | |
| CTX-4 | No raw audio; redacted logs; content logs only under explicit policy | privacy.md | logs carry ids only | T-rt `logs_and_metrics…` | VERIFIED | |
| CTX-5 | Session/tenant isolation; clearing on cancellation/shutdown | arch | per stream; cleared at finish | T-proc; T-rt | VERIFIED | CE has no tenant concept; isolation is per stream and session |
| CTX-6 | Policy-aware local/offline/remote selection and disclosure | arch | `LocalityPolicy`; locality in status, provenance and logs | T-proc; T-rt `start_errors_policy…` | VERIFIED | |

## 8. Execution and fault handling

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| EXE-1 | Supervised workers, bounded queues, cancellation, deadlines, output limits | arch | worker, budget | T-work; T-proc | VERIFIED | |
| EXE-2 | No blocking of capture callback or control plane | arch | `push` is non-blocking; the core never waits on an adapter | T-work `push never blocks` | VERIFIED | |
| EXE-3 | Backpressure, coalescing, stale rejection, overload and failure events | arch | — | T-work; T-proc | VERIFIED | |
| EXE-4 | Bounded retries only where justified; error classification | arch | no retries (a deterministic adapter gains nothing from them); `CeError` categories and degrade reasons | T-proc errors | VERIFIED | |
| EXE-5 | Scripted adapter for lifecycle, failure, revision, concurrency tests | `sem/scripted.rs` | — | T-work; T-proc | VERIFIED | |
| EXE-6 | Capability/config validation; clear unsupported-feature errors | arch | `semantic.feature_unsupported`, `semantic.act_label_unsupported`, `config.semantic_invalid` | T-proc; T-core | VERIFIED | |

## 9. Evaluation

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| EVA-1 | Reproducible corpus; synthetic separated; licence, provenance, split, collection, intended use | `fx/PROVENANCE.md` | `fx/corpus.jsonl` with a `source` field | — | VERIFIED | no consented or licensed real data used |
| EVA-2 | No large dataset acquisition without approval | — | — | none | VERIFIED | |
| EVA-3 | Entity P/R/F1 per category; exact/normalized; span policy | ADR-0026 | `eval.rs` | `phase-5-evaluation.md` | VERIFIED | |
| EVA-4 | Reference accuracy, abstention, error analysis | ADR-0026 | `eval.rs` | evaluation doc | VERIFIED | |
| EVA-5 | Intent macro-F1, per-class, confusion, abstention | ADR-0026 | `eval.rs` | evaluation doc | VERIFIED | |
| EVA-6 | Context consistency, contradiction, correction, stale context, isolation tests | — | — | T-proc (`contradictions…`, cascade, TTL/distance, isolation); robustness replay | VERIFIED | contradictions are kept, not resolved |
| EVA-7 | Latency to first observation, revision rate, stale results, finalization, cancellation, queue delay | ADR-0026 | `eval.rs` streaming; `SemanticCounters` | evaluation doc; T-proc/T-work | VERIFIED | first observation on the synthetic timeline; cancellation and queue delay are measured in tests and counters, not in the corpus |
| EVA-8 | CPU, memory, concurrency, output size, latency distributions; platform/config disclosed | evaluation doc | example: throughput, 4 threads, RSS, output bytes | evaluation doc | VERIFIED | container only; no device |
| EVA-9 | Robustness: revisions, ambiguity, incomplete speech, ASR errors, injected failures; multilingual only where supported | — | robustness replays; asr split; T-work faults; non-English test | evaluation doc; T-proc `non_english…` | VERIFIED | English only |
| EVA-10 | Dataset hashes, splits, fingerprints, version, hardware; tune/held-out separation | ADR-0026 | sha256 in PROVENANCE; fnv in the report | commits `e18acef` (corpus) → `b1e5a2d` (freeze) → scoring | VERIFIED | |
| EVA-11 | No generalized claims from small synthetic samples | evaluation doc | — | wording reviewed | VERIFIED | |

## 10. Observability

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| OBS-1 | Latency distributions, first output, update/retraction counts, context size, queue delay, cancellation, errors, degradation | arch | `SemanticCounters`, `SemanticStatus`, metrics | T-rt status; T-work counters | VERIFIED | |
| OBS-2 | Correlate session/utterance/revision/adapter without text | arch | events carry ids; logs carry stream, adapter, config | T-rt no-content test | VERIFIED | |
| OBS-3 | Bounded, redacted diagnostics; metrics without text | arch | reservoirs bounded (1024); context events without content | T-rt; T-br context events | VERIFIED | |
| OBS-4 | Configuration and provenance captured for replay | arch | config and profile fingerprints in `started`, status and provenance | T-proc | VERIFIED | |

## 11. Testing gates

| ID | Requirement | Evidence | Status | Notes |
|---|---|---|---|---|
| TST-1 | Contract/schema, span/provenance, uncertainty, profile tests | T-core (11 semantic), T-proc | VERIFIED | |
| TST-2 | Adapter startup/shutdown, input, lifecycle, revision, cancellation, timeout, failure, limits, capability tests | T-proc, T-work, T-rt | VERIFIED | |
| TST-3 | Context bounds, TTL, reset, eviction, isolation, correction, privacy | T-proc, T-rt | VERIFIED | |
| TST-4 | Streaming revisions, stale results, dedup, ordering, finalization | T-proc, T-work | VERIFIED | |
| TST-5 | Deterministic replay, robustness, oversized/malformed payloads, load | T-proc `replay_is_deterministic`, `invalid_inputs…`; T-eval; T-work overload; throughput | VERIFIED | |
| TST-6 | Concurrency/resource isolation; one session's failure doesn't affect others | T-work `one_failing_worker…`; T-rt streams | VERIFIED | |
| TST-7 | fmt, clippy, docs, MSRV, platforms, features | `scripts/verify.sh` (fmt, clippy -D, doc -D, all features, 5 cross targets); `cargo +1.85.0 test` | VERIFIED | |
| TST-8 | Existing CE and Talktual regression gates; exact commands/results | report §6–7 | VERIFIED | |

## 12. Implementation sequence (PDF A–H; supplement stages A–M)

| ID | Stage | Status | Evidence |
|---|---|---|---|
| SEQ-A | Reconnaissance, baseline, research, design | VERIFIED | research doc, ADR-0025 (`1b7edb1`) |
| SEQ-B | Contracts, schemas, capabilities, fake adapter, tests | VERIFIED | `1b7edb1`, `afb1bb2` |
| SEQ-C | Pipeline, bounded execution, events, transcript revision integration | VERIFIED | `afb1bb2`, `c886550` |
| SEQ-D | Context manager, policies, reset/expiry/limits, isolation | VERIFIED | `afb1bb2`, `283e373` |
| SEQ-E | Entity extraction, reference resolution, optional intent | VERIFIED | `afb1bb2`, `b1e5a2d`; no model acquired |
| SEQ-F | Domain profiles and validation | VERIFIED | `1b7edb1`, `afb1bb2` |
| SEQ-G | Replay evaluation, observability, robustness, performance, regression | VERIFIED | `e18acef`, `b1e5a2d`, `f24fd4d`, `283e373` |
| SEQ-H | Docs, ADRs, report, handoff | VERIFIED | `05133de`, report and handoff commit |

## 13. Deliverables

| ID | Deliverable | Location | Status |
|---|---|---|---|
| DEL-1 | Implementation within CE boundaries; documented ownership | `core`, `sem`, `rt`, `fac`; arch "Crates and ownership" | VERIFIED |
| DEL-2 | Phase report, ADRs, contract docs, provider comparison | `phase-5-report.md`, ADR-0025/0026, `semantic.md`, `phase-5-research.md` | VERIFIED |
| DEL-3 | Replay corpus manifest and reproducible commands; no unapproved data | `fx/PROVENANCE.md`; evaluation doc | VERIFIED |
| DEL-4 | Test and verification report with counts, commands, failures, limitations | report §6–7 | VERIFIED |
| DEL-5 | Quality/performance results with hardware, data, split, config | `phase-5-evaluation.md` | VERIFIED |
| DEL-6 | Downstream integration notes; minimal UI for integration testing only | `talktual-integration.md` "Phase 5"; Engine Lab panel | VERIFIED |

## 14. Acceptance criteria

| ID | Criterion | Evidence | Status |
|---|---|---|---|
| ACC-1 | Modular, provider-agnostic; core independent of model SDKs | SCO-4, ARC-1 | VERIFIED |
| ACC-2 | Typed, versioned, evidence-linked outputs with uncertainty and provenance | CON-2…CON-9; T-core | VERIFIED |
| ACC-3 | Immutable transcript; outputs reference revision/span | ARC-5; T-rt | VERIFIED |
| ACC-4 | Context bounded, isolated, resettable, expiry-aware, ephemeral by default | CTX-1…CTX-6 | VERIFIED |
| ACC-5 | Streaming revision/finalization, stale input, cancellation, overload verified | STR-1…STR-4; T-work | VERIFIED |
| ACC-6 | Deterministic adapter tests without external services | ARC-8 | VERIFIED |
| ACC-7 | ≥ 1 practical capability end to end; others gated | entity extraction (and references, acts) through the runtime, bridge, real model and browser; provider adapters gated by policy and approval | VERIFIED |
| ACC-8 | Reproducible methodology/results; explicit limitations; no inflated claims | ADR-0026; evaluation doc | VERIFIED |
| ACC-9 | Existing feasible regression gates pass; gaps documented | report §6, §10, §13 | VERIFIED |
| ACC-10 | Phase report and handoff; Phase 15 remains the certification gate | `phase-5-report.md`, `phase-5-handoff.md` | VERIFIED |

## 15. Change control

| ID | Requirement | Status | Notes |
|---|---|---|---|
| CHG-1 | Implement Phase 5 only | VERIFIED | |
| CHG-2 | Stop and request approval for: earlier-phase architectural change, large dependency/model download, licensing risk, paid service, credentials, deployment, destructive action, major platform limitation, scope expansion | VERIFIED | none was needed for the delivered work; approval is requested for REF-1b, PRO-2b, OBJ-5b and ENT-3b (report §12) |
| CHG-3 | Protect Git state and user changes | VERIFIED | only additive commits on the session branch; no force push |
