# Phase 6 report — Translation Intelligence

Branch `deepak/gallant-hamilton-wegi2i`, baseline `910b898` (main with
Phases 1–5 merged, PR #10). Date: 2026-11-24. Environment: Linux x86_64
cloud container, 4 vCPU, Rust 1.97.0 (MSRV 1.85.0 also run); **no audio
hardware, no GPU, no real MT model**.

Related: [ledger](phase-6-ledger.md) · [evaluation](phase-6-evaluation.md)
· [research](phase-6-research.md) · [architecture](../architecture/translation.md)
· ADR-0027, ADR-0028, ADR-0029 · [handoff](phase-6-handoff.md).

## 1. Baseline and repository state

* PR #10 (Phases 4–5) was merged by the user; the branch was
  fast-forwarded to `910b898` (no rewrite, no force push); the tree was
  clean, no submodules, no other user changes.
* Baseline gates on `910b898`, before Phase 6 code was relied on:
  fmt, clippy (`-D warnings`), tests **318 default / 322 all features, 0
  failed**, rustdoc (`-D warnings`), four smoke examples, Phase 3–5 replay
  gates — pass. The `verify.sh` run's last two steps were contaminated by
  Phase 6 contract edits made while it ran (it reported the new missing
  health fields); they were re-run on a **clean worktree of `910b898`**:
  5 cross-target checks pass; `cargo deny` pass. Talktual: tsc pass, 549
  unit, 60 server, build pass.
* Not run (need a local model directory): sherpa/bridge gates, browser
  e2e. No pre-existing failure was found.

## 2. Implementation summary

* **Contracts** (`ce-core::translation`): validated, fingerprinted
  `TranslationConfig`; `TranslationCapabilities` (directed pairs,
  locality, detection, partial input, incremental output, context,
  terminology, placeholders, limits, determinism, score semantics,
  model); `TranslationResult` linked to the exact source revision (stream,
  utterance, revision, revision status, characters, frame and time
  range), with source-language provenance, state, uncertainty flags,
  optional score with semantics, provenance and timing;
  `TranslationEvent` (started, result, withdrawn, unavailable,
  input_rejected, degraded, failed, stopped); status, counters,
  `TerminologyProfile`.
* **`ce-translate`** (no new third-party dependency): adapter boundary;
  `ce.translate.memory` (deterministic offline translation memory, never
  invents); scripted adapter with fault injection; terminology matching,
  placeholder protection and fidelity flags; processor (revision rules,
  partial policies, language policy, explicit selection and fallback,
  privacy, bounded hints, validation, supersession, withdrawal);
  supervised coalescing worker; evaluation (corpus, sacreBLEU-default
  BLEU, chrF2, screens, replay).
* **Runtime/facade**: transcript and Phase 5 observation forwarding
  (non-blocking, read-only clones); `EventPayload::Translation` with
  `Content`/`Operational` classes; stop ordering after recognitions and
  semantics; stall detection; `start_translation`, `stop_translation`,
  `reset_translation`, `translation_status`, `translation_adapters`;
  builder `translation_adapter`, `terminology_profile`; serializable
  commands.

## 3. Architecture and design decisions

See `architecture/translation.md`. Key decisions:
* one new crate mirroring Phase 5's structure (ADR-0027);
* finals-only by default; `committed`/`full` partial policies with
  gapless sequences, supersession and explicit withdrawal (ADR-0029);
* precedence: explicit config > source text > terminology > hints >
  adapter defaults; hints never rewrite text (ADR-0029);
* local-first: remote adapters need `privacy.allow_remote` and are never
  implicit fallbacks; no automatic retry by default (ADR-0028).

## 4. Adapter research and selection

`phase-6-research.md`, ADR-0028. Crate facts were verified on crates.io;
model facts are prior knowledge (Hugging Face/GitHub blocked here) and
must be verified before acquisition. **Real MT is BLOCKED pending
approval**; recommended first: OPUS-MT (Marian) via CTranslate2 (`ct2rs`,
MIT) as a separately built adapter, en↔es. NLLB-200 rejected
(non-commercial licence); hosted APIs out of scope.

## 5. Requirements ledger

`phase-6-ledger.md`: every bullet of the PDF has a row. Summary:

| Status | Rows (IDs) |
|---|---|
| BLOCKED | ADP-2 (real adapter), OBS-4 (real-adapter resources), SEQ stage F |
| DEFERRED | DEP-4 (Talktual bridge), LNG-6 (code-switching/dialects/transliteration), FID-3 (ambiguity), MET-3 (human evaluation) |
| PARTIAL | REC-6, CTX-1, CTX-4, FID-1, FID-2, FID-4, STR-7, RUN-5, OBS-3, ERR-1, ERR-2, CMP-1, SEQ |
| NOT APPLICABLE | TRM-6 |
| VERIFIED | all others |

## 6. Tests and exact results

| Gate | Command | Result |
|---|---|---|
| all CE gates | `scripts/verify.sh` | **all gates passed** (exit 0): fmt, clippy all features `-D warnings`, tests, rustdoc `-D warnings`, 4 smoke examples, Phase 3–6 replay gates, 5 cross targets, `cargo deny` |
| tests, default | `cargo test --workspace --all-targets` | 357 passed, 0 failed (baseline 318) |
| tests, all features | `cargo test --workspace --all-features` | 361 passed, 0 failed (baseline 322) |
| MSRV | `cargo +1.85.0 test --workspace --all-targets` | 357 passed, 0 failed |
| translation gate | `cargo test --release -p ce-translate --test evaluation` | in verify.sh |
| Talktual | `npx tsc --noEmit`, `npm test`, `npm run test:server`, `npm run build` | pass, 549 passed, 60 passed, pass |

New Phase 6 tests (all run): ce-core translation config/model (4),
ce-translate unit (8: metrics incl. sacreBLEU reference values,
fidelity, memory, corpus/leak), `tests/processor.rs` (11),
`tests/worker.rs` (7), `tests/evaluation.rs` (2),
`communication-engine/tests/translation_runtime.rs` (7). The first six
runtime tests were run 5× consecutively without failure (the seventh,
the no-content log test, was added last and run in the final gates).

Failures found and fixed during the phase (none pre-existing): the
oracle evaluation device keyed references by source only (caught by its
own 100-score sanity check); CE BLEU initially differed from sacreBLEU
(tokenizer and smoothing; now identical); a wrong shutdown-reason
expectation in a test (first stop request wins); an `eprintln!` and an
unescaped rustdoc link caught by the gates.

## 7. Evaluation methodology and metrics

`phase-6-evaluation.md`. Authored parallel corpus (`fixtures/translation`,
PROVENANCE.md): 92 items, dev 30 / held-out test 62, four directed pairs
plus an en→es fidelity set. Metrics: BLEU (sacreBLEU defaults; identical
to sacreBLEU 2.4.3 on 20/20 hypothesis sets), chrF2 (identical on 19/20,
one differs by 0.0005), coverage, digit/negation/question screens; pair
level only.

Results (held-out): `ce.translate.memory` covers **0/62** (every item
reported `no_coverage`, nothing invented); dev 30/30 exact (memory built
from dev). Identity lower bound: BLEU 0.7–2.4, chrF 16.5–18.1. Oracle:
100 on every pair (metric sanity). **No real translation quality is
established.**

## 8. Performance and resources

Engine overhead only (no model), release, this container: processor p50
5.8 µs / p95 7.8 µs per request (trivial adapter); 8,000 inputs through the
supervised worker in 236 ms. Bounded state: utterance table ≤ 256, hints
≤ 64 + 64 pending, queue ≤ 2 × `budget.queue`. No RTF, model memory or
latency figures exist (no model).

## 9. Known limitations and risks

1. No real MT; translation quality unknown (the main risk for Phase 7+).
2. Tiny authored corpus, single references, no human evaluation.
3. Terminology is not inflection-aware; screens are heuristic.
4. Hints use only final Phase 5 entities; no epistemic field on hints.
5. In-process adapters are not preempted; deadlines discard late results
   and the stall detector abandons hung workers.
6. Talktual bridge does not expose translation yet.

## 10. Hardware and cross-platform evidence

No hardware or device testing. Cross-platform evidence is compile-only
(iOS, Android, Windows MSVC, macOS, wasm32 for the non-runtime crates)
via `verify.sh`; MSRV 1.85.0 tests run.

## 11. Changed files

40 files since `910b898` (excluding this report and the final
commits' docs): ce-core translation contracts and wiring (`command`,
`config`, `event`, `health`, `lib`); new `crates/ce-translate`
(src, tests, example); runtime `core.rs`, `core/translation.rs`, forwarding
hooks in `core/speech.rs` and `core/semantic.rs`, `metrics.rs`, facade
`lib.rs`; `tests/translation_runtime.rs`; fixtures; ADR-0027–0029;
architecture, research, evaluation, ledger, report, handoff; README,
ARCHITECTURE, events.md, privacy.md; `verify.sh`.

## 12. Deferred backlog (future consideration points)

| Item | Why deferred | Consider at |
|---|---|---|
| Real MT adapter (OPUS-MT via CTranslate2, en↔es) | needs model download approval | as soon as approved; before Phase 7 relies on translation |
| Talktual bridge translation output | nothing meaningful to show without real MT | with the first real adapter |
| Code-switching, dialects, transliteration | no evidence or adapter | with a real MT system and a mixed test set |
| Human evaluation (meaning, negation, register, hallucination) | needs system outputs | with the first real adapter |
| Inflection-aware terminology | needs morphology resources | when a domain pack requires it |
| Reference/act hints | no adapter consumes them | when a context-capable adapter exists |

## 13. Git and PR status

Commits on `deepak/gallant-hamilton-wegi2i`: `230be0b` (checkpoint),
`daacc16` (tests, evaluation harness), and the final Phase 6 commit
(gate fixes, evaluation gate, documentation). All
end with "Built by Deepak and Claude". No PR opened (only on request).

## 14. Acceptance recommendation

| Criterion (PDF §20) | Status |
|---|---|
| Engine-neutral contracts and adapter lifecycle exist and are tested | met |
| Pair, direction, capability, uncertainty, provenance explicit | met |
| Source transcript and Phase 5 results immutable | met (tested) |
| Revision linkage, stale rejection, streaming/batch, cancel, reset, shutdown, failure verified | met |
| Context/domain hints bounded, optional, tested; source not rewritten | met |
| Fake adapter and replay/evaluation deterministic | met |
| Real adapter isolated, licensed, optional, evidenced, approval respected | **not met — blocked on approval** (approval respected) |
| Credible methodology, explicit limitations | met |
| Bounds, observability, privacy, failure behaviour documented and tested | met |
| All gates and Talktual regressions pass or recorded | met (§6) |
| Every requirement reconciled in the ledger | met |
| Report and handoff complete | met |

**Recommendation: accept Phase 6 as architecturally complete with one
blocked component.** Engine-side translation (contracts, lifecycle,
revision semantics, context, terminology, privacy, bounded execution,
evaluation infrastructure) is implemented and verified. Translation
**quality** is not established; do not treat Phase 6 as delivering
usable machine translation until a real adapter is approved, built and
evaluated with the existing harness. Phase 15 remains the
production/certification gate.
