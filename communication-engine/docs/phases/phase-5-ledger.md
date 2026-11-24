# Phase 5 requirements ledger — Semantic & Contextual Intelligence

Source: *Communication Engine — Phase 5, Semantic & Contextual Intelligence —
Engineering Specification* (PDF, authoritative) and the user's execution
supplement. Every bullet of the PDF has its own ID. Status values:
NOT STARTED, IN PROGRESS, IMPLEMENTED (code exists and works),
VERIFIED (evidence recorded), BLOCKED, DEFERRED. Evidence notes say
whether behaviour is tested, simulated or mocked.

Legend for locations: `core` = `crates/ce-core/src/semantic/`, `sem` =
`crates/ce-semantic/src/`, `rt` = `crates/communication-engine/src/runtime/core/semantic.rs`,
`fac` = facade `crates/communication-engine/src/lib.rs`, `ev` = evaluation
(`sem/eval.rs`, `crates/ce-semantic/examples/`), `fx` = `fixtures/semantic/`.

## 1. Objectives

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| OBJ-1 | Semantic processing consumes Phase 4 transcript events and produces typed, versioned observations | | | | NOT STARTED | |
| OBJ-2 | Progressive interpretation, utterance/session finalization, correction handling | | | | NOT STARTED | |
| OBJ-3 | Entities, relations, attributes, reference resolution, optional intent/communicative acts, no universal fixed taxonomy required | | | | NOT STARTED | |
| OBJ-4 | Uncertainty, evidence, provenance, temporal and transcript linkage; never silently turn hypotheses into facts | | | | NOT STARTED | |
| OBJ-5 | Deterministic, local and external/provider adapters behind capability contracts | | | | NOT STARTED | |
| OBJ-6 | Context management: explicit scope, isolation, size bounds, expiry, privacy controls | | | | NOT STARTED | |
| OBJ-7 | Extensible domain profiles; generic core domain-neutral | | | | NOT STARTED | |
| OBJ-8 | Replay evaluation, instrumentation, gates, tests, documented limitations | | | | NOT STARTED | |

## 2. Scope and boundaries

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| SCO-1 | In scope delivered: contracts/schemas, adapters/capabilities, entities, references, optional acts, context state, incremental updates, uncertainty/evidence, correction/retraction, profiles, privacy, bounded execution, observability, replay/eval, docs | | | | NOT STARTED | tracked by the rows below |
| SCO-2 | Out of scope not implemented: translation, orchestration/timing redesign, TTS, diarization, new acoustic processing, product UI, personal memory persistence beyond configured context, cloud deployment/billing, autonomous external actions, broad core redesign | | | | NOT STARTED | verified by final review |
| SCO-3 | Do not modify prior phase implementations unnecessarily | | | | NOT STARTED | |
| SCO-4 | Do not couple core to one LLM/provider | | | | NOT STARTED | |
| SCO-5 | Do not treat semantic outputs as ground truth | | | | NOT STARTED | |
| SCO-6 | Do not invent context | | | | NOT STARTED | |
| SCO-7 | Do not overwrite transcripts | | | | NOT STARTED | |
| SCO-8 | Do not persist audio or personal content by default | | | | NOT STARTED | |
| SCO-9 | No heavyweight model downloads without approval | | | | NOT STARTED | |

## 3. Reconnaissance and safeguards

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| REC-1 | Inspect Git status, branch, remotes, commits, pending changes; preserve user changes | | | | IN PROGRESS | |
| REC-2 | Read Phase 1–4 reports, ADRs, contracts, Phase 4 ASR events, Phase 3 events, tests, verification scripts | | | | IN PROGRESS | |
| REC-3 | Confirm baseline and PR state; do not assume reported results | | | | IN PROGRESS | |
| REC-4 | Identify MSRV, crate boundaries, features, platforms, test conventions, privacy/redaction, event sequencing | | | | IN PROGRESS | |
| REC-5 | Research methods/providers: licenses, costs, privacy, offline feasibility, latency, memory, concurrency, evaluation | | | | NOT STARTED | |
| REC-6 | No large downloads, datasets, paid calls, credentials, deployments, destructive actions without approval | | | | IN PROGRESS | standing rule |
| REC-7 | Document alternatives before implementation; external provider SDKs never enter core | | | | NOT STARTED | |

