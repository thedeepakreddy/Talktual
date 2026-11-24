# Phase 6 independent completeness and evidence audit

Audited state: `main` at `f3213e2` (merge of PR #11; same tree as
`f3b7cae`). Date: 2026-11-24. Environment: Linux x86_64 cloud container,
Rust 1.97.0 and 1.85.0. **No audio hardware, GPU or real MT model.**
This audit does not certify for production; Phase 15 is the
certification gate. Related: [ledger](phase-6-ledger.md) (corrected),
[report](phase-6-report.md), [evaluation](phase-6-evaluation.md).

## 1. Method

* Read the Phase 6 code (`ce-core::translation`, `ce-translate`, runtime
  `core/translation.rs` and its hooks), tests, fixtures, ADR-0027–0029
  and the Phase 6 docs, and compared them with the ledger.
* Diffed Phase 6 against the Phase 5 baseline `910b898` to look for
  regressions.
* Wrote a regression test for each suspected defect and **ran it before
  any fix** (all four failed). Fixed only Phase 6 code, then reran
  everything.
* No model download, real MT adapter, paid API, credentials, deployment
  or external transmission. Git history was not rewritten.

## 2. Defects found (identified before modifying code)

| ID | Defect | Specification violated | Reproduction (failed before the fix) | Fix |
|---|---|---|---|---|
| D1 | A Final revision set `finished = true` **before** translation. If translating it then failed the processor (consecutive-error limit returns `Err` before `unavailable`; or an adapter panic), `fail`/`finish` withdraws only unfinished utterances. That left the utterance's earlier provisional or stable results current. | Stale results must not remain current (STR-4, STA-4) | `audit_d1_failed_final_withdraws_earlier_results`: events were `result, degraded, failed, stopped`, with no withdrawal | Mark the utterance finished only after `translate` returns `Ok`. A panic unwinds before the flag is set. |
| D2 | `reset()` cleared the utterance table, so a later revision of an open utterance restarted at sequence 0. That is at or below the withdrawn `through_sequence`, and consumers keep the highest sequence (handoff rule), so the new result was invisible or wrongly dropped. | Reset semantics and supersession by sequence (STR-5, STA-4) | `audit_d2_reset_keeps_sequences_monotonic`: "sequence 0 reuses a withdrawn number" | Keep sequence counters; reset only revision state. Withdrawals are deduplicated (`withdrawn_at`), so finish after reset does not withdraw twice. |
| D3 | `hint_from` admitted any final entity, including Phase 5 `hypothesis` and `resolved_interpretation` observations. The ledger said the opposite. | Distinguish spoken from inferred (CTX-4); ledger accuracy | `audit_d3_d4_…`: hints were `[eight, linked, maybe, seven]` | Admit only `observation` and `user_confirmed_fact`. |
| D4 | A Phase 5 correction cascade retracts observations, but the stored hint stayed (stale context). Also, the worker queued every observation, so provisional ones could evict pending finals from its 64-slot queue. | Context is bounded and current (CTX-2, CTX-6) | same test: retracted `eight` was still passed | Hints are keyed by observation id: a newer version replaces the old one and a retraction removes it. The worker queues only Final and Retracted observations. |
| D5 | `terms_applied` counted terminology terms found in the output even when the adapter had no terminology capability (for example `ce.translate.memory`). | Honest provenance (RES-5) | `audit_d5_…`: `terms_applied` was 1, expected 0 | Count those terms only for terminology-capable adapters; protected terms still count. |

The worker filter in D4 is covered by inspection, not by a dedicated
worker test.

Files changed: `crates/ce-translate/src/processor.rs`,
`crates/ce-translate/src/worker.rs`, `crates/ce-translate/tests/processor.rs`
(4 tests), `docs/phases/phase-6-{ledger,report,audit}.md`. No public API,
contract, config or event schema changed.

## 3. Requirement audit table

Statuses are my independent verdicts. Rows the ledger already marks
PARTIAL, BLOCKED or DEFERRED were re-checked and agree unless a row
says otherwise. "Tests" names test files that were run in this audit.

