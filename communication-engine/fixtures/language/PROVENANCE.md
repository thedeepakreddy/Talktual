# Language-identification corpus — provenance

`lid_corpus.json` (schema `ce.lid-corpus.v1`) holds 134 short
conversational sentences used to choose and evaluate the Phase 4 text
language identifier (ADR-0020) and its abstention policy (ADR-0022).

| Property | Value |
|---|---|
| Origin | **Authored for the Communication Engine** in Phase 4 (2026-11-24). Original text; no third-party corpus, website, book or dataset was copied. |
| People | No personal data. Sentences are generic travel / everyday phrases; any names are absent. |
| Languages | de, en, es, fr, it, nl, pt (7), plus 8 mixed-language items |
| Label method | The label is the language the sentence was **written in** (authoring intent). Mixed items carry character spans `[start, end, language]`, also by authoring intent. No annotator agreement study; labels were not independently verified by native speakers. |
| Length classes | `short` (≲ 3 words), `medium`, `long` (≳ 8 words), assigned at authoring |
| License | Same as the Communication Engine repository |

## Splits

| Split | Items | Use |
|---|---|---|
| `dev` | 42 (7 languages × 6; 14 short, 24 medium, 4 long) | Identifier selection and the `min_letters = 10` evidence threshold; the split-half check was judged on dev only |
| `test` | 92 (7 × 12 single-language: 28 short, 31 medium, 25 long; plus 8 mixed) | Held out; only reported (and used, with thresholds set below the reported values, by `ce-speech/tests/evaluation.rs`) |

**Leakage disclosure.** When candidate identifiers were first probed (a
scratch crate outside the workspace), the probe printed test-split numbers
alongside the dev numbers. The selection is supported by dev alone (see
ADR-0020), and all thresholds were set on dev only, but the test split was
seen once before the choice was final. Treat the test numbers as mildly
optimistic.

## Limitations

* Small (≈ 12 test items per language); differences of one or two items
  are noise.
* Written text, not ASR output: no recognition errors, no disfluencies,
  standard orthography and punctuation. Language identification on real
  transcripts will be worse, especially on short utterances.
* Seven European languages only; the identifier names 16 languages but the
  other nine are not evaluated.
* Mixed items are written code-switching with a single switch point; real
  code-switching is more varied.
