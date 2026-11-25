# Phase 7 requirements ledger — Streaming Orchestration & Timing Intelligence

Source: *Communication Engine — Phase 7, Streaming Orchestration & Timing
Intelligence* (PDF, authoritative). Every bullet has an ID (section
prefix + number).

**Status**:
* VERIFIED: evidence exists — a test that was run, a recorded measurement
  or a document. Code alone does not count.
* PARTIAL, MISSING, BLOCKED, DEFERRED, NOT APPLICABLE and NOT TESTED.

**Locations**:
* `core`: `crates/ce-core/src/orchestration/`.
* `orc`: `crates/ce-orchestrate/src/`.
* `rt`: `crates/communication-engine/src/runtime/core/pipeline.rs` and the
  generation tagging in `core/*.rs`.
* `fac`: the facade, `crates/communication-engine/src/lib.rs`.

**Tests**:
* `U`: unit tests in `ce-orchestrate`.
* `O`: `ce-orchestrate/tests/replay.rs`, scripted replay.
* `R`: `communication-engine/tests/orchestration_runtime.rs`.
* `C`: ce-core contract unit tests.

**Docs**: `arch` is `architecture/orchestration.md`; ADR-0030 to
ADR-0033.

**Counts**:
* `U`: 15 unit tests in `ce-orchestrate` plus 4 ce-core contract tests.
* `O`: 7 replay tests.
* `R`: 11 runtime tests.
* 1 fairness unit test in `core.rs` (`scheduling_tests`).
* `W` refers to the existing Phase 5–6 worker tests.

## 2. Goals

| ID | Requirement | Implementation | Evidence | Status | Disposition |
|---|---|---|---|---|---|
| GOL-1 | Explicit configurable execution relationships | `PipelineSpec`, `orc::graph` dependency rules | U graph; R start | VERIFIED | — |
| GOL-2 | Stream/session/utterance/frame/event correlation and causality | observer correlation by (stream, utterance, revision), envelope session/correlation ids, generation and epoch | U causal; R | VERIFIED | — |
| GOL-3 | Timestamp provenance, clock domains, offsets, uncertainty, discontinuities | `orc::timeline` (media time, reported monotonic, offset, segment, uncertainty) | U timeline; O | VERIFIED | uncertainty is `unknown` unless the audio layer reports a bound |
| GOL-4 | Partial/final revisions and downstream updates | `orc::reconcile`, causal check | U, O | VERIFIED | — |
| GOL-5 | Bounded queues, backpressure, concurrency, deadline and cancellation policies | existing queues; forwarders (64); `max_pipelines`; abandoned-worker cap (AUD-01); deadline policy; stop modes | U, R, `orchestration_corrections` | VERIFIED | core-channel saturation not forced (see EVL-6) |
| GOL-6 | Lifecycle, reset, restart, shutdown, stale rejection, recovery | pipeline state machine; generations; epochs | U, R | VERIFIED | — |
| GOL-7 | Optional modules, partial pipelines, explicit degraded operation | required/optional; `stage_degraded` | U, R | VERIFIED | — |
| GOL-8 | Privacy-safe metrics, diagnostics, correlation | operational events and status (ids, counts, durations) | R privacy test | VERIFIED | — |
| GOL-9 | Control-plane work off real-time audio callbacks | the observer runs on the core task; audio callbacks untouched | inspection (no `ce-audio` change) | VERIFIED | — |
| GOL-10 | Deterministic replay, scripted adapters, fault injection | O replay with scripted events; R with scripted adapters and faults | O, R | VERIFIED | — |

## 3. Non-goals

| ID | Requirement | Evidence | Status |
|---|---|---|---|
| NG-1 | No new ASR/translation/semantic/VAD/TTS/voice/diarization/acoustic models | diff: no adapter or model added | VERIFIED |
| NG-2 | No Phase 8–11 capabilities, memory, UI, SDK, hosted API, billing, deployment | diff | VERIFIED |
| NG-3 | No unapproved downloads, paid APIs, external transfer | no network code; `cargo deny`; no new third-party dependency | VERIFIED |
| NG-4 | No rewrite of accepted contracts | additive contracts only (see CMP-6) | VERIFIED |
| NG-5 | No unrelated refactoring | diff review | VERIFIED |

