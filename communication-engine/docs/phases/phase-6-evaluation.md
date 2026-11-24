# Phase 6 evaluation — methodology, provenance, results, error analysis

Date: 2026-11-24 · Linux x86_64 container (4 vCPU), Rust 1.97.0, release
builds. **Evidence class: replay on authored text through the real
processor.** No real machine-translation system was evaluated (blocked on
model approval, ADR-0028); no audio, hardware or human evaluation. These
numbers describe this corpus and these baselines only.

Reproduce:

```bash
cargo run --release -p ce-translate --example evaluate_translation -- test
cargo run --release -p ce-translate --example evaluate_translation -- dev
cargo test --release -p ce-translate --test evaluation     # pinned gate
```

## Corpus and splits

`fixtures/translation/` (PROVENANCE.md): 92 authored parallel items —
72 general conversational items in four directed pairs (en→es, es→en,
en→fr, en→de; 6 dev + 12 test each) and 20 en→es fidelity items
(negation, numbers, questions, names, terminology, correction, register;
6 dev + 14 test). `dev` (30) builds the translation memory and was used
for development; `test` (62) is held out. A test asserts that no test
source is in the memory. Single reference per item, one author.

## Systems compared

| System | What it is | Why it is here |
|---|---|---|
| `ce.translate.memory` 1.0.0 | exact normalized segment match over the dev-only memory | the shipped offline baseline |
| identity | returns the source text | lower bound (what "no translation" scores) |
| oracle | returns the reference | metric sanity check (must be 100) |

Identity and oracle are evaluation devices, not translation systems.

## Metrics

* **BLEU** — corpus-level, sacreBLEU defaults (13a tokenization,
  case-sensitive, 4-grams, brevity penalty, exp smoothing).
* **chrF** — chrF2 (character 1–6-grams, β = 2, whitespace removed).
* **Coverage** — items translated / items; the rest are typed
  `unavailable` outcomes (here `no_coverage`), never empty or invented text.
* **Screens** (automatic, conservative): digits preserved, negation cue
  agreement (small per-language cue lists), question mark preserved. They
  catch some errors; passing them proves nothing.
* "covered" columns score only translated items; "all" columns score
  every item with an empty output where unavailable.

### Metric implementation check (sacreBLEU 2.4.3)

CE's metrics were compared with sacreBLEU 2.4.3 (`corpus_bleu`,
`corpus_chrf`, defaults) on the held-out references with identity outputs
and three deterministic perturbations of the references (last word
dropped, first word moved to the end, lowercased) plus the oracle:
**BLEU identical on 20/20 hypothesis sets (4 decimals); chrF identical on
19/20, the remaining one (en-es identity) differs by 0.0005.**

| hypotheses | CE BLEU | sacreBLEU BLEU | CE chrF | sacreBLEU chrF |
|---|---|---|---|---|
| en-de.identity.hyp | 0.6891 | 0.6891 | 16.8285 | 16.8285 |
| en-es.identity.hyp | 2.3812 | 2.3812 | 18.0898 | 18.0903 |
| en-fr.identity.hyp | 0.7824 | 0.7824 | 16.4882 | 16.4882 |
| es-en.identity.hyp | 0.7912 | 0.7912 | 16.9666 | 16.9666 |
| en-de.drop.hyp | 82.3633 | 82.3633 | 84.0644 | 84.0644 |
| en-es.drop.hyp | 79.2633 | 79.2633 | 79.7375 | 79.7375 |
| en-fr.drop.hyp | 82.9029 | 82.9029 | 79.2133 | 79.2133 |
| es-en.drop.hyp | 81.8731 | 81.8731 | 82.2235 | 82.2235 |
| en-de.swap.hyp | 87.5765 | 87.5765 | 91.9450 | 91.9450 |
| en-es.swap.hyp | 85.3317 | 85.3317 | 92.6177 | 92.6177 |
| en-fr.swap.hyp | 85.3738 | 85.3738 | 92.9295 | 92.9295 |
| es-en.swap.hyp | 86.5296 | 86.5296 | 93.1153 | 93.1153 |
| en-de.lower.hyp | 35.8512 | 35.8512 | 77.3161 | 77.3161 |
| en-es.lower.hyp | 80.1049 | 80.1049 | 95.3200 | 95.3200 |
| en-fr.lower.hyp | 81.7836 | 81.7836 | 96.2698 | 96.2698 |
| es-en.lower.hyp | 78.1283 | 78.1283 | 93.6450 | 93.6450 |
| en-de.oracle.hyp | 100.0000 | 100.0000 | 100.0000 | 100.0000 |
| en-es.oracle.hyp | 100.0000 | 100.0000 | 100.0000 | 100.0000 |
| en-fr.oracle.hyp | 100.0000 | 100.0000 | 100.0000 | 100.0000 |
| es-en.oracle.hyp | 100.0000 | 100.0000 | 100.0000 | 100.0000 |

