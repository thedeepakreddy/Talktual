# Phase 6 requirements ledger — Translation Intelligence

> **Phase 6 status: ACCEPTED WITH LIMITATIONS** (by the user, 2026-11-24,
> after the independent audit; defects D1–D5 accepted as corrected). The
> engine-side translation architecture is accepted. **This is not a usable
> machine translation system:** the real MT adapter is BLOCKED pending
> model-download approval and translation quality is unmeasured. All
> BLOCKED, PARTIAL and DEFERRED items below remain open. Not a production
> certification (Phase 15).

Source: *Communication Engine — Phase 6, Translation Intelligence — Full
Engineering Specification* (PDF, authoritative). Every requirement bullet
has an ID (section prefix + number).

**Status**: VERIFIED (evidence recorded), PARTIAL, MISSING, BLOCKED,
DEFERRED, NOT APPLICABLE. VERIFIED means evidence exists — a test that
was run, a recorded measurement or a document — not merely code.

**Locations**: `core` = `crates/ce-core/src/translation/`; `tr` =
`crates/ce-translate/src/`; `rt` = `crates/communication-engine/src/runtime/core/translation.rs`;
`fac` = `crates/communication-engine/src/lib.rs`. **Tests**: `P` =
`ce-translate/tests/processor.rs`, `W` = `ce-translate/tests/worker.rs`,
`E` = `ce-translate/tests/evaluation.rs`, `R` =
`communication-engine/tests/translation_runtime.rs`, `U` = unit tests in
the named module. **Docs**: ADR-0027/0028/0029, `architecture/translation.md`
(`arch`), `phase-6-research.md` (`res`), `phase-6-evaluation.md` (`eval`).

## 1. Mission

| ID | Requirement | Implementation | Tests / evidence | Status | Risk / disposition |
|---|---|---|---|---|---|
| MIS-1 | Translation is a core CE module, not a Talktual feature | `ce-translate`, `rt`, `fac` | R (facade end to end) | VERIFIED | — |
| MIS-2 | Source transcript and Phase 5 outputs immutable | read-only clones in `rt`; processor takes `&TranscriptRevision` | P `transcripts_and_observations_are_not_mutated`; R final transcripts equal the ASR script | VERIFIED | — |
| MIS-3 | Outputs derived, linked to exact source revisions | `TranslationSource` | P `finals_only…`, R linkage loop | VERIFIED | — |
| MIS-4 | Path from local/offline to optional hosted adapters, no hosted service now | `Locality`, `privacy.allow_remote`, adapter trait | P `selection_fallback…` (remote refused/allowed) | VERIFIED | no hosted adapter exists (by design) |
| MIS-5 | Translation does not imply understanding or safe execution | docs (`arch`, ADR-0027, facade docs, privacy.md); no action API | inspection | VERIFIED | — |

## 2. Goals and non-goals

| ID | Requirement | Implementation | Evidence | Status | Risk / disposition |
|---|---|---|---|---|---|
| GOL-1 | Pair/direction-aware operation with explicit capability | `LanguagePair` (directed), `TranslationCapabilities::pairs` | U `core::model` (directed pairs), P unsupported pair | VERIFIED | — |
| GOL-2 | Batch and incremental/streaming compatibility where supported | `PartialPolicy`, `partial_input`, `incremental_output` | P partial policies; `partials_unsupported` start error | VERIFIED | no adapter has incremental output |
| GOL-3 | Revision handling, cancellation, stale rejection, finalization | processor revision rules, withdrawal | P stale/duplicate/late, cancellation; W coalescing | VERIFIED | — |
| GOL-4 | Context-aware with bounded Phase 5 signals, no hallucinated facts | `ContextPolicy`, `hints_for` | P `context_hints_are_bounded…`; R hints reach translation | VERIFIED | — |
| GOL-5 | Domain terminology with scope, versioning, precedence | `TerminologyProfile`, `fidelity` | U `core::model`, `fidelity`; P terminology | VERIFIED | inflection not handled |
| GOL-6 | Privacy-first, local-first, explicit external boundary | defaults, `allow_remote` | P, R (`remote_not_allowed`, implicit selection skips remote) | VERIFIED | — |
| GOL-7 | Reproducible evaluation, honest limitations | `eval`, example, E | `eval` doc; E pinned gate | VERIFIED | no real MT evaluated (BLOCKED, ADP-2) |
| GOL-8 | Adapter interchangeability, no Talktual coupling | trait + factory; builder registration | P/R run scripted, memory, fallback chains | VERIFIED | — |
| NG-1..8 | No TTS/voice, ASR/semantic rebuild, diarization/DSP, Phase 7/11 redesign, UI/platform, autonomous action, universal coverage, unapproved downloads/paid/credentials/transfer | none implemented | `git diff --stat` (only CE translation code and docs; Phase 4/5 code changed only for forwarding hooks) | VERIFIED | — |