## 4. Principles

| ID | Requirement | Implementation | Evidence | Status | Disposition |
|---|---|---|---|---|---|
| PRI-1 | Orchestration is control-plane, not callback work | core task only | inspection | VERIFIED | — |
| PRI-2 | Modules independently replaceable and supervised | stages remain separate workers; orchestration calls existing start/stop | R | VERIFIED | — |
| PRI-3 | Explicit typed contracts | `ce-core::orchestration` | C | VERIFIED | — |
| PRI-4 | Bounded queues, concurrency, state | bounds listed in `arch` §5 | U bounds; R long run | VERIFIED | — |
| PRI-5 | Async ops have lifecycle and cancellation | stop drain/cancel, bounded timeouts, forwarder cancellation | R | VERIFIED | — |
| PRI-6 | Timestamp provenance never discarded or rewritten | the observer only reads; media time added next to the reported times | U | VERIFIED | — |
| PRI-7 | Late/stale results rejected or handled observably | generation admission; causal classes; deadline outcomes | U, R | VERIFIED | — |
| PRI-8 | Explicit, verified failure isolation and recovery | degraded/failed transitions; restart | R | VERIFIED | — |
| PRI-9 | Config validated and fingerprinted without secrets | `validate`, `fingerprint` | U, C | VERIFIED | — |
| PRI-10 | Privacy-first; synthetic evidence is not real-world evidence | report §evidence | report | VERIFIED | — |

## 5. Reconnaissance

| ID | Requirement | Evidence | Status |
|---|---|---|---|
| REC-1 | Inspect repo, branch, Git, main, reports, contracts, ADRs, modules, tests, scripts | report §1; `arch` §1 | VERIFIED |
| REC-2 | Map event flow, IDs, sequence/revision, timestamps, queues, overflow, supervision, cancel, reset, shutdown | `arch` §1 | VERIFIED |
| REC-3 | Review the Phase 6 handoff (Started resets state; worker-local sequences) | `arch` §1, §4; U translation causal tests | VERIFIED |
| REC-4 | Identify reusable contracts and gaps without duplication | `arch` §1 "Gaps found" | VERIFIED |
| REC-5 | Baseline gates and ledger | report §1 (clean worktree of `8c92318`) | VERIFIED |
| REC-6 | Propose the architecture first; stop if the tree has unexplained changes | `arch` and this ledger written before code; the tree was clean | VERIFIED |

## 6. Orchestration graph

| ID | Requirement | Implementation | Evidence | Status | Disposition |
|---|---|---|---|---|---|
| GRA-1 | Explicit dependencies, no implicit global order | `orc::graph::Graph` from the stage rules | U | VERIFIED | — |
| GRA-2 | Validate missing deps, incompatibilities, invalid inputs, cycles before startup | `validate`; runtime pre-flight (stream, adapters, target, running stages, capacity) | U; R `invalid_pipelines…`, `a_required_stage_failing_at_start…` | VERIFIED | start-time-only failures (worker capacity) roll back, tested |
| GRA-3 | Minimal/linear and optional branched pipelines | any subset honouring the dependencies | U; R recognition-only and full | VERIFIED | — |
| GRA-4 | Deterministic dispatch and completion semantics | Kahn order with declaration tie-break; start in order, stop in reverse or upstream-first | U order; `arch` §3 | VERIFIED | — |
| GRA-5 | Fingerprint graph and config | `ValidatedGraph::fingerprint` | U | VERIFIED | — |
| GRA-6 | No hot reconfiguration; changes at safe boundaries | changes apply at `restart` only | `arch` §2; R restart | VERIFIED | — |

## 7. Execution stages