## Results — held-out test split

| pair | adapter | items | covered | BLEU (covered) | chrF (covered) | BLEU (all) | chrF (all) | exact | number err | negation err | question err | unavailable |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| en-de | ce.translate.memory | 12 | 0/12 | — | — | 0.0 | 0.0 | 0 | 0 | 0 | 0 | NoCoverage:12 |
| en-de | identity | 12 | 12/12 | 0.7 | 16.8 | 0.7 | 16.8 | 0 | 0 | 2 | 0 | — |
| en-de | oracle | 12 | 12/12 | 100.0 | 100.0 | 100.0 | 100.0 | 12 | 0 | 0 | 0 | — |
| en-es | ce.translate.memory | 26 | 0/26 | — | — | 0.0 | 0.0 | 0 | 0 | 0 | 0 | NoCoverage:26 |
| en-es | identity | 26 | 26/26 | 2.4 | 18.1 | 2.4 | 18.1 | 0 | 0 | 4 | 0 | — |
| en-es | oracle | 26 | 26/26 | 100.0 | 100.0 | 100.0 | 100.0 | 26 | 0 | 0 | 0 | — |
| en-fr | ce.translate.memory | 12 | 0/12 | — | — | 0.0 | 0.0 | 0 | 0 | 0 | 0 | NoCoverage:12 |
| en-fr | identity | 12 | 12/12 | 0.8 | 16.5 | 0.8 | 16.5 | 0 | 0 | 2 | 0 | — |
| en-fr | oracle | 12 | 12/12 | 100.0 | 100.0 | 100.0 | 100.0 | 12 | 0 | 0 | 0 | — |
| es-en | ce.translate.memory | 12 | 0/12 | — | — | 0.0 | 0.0 | 0 | 0 | 0 | 0 | NoCoverage:12 |
| es-en | identity | 12 | 12/12 | 0.8 | 17.0 | 0.8 | 17.0 | 0 | 0 | 1 | 0 | — |
| es-en | oracle | 12 | 12/12 | 100.0 | 100.0 | 100.0 | 100.0 | 12 | 0 | 0 | 0 | — |

| pair/category | items | covered |
|---|---|---|
| en-de/general | 12 | 0 |
| en-es/correction | 1 | 0 |
| en-es/general | 12 | 0 |
| en-es/names | 2 | 0 |
| en-es/negation | 3 | 0 |
| en-es/numbers | 3 | 0 |
| en-es/question | 2 | 0 |
| en-es/register | 1 | 0 |
| en-es/terminology | 2 | 0 |
| en-fr/general | 12 | 0 |
| es-en/general | 12 | 0 |
Translation-memory coverage by category (test):

| pair/category | items | covered |
|---|---|---|
| en-de/general | 12 | 0 |
| en-es/correction | 1 | 0 |
| en-es/general | 12 | 0 |
| en-es/names | 2 | 0 |
| en-es/negation | 3 | 0 |
| en-es/numbers | 3 | 0 |
| en-es/question | 2 | 0 |
| en-es/register | 1 | 0 |
| en-es/terminology | 2 | 0 |
| en-fr/general | 12 | 0 |
| es-en/general | 12 | 0 |

## Results — development split (memory built from it)

