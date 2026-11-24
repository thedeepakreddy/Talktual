# ADR-0028 — Translation adapters: baseline, real-MT candidates, selection

Status: Accepted (Phase 6) · Date: 2026-11-24

## Decision
* **Shipped adapters**: `ce.translate.memory` 1.0.0 — a deterministic,
  offline translation memory (exact match after normalization: lowercase,
  punctuation removed, whitespace collapsed; conflicting entries
  rejected; `NoCoverage` otherwise; never composes or guesses), and a
  scripted adapter for tests (fault injection). Neither is general machine
  translation; the memory is a phrasebook/domain-memory baseline.
* **Real MT adapter: BLOCKED pending approval** (model download). The
  candidates and what each needs are in `docs/phases/phase-6-research.md`.
  Recommended first candidate if approved: OPUS-MT (Marian) models run
  through CTranslate2 (`ct2rs`, MIT), in a separately built adapter under
  `adapters/` like `ce-asr-sherpa` (native dependency, own MSRV), per pair
  (en↔es first), int8 on CPU. Licences and sizes must be verified from the
  model cards before acquisition (the network policy here blocks
  Hugging Face metadata).
* **Rejected**: NLLB-200 (CC-BY-NC — non-commercial), hosted APIs (paid,
  credentials, transcript transfer — outside Phase 6 scope), Python stacks
  (Argos Translate) in-process.
* **Selection**: the configured adapter, else the first registered adapter
  that supports the pair and the privacy policy. **Fallback** only to
  adapters listed in `TranslationConfig::fallback`, tried in order when
  the previous one reports `unsupported_pair` or `no_coverage` (never on
  errors or timeouts); recorded in `provenance.fallback_from`. A `remote`
  adapter is refused at start unless `privacy.allow_remote` (so it can
  never be an implicit fallback). No silent substitution of adapters,
  languages or privacy modes.
* **Retries**: none by default (`budget.max_retries = 0`); at most 2 on
  the same adapter when configured; never across adapters.

## Evidence
Pair-level held-out results, error analysis and the sacreBLEU parity
check: `docs/phases/phase-6-evaluation.md`.