| ID | Requirement | Implementation | Evidence | Status | Disposition |
|---|---|---|---|---|---|
| STG-1 | Ingress and correlation | signals admitted per (stream, generation); session and correlation from the stream | R | VERIFIED | — |
| STG-2 | Temporal handling | observer timeline | U, O | VERIFIED | — |
| STG-3 | Capability dispatch | existing forwarding (transcripts to semantics and translation; observations to translation) | R (Phase 5–6 tests unchanged) | VERIFIED | — |
| STG-4 | Asynchronous result collection without blocking ingress | per-worker forwarders; non-blocking pushes | R | VERIFIED | — |
| STG-5 | Causal validation of epoch, stream, utterance, revision, event context | generation admission and observer | U, R | VERIFIED | — |
| STG-6 | Downstream dispatch by explicit policy | forwarding rules documented in `arch` §2 | docs | VERIFIED | — |
| STG-7 | Minimal bounded state reconciliation | reconcile table (256) | U bounds | VERIFIED | — |
| STG-8 | Lifecycle/recovery and observability | pipeline events and status | R | VERIFIED | — |

## 8. Temporal

| ID | Requirement | Implementation | Evidence | Status | Disposition |
|---|---|---|---|---|---|
| TMP-1 | Audit and reuse existing timestamp contracts | `arch` §1; reuse of `frame_index`, `time_range`, `Discontinuity` | docs | VERIFIED | — |
| TMP-2 | Document origin, clock domain, units, precision, monotonic/wall, capture vs arrival, transformations | `arch` §4 and the timing table in `phase-7-timing.md` | docs | VERIFIED | — |
| TMP-3 | Preserve originals alongside normalized values | `NormalizedSpan` keeps the reported times | U | VERIFIED | — |
| TMP-4 | Normalize only when defensible; document assumptions and uncertainty | media time is exact; the monotonic mapping is reported, not computed; uncertainty is unknown | U; docs | VERIFIED | — |
| TMP-5 | Detect out-of-order, duplicate, clock jumps, gaps, resets, source changes, discontinuities | timeline and reconcile classes | U, O | VERIFIED | a device change appears as a stream invalidation (Phase 2), not to the observer |
| TMP-6 | Never rewrite history, invent timing or imply synchronization | read-only observer; unknown timing flagged | U | VERIFIED | — |
| TMP-7 | Monotonic time for durations, deadlines, latency | `Clock::monotonic`, `Instant` | inspection; U | VERIFIED | — |

## 9. Causality, correlation, revisions

| ID | Requirement | Implementation | Evidence | Status | Disposition |
|---|---|---|---|---|---|
| CAU-1 | Correlate stream/session, utterance, audio range, output, sequence, revision | observer keys; envelope ids | U, R | VERIFIED | — |
| CAU-2 | Causal references, not arrival order | checks use the source (utterance, revision) of each output | U | VERIFIED | — |
| CAU-3 | Respect ASR partial/stable/revision/final semantics | reconcile follows ADR-0021 | U | VERIFIED | — |
| CAU-4 | Reconcile downstream state on revisions; no duplicate append | `superseded`/`duplicate` classes; Phase 5–6 replacement unchanged | U | VERIFIED | — |
| CAU-5 | Reject stale outputs after stop/reset/restart by generation | admission by generation on every stage signal | U epoch; R restart; **`stale_output_runtime`** (feature `test-hooks`): end to end through a replaced or abandoned worker's real sender, forwarder, admission and publication, including 20 signals interleaved with live output (AUD-03) | VERIFIED | scope: the test sends the late signal through the worker's real sender; the worker thread producing one itself (the natural race) was not forced |
| CAU-6 | Translation Started resets consumer state; sequences are not global identity | observer translation view | U | VERIFIED | — |
| CAU-7 | Semantic/translation outputs identify their source revision | existing `SourceRef` and `TranslationSource`; checked | U | VERIFIED | — |
| CAU-8 | Explicit dedup, idempotency, stale policy | `arch` §3–§4 | docs; U | VERIFIED | — |

## 10. Scheduling, deadlines, backpressure