| pair | adapter | items | covered | BLEU (covered) | chrF (covered) | BLEU (all) | chrF (all) | exact | number err | negation err | question err | unavailable |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| en-de | ce.translate.memory | 6 | 6/6 | 100.0 | 100.0 | 100.0 | 100.0 | 6 | 0 | 0 | 0 | — |
| en-de | identity | 6 | 6/6 | 1.7 | 13.9 | 1.7 | 13.9 | 0 | 0 | 0 | 0 | — |
| en-de | oracle | 6 | 6/6 | 100.0 | 100.0 | 100.0 | 100.0 | 6 | 0 | 0 | 0 | — |
| en-es | ce.translate.memory | 12 | 12/12 | 100.0 | 100.0 | 100.0 | 100.0 | 12 | 0 | 0 | 0 | — |
| en-es | identity | 12 | 12/12 | 0.9 | 13.7 | 0.9 | 13.7 | 0 | 0 | 1 | 0 | — |
| en-es | oracle | 12 | 12/12 | 100.0 | 100.0 | 100.0 | 100.0 | 12 | 0 | 0 | 0 | — |
| en-fr | ce.translate.memory | 6 | 6/6 | 100.0 | 100.0 | 100.0 | 100.0 | 6 | 0 | 0 | 0 | — |
| en-fr | identity | 6 | 6/6 | 1.8 | 16.1 | 1.8 | 16.1 | 0 | 0 | 0 | 0 | — |
| en-fr | oracle | 6 | 6/6 | 100.0 | 100.0 | 100.0 | 100.0 | 6 | 0 | 0 | 0 | — |
| es-en | ce.translate.memory | 6 | 6/6 | 100.0 | 100.0 | 100.0 | 100.0 | 6 | 0 | 0 | 0 | — |
| es-en | identity | 6 | 6/6 | 1.6 | 10.1 | 1.6 | 10.1 | 0 | 0 | 0 | 0 | — |
| es-en | oracle | 6 | 6/6 | 100.0 | 100.0 | 100.0 | 100.0 | 6 | 0 | 0 | 0 | — |

| pair/category | items | covered |
|---|---|---|
| en-de/general | 6 | 6 |
| en-es/correction | 1 | 1 |
| en-es/general | 6 | 6 |
| en-es/names | 1 | 1 |
| en-es/negation | 1 | 1 |
| en-es/numbers | 1 | 1 |
| en-es/question | 1 | 1 |
| en-es/terminology | 1 | 1 |
| en-fr/general | 6 | 6 |
| es-en/general | 6 | 6 |

## Error analysis

* **Coverage, not accuracy, is the memory's failure mode.** It is exact
  on every dev item (by construction) and covers **0 of 62** held-out
  items in every pair and category: a translation memory does not
  generalize to unseen sentences, and it says so (`no_coverage` on 62/62)
  instead of producing something. No meaning reversal, omission,
  addition, number, negation or terminology error can occur in its
  output because it emits only stored human translations — the risk moves
  to the content of the memory (provenance per segment, conflicts
  rejected at load).
* **Identity** shows what an untranslated output scores: BLEU 0.7–2.4,
  chrF 16.5–18.1 (shared names, digits and cognates). Its negation screen
  fires on 1–4 items per pair — the screen works on real divergences
  (source negation present, target cue list absent).
* **Reference looseness**: some general references are equivalents
  rather than literal translations ("No problem" / "De nada"), so a
  correct alternative would score below 100. With a single reference per
  item, BLEU/chrF under-reward valid variation; this matters once a real
  MT system is evaluated.
* **Streaming/revision errors**: covered by lifecycle tests rather than
  this corpus — stale, duplicate and late inputs are rejected; superseded
  partials are coalesced; cancellations, resets, stops and failed finals
  withdraw results; late results are discarded (`tests/processor.rs`,
  `tests/worker.rs`, `communication-engine/tests/translation_runtime.rs`).
* **Not measurable here** (no real MT output): idiom, register, gender,
  hallucination and ambiguity behaviour of a model; first-useful-output
  and stability delay of a model; resource use of a model. These are the
  first measurements to make when a real adapter is approved.

## Performance (engine overhead only; this machine)

| Measurement | Result | Source |
|---|---|---|
| Processor overhead per request, trivial adapter (validation, protection, fidelity checks, events) | p50 5.8 µs, p95 7.8 µs, max 8.8 µs (38 requests × 200 runs) | `evaluate_translation -- overhead` (release) |
| Through the supervised worker thread (queue, coalescing, signals) | 8,000 inputs in 236 ms (≈ 30 µs each, incl. back-pressure waits) | `tests/worker.rs::long_streams_stay_bounded` (release) |
| Bounded state | per-stream utterance table ≤ 256, hints ≤ 64 stored / 64 pending, queue ≤ 2 × `budget.queue` | code + worker tests |

There is no model, so no RTF, first-output latency, memory or CPU figure
for translation itself; p50/p95 are reported only for the overhead
micro-measurement, whose sample size supports them.

## Limitations

1. No real MT system evaluated; quality of translation is **not**
   established by this phase.
2. 62 held-out items, four pairs, one domain, single authored
   references, no human evaluation.
3. Screens are heuristic (negation cues, digits, question marks).
4. Performance numbers are engine overhead on a cloud container, not a
   device.
