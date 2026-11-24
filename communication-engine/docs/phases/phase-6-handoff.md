# Phase 6 handoff — Translation Intelligence (IN PROGRESS)

Kept current during Phase 6 so work can resume after an interruption. The
Phase 6 PDF is authoritative; this file records state, decisions and
remaining work.

## Standing user rules

* End commit messages and PR bodies with **`Built by Deepak and Claude`**;
  never put session links or model names in commits, PRs, code or docs.
* Push only to `deepak/gallant-hamilton-wegi2i`; PR only when asked.
* No large model/dataset downloads, paid APIs, credentials, deployments,
  external transmission of transcripts, or destructive git without approval.
* Never claim a test passed unless it was run; no hardware exists here.

## Baseline (main `910b898`, Phases 1–5 merged via PR #10)

Recorded before Phase 6 changes, on clean code:
* `scripts/verify.sh` steps fmt, clippy, tests (318 default / 322 all
  features), docs, smoke examples, Phase 3–5 replay gates: pass.
* Its last two steps were contaminated by Phase 6 edits made while it ran
  (missing new health fields). Re-run on a clean worktree of `910b898`:
  5 cross-target checks pass; `cargo deny` pass.
* Talktual: tsc pass, 549 unit tests, 60 server tests, build pass.
* Not run: sherpa/bridge gates and browser e2e (need a local model dir).

## Research and decisions so far

* Real MT candidates need model downloads (approval required):
  OPUS-MT/Marian via CTranslate2 (`ct2rs` 0.11.3, MIT), candle Marian
  (`candle-transformers` 0.11.1, MIT/Apache-2.0), Bergamot (C++, no Rust
  crate on crates.io), NLLB-200 (non-commercial licence — reject), M2M-100
  / MADLAD (large), hosted APIs (paid, credentials, external transfer —
  out of scope). Hugging Face and GitHub metadata are blocked by the
  network policy here, so model licences/sizes are from prior knowledge
  and must be verified before acquisition. **Real MT adapter: BLOCKED
  pending approval.**
* Implemented baseline: `ce.translate.memory` — deterministic offline
  translation memory (exact normalized segment match, `NoCoverage`
  otherwise, never invents), plus a scripted adapter with fault injection.
* sacreBLEU 2.4.3 (Apache-2.0) installed only in a scratch venv to
  cross-check CE's BLEU/chrF (not a workspace dependency).

## Done (checkpoint commit)

* `ce-core::translation` (config, model incl. terminology profile),
  wired into events (`EventPayload::Translation`, topic `Translation`),
  commands (`StartTranslation`, `StopTranslation`, `ResetTranslation`,
  `QueryTranslation`, `ListTranslationAdapters`), `EngineConfig::translation`,
  metrics and health.
* `ce-translate`: adapter, scripted, memory, fidelity (terms, protection,
  checks), processor, worker, metrics, eval (corpus, BLEU, chrF, fidelity
  screens, replay).
* Runtime `runtime/core/translation.rs`; transcripts and Phase 5
  observations forwarded (read-only copies); stop ordering after
  semantics; facade `start_translation`, `stop_translation`,
  `reset_translation`, `translation_status`, `translation_adapters`;
  builder `translation_adapter`, `terminology_profile`.
* Fixtures: `fixtures/translation/{corpus.json, memory.json,
  terminology-hospitality-en-es.json}`.
* Tests: ce-core translation (config/model), ce-translate unit (8) and
  `tests/processor.rs` (11). Workspace: 341 passed, 0 failed; clippy clean.

## Remaining

1. Runtime integration tests (facade + fake audio + scripted ASR + Phase 5
   hints; immutability; stop/reset/shutdown; stall; concurrency; overload).
2. Worker tests (coalescing, saturation, panic, stall, drain policy).
3. Evaluation example + report (cross-check BLEU/chrF with sacreBLEU; pair
   tables; error analysis; TM coverage; oracle/identity bounds).
4. ADRs 0027–0029, `docs/architecture/translation.md`, research doc,
   evaluation doc, requirements ledger, Phase 6 report; README/
   ARCHITECTURE; verify.sh; MSRV/cross/deny; Talktual gates; commit/push.