| ID | Requirement | Implementation | Evidence | Status | Disposition |
|---|---|---|---|---|---|
| SCH-1 | Bound concurrency, queues, state, tasks and threads | `max_pipelines`; one forwarder per worker; **abandoned-worker cap** `max_abandoned_workers` with live accounting and typed refusal (AUD-01); existing caps | U; R; `aud01_*` (cap, concurrency, recovery, lifecycle); R1 | VERIFIED | bound = cap − 1 + workers running when the cap is reached (documented). Before the audit this row was VERIFIED without that bound: AUD-01 showed unbounded growth |
| SCH-2 | No starvation among eligible streams; justify priority | unbiased data select; control priority (ADR-0033) | U fairness (`next_data`); `aud04_…` (a saturated stage does not stop another stream); audit R4 (command latency under a burst) | PARTIAL | scheduler fairness while a stage's *core* channel is saturated was not forced end to end (AUD-04) |
| SCH-3 | CPU-heavy work off ingress/callback paths | unchanged | inspection | VERIFIED | — |
| SCH-4 | Per-stage capacity and overflow behaviour | `arch` §5 table | docs | VERIFIED | — |
| SCH-5 | Critical lifecycle/control/final events never silently dropped | finals are never coalesced (Phase 4–6); stale rejections reported | R | VERIFIED | — |
| SCH-6 | Coalesce partials only when revision semantics remain safe | unchanged Phase 5–6 coalescing | existing W tests | VERIFIED | — |
| SCH-7 | Report rejection, coalescing, shedding, overflow, degradation | existing counters plus orchestration counters | R | VERIFIED | — |
| SCH-8 | Queue wait separate from processing | translation `queued_ms`/`adapter_ms`; observer overhead | U | VERIFIED | — |
| SCH-9 | Monotonic deadlines; typed, observable outcomes | `DeadlinePolicy`, `deadline_missed` | U; R | VERIFIED | — |
| SCH-10 | Late results never mutate superseded or expired state | revision rules; generation rejection | U | VERIFIED | — |
| SCH-11 | Bounded retries, explicit idempotency, no silent retry/fallback | none added | inspection | VERIFIED | — |

## 11. Lifecycle

| ID | Requirement | Implementation | Evidence | Status | Disposition |
|---|---|---|---|---|---|
| LIF-1 | Explicit, validated transitions | `PipelineState::can_transition_to` | C | VERIFIED | — |
| LIF-2 | Validate deps/config before startup | graph validation and pre-flight, **also before restart teardown** (AUD-02) | R; `aud02_*` | VERIFIED | — |
| LIF-3 | Stop: drain, cancel or abandon with bounded timeout | `StopMode`; per-stage outcomes | R | VERIFIED | — |
| LIF-4 | Reset without corrupting ids or correlation | reset in order; utterance and sequence continuity | R | VERIFIED | — |
| LIF-5 | Propagate cancellation safely | stop order; forwarder cancellation | R | VERIFIED | — |
| LIF-6 | Distinguish required from optional failures | `Requirement` | U, R | VERIFIED | — |
| LIF-7 | Optional failure gives explicit degraded mode; required failure stops | runtime handling | R | VERIFIED | — |
| LIF-8 | Restart gives a new epoch/generation; prior results rejected | epoch, generations | R; `stale_output_runtime` | VERIFIED | — |
| LIF-9 | Shutdown reports unclean termination honestly; no restart loops | existing shutdown report; no auto-restart | R | VERIFIED | — |
| LIF-10 | Recovery observable and verified | restart after failure | R | VERIFIED | — |

## 12. Failure isolation

| ID | Requirement | Implementation | Evidence | Status | Disposition |
|---|---|---|---|---|---|
| FAI-1 | Classify invalid config, unavailable capability, timeout, overload, cancellation, worker failure, protocol violation, stale event, temporal error, shutdown failure | `OrchestrationFault` classes | C | VERIFIED | — |
| FAI-2 | Fault-isolate optional modules | degraded mode | R | VERIFIED | — |
| FAI-3 | Emit a reason for degraded operation | `stage_degraded{reason}` | R | VERIFIED | — |
| FAI-4 | Test saturation, delays, hangs, cancellation, restart, out-of-order, recovery | O, R | O, R | VERIFIED | — |
| FAI-5 | Prevent memory/task leaks and runaway retries | bounded tables; abandoned-worker cap (AUD-01); forwarders end with workers; no retries | U; R; `aud01_*`; R1 | VERIFIED | hung threads persist until they exit (not killable); they are bounded by the cap |

