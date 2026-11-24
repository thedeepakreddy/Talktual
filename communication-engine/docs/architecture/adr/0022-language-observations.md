# ADR-0022 — Language observations: scope, uncertainty and privacy

Status: Accepted (Phase 4) · Date: 2026-11-24

## Decision
* **Scope is explicit** (`LanguageScope`): `Utterance` (one final text),
  `Stream` (aggregate of decided utterances so far) and `Span` (character
  range within an utterance, **reserved — not produced in Phase 4**). The
  scopes are separate observations and never substituted for one another;
  a session-level aggregate is the host's to build from stream
  observations.
* **Provenance** on every observation: `source` (`Declared` from a `Fixed`
  hint — not inferred; `Recognizer`; `TextIdentifier {id, version}`;
  `Aggregate`), `evidence` (letters, or decided utterances), and a
  `fingerprint` of identifier + policy.
* **Uncertainty.** Candidates are ranked; scores appear only with declared
  `score_semantics` (whichlang has none). Abstention reasons:
  `InsufficientEvidence` (fewer than `min_letters` = 10 letters; set on the
  dev split: 0.95 → 1.00 accuracy at 86 % coverage), `LowConfidence`
  (score below `min_score`, only for identifiers with scores), `Ambiguous`
  (the two halves of the text, each with ≥ 10 letters, are identified as
  different languages — mixed-language evidence; or no stream majority),
  `Unsupported` (outside an `Expected` set). The stream aggregate needs
  `session_min_utterances` (2) decided utterances and a share of at least
  `session_min_share` (0.6), ties are ambiguous, and an event is emitted
  only when the decision changes.
* **Code-switching.** Span-level detection is not supported by the
  available identifiers with measurable quality, so
  `SpeechCapabilities::code_switching = false`. What *is* measured: the
  split-half check makes 7 of 8 held-out mixed items abstain as
  `Ambiguous` (costing 1 of 42 dev and 8 of 84 test single-language items
  in coverage; it can be switched off with `split_check = false`).
* **Privacy.** Observations describe the language of **text**, never the
  speaker: CE infers no nationality, ethnicity, identity, location,
  emotion, health or other trait, and does not analyse accent or voice for
  language. Language events are classified `Personal`, transcripts
  `Content` (ADR-0007); runtime logs never include transcript text or
  language decisions.

## Consequences
Hosts get language information they can trust to say "don't know". Real
transcripts (with recognition errors) will lower coverage and accuracy,
particularly on short utterances; this is unmeasured until real ASR runs.
