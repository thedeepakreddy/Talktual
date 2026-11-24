# Semantic replay corpus — provenance

`corpus.jsonl` (schema `ce.semantic.corpus.v1`) and `profiles/home.json`.

| Split | Dialogues | Utterances | Source | Use |
|---|---|---|---|---|
| `dev` | 14 | 39 | authored for CE | rule development and error analysis |
| `test` | 15 | 45 | authored for CE | held out: scored, never used to change rules |
| `asr` | 10 | 10 | real recognizer output (see below) | held out: scored, never used to change rules |

## Sources

* **`authored`** — utterances written for CE.
  * Style: upper case without punctuation, like the Phase 4 recognizer.
  * Gold: entity categories, mentions and normalized values; referring
    expressions with the expected outcome; one act label per utterance, or
    `null` where abstention is expected (greetings, fragments).
  * Contains no person's speech or personal data. Names, numbers and
    amounts are invented.
* **`asr-real`** — the final hypotheses of the real sherpa-onnx streaming
  Zipformer INT8 model (Phase 4 baseline, ADR-0023) for the ten synthetic
  (CMU Flite TTS) speech fixtures in `fixtures/speech/`. They are copied
  verbatim from the table in `docs/phases/phase-4-real-asr.md`.
  * One utterance carries the real partial revisions from the same
    document.
  * The TTS input text (the intended words) is in each utterance's `note`.
  * Gold is annotated on the hypothesis text. A value the speaker said but
    the recognizer lost (e.g. "tomorrow" → "TO MO") is a gold entity with
    `mention: null`, which always counts as a false negative.
  * Synthetic speech: no person's voice.

## Annotation conventions

* **Entities.** Only the categories `ce.semantic.rules` defines and the
  profile's categories are annotated. Locations, names and organizations
  are not annotated (no supported category), even when present.
* **References.** The gold target is the *annotated entity* the expression
  denotes. When it denotes something that is not an annotated entity (an
  event, an object without a category), the gold outcome is `unresolved`.
* **Splits.** Fixed when the corpus was committed, before any scoring run.
  The test and asr splits were written in one pass, before the dev run, and
  are not used to change rules (see `docs/phases/phase-5-evaluation.md`).
* **Licence and intended use.** Repository licence. For evaluating CE
  semantic adapters only. This is a small, synthetic, English sample:
  results describe this corpus, not general quality.

## Identity

`sha256sum fixtures/semantic/corpus.jsonl fixtures/semantic/profiles/home.json`
at commit time:

```
afa77e9fd08fb8e48d3c832330cb5cb817e89394709dc2d254a80fb595c52527  corpus.jsonl
12b8377e9b27b255ed54048c3ba380ffc385143962b8cdd7cd9065a6a0f18653  profiles/home.json
```

The evaluator also prints an FNV-1a 64 identity of each file.