## 13. Privacy

| ID | Requirement | Evidence | Status |
|---|---|---|---|
| PRV-1 | Bounded, ephemeral state | U bounds; observer dropped with the pipeline | VERIFIED |
| PRV-2 | No full audio/transcripts for diagnostics | the observer stores ids, ranges and counts | VERIFIED |
| PRV-3 | Redaction, minimization, correlation ids | operational events; R privacy test | VERIFIED |
| PRV-4 | No external upload | no network code | VERIFIED |
| PRV-5 | Clear state at lifecycle boundaries | observer cleared on stop and restart | VERIFIED |
| PRV-6 | No persistent history | none | VERIFIED |

## 14. Compatibility and configuration

| ID | Requirement | Implementation | Evidence | Status | Disposition |
|---|---|---|---|---|---|
| CMP-1 | Validate before activation | graph and pre-flight | R | VERIFIED | — |
| CMP-2 | Report unavailable or incompatible capabilities explicitly | typed errors (`orchestration.*`) | R | VERIFIED | — |
| CMP-3 | No model/provider names hard-coded in core | adapters resolved via the existing registries | inspection | VERIFIED | — |
| CMP-4 | Compatibility considers contracts, versions, config, resources | stage rules; adapter resolution; capacity | U, R | PARTIAL | adapter contract versions are not compared (one contract version exists) |
| CMP-5 | Fingerprint without secrets | U | U | VERIFIED | — |
| CMP-6 | Preserve behaviour; contract changes need analysis and an ADR | additive contracts; behaviour change (stale rejection, fair select) in ADR-0031/0033 | ADR | VERIFIED | — |

## 15. Observability

| ID | Requirement | Evidence | Status |
|---|---|---|---|
| OBS-1 | Graph/session lifecycle and epochs | R events | VERIFIED |
| OBS-2 | Dispatch/completion/failure/timeout events | stage events; stop outcomes | VERIFIED |
| OBS-3 | Queue wait, processing and end-to-end latency | status latency summaries | VERIFIED |
| OBS-4 | Stale/rejected/duplicate events and ordering violations | counters and events | VERIFIED |
| OBS-5 | Backpressure, coalescing, drop and rejection counts | existing plus new counters | VERIFIED |
| OBS-6 | Timeline normalization, discontinuities, uncertainty | timeline status | VERIFIED |
| OBS-7 | Recovery, degraded mode and shutdown outcomes | events | VERIFIED |
| OBS-8 | Concurrency/resource counts | pipelines, `worker_forwarders`, `abandoned_workers`, `starts_refused_abandoned` | VERIFIED |
| OBS-9 | Trace/correlation ids and config fingerprints | envelope ids; fingerprint | VERIFIED |
| OBS-10 | No sensitive payloads in diagnostics | R privacy test | VERIFIED |

## 16. Evaluation

| ID | Requirement | Evidence | Status |
|---|---|---|---|
| EVL-1 | Graph validation and causal correctness | U, O | VERIFIED |
| EVL-2 | Temporal mapping, uncertainty, discontinuities | U, O | VERIFIED |
| EVL-3 | Revision, stale/duplicate, restart correctness | U, O, R | VERIFIED |
| EVL-4 | Multi-stream isolation and fairness | R multi-stream; `aud04_…`; U fairness | PARTIAL: isolation verified; core-channel saturation fairness not forced |
| EVL-5 | Cancellation, deadlines, lifecycle, recovery | R | VERIFIED |
| EVL-6 | Backpressure under sustained and burst load | `aud04_…` (sustained two-stream load, blocked stage: bounded backlog, coalescing, no refused inputs, recovery); `tagged_sender_is_bounded…` (forwarder backpressure) | PARTIAL: core-channel saturation not forced |
| EVL-7 | Fault injection, long-duration and repeated lifecycle | R: panics, hangs, missing adapters, 20 rounds; O: 2,000 utterances | PARTIAL: no long wall-clock run |
| EVL-8 | Bounded queues, state, memory, tasks | U, R | VERIFIED |
| EVL-9 | Latency: ingress-to-dispatch, queue wait, module duration, overhead, end-to-end | `phase-7-evaluation.md`: observer overhead isolated (121–137 ns) and in the runtime (p50 1.0–1.2 µs, p95 2.6–3.0 µs, release, wall clock); translation queue and adapter times; end-to-end only on the manual test clock | PARTIAL: no real-clock end-to-end, no real model or hardware |
| EVL-10 | Repeat tests; variance, environment, workload | evaluation | VERIFIED |
| EVL-11 | Separate synthetic from real evidence | evaluation and report | VERIFIED |