## 3. Dependencies and boundaries

| ID | Requirement | Implementation | Evidence | Status | Risk / disposition |
|---|---|---|---|---|---|
| DEP-1 | Consume Phase 4 through existing contracts, no mutation | `TranscriptRevision` consumed as is | P, R | VERIFIED | — |
| DEP-2 | Phase 5 context as hints; explicit vs inferred distinguished | final entities only; `same_utterance`; conflict flag | P context test | VERIFIED | hints carry no epistemic field yet (entities are `observation`) |
| DEP-3 | Runtime identity, lifecycle, cancellation, events, bounded execution without Phase 7 redesign | `rt` mirrors Phase 5 orchestration | R | VERIFIED | — |
| DEP-4 | Talktual bridge: minimal integration where bridge exists | — | — | DEFERRED | bridge (`adapters/talktual-bridge`) not extended; nothing to show without a real MT adapter. Consider with the first real adapter. |
| DEP-5 | Phase 7 not implemented | — | inspection | VERIFIED | — |

## 4. Reconnaissance

| ID | Requirement | Evidence | Status |
|---|---|---|---|
| REC-1 | Inspect structure, APIs, registration, supervision, events, queues, errors, config, diagnostics | report §2; `rt` generated from Phase 5 pattern | VERIFIED |
| REC-2 | Read Phase 1–5 reports, ADRs, Phase 5 ledger/eval/handoff | report §1 | VERIFIED |
| REC-3 | Inspect tests, testkit, replay, bridge, Talktual | report §1 (bridge not extended: DEP-4) | VERIFIED |
| REC-4 | Verify branch, commit, status, untracked, submodules; preserve work | fast-forward only; report §1, §13 | VERIFIED |
| REC-5 | Record baseline gates and pre-existing failures before modifying | report §1 (contaminated steps re-run on a clean worktree) | VERIFIED |
| REC-6 | Structured research comparison | `res` | PARTIAL — model facts are prior knowledge (Hugging Face/GitHub blocked here); crate facts verified |
| REC-7 | No large acquisition without approval | only crates.io metadata and sacreBLEU (scratch venv) | VERIFIED |

## 5. Architecture

