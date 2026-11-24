# Translation fixtures — provenance

All text here was written for the Communication Engine; no third-party
corpus is included. Licence: same as the repository.

| File | Contents | Construction |
|---|---|---|
| `corpus.json` | 92 parallel items: 72 general items in 4 directed pairs (en→es, es→en, en→fr, en→de; 18 per pair: 6 dev, 12 test) and 20 en→es fidelity items (negation, numbers, questions, names, terminology, correction, register; 6 dev, 14 test); `dev` 30 / `test` 62 | General items are aligned **by construction** from `fixtures/language/lid_corpus.json`: the same meaning was authored in each language, so item k of one language is a translation of item k of another. Some pairs are loose equivalents rather than literal translations (e.g. "No problem" / "De nada"), which lowers reference-based scores of correct alternatives. Fidelity items were authored for Phase 6. |
| `memory.json` | 30 translation-memory segments | The `dev` split of `corpus.json` only (never `test`; a test checks that no test source is in the memory). |
| `terminology-hospitality-en-es.json` | 5 terms (one `do_not_translate`) | Authored for Phase 6 tests. |

Splits: `dev` is used for tuning and for building the memory; `test` is
held out and only reported. References are single, authored by one
writer (no inter-annotator agreement), so they reflect one acceptable
rendering, not all.

Limitations: tiny (62 held-out items), conversational/hospitality only,
four pairs, no human evaluation of system outputs (no real MT system was
available to evaluate).