## 4. Architecture principles

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| ARC-1 | Provider-agnostic typed core; semantic adapters isolated | | | | NOT STARTED | |
| ARC-2 | Capability discovery, versioned metadata, validated configs | | | | NOT STARTED | |
| ARC-3 | Supervised, cancellable, bounded workers isolated from control plane and audio callbacks | | | | NOT STARTED | |
| ARC-4 | Bounded context, queues, output size, concurrency, deadlines; explicit overload/cancellation | | | | NOT STARTED | |
| ARC-5 | Transcript immutable; outputs reference revisions and spans | | | | NOT STARTED | |
| ARC-6 | Explicit types for observation / hypothesis / resolved interpretation / user-confirmed fact | | | | NOT STARTED | |
| ARC-7 | Stable IDs, revisions, session/utterance linkage, timestamps/spans, adapter/config fingerprints, provenance | | | | NOT STARTED | |
| ARC-8 | Deterministic tests need neither network nor credentials | | | | NOT STARTED | |

## 5. Contracts

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| CON-1 | SemanticInput: session/stream/utterance, transcript revision, text/span, language, recognition confidence if available, timing, context snapshot, policy | | | | NOT STARTED | |
| CON-2 | SemanticObservation: ID/schema version, source revision/span, type, payload, status, uncertainty, provenance, timestamps, adapter fingerprint | | | | NOT STARTED | |
| CON-3 | Entity: category, normalized value only when justified, raw mention/span, attributes, uncertainty, provenance | | | | NOT STARTED | |
| CON-4 | Relation: typed links between entity/observation IDs with evidence and uncertainty | | | | NOT STARTED | |
| CON-5 | ReferenceResolution: mention, candidate targets, supporting context, confidence/uncertainty, unresolved state | | | | NOT STARTED | |
| CON-6 | CommunicativeAct/Intent: optional extensible taxonomy, evidence, uncertainty, schema version | | | | NOT STARTED | |
| CON-7 | ContextItem: scoped, typed, bounded, provenance, expiry, confidence/status, policy classification, source linkage | | | | NOT STARTED | |
| CON-8 | DomainProfile: versioned terminology, aliases, context hints, schemas, evaluation identity; validated, bounded | | | | NOT STARTED | |
| CON-9 | SemanticEvent: output / update / finalize / error / degraded / cancel / context lifecycle | | | | NOT STARTED | |

## 6. Capabilities

### 6.1 Entity and structured extraction

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| ENT-1 | Entities and attributes with original mentions/source spans | | | | NOT STARTED | |
| ENT-2 | Domain-neutral categories and extensions, no industry hard-coding | | | | NOT STARTED | |
| ENT-3 | Normalize dates, names, quantities, locations etc. only when supported | | | | NOT STARTED | |
| ENT-4 | Normalization separate from raw mention; never invent missing fields | | | | NOT STARTED | |

### 6.2 Contextual interpretation and reference resolution

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| REF-1 | Resolve pronouns, ellipsis, conversational references only with permitted scoped context | | | | NOT STARTED | |
| REF-2 | Candidates or unresolved ambiguity instead of forced resolution | | | | NOT STARTED | |
| REF-3 | No cross-session leakage; context scope explicit | | | | NOT STARTED | |
| REF-4 | Reset, expiry, correction, invalidation | | | | NOT STARTED | |

### 6.3 Optional intent / communicative acts

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| ACT-1 | Optional request/question/correction/confirmation/refusal/acknowledgement/statement taxonomy | | | | NOT STARTED | |
| ACT-2 | Taxonomy configurable/versioned; not mandatory for all utterances | | | | NOT STARTED | |
| ACT-3 | Evaluate ambiguous, non-classifiable, out-of-taxonomy examples | | | | NOT STARTED | |

### 6.4 Streaming / revision

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| STR-1 | Consume provisional and committed revisions without stale results becoming current | | | | NOT STARTED | |
| STR-2 | Observations may update, stabilize or retract on transcript changes | | | | NOT STARTED | |
| STR-3 | Defined ordering, deduplication, late arrival, finalization | | | | NOT STARTED | |
| STR-4 | Bounded speculative work; cancel/restart when justified | | | | NOT STARTED | |