| ID | Requirement | Implementation | Evidence | Status | Risk / disposition |
|---|---|---|---|---|---|
| ARC-1 | Contracts in the correct layer; no adapter types in core | `core`; adapter types in `tr::adapter` | inspection, compiles with `core` alone | VERIFIED | — |
| ARC-2 | Adapter API: config, capabilities, lifecycle, requests, results, cancellation, typed errors | `tr::adapter`, `check_setup` | P, W | VERIFIED | per-request cancellation is not preemptive (in-process adapters run to completion; deadline + stall detection cover it) |
| ARC-3 | Capability discovery: pairs, modes, streaming, context/terminology, limits, runtime needs | `TranslationCapabilities`; `translation_adapters()` | R discovery | VERIFIED | — |
| ARC-4 | Adapters replaceable without consumer change | trait | P/R with 4 adapter kinds | VERIFIED | — |
| ARC-5 | Not in audio callbacks or control plane | worker thread; non-blocking push | W, R (capture/recognition continue on failure) | VERIFIED | — |
| ARC-6 | Supervised workers, bounded queues, cancellation, backpressure, overload behaviour | `tr::worker` | W | VERIFIED | — |
| ARC-7 | Config fingerprints / versions | `TranslationConfig::fingerprint`, provenance | U config; P provenance | VERIFIED | — |
| ADP-1 | Deterministic fake adapter, no network/model/credentials | `tr::scripted` | P, W, R | VERIFIED | — |
| ADP-2 | Real adapter only if viable, licensed, isolated, approved, testable | — | ADR-0028 | **BLOCKED** | needs approval to download a model (recommended: OPUS-MT via CTranslate2, en↔es) |
| ADP-3 | Provider specifics outside neutral contracts | `memory` in `tr` only | inspection | VERIFIED | — |
| ADP-4 | No silent substitution of adapters, languages, privacy | explicit selection; fallback recorded | P (`fallback_from`), R | VERIFIED | — |
| ADP-5 | Document adapter-specific limits | memory capabilities and docs | `memory.rs` docs, `arch` | VERIFIED | — |
| SEL-1 | Selection respects pair, capability, privacy, context/streaming, config | `select_adapters`, `check_setup` | R (remote skipped, partials refused) | VERIFIED | — |
| SEL-2 | Unsupported request → explicit typed outcome | `UnavailableReason` | P | VERIFIED | — |
| SEL-3 | Fallback explicit, policy-controlled, recorded, testable | `fallback` list | P | VERIFIED | — |
| SEL-4 | No automatic external fallback without opt-in | `remote_not_allowed` at start | P, R | VERIFIED | — |

## 6. Contracts

| ID | Requirement | Implementation | Evidence | Status | Risk / disposition |
|---|---|---|---|---|---|
| REQ-1 | Session and stream identifiers | stream in results/events; session on the envelope | R (session asserted) | VERIFIED | — |
| REQ-2 | Source transcript and revision identity | `TranslationSource` | P, R | VERIFIED | — |
| REQ-3 | Source language: explicit/detected/uncertain with provenance only if upstream supplies it | `SourceLanguageRef`, `SourceLanguageProvenance` | P language test | VERIFIED | — |
| REQ-4 | Target and explicit direction | `target`, directed pairs | P, U | VERIFIED | — |
| REQ-5 | Context/domain profile id and version when used | `provenance.terminology` (`ProfileRef`) | P terminology | VERIFIED | — |
| REQ-6 | Request/config fingerprint, cancellation identity, bounded metadata | config fingerprint; utterance+sequence identity; bounded fields | P | VERIFIED | no separate request-id type (utterance + revision + sequence identify a request) |
| RES-1 | Source linkage incl. timing/frame range | `frame_range`, `time_range` | R | VERIFIED | — |
| RES-2 | Target language and adapter identity/version | result fields | P | VERIFIED | — |
| RES-3 | Text and explicit status | `TranslationState` + withdrawn/unavailable events | P, R | VERIFIED | — |
| RES-4 | Revision/sequence identity and ordering | `sequence` (gapless) | P supersede test | VERIFIED | — |
| RES-5 | Provenance without excessive sensitive detail | `TranslationProvenance` (counts, ids); `terms_applied` counts terms only for terminology-capable adapters, plus protected terms (audit fix D5) | P, P `audit_d5_terms_applied_reflects_the_adapter` | VERIFIED | — |
| RES-6 | Typed error/uncertainty/diagnostics | `UncertaintyFlag`, `UnavailableReason`, `ErrorReport` | P | VERIFIED | — |
| RES-7 | Timing/resource data | `TranslationTiming`, counters percentiles | P (fields), `eval` overhead | VERIFIED | no model timing (no model) |
| STA-1 | Provisional | `Provisional` | P full policy | VERIFIED | — |
| STA-2 | Stable/committed with stated revision rules | `Stable` + ADR-0029 | P committed policy | VERIFIED | — |
| STA-3 | Final | `Final` | P, R | VERIFIED | — |
| STA-4 | Superseded/withdrawn | supersession by sequence; `Withdrawn` | P, W, R | VERIFIED | — |
| STA-5 | Failed/incomplete explicit, never invented | `Unavailable`, `Failed` | P malformed/timeout/errors; E memory `no_coverage` | VERIFIED | — |
| IMM-1 | No change to transcript, LID, Phase 5 | clones only | P, R | VERIFIED | — |
| IMM-2 | Deterministic identity/sequence for repeats and revisions | revision rules, `sequence` | P, E (deterministic replay) | VERIFIED | — |
| IMM-3 | Duplicate/delayed/out-of-order events cannot corrupt state | rejection + coalescing never replaces newer | P, W | VERIFIED | — |
| IMM-4 | Translated text never confused with source | separate event type, `Content` class, docs | R | VERIFIED | — |

