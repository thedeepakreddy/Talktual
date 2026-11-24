# ADR-0029 — Translation revisions, context precedence and privacy

Status: Accepted (Phase 6) · Date: 2026-11-24

## Revision and streaming semantics
* Inputs of an utterance must increase in revision: older → `stale`,
  equal → `duplicate`, after final/cancel → `after_final`; reported, never
  translated. The worker coalesces queued partials (a newer one replaces
  an older; an older never replaces a newer).
* `PartialPolicy`: `off` (default; finals only, batch), `committed`
  (partials' committed text → `stable` results, only when it grew),
  `full` (whole snapshot → `provisional`). Partial policies require an
  adapter that declares `partial_input` (else start fails).
* Results of an utterance have gapless `sequence`s; a newer result
  supersedes earlier ones. `stable` results may be superseded by longer
  stable results and by the final; the final re-translates the whole
  utterance and may differ (a translated prefix is not a prefix of the
  translation). No adapter declares incremental output in Phase 6.
* **Withdrawal**: cancellation of the source utterance, reset, stop before
  the final, or an untranslatable final withdraws earlier results
  (`withdrawn{through_sequence}`), so stale output never remains current.
* A result later than `deadline_ms` is discarded (`unavailable{timeout}`),
  never presented.

## Language handling
Explicit source (config) > Phase 4 utterance decision (with provenance) >
adapter detection (only if declared; flagged `detected_source_language`).
An abstained Phase 4 decision with a non-detecting adapter is
`unavailable{uncertain_source}` — never guessed. A disagreeing Phase 4
decision under an explicit source is flagged. Same source and target →
`unavailable{same_language}`. Code-switching, dialects and transliteration
are not handled (no evidence; see the evaluation).

## Context precedence
Explicit configuration > source text > terminology constraints > Phase 5
hints > adapter defaults. Hints are final Phase 5 entities of the stream,
bounded (`max_hints`, `max_hint_chars`, `max_utterance_distance`), passed
only to adapters that declare `context_hints`. A same-utterance hint whose
mention is not in the source is dropped and flagged `context_conflict`.
Hints never rewrite source text. No persistent memory is introduced.

## Terminology
Profiles are scoped to a directed pair and domain, versioned, with
provenance; one source term has one target (conflicts rejected);
whole-word, longest-first matching (guard against over-application);
`do_not_translate` terms and digit runs are protected by `⟦n⟧`
placeholders only for adapters that declare `placeholders`; lost
placeholders are flagged, never re-inserted by guessing; missing target
terms are flagged `term_missing`. Inflection is not handled.

## Privacy
Local-first: default adapters are in-process; remote needs explicit
opt-in. Results are `Content`; other events carry ids, sizes and reasons
only. No transcript or translation is logged, persisted or transmitted
by CE (events, logs, health and metrics are tested for content).