### 6.5 Domain profiles

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| PRO-1 | Profiles are data/config, not coupled industry logic | | | | NOT STARTED | |
| PRO-2 | Terminology, aliases, context hints, optional schemas | | | | NOT STARTED | |
| PRO-3 | Version/fingerprint, validate, bound and evaluate profiles | | | | NOT STARTED | |
| PRO-4 | Profiles never override evidence or manufacture certainty | | | | NOT STARTED | |

### 6.6 Uncertainty / evidence

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| UNC-1 | Document adapter confidence semantics; no cross-provider score comparison without calibration | | | | NOT STARTED | |
| UNC-2 | Evidence links to transcript spans/context; explicit vs inferred distinguished | | | | NOT STARTED | |
| UNC-3 | Abstention/unresolved states and low-confidence filtering | | | | NOT STARTED | |
| UNC-4 | No calibration claims without calibration tests | | | | NOT STARTED | |

## 7. Context lifecycle and privacy

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| CTX-1 | Utterance/session scope defined; longer scope only if explicitly supported/approved | | | | NOT STARTED | |
| CTX-2 | Ephemeral by default; no cross-session reuse without explicit policy/design | | | | NOT STARTED | |
| CTX-3 | TTL, item/byte/token limits, eviction, clear/reset API, teardown | | | | NOT STARTED | |
| CTX-4 | No raw audio by default; redacted logs; user content logged only under explicit debug/test policy | | | | NOT STARTED | |
| CTX-5 | Session/tenant isolation; clearing on cancellation/shutdown | | | | NOT STARTED | |
| CTX-6 | Policy-aware local/offline/remote adapter selection and disclosure | | | | NOT STARTED | |

## 8. Execution and fault handling

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| EXE-1 | Supervised workers, bounded queues, cancellation, deadlines, output limits | | | | NOT STARTED | |
| EXE-2 | No blocking of capture callback or control plane | | | | NOT STARTED | |
| EXE-3 | Backpressure, coalescing, stale-input rejection, overload and failure events | | | | NOT STARTED | |
| EXE-4 | Bounded retries only where justified; error classification | | | | NOT STARTED | |
| EXE-5 | Scripted adapter for lifecycle, failure, revision, concurrency tests | | | | NOT STARTED | |
| EXE-6 | Capability/config validation; clear unsupported-feature errors | | | | NOT STARTED | |

## 9. Evaluation

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| EVA-1 | Reproducible replay corpus; synthetic separated from consented/licensed data; license, provenance, split, collection, intended use | | | | NOT STARTED | |
| EVA-2 | No large dataset acquisition without approval | | | | NOT STARTED | |
| EVA-3 | Entity P/R/F1 per category; exact/normalized match; span policy | | | | NOT STARTED | |
| EVA-4 | Reference resolution accuracy, unresolved/abstention rates, error analysis | | | | NOT STARTED | |
| EVA-5 | Optional intent macro-F1, per-class scores, confusion matrix, abstention | | | | NOT STARTED | |
| EVA-6 | Context consistency, contradiction, correction, stale context, isolation tests | | | | NOT STARTED | |
| EVA-7 | Streaming latency to first observation, revision rate, stale results, finalization, cancellation, queue delay | | | | NOT STARTED | |
| EVA-8 | CPU, memory, concurrency, output size, latency distributions; platform/config disclosed | | | | NOT STARTED | |
| EVA-9 | Robustness: revisions, ambiguity, incomplete speech, ASR errors, injected failures; multilingual only where supported | | | | NOT STARTED | |
| EVA-10 | Dataset hashes/IDs, splits, profile/adapter/config fingerprints, version, hardware; tune/held-out separation | | | | NOT STARTED | |
| EVA-11 | No generalized quality claims from small synthetic samples | | | | NOT STARTED | |

## 10. Observability

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| OBS-1 | Latency distributions, first useful output, update/revision/retraction counts, context size, queue delay, cancellation, errors, degradation | | | | NOT STARTED | |
| OBS-2 | Correlate session/utterance/revision/adapter without exposing text | | | | NOT STARTED | |
| OBS-3 | Bounded, redacted diagnostics; metrics without text | | | | NOT STARTED | |
| OBS-4 | Configuration/provenance captured for replay | | | | NOT STARTED | |

## 11. Testing gates