## 7. Language handling

| ID | Requirement | Implementation | Evidence | Status | Risk / disposition |
|---|---|---|---|---|---|
| LNG-1 | Source from user/Phase 4/adapter/unknown with provenance | `SourceLanguageProvenance` | P | VERIFIED | — |
| LNG-2 | Explicit overrides detected | `source_language` | P (disagreement flagged) | VERIFIED | — |
| LNG-3 | Uncertain stays uncertain | `UncertainSource` | P | VERIFIED | — |
| LNG-4 | Target explicit; no implicit target change | `no_target` start error | P, R | VERIFIED | — |
| LNG-5 | Capability-driven pairs; unsupported → explicit | `supports`, `UnsupportedPair` | P | VERIFIED | — |
| LNG-6 | Evaluate code-switching, dialects, mixed text, transliteration, scripts; implement only where evidenced | not implemented | `eval` limitations; `arch` "not supported" | DEFERRED | no adapter or evidence; needs a real MT system and a mixed-language test set |
| LNG-7 | No broad multilingual claim from few pairs | docs | `eval`, report | VERIFIED | — |

## 8. Context

| ID | Requirement | Implementation | Evidence | Status | Risk / disposition |
|---|---|---|---|---|---|
| CTX-1 | Optional typed hints (values, entities, refs, acts, profile) | final entity hints with normalized value | P, R | PARTIAL | only entities are used (references/acts not passed: no adapter consumes them yet) |
| CTX-2 | Bounded by size, age, privacy, relevance | `ContextPolicy`, stored-hint caps | P | VERIFIED | — |
| CTX-3 | Context never overrides source facts | conflict drop + flag; hints not used to edit text | P | VERIFIED | — |
| CTX-4 | Distinguish spoken from inferred | `same_utterance`; only final entities whose epistemic kind is `observation` or `user_confirmed_fact` are admitted (audit fix D3) | P `audit_d3_d4_hints_are_stated_and_follow_retraction` | PARTIAL | the hint itself carries no epistemic field (all admitted hints are stated). Before the audit, hypotheses and resolved interpretations were admitted too: the earlier row was inaccurate |
| CTX-5 | Contradictory context conservative and observable | `context_conflict` | P | VERIFIED | — |
| CTX-6 | No persistent personal memory | per-worker, bounded, cleared on reset; a retracted observation removes its hint (audit fix D4) | P reset, P `audit_d3_d4_…`, inspection | VERIFIED | — |
| PRE-1 | Deterministic precedence policy, documented | ADR-0029 | P (explicit vs detected, conflict) | VERIFIED | — |
| TRM-1 | Profile structure only as justified | `TerminologyProfile` (pair, domain, version, provenance, terms) | U | VERIFIED | — |
| TRM-2 | Scoped to pair, profile, version, provenance | profile fields; `terminology_mismatch` | P | VERIFIED | — |
| TRM-3 | Injection/application only when adapter capability permits | `terms` passed only if `terminology`; protection only if `placeholders` | P | VERIFIED | — |
| TRM-4 | Conflict, capitalization, inflection, untranslatable behaviour | conflicts rejected; `case_sensitive`; `do_not_translate`; inflection documented as unsupported | U, P | VERIFIED | inflection unsupported (documented) |
| TRM-5 | Tests for consistency and harmful over-application | whole-word longest-first | U `fidelity`, P | VERIFIED | — |
| TRM-6 | Industry packs not required | one hospitality profile | — | NOT APPLICABLE | — |

## 9. Fidelity