| Requirement ID | Requirement | Actual implementation | Evidence | Status | Defect or limitation | Recommended disposition |
|---|---|---|---|---|---|---|
| MIS-1 / ARC | Engine-neutral translation contracts and adapter boundary | `ce-core::translation`; `TranslationAdapter` and `TranslationFactory` in `ce-translate` | U, P; contracts are additive (no deletions in `ce-core` against `910b898`) | VERIFIED | — | accept |
| MIS-2 / TST-7 | Source transcripts and Phase 5 outputs are immutable | processor takes `&TranscriptRevision`; runtime forwards clones | P `transcripts_and_observations_are_not_mutated`; R | VERIFIED | — | accept |
| REQ / RES | Results linked to the exact source revision, with state, flags, score semantics and provenance | `TranslationResult`, `TranslationSource`, `TranslationProvenance` | P, R | VERIFIED | `terms_applied` was overstated (D5) | fixed |
| STA-1..4 | Provisional, stable, final, withdrawn and unavailable | `TranslationState`; `Withdrawn`, `Unavailable` | P, W | VERIFIED | D1 and D2 left stale or invisible results | fixed |
| STR-1..6 | Partial policies, supersession, cancel, reset, races | `PartialPolicy`; revision rules; coalescing worker | P, W, R | VERIFIED | — | accept |
| STR-7 | First output, stability and finalization delay | `first_result_ms`, queue and adapter ms | P fields; overhead run | PARTIAL | no finalization-delay metric; scripted adapters only | keep PARTIAL; measure with a real adapter |
| LNG-1..5 | Explicit or detected source, disagreement flag, same-language, unsupported pair | `SourceLanguagePolicy`, `UncertaintyFlag` | P | VERIFIED | — | accept |
| LNG-6 | Code-switching, dialects, transliteration | none | — | DEFERRED | no adapter or evidence | backlog |
| SEL / fallback | Explicit selection; fallback only through the configured list | `adapters` vector; `fallback_from` | P | VERIFIED | — | accept |
| CTX-1 | Typed hints | final entity hints only | P | PARTIAL | references and acts not passed | keep PARTIAL |
| CTX-2 / CTX-6 | Bounded, current context with no persistence | 64 stored + 64 pending; hints cleared on reset | P, W | VERIFIED (after fix) | a retracted hint stayed; provisional observations could evict pending finals (D4) | fixed |
| CTX-4 | Distinguish spoken from inferred | stated-only admission | P | PARTIAL | inferences were admitted (D3); the hint has no epistemic field | fixed; field still absent |
| — | Hint availability | hints present when the final is translated | inspection | limitation | timing-dependent: a later entity is not used | documented |
| TRM-1..5 | Terminology profiles, longest-first whole-word matching, placeholder protection | `fidelity.rs`; `TerminologyProfile` | U, P | VERIFIED | not inflection-aware | documented |
| FID-1,2,4 | Number, negation, question and term screens | `fidelity`; eval screens | U, E | PARTIAL | heuristic only | keep PARTIAL |
| FID-3 | Ambiguity | none | — | DEFERRED | — | backlog |
| RUN-1..4 | Supervised worker, bounded queue, coalescing, overload | `worker.rs` | W (7) | VERIFIED | — | accept |
| RUN-5 / ERR-1,2 | Deadline, hang and panic handling | deadline discards late results; stall detector; `catch_unwind` | W, R | PARTIAL | in-process adapters are not preempted; a new worker restarts sequences at 0 | keep PARTIAL; document for consumers |
| — | Utterance-table bound | 256 entries; evicts the oldest finished entry, else the oldest | inspection | limitation | more than 256 open utterances lose state | documented |
| PRV-1..7 | Local-first operation; `allow_remote` defaults to false; no content in logs; redaction | config defaults; ids-only logs | R `logs_health_and_metrics_carry_no_translation_content`; P | VERIFIED | — | accept |
| OBS-1..3 | Tracing, counters and status without raw text | counters, reservoirs, health | P, R | VERIFIED (OBS-3 PARTIAL as in the ledger) | — | accept |
| OBS-4 | Real-adapter resources | — | — | BLOCKED | no real model | wait for approval |
| ADP-1 | Deterministic offline adapter | `ce.translate.memory` 1.0.0 (never invents) | U, E | VERIFIED | — | accept |
| ADP-2 | Real MT adapter | not implemented; research recommends OPUS-MT via CTranslate2 | `phase-6-research.md` | BLOCKED | needs approval to download a model | wait for approval (respected) |
| EVC / MET | Corpus, metrics, methodology | 92 authored items (dev 30 / test 62); memory built from dev; sacreBLEU-default BLEU, chrF2 | E (pinned gate); sacreBLEU 2.4.3 parity 20/20 BLEU, 19/20 chrF | VERIFIED as infrastructure | synthetic and tiny; single references; no real system was evaluated | see §5 |
| MET-3 | Human evaluation | — | — | DEFERRED | needs system outputs | backlog |
| DEP-4 | Talktual bridge | not extended | — | DEFERRED | — | backlog |
| GATE | All gates pass | `scripts/verify.sh` | §4 | VERIFIED | — | accept |
| LEDGER | Every requirement reconciled | `phase-6-ledger.md` | this audit | VERIFIED (after correction) | the CTX-4 row was inaccurate | corrected |

