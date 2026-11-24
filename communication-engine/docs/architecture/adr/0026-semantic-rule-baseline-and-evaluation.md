# ADR-0026 — Deterministic rule baseline and replay evaluation for semantics

Status: Accepted (Phase 5) · Date: 2026-11-24

## Context
Phase 5 needs at least one practical semantic capability end to end
without model downloads, paid services or credentials, all of which are
unapproved. It also needs reproducible evaluation that does not claim
more than the data supports. Recognizer output is upper case without
punctuation, and real ASR errors split words ("TO MO" for "tomorrow").

## Decision
* **Baseline adapter.** `ce.semantic.rules` 1.0.0: English rules for
  spoken numbers, quantities, money, percentages, times, dates, relative
  days, weekdays, durations, digit sequences and profile terms; scoped
  reference resolution; and the optional `ce.acts` 1.0.0 taxonomy with
  abstention. In process and deterministic. Certainty is a rule tier,
  never a probability.
* **Corpus.** `fixtures/semantic/corpus.jsonl`:
  * `dev`: 14 dialogues, 39 utterances;
  * `test`: 15 dialogues, 45 utterances, authored in the same pass as dev
    and held out;
  * `asr`: the 10 real sherpa-onnx hypotheses of the Phase 4 synthetic
    speech fixtures.

  It was committed before any scoring run.
* **Process.**
  1. Rules were written.
  2. The corpus was committed.
  3. The dev split was scored and its errors fixed, with generic rules
     only (no word added because it appears in a test item).
  4. The adapter was frozen and committed.
  5. The test and asr splits were then scored once.

  Their scores are pinned in `crates/ce-semantic/tests/evaluation.rs`.
* **Metrics.**
  * Entity P/R/F1 per category, with exact-span, overlap and normalized
    matching, at two certainty thresholds.
  * Reference accuracy, resolution precision, abstention rate and a
    confusion matrix.
  * Act macro-F1, coverage, selective accuracy, abstention recall and a
    confusion matrix.
  * Streaming counts and synthetic-timeline latency.
  * Robustness replays: duplicated and stale partials, and finals only.
  * Throughput and memory.
* **Payload tag.** `ObservationPayload` is tagged `type`, because the
  relation and reference payloads have their own `kind`-like fields. A
  round-trip test covers every payload.

## Consequences
* Results describe a small, synthetic, English corpus written by the same
  team that wrote the rules. They are not a general quality claim.
* ML NER (GLiNER/ONNX) and hosted LLM adapters stay deferred until the
  user approves downloads or paid use. The contract and locality policy
  already support them.
* Any rule change changes the pinned scores. It must be re-evaluated on
  dev only and re-recorded with the evaluation document.