| ID | Requirement | Implementation | Evidence | Status | Risk / disposition |
|---|---|---|---|---|---|
| FID-1 | Preserve meaning, negation, questions, quantities, dates, names | protection of digits/DNT terms; screens; flags | P, E screens on oracle; `eval` | PARTIAL | enforcement depends on the adapter; CE detects digits/terms loss and screens negation/questions in evaluation only |
| FID-2 | No invented gender/identity/intent/emotion/facts | memory never composes; no inference | E (`no_coverage`, nothing invented) | PARTIAL | cannot be evaluated for a generative model until one exists |
| FID-3 | Preserve ambiguity where appropriate | not addressed by baselines | — | DEFERRED | needs a real MT system |
| FID-4 | Define idioms, slang, profanity, register, tone, formatting treatment | ADR-0029 / `arch`: no filtering, no rewriting; register item in corpus | docs | PARTIAL | policy defined; behaviour unmeasurable without MT |
| FID-5 | Translate, do not rewrite/summarize/answer/execute | memory returns stored translations only; docs | E | VERIFIED | — |
| FID-6 | No silent omission of difficult content | no filtering exists; unavailable is explicit | P | VERIFIED | — |
| FID-7 | Errors and uncertainty visible | flags, unavailable, degraded, counters | P | VERIFIED | — |

## 10. Streaming and revisions

| ID | Requirement | Implementation | Evidence | Status | Risk / disposition |
|---|---|---|---|---|---|
| STR-1 | Streaming only where adapter/contract support it | `partials_unsupported` | P | VERIFIED | — |
| STR-2 | Every output tied to source revision and target | result fields | R | VERIFIED | — |
| STR-3 | Partial behaviour, replacement, stable/segment commitment, finalization, withdrawal defined | ADR-0029, `arch` | P | VERIFIED | — |
| STR-4 | Source revisions supersede translations; stale not current | supersession + withdrawal + rejection; an utterance is marked finished only after its final was handled, so a processor failure during a final withdraws earlier results (audit fix D1) | P, W, P `audit_d1_failed_final_withdraws_earlier_results` | VERIFIED | — |
| STR-5 | Cancellation/reset/teardown invalidate pending work | reset, stop, cancel, session close; reset keeps per-utterance sequence counters, so post-reset results never reuse a withdrawn sequence; withdrawals are not repeated (audit fix D2) | P, W, R, P `audit_d2_reset_keeps_sequences_monotonic` | VERIFIED | a new worker (after failure or restart) still starts sequences at 0: consumers must key on the worker run (`Started`) as well |
| STR-6 | Race tests: revisions during translation, delays, cancels, resets, failures | coalescing under slow adapter; reset mid-stream; panic | W, R | VERIFIED | — |
| STR-7 | Measure first output, first useful output, stability delay, finalization delay | `first_result_ms`, queue/adapter ms | P fields; `eval` overhead | PARTIAL | measured only with scripted adapters; "first useful output" undefined without real MT quality |

## 11. Runtime, limits and failures

| ID | Requirement | Implementation | Evidence | Status | Risk / disposition |
|---|---|---|---|---|---|
| RUN-1 | Bound input, output, context, queue, concurrency, resources, pending jobs | budget, context caps, `max_concurrent`, 2× queue | P, W, R | VERIFIED | — |
| RUN-2 | Overload/degradation policy and diagnostics | drop-oldest-partial, `degraded` | W | VERIFIED | — |
| RUN-3 | Panic, malformed, timeout, unsupported, cancel, exhaustion, shutdown | supervisor, validation, deadline | P, W, R | VERIFIED | — |
| RUN-4 | No unsafe automatic retry | `max_retries = 0` default | P (call counts) | VERIFIED | — |
| RUN-5 | Retries bounded, justified, observable | ≤ 2, same adapter | P | PARTIAL | retries are counted in adapter calls/time, no dedicated retry counter |
| RUN-6 | No automatic external failover | remote refused | P, R | VERIFIED | — |
| RUN-7 | Shutdown cancels/drains per policy; unclean reported | drain on shutdown; stuck workers counted | R (`report.clean`) | VERIFIED | — |

## 12. Privacy and security