## 4. Verification results

These are from runs I executed and whose output I observed, after the
fixes:

| Gate | Command | Result |
|---|---|---|
| Regression tests before the fix | `cargo test -p ce-translate --test processor audit` | **4 failed** (D1, D2, D3/D4, D5), as expected |
| ce-translate | `cargo test -p ce-translate` | 8 + 2 + 15 + 7 passed, 0 failed |
| All CE gates | `scripts/verify.sh` | **all gates passed**, exit 0. Covers fmt, clippy (`-D warnings`, all features), tests, rustdoc (`-D warnings`), smoke examples, the Phase 3–6 replay and evaluation gates, cross-target checks and `cargo deny` (duplicate-crate warnings only, as at baseline). |
| Tests, default features | `verify.sh` step | **361 passed, 0 failed** (357 before the audit) |
| Tests, all features | `verify.sh` step | **365 passed, 0 failed** (361 before) |
| MSRV | `cargo +1.85.0 test --workspace --all-targets` | **361 passed, 0 failed** |

Not run in this audit: Talktual (`tsc`, npm tests, build). The fixes
touch only `ce-translate`, which Talktual does not consume (the bridge
was not extended). Also not run: the sherpa and bridge gates and browser
e2e, which need a local model.

## 5. Evidence quality

* **Real vs synthetic.** All translation behaviour is exercised with
  scripted adapters and the translation memory. No real MT output exists.
  Performance figures (p50 5.8 µs) are engine overhead with a trivial
  adapter, not translation latency.
* **Leakage.** The memory is built from dev only and covers 0/62 test
  items, so held-out scores cannot reflect memorised test items. Metric
  parity with sacreBLEU was checked against test references. That
  validates the metric, not a system, and tunes nothing.
* **Sample size.** 62 held-out items over 4 pairs (about 15 per pair),
  with single references. That is enough for a regression gate, not for
  quality claims. The evaluation report already says so.
* **Identity and oracle bounds** (BLEU 0.7–2.4, and 100) are sanity
  checks only.

## 6. Architecture and regression review

* Phase 1–5 changes against `910b898` are additive only: `ce-core`
  enums, events and health gained variants or fields with no deletions.
  The speech and semantic runtime modules gained forwarding hooks and a
  select arm. `ce-audio`, `ce-audio-intel`, `ce-speech`, `ce-semantic`
  and the adapters are unchanged. All Phase 3–5 replay gates pass.
* Scope: no real MT adapter or model and no new third-party dependency
  (cargo deny passes). The audit fixes are confined to `ce-translate`.
* Concurrency: forwarding is non-blocking (`push` or `observe` into
  bounded queues). The worker is supervised (`catch_unwind`, heartbeat).
  The D4 filter reduces queue pressure.
* Dependency, licence, MSRV and platform: no new dependencies. MSRV
  1.85.0 tests pass. The cross-target checks (iOS, Android, Windows,
  macOS, wasm32 for non-runtime crates) are compile-only; there is no
  device evidence.

## 7. Git state

* `main` = `origin/main` = `f3213e2`. The working branch
  `deepak/gallant-hamilton-wegi2i` was fast-forwarded to `f3213e2` (no
  rewrite). The audit fixes are committed on it.
* The remote working branch had been deleted after PR #11 merged. Pushing
  the audit commit recreates it. No PR was opened.
* `claude/home-wave-choreography` and `claude/kind-archimedes-n6y11p`
  still exist on origin. Both are fully merged into main. This session
  cannot delete them (HTTP 403); delete them in the GitHub UI if wanted.

## 8. Outstanding issues

1. No real MT adapter (BLOCKED on model-download approval), so
   translation quality is unknown.
2. STR-7, CTX-1, CTX-4 (no epistemic field on hints), FID screens and
   RUN-5/ERR-1/2 (no adapter preemption) remain PARTIAL.
3. Sequences restart at 0 in a new worker run. Consumers must reset
   per-utterance state on `Started`.
4. Hint use is timing-dependent; the utterance table is bounded at 256.
5. The worker-level D4 filter has no dedicated test.
6. Talktual regressions were not rerun in this audit (not affected).

## 9. Recommendation

**ACCEPT WITH LIMITATIONS.** The engine-side translation architecture
is complete and verified after five correctness fixes. That covers
contracts, revision semantics, lifecycle, bounded execution, privacy and
evaluation infrastructure. Phase 6 does not deliver usable machine
translation: the real adapter is blocked on approval and no quality has
been measured. This is not a production certification.