| ID | Requirement | Design | Implementation | Evidence | Status | Notes |
|---|---|---|---|---|---|---|
| TST-1 | Contract/schema, span/provenance, uncertainty, profile tests | | | | NOT STARTED | |
| TST-2 | Adapter startup/shutdown, input, lifecycle, revision, cancellation, timeout, failure, limits, capability tests | | | | NOT STARTED | |
| TST-3 | Context bounds, TTL, reset, eviction, isolation, correction, privacy tests | | | | NOT STARTED | |
| TST-4 | Streaming revisions, stale results, dedup, ordering, finalization tests | | | | NOT STARTED | |
| TST-5 | Deterministic replay, robustness, oversized/malformed payloads, load tests | | | | NOT STARTED | |
| TST-6 | Concurrency/resource isolation; one session's failure doesn't impact others | | | | NOT STARTED | |
| TST-7 | fmt, clippy, docs, MSRV/platform/features | | | | NOT STARTED | |
| TST-8 | Existing CE and Talktual regression gates; exact commands/results | | | | NOT STARTED | |

## 12. Implementation sequence (PDF A–H; supplement stages A–M)

| ID | Stage | Status | Notes |
|---|---|---|---|
| SEQ-A | Reconnaissance, baseline, research, design | IN PROGRESS | |
| SEQ-B | Contracts, schemas, capabilities, fake adapter, tests | NOT STARTED | |
| SEQ-C | Pipeline, bounded execution, events, transcript revision integration | NOT STARTED | |
| SEQ-D | Context manager, policies, reset/expiry/limits, isolation | NOT STARTED | |
| SEQ-E | Entity extraction, reference resolution, optional intent | NOT STARTED | |
| SEQ-F | Domain profiles and validation | NOT STARTED | |
| SEQ-G | Replay eval, observability, robustness, performance, regression | NOT STARTED | |
| SEQ-H | Docs, ADRs, report, handoff | NOT STARTED | |

## 13. Deliverables

| ID | Deliverable | Location | Status | Notes |
|---|---|---|---|---|
| DEL-1 | Implementation within CE boundaries; documented ownership | | NOT STARTED | |
| DEL-2 | Phase report, ADRs, contract/schema docs, provider/method comparison | | NOT STARTED | |
| DEL-3 | Replay fixtures/corpus manifest, reproducible commands; no unapproved datasets/models | | NOT STARTED | |
| DEL-4 | Test and verification report: counts, commands, failures, limitations | | NOT STARTED | |
| DEL-5 | Quality/performance results with hardware, data, split, config | | NOT STARTED | |
| DEL-6 | Downstream integration notes; no UI beyond minimal integration testing | | NOT STARTED | |

## 14. Acceptance criteria

| ID | Criterion | Evidence | Status |
|---|---|---|---|
| ACC-1 | Modular, provider-agnostic; core independent of external model SDKs | | NOT STARTED |
| ACC-2 | Typed, versioned, evidence-linked outputs with uncertainty and provenance | | NOT STARTED |
| ACC-3 | Immutable transcript; outputs reference revision/span | | NOT STARTED |
| ACC-4 | Context bounded, isolated, resettable, expiry-aware, ephemeral by default | | NOT STARTED |
| ACC-5 | Streaming revision/finalization, stale-input, cancellation, overload verified | | NOT STARTED |
| ACC-6 | Deterministic adapter tests without external services | | NOT STARTED |
| ACC-7 | ≥ 1 practical capability end to end; others gated | | NOT STARTED |
| ACC-8 | Reproducible methodology/results, explicit limitations, no inflated claims | | NOT STARTED |
| ACC-9 | Existing feasible regression gates pass; platform/dependency gaps documented | | NOT STARTED |
| ACC-10 | Phase report and handoff; Phase 15 remains the certification gate | | NOT STARTED |

## 15. Change control

| ID | Requirement | Status | Notes |
|---|---|---|---|
| CHG-1 | Implement Phase 5 only | IN PROGRESS | |
| CHG-2 | Stop and request approval for: earlier-phase architectural change, large dependency/model download, licensing risk, paid service, credentials, deployment, destructive action, major platform limitation, scope expansion | IN PROGRESS | |
| CHG-3 | Protect Git state and user changes | IN PROGRESS | |