| ID | Requirement | Implementation | Evidence | Status | Risk / disposition |
|---|---|---|---|---|---|
| PRV-1 | Local-first, minimization | defaults | P, R | VERIFIED | — |
| PRV-2 | No external transmission without approval and opt-in | `allow_remote` | P, R | VERIFIED | — |
| PRV-3 | No credentials/keys/raw text in logs, reports, repo | runtime logs ids only | R `logs_health_and_metrics_carry_no_translation_content` (logs, health, metrics) and no content in non-result events | VERIFIED | — |
| PRV-4 | Redacted diagnostics; no persistent storage | `ErrorReport` redaction; no storage | inspection | VERIFIED | — |
| PRV-5 | Document adapter data handling, retention, network, licence | `memory.rs` docs, `res`, privacy.md | docs | VERIFIED | — |
| PRV-6 | Validate input size/config; protect against malformed output | budget, validation | P | VERIFIED | — |
| PRV-7 | No new credential/billing infrastructure | none | inspection | VERIFIED | — |

## 13. Observability and performance

| ID | Requirement | Implementation | Evidence | Status | Risk / disposition |
|---|---|---|---|---|---|
| OBS-1 | Trace revision, request, stream, adapter, pair without raw text | ids in events/logs | R | VERIFIED | — |
| OBS-2 | Queue delay, adapter time, first output, finalization, revisions, cancels, errors, overload | counters + percentiles | P, W (counters) | VERIFIED | — |
| OBS-3 | RTF/throughput; provider latency separate from engine overhead | `eval -- overhead`, worker throughput | `eval` performance table | PARTIAL | engine overhead only; no provider |
| OBS-4 | Memory/CPU for real adapters | — | — | BLOCKED | no real adapter (ADP-2) |
| OBS-5 | P50/P95/P99 only when supported | only for the overhead micro-benchmark | `eval` | VERIFIED | — |
| OBS-6 | Reproducible fingerprints and manifests | config fingerprint, memory fingerprint, corpus manifest | E | VERIFIED | — |
| OBS-7 | No real-device claims from synthetic measurements | report wording | report §10 | VERIFIED | — |

## 14. Evaluation

| ID | Requirement | Implementation | Evidence | Status | Risk / disposition |
|---|---|---|---|---|---|
| EVC-1 | Licensed/authorized data with provenance, languages, domains, counts, preprocessing | authored corpus | `fixtures/translation/PROVENANCE.md` | VERIFIED | — |
| EVC-2 | Distinct dev and held-out sets | splits | U leak test, E | VERIFIED | — |
| EVC-3 | No repeated tuning on held-out | memory from dev only; nothing tuned | E | VERIFIED | — |
| EVC-4 | Synthetic data cannot establish real quality | stated | `eval` | VERIFIED | — |
| EVC-5 | Record selection and exclusions; no large download | PROVENANCE | — | VERIFIED | — |
| MET-1 | BLEU/chrF with justified method | sacreBLEU-default BLEU, chrF2 | U, sacreBLEU parity 20/20 BLEU, 19/20 chrF | VERIFIED | — |
| MET-2 | Semantic metrics only after licence/model review | not used; reason in `res` | `res` | VERIFIED | — |
| MET-3 | Human evaluation when feasible | — | — | DEFERRED | nothing to review without a real MT system |
| MET-4 | Metric scores alone ≠ quality | stated | `eval` | VERIFIED | — |
| MET-5 | Pair-level, not pooled | pair tables | `eval` | VERIFIED | — |
| ERR-1 | Errors by pair, domain, length, source quality, context use | category/pair tables; memory coverage by category | `eval` | PARTIAL | length/source-quality slices trivial (coverage 0 on test) |
| ERR-2 | Meaning reversal, negation, omission, addition, entity, number, terminology, idiom, register, ambiguity | screens for numbers/negation/questions; flags | `eval` | PARTIAL | most categories need a real system |
| ERR-3 | Streaming/revision errors | lifecycle tests | P, W, R | VERIFIED | — |
| ERR-4 | Performance/resource errors | overhead, overload, stall tests | W, R, `eval` | VERIFIED | — |
| ERR-5 | Limitations and sample size stated | `eval` | — | VERIFIED | — |
| CMP-1 | Controlled comparison, engine overhead separate | same corpus/preprocessing for all systems; overhead measured separately | `eval` | PARTIAL | only baselines compared (no real candidates runnable) |