## 17. Required test matrix

| ID | Requirement | Evidence | Status |
|---|---|---|---|
| TST-1 | Unit: graph, timeline, causal, transitions, deadlines, reconciliation | U, C | VERIFIED |
| TST-2 | Scripted integration and replay | O, R | VERIFIED |
| TST-3 | Out-of-order, duplicate, stale | U, O | VERIFIED |
| TST-4 | Stale results after reset, stop, restart | U, R | VERIFIED |
| TST-5 | Translation Started and sequence reset | U, O | VERIFIED |
| TST-6 | Queue saturation, overflow, backpressure, coalescing | `aud04_…`; `tagged_sender_is_bounded…`; existing Phase 3–6 tests | VERIFIED (stage level; see EVL-6) |
| TST-7 | Concurrent multi-stream isolation | R | VERIFIED |
| TST-8 | Cancellation, timeout, hung worker, shutdown | R | VERIFIED |
| TST-9 | Clock discontinuity, gaps, unknown timing | U, O | VERIFIED |
| TST-10 | Long-duration, repeated lifecycle, resource bounds | R 20 rounds with forwarders back to 0; O bounded tables | PARTIAL: repeated lifecycle and bounds verified; no long-duration run |
| TST-11 | Cross-platform compile and repository gates | `verify.sh` | VERIFIED |
| TST-12 | Talktual checks if affected | the bridge is not changed; checks run anyway (report §6) | VERIFIED |

## 18. Deliverables

| ID | Deliverable | Location | Status |
|---|---|---|---|
| DOC-1 | Implementation report | `phase-7-report.md` | VERIFIED |
| DOC-2 | Traceability ledger | this file | VERIFIED |
| DOC-3 | Architecture and graph semantics | `arch` | VERIFIED |
| DOC-4 | Timestamp, clock-domain and timeline contracts | `phase-7-timing.md` | VERIFIED |
| DOC-5 | Lifecycle, recovery, degraded mode | `arch` §3 | VERIFIED |
| DOC-6 | Configuration/compatibility guide | `arch` §2; `phase-7-timing.md` §config | VERIFIED |
| DOC-7 | Metric definitions | `phase-7-evaluation.md` §metrics | VERIFIED |
| DOC-8 | Evaluation with evidence provenance | `phase-7-evaluation.md` | VERIFIED |
| DOC-9 | ADRs | ADR-0030 to ADR-0033 | VERIFIED |
| DOC-10 | Tests and fixtures | U, O, R | VERIFIED |
| DOC-11 | Handoff | `phase-7-handoff.md` | VERIFIED |
| DOC-12 | Git commits/PR and closure evidence | report §11 and §13; audit and corrections committed | PARTIAL — the verification audit and closure are pending |

## 19. Acceptance criteria

The full acceptance matrix is in report §acceptance. ACC-18 (independent
audit and closure) is **pending** by design.

## 20. Stop conditions and 25. Risks

The stop conditions were respected (report §scope). The risk register is
addressed in report §limitations.

## Audit corrections

The Phase 7 audit (`phase-7-audit.md`, `cf250b8`) returned REQUIRES
CORRECTIONS. The rows above were updated for AUD-01 to AUD-15, and the
audit's §13 gives each finding's disposition.

Counts after the corrections:
* `R` also covers `orchestration_corrections.rs` (7 tests) and
  `stale_output_runtime.rs` (2 tests, feature `test-hooks`).
* `O` has 9 replay tests.
* Plus the `tagged_sender` unit test.
* `orchestration_audit.rs` (5 tests) holds `#[ignore]`d reproductions.