## 15–21. Tests, ledger, sequence, scope, deliverables, acceptance

| ID | Requirement | Evidence | Status | Risk / disposition |
|---|---|---|---|---|
| TST-1 | Contracts/config/capability validation; pairs; provenance/uncertainty | U, P | VERIFIED | — |
| TST-2 | Fake-adapter lifecycle | P, W | VERIFIED | — |
| TST-3 | Provisional/stable/final transitions, replacement/withdrawal | P | VERIFIED | — |
| TST-4 | Stale suppression, ordering, idempotency, cancel, reset, shutdown | P, W, R | VERIFIED | — |
| TST-5 | Context/terminology bounds, conflicts, privacy | P | VERIFIED | — |
| TST-6 | Failure injection: timeout, panic, malformed, unsupported, saturation, overload, partial output, cancellation | P, W, R | VERIFIED | — |
| TST-7 | Phase 4 and Phase 5 integration with upstream immutability | R | VERIFIED | — |
| TST-8 | Replay, held-out, reproducibility | E | VERIFIED | — |
| TST-9 | Cross-platform builds, MSRV, deny, docs, examples, Talktual gates | `verify.sh` all gates passed; MSRV 357/0; Talktual tsc, 549, 60, build | VERIFIED | cross-platform is compile-only |
| TST-10 | No fabricated results; commands and environment reported | report §6 | VERIFIED | — |
| LED-1 | Ledger with ids, locations, tests, evidence, status, risk, disposition | this file | VERIFIED | — |
| SEQ-A..H | Stages A–H | A–E, G, H done; F blocked | report | PARTIAL | stage F (real adapter) BLOCKED |
| SCO-1..5 | Scope control: no extra phases; deferred backlog; explicit non-compliance; approval for impactful changes; Phase 5 closed, Phase 7 future | report §12 backlog | VERIFIED | Phase 4/5 code touched only to forward events (hooks), and one Phase 2 tap API extended earlier in Phase 4 — no behaviour change |
| DEL-1..9 | Deliverables | report, this ledger, `eval`, ADRs, tests, docs, handoff | VERIFIED | — |
| ACC-1..12 | Acceptance criteria | report §14 | see report | — |

## Audit corrections (independent completeness audit)

An audit of the merged Phase 6 (`f3213e2`) found five defects. Each was
first reproduced with a regression test that failed, then fixed.

| Defect | Violates | Fix | Test |
|---|---|---|---|
| D1: a final marked its utterance finished before translation, so when translating it failed the processor (repeated errors, panic), `fail`/`finish` skipped the utterance and its earlier provisional or stable results stayed current | STR-4, STA-4 (stale results not current) | mark finished only after the final is handled | `audit_d1_failed_final_withdraws_earlier_results` |
| D2: `reset` cleared the utterance table, so later revisions of an open utterance restarted at sequence 0, which is at or below the withdrawn `through_sequence` (a consumer keeping the highest sequence ignores them) | STR-5, STA-4 | keep sequence counters; reset only revision state; deduplicate withdrawals | `audit_d2_reset_keeps_sequences_monotonic` |
| D3: hypotheses and resolved interpretations were admitted as hints | CTX-4 | admit only `observation`/`user_confirmed_fact` | `audit_d3_d4_…` |
| D4: a retracted Phase 5 observation stayed as a hint (stale context); and provisional observations could evict pending finals from the worker's 64-slot hint queue | CTX-2/CTX-6, bounded context | hints are keyed by observation id; a new version replaces and a retraction removes; the worker queues only final and retracted observations | `audit_d3_d4_…` (processor); the worker filter is covered by inspection only |
| D5: `terms_applied` counted terms found in the output even when the adapter had no terminology capability | RES-5 (provenance honesty) | count them only for terminology-capable adapters | `audit_d5_terms_applied_reflects_the_adapter` |

After the fixes: `scripts/verify.sh` all gates passed; tests 361 default
/ 365 all features / 361 on MSRV 1.85.0, 0 failed (was 357/361/357).
