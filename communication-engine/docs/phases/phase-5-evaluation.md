# Phase 5 evaluation — semantic interpretation

Date: 2026-11-24. Adapter `ce.semantic.rules` 1.0.0, default
`SemanticConfig` (config fingerprint `e89669d1c69688b6`; with the
`example.home` profile on profile dialogues). Host: Linux x86_64 cloud
container, Intel Xeon @ 2.30 GHz, 4 logical CPUs, 16 GB; no GPU and no
device hardware.

**Read this first.**
* The corpus is small, synthetic and English.
* Its dev split was used to fix rules.
* The `test` and `asr` splits were scored once, after the rules were
  frozen (commit `b1e5a2d`, after the corpus commit `e18acef`).
* These numbers describe this corpus only. They are not a quality claim
  about semantic interpretation in general, nor about other speakers,
  domains or languages. No score is calibrated.

## Evidence labels

| Evidence | What it is |
|---|---|
| **replay, authored** | `dev` and `test` splits: text written for CE in ASR style, replayed through the real `SemanticProcessor` + rule adapter with synthesized partial revisions |
| **replay, real ASR** | `asr` split: verbatim final hypotheses (and, for one item, the real partial revisions) of the real sherpa-onnx model on the synthetic TTS speech fixtures, copied from `phase-4-real-asr.md` |
| **real model, end to end** | the bridge test `the_real_model_and_semantics_find_the_time_in_a_fixture`: TTS fixture audio → CE capture → Phase 3 → real sherpa-onnx → Phase 5 → WebSocket (synthetic speech) |
| **real model, browser** | `e2e/ce-bridge.mjs`: Chromium fake microphone playing a TTS fixture → Talktual Engine Lab → bridge → CE (all phases) |
| **scripted** | unit, worker and runtime tests with the scripted ASR and semantic adapters (lifecycle, faults, limits); not quality evidence |

## Method

* **Partial revisions.** Each utterance is replayed as partial revisions,
  then the final. Synthesized partials are word prefixes with committed
  text one word behind, one revision per 300 ms of synthetic audio. The
  `asr` direction item uses the real revisions.
* **What is scored.** The final observations of each utterance, held at
  the end of its dialogue, after any correction cascades.
* **Entities.**
  * Exact: same category and span.
  * Overlap: same category, overlapping span.
  * Normalized: an exact match whose value equals the gold value.
  * Gold entities with `mention: null` (values the recognizer lost) always
    count as false negatives.
  * Scores are reported at every certainty and at `medium` or better.
* **References.**
  * Correct means the outcome matches, and for `resolved`, the target's
    mention too.
  * `ambiguous` and `unresolved` predictions are abstentions.
  * The gold target is the annotated entity the expression denotes; an
    expression for an unannotated thing is gold `unresolved`.
* **Acts.**
  * Per-label P/R/F1, and macro-F1 over labels with support.
  * Coverage: the share of utterances labeled.
  * Selective accuracy: correct among labeled.
  * Abstention recall: gold `null` predicted as abstention.
* **Robustness.** Every dialogue is also replayed with:
  * every partial delivered twice and the previous partial re-sent after
    each (stale), with the final twice;
  * finals only.

  Finals must match the clean replay in meaning: category, span, mention,
  normalized value, reference target mention, act decision and certainty.
* **Throughput.** Every dialogue replayed 200 times on one thread, plus
  the process's peak RSS (`VmHWM`).

Reproduce:
`cargo run --release -p ce-semantic --example evaluate_semantic -- all --errors`.
The regression gate is
`cargo test --release -p ce-semantic --test evaluation`.

## Summary

| Split | Utterances | Entity exact F1 (all / ≥ medium) | Normalized correct | Reference accuracy (gold n) | Act macro-F1 · coverage · selective acc. | Robustness |
|---|---|---|---|---|---|---|
| dev (tuned on) | 39 | 1.000 / 1.000 | 28/28 | 0.727 (11) | 0.975 · 0.846 · 0.970 | 14/14 · 14/14 |
| **test (held out)** | 45 | **0.907 / 0.904** | 34/34 | **0.733** (15) | **0.929** · 0.822 · 0.946 | 15/15 · 15/15 |
| **asr (held out, real ASR)** | 10 | **0.500 / 0.500** | 2/2 | n/a (0) | **0.810** · 0.800 · 1.000 | 10/10 · 10/10 |

### Held-out error analysis

| Split | Item | Error | Cause | Category |
|---|---|---|---|---|
| test | test-04 u0 | `A DOZEN EGGS` missed | `dozen` is not supported | coverage gap |
| test | test-05 u2 | `SIX` read as a number, gold time | no cue word; the time reading needs dialogue context | ambiguity, would need context |
| test | test-10 u1 | `FIFTEEN POUNDS` quantity (`low`, ambiguous), gold GBP | currency versus weight is not decided | deliberate abstention by design; costs P and R |
| test | test-12 u0 | `MINUS FIVE` → `FIVE` | negative numbers are not supported | coverage gap |
| test | test-01 u1 | `HOW MUCH WILL THAT COST`: reference missed | `that` before a verb outside the follow list | detection gap |
| test | test-01 u3 | `OKAY BOOK IT`: `IT` resolved to the price | the antecedent (the flight) is not an entity; the only compatible item is the money | wrong resolution (1 of 4 resolutions) |
| test | test-03 u1 | `THEN` ambiguous between `WEDNESDAY` and `HALF PAST TWO` | a composite time expression is two items | abstention |
| test | test-06 u2 | `GOT IT THANKS`: spurious reference `IT` | idiom | spurious |
| test | test-12 u1 | `THAT IS VERY COLD` unresolved | `number` is not a compatible singular category | abstention |
| test | acts | `OKAY BOOK IT` → acknowledgement; `AND LOCK …` no cue; `AND THE REST …` → statement; `ON SUNDAYS WE …` no cue | cue lists, leading prepositions | |
| asr | rms_meeting, slt_thanks | `tomorrow` / `today` lost (`TO MO`, `TO DAY`) | recognizer word splits | ASR error |
| asr | awb_directions | `two blocks` lost (`TWO BALL`); a spurious `number TWO` | ASR error | ASR error |
| asr | kal16_station | `GOOD MORNING COULD YOU …` → abstained (out of taxonomy) | a greeting at the start hides the polite-request cue | rule gap (not fixed: found on a held-out split) |
| asr | rms_no | empty hypothesis (`No` lost) | ASR deletion | ASR error |

On the real-ASR split, 3 of 5 gold entities were lost by the recognizer
before semantics ran. Semantic precision on what remained was 2/3.

### Known annotation ambiguity

* "CAN WE MOVE IT TO …" (dev) is gold `request`.
* "CAN WE PAY HALF OF IT TODAY" (test) is gold `question`.

Both readings are defensible. The rules label both `question`, so the dev
item counts as an error, and no rule was added for it. The gold was not
changed after scoring.

## Streaming, latency, performance

| Split | Inputs (partials) | Events per input | Retracted before final | Finals first seen in a partial | First observation, synthetic audio ms (p50 / p95 / max) | Processor time per input, ms (p50 / p95 / p99 / max) |
|---|---|---|---|---|---|---|
| dev | 206 (167) | 1.70 | 22 | 61 of 82 | 1200 / 1800 / 1800 | 0.006 / 0.013 / 0.022 / 0.039 |
| test | 247 (202) | 1.67 | 22 | 70 of 103 | 900 / 2100 / 2400 | 0.006 / 0.011 / 0.016 / 0.027 |
| asr | 95 (85) | 1.10 | 1 | 10 of 12 | 1200 / 1500 / 1500 | 0.005 / 0.008 / 0.013 / 0.013 |

* "Retracted before final" counts speculative output withdrawn, e.g. a
  provisional `number TWENTY` that became `duration TWENTY FIVE MINUTES`.
  The robustness replays show this never changes the finals.
* Processor time per input is in the low microseconds: p95 ≤ 0.013 ms in
  every split, max 0.039 ms. Timings vary between runs; the table is the
  run printed below. That is far below the 250 ms deadline. Machine and load
  dependent.
* Output size: JSON bytes of the semantic events each input produces.
  * p50 / p95 / max: dev 930 / 2729 / 4545; test 874 / 2846 / 4668;
    asr 812 / 1692 / 2642.
  * The bound is `max_observations` (64) per input.
* Throughput:
  * one thread: about 100 000–140 000 inputs/s across runs (this run
    100 651, 9.9 µs per input including per-dialogue processor setup);
  * 4 threads, each with its own processors: 328 482 inputs/s aggregate.

  Peak RSS of the evaluation process is 4–6 MB. Both measurements are
  from this container, with no device measurement.
* Live latency through the runtime (real model, bridge test): the final
  semantic observation arrives with the final transcript. The bridge ends
  the session after semantics drain. Per-observation wall latency through
  the runtime was not separately measured.

## End to end with the real model

From `cargo test --release --test bridge the_real_model_and_semantics -- --ignored --nocapture`
(synthetic speech `dev_rms_meeting`, 16 kHz, activity segmentation):

```
finals: ["THE MEETING STARTS AT NINE THIRTY TO MORROW"]
final observation: "entity" "time_of_day" "NINE THIRTY" {"type":"time_of_day","value":{"hour":9,"minute":30,"pm":null}}
final observation: "act" null null null
```

* The time is found and normalized. am/pm stays unknown because it was
  not said.
* "TO MORROW" (an ASR split) yields no relative day.
* An act observation was finalized too. This printout shows only the
  entity fields, so it does not show the act's label.
* In the browser (`e2e/ce-bridge.mjs`, `dev_kal16_station`, real model),
  the Engine Lab listed "act: abstained (out of taxonomy) · final ·
  hypothesis · certainty low" for "GOOD MORNING COULD YOU TELL ME WHERE
  THE DRAIN STATION IS". That is the same limitation as in the asr split.

## Raw output

```text
# Semantic replay evaluation

* adapter: ce.semantic.rules 1.0.0 (in process, deterministic)
* config fingerprint (default, no profile): e89669d1c69688b6
* corpus: fixtures/semantic/corpus.jsonl, fnv b84a9ab95c1ccfe2 (39 dialogues)
* profile: example.home@1.0.0 file fnv 39955f82f9e39f80 profile fingerprint e8e8ada1a05bead4
* host: linux x86_64 · 4 logical CPUs

## Split `dev` — 14 dialogues, 39 utterances

### Entities (predictions with certainty ≥ low)

| match | TP | FP | FN | P | R | F1 |
|---|---|---|---|---|---|---|
| exact span | 28 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| overlapping span | 28 | 0 | 0 | 1.000 | 1.000 | 1.000 |

| category (exact) | TP | FP | FN | P | R | F1 |
|---|---|---|---|---|---|---|
| date | 3 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| digit_sequence | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| duration | 4 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| money | 2 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| ordinal | 2 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| percentage | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| profile:device | 2 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| quantity | 6 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| relative_day | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| time_of_day | 3 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| weekday | 3 | 0 | 0 | 1.000 | 1.000 | 1.000 |

Normalized values correct: 28/28 exact matches with a gold value. Lost to ASR (always FN): 0.

### Entities (predictions with certainty ≥ medium)

| match | TP | FP | FN | P | R | F1 |
|---|---|---|---|---|---|---|
| exact span | 28 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| overlapping span | 28 | 0 | 0 | 1.000 | 1.000 | 1.000 |

| category (exact) | TP | FP | FN | P | R | F1 |
|---|---|---|---|---|---|---|
| date | 3 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| digit_sequence | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| duration | 4 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| money | 2 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| ordinal | 2 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| percentage | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| profile:device | 2 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| quantity | 6 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| relative_day | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| time_of_day | 3 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| weekday | 3 | 0 | 0 | 1.000 | 1.000 | 1.000 |

Normalized values correct: 28/28 exact matches with a gold value. Lost to ASR (always FN): 0.

### References

gold 11 · detected 11 · correct 8 · **accuracy 0.727** · spurious 0 · predicted resolved/ambiguous/unresolved 3/1/7 · resolution precision 1.000 · abstention rate 0.727

| gold \ predicted | counts |
|---|---|
| resolved | ambiguous: 1, resolved: 3, unresolved: 2 |
| unresolved | unresolved: 5 |

### Communicative acts (`ce.acts` 1.0.0)

utterances 39 · **macro-F1 0.975** · coverage 0.846 · selective accuracy 0.970 · gold abstentions 5 · abstention recall 1.000

| label | TP | FP | FN | P | R | F1 |
|---|---|---|---|---|---|---|
| acknowledgement | 2 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| confirmation | 2 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| correction | 3 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| question | 6 | 1 | 0 | 0.857 | 1.000 | 0.923 |
| refusal | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| request | 9 | 0 | 2 | 1.000 | 0.818 | 0.900 |
| statement | 9 | 0 | 0 | 1.000 | 1.000 | 1.000 |

| gold \ predicted | counts |
|---|---|
| abstain | abstain: 5 |
| acknowledgement | acknowledgement: 2 |
| confirmation | confirmation: 2 |
| correction | correction: 3 |
| question | question: 6 |
| refusal | refusal: 1 |
| request | abstain: 1, question: 1, request: 9 |
| statement | statement: 9 |

### Streaming (synthetic timeline: one revision per 300 ms)

inputs 206 (partials 167) · observation events {"finalized": 61, "new": 104, "retracted": 22, "stabilized": 22, "updated": 141} · events per input 1.699 · retracted before final 22 · finals 82 (first seen in a partial: 61)
first observation (audio ms) p50 1200 p95 1800 max 1800 (n=31) · processor time per input (ms) p50 0.006 p95 0.013 p99 0.022 max 0.039 (n=206)
output per input (JSON bytes of semantic events) p50 930 p95 2729 p99 4321 max 4545

### Robustness

duplicated + stale partials: 14/14 dialogues with identical finals (336 stale/duplicate inputs rejected) · finals only: 14/14 identical finals

### Errors

* dev-01 u1 reference_outcome: IT: unresolved (gold resolved Some("SEVEN"))
* dev-08 u2 reference_outcome: IT: ambiguous (gold resolved Some("THERMOSTAT"))
* dev-13 u1 reference_outcome: THAT: unresolved (gold resolved Some("TWELFTH OF JUNE NINETEEN NINETY"))
* dev-04 u1 act: "CAN WE MOVE IT TO THE FIFTH OF MARCH": gold Some("request"), got Some("question")
* dev-06 u2 act: "PREHEAT THE OVEN TO ONE HUNDRED EIGHTY DEGREES": gold Some("request"), got None

## Split `test` — 15 dialogues, 45 utterances

### Entities (predictions with certainty ≥ low)

| match | TP | FP | FN | P | R | F1 |
|---|---|---|---|---|---|---|
| exact span | 34 | 3 | 4 | 0.919 | 0.895 | 0.907 |
| overlapping span | 35 | 2 | 3 | 0.946 | 0.921 | 0.933 |

| category (exact) | TP | FP | FN | P | R | F1 |
|---|---|---|---|---|---|---|
| date | 2 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| digit_sequence | 2 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| duration | 3 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| money | 2 | 0 | 1 | 1.000 | 0.667 | 0.800 |
| number | 0 | 2 | 1 | 0.000 | 0.000 | 0.000 |
| ordinal | 2 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| percentage | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| profile:device | 3 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| profile:room | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| quantity | 5 | 1 | 1 | 0.833 | 0.833 | 0.833 |
| relative_day | 3 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| time_of_day | 6 | 0 | 1 | 1.000 | 0.857 | 0.923 |
| weekday | 4 | 0 | 0 | 1.000 | 1.000 | 1.000 |

Normalized values correct: 34/34 exact matches with a gold value. Lost to ASR (always FN): 0.

### Entities (predictions with certainty ≥ medium)

| match | TP | FP | FN | P | R | F1 |
|---|---|---|---|---|---|---|
| exact span | 33 | 2 | 5 | 0.943 | 0.868 | 0.904 |
| overlapping span | 34 | 1 | 4 | 0.971 | 0.895 | 0.932 |

| category (exact) | TP | FP | FN | P | R | F1 |
|---|---|---|---|---|---|---|
| date | 2 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| digit_sequence | 2 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| duration | 3 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| money | 2 | 0 | 1 | 1.000 | 0.667 | 0.800 |
| number | 0 | 2 | 1 | 0.000 | 0.000 | 0.000 |
| ordinal | 2 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| percentage | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| profile:device | 3 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| profile:room | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| quantity | 4 | 0 | 2 | 1.000 | 0.667 | 0.800 |
| relative_day | 3 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| time_of_day | 6 | 0 | 1 | 1.000 | 0.857 | 0.923 |
| weekday | 4 | 0 | 0 | 1.000 | 1.000 | 1.000 |

Normalized values correct: 33/33 exact matches with a gold value. Lost to ASR (always FN): 0.

### References

gold 15 · detected 14 · correct 11 · **accuracy 0.733** · spurious 1 · predicted resolved/ambiguous/unresolved 4/1/9 · resolution precision 0.750 · abstention rate 0.714

| gold \ predicted | counts |
|---|---|
| resolved | ambiguous: 1, resolved: 3, unresolved: 1 |
| unresolved | missed: 1, resolved: 1, unresolved: 8 |

### Communicative acts (`ce.acts` 1.0.0)

utterances 45 · **macro-F1 0.929** · coverage 0.822 · selective accuracy 0.946 · gold abstentions 7 · abstention recall 0.857

| label | TP | FP | FN | P | R | F1 |
|---|---|---|---|---|---|---|
| acknowledgement | 1 | 1 | 0 | 0.500 | 1.000 | 0.667 |
| confirmation | 2 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| correction | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| question | 7 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| refusal | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| request | 9 | 0 | 2 | 1.000 | 0.818 | 0.900 |
| statement | 14 | 1 | 1 | 0.933 | 0.933 | 0.933 |

| gold \ predicted | counts |
|---|---|
| abstain | abstain: 6, statement: 1 |
| acknowledgement | acknowledgement: 1 |
| confirmation | confirmation: 2 |
| correction | correction: 1 |
| question | question: 7 |
| refusal | refusal: 1 |
| request | abstain: 1, acknowledgement: 1, request: 9 |
| statement | abstain: 1, statement: 14 |

### Streaming (synthetic timeline: one revision per 300 ms)

inputs 247 (partials 202) · observation events {"finalized": 70, "new": 125, "retracted": 22, "stabilized": 25, "updated": 171} · events per input 1.672 · retracted before final 22 · finals 103 (first seen in a partial: 70)
first observation (audio ms) p50 900 p95 2100 max 2400 (n=38) · processor time per input (ms) p50 0.006 p95 0.011 p99 0.016 max 0.027 (n=247)
output per input (JSON bytes of semantic events) p50 874 p95 2846 p99 4316 max 4668

### Robustness

duplicated + stale partials: 15/15 dialogues with identical finals (408 stale/duplicate inputs rejected) · finals only: 15/15 identical finals

### Errors

* test-04 u0 entity_missed: quantity "A DOZEN EGGS"
* test-05 u2 entity_missed: time_of_day "SIX"
* test-05 u2 entity_spurious: number "SIX"
* test-10 u1 entity_missed: money "FIFTEEN POUNDS"
* test-10 u1 entity_spurious: quantity "FIFTEEN POUNDS"
* test-12 u0 entity_missed: number "MINUS FIVE"
* test-12 u0 entity_spurious: number "FIVE"
* test-01 u1 reference_missed: THAT
* test-01 u3 reference_wrong: IT → Some("THREE HUNDRED AND TWENTY DOLLARS") (gold Unresolved None)
* test-03 u1 reference_outcome: THEN: ambiguous (gold resolved Some("HALF PAST TWO"))
* test-06 u2 reference_spurious: IT
* test-12 u1 reference_outcome: THAT: unresolved (gold resolved Some("MINUS FIVE"))
* test-01 u3 act: "OKAY BOOK IT": gold Some("request"), got Some("acknowledgement")
* test-07 u2 act: "AND LOCK THE FRONT DOOR TOO": gold Some("request"), got None
* test-08 u3 act: "AND THE REST IN TWO WEEKS": gold None, got Some("statement")
* test-13 u1 act: "ON SUNDAYS WE CLOSE AT FOUR": gold Some("statement"), got None

## Split `asr` — 10 dialogues, 10 utterances

### Entities (predictions with certainty ≥ low)

| match | TP | FP | FN | P | R | F1 |
|---|---|---|---|---|---|---|
| exact span | 2 | 1 | 3 | 0.667 | 0.400 | 0.500 |
| overlapping span | 2 | 1 | 3 | 0.667 | 0.400 | 0.500 |

| category (exact) | TP | FP | FN | P | R | F1 |
|---|---|---|---|---|---|---|
| number | 0 | 1 | 0 | 0.000 | 0.000 | 0.000 |
| ordinal | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| quantity | 0 | 0 | 1 | 0.000 | 0.000 | 0.000 |
| relative_day | 0 | 0 | 2 | 0.000 | 0.000 | 0.000 |
| time_of_day | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |

Normalized values correct: 2/2 exact matches with a gold value. Lost to ASR (always FN): 3.

### Entities (predictions with certainty ≥ medium)

| match | TP | FP | FN | P | R | F1 |
|---|---|---|---|---|---|---|
| exact span | 2 | 1 | 3 | 0.667 | 0.400 | 0.500 |
| overlapping span | 2 | 1 | 3 | 0.667 | 0.400 | 0.500 |

| category (exact) | TP | FP | FN | P | R | F1 |
|---|---|---|---|---|---|---|
| number | 0 | 1 | 0 | 0.000 | 0.000 | 0.000 |
| ordinal | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| quantity | 0 | 0 | 1 | 0.000 | 0.000 | 0.000 |
| relative_day | 0 | 0 | 2 | 0.000 | 0.000 | 0.000 |
| time_of_day | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |

Normalized values correct: 2/2 exact matches with a gold value. Lost to ASR (always FN): 3.

### References

gold 0 · detected 0 · correct 0 · **accuracy n/a** · spurious 0 · predicted resolved/ambiguous/unresolved 0/0/0 · resolution precision n/a · abstention rate n/a

| gold \ predicted | counts |
|---|---|

### Communicative acts (`ce.acts` 1.0.0)

utterances 10 · **macro-F1 0.810** · coverage 0.800 · selective accuracy 1.000 · gold abstentions 0 · abstention recall n/a

| label | TP | FP | FN | P | R | F1 |
|---|---|---|---|---|---|---|
| acknowledgement | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| confirmation | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| correction | 0 | 0 | 0 | 0.000 | 0.000 | 0.000 |
| question | 1 | 0 | 0 | 1.000 | 1.000 | 1.000 |
| refusal | 0 | 0 | 1 | 0.000 | 0.000 | 0.000 |
| request | 3 | 0 | 1 | 1.000 | 0.750 | 0.857 |
| statement | 2 | 0 | 0 | 1.000 | 1.000 | 1.000 |

| gold \ predicted | counts |
|---|---|
| acknowledgement | acknowledgement: 1 |
| confirmation | confirmation: 1 |
| question | question: 1 |
| refusal | abstain: 1 |
| request | abstain: 1, request: 3 |
| statement | statement: 2 |

### Streaming (synthetic timeline: one revision per 300 ms)

inputs 95 (partials 85) · observation events {"finalized": 10, "new": 13, "retracted": 1, "stabilized": 2, "updated": 78} · events per input 1.095 · retracted before final 1 · finals 12 (first seen in a partial: 10)
first observation (audio ms) p50 1200 p95 1500 max 1500 (n=3) · processor time per input (ms) p50 0.005 p95 0.008 p99 0.013 max 0.013 (n=95)
output per input (JSON bytes of semantic events) p50 812 p95 1692 p99 2642 max 2642

### Robustness

duplicated + stale partials: 10/10 dialogues with identical finals (172 stale/duplicate inputs rejected) · finals only: 10/10 identical finals

### Errors

* asr-test_awb_directions u0 entity_spurious: number "TWO"
* asr-dev_kal16_station u0 act: "GOOD MORNING COULD YOU TELL ME WHERE THE TRAIN STATION IS": gold Some("request"), got None
* asr-test_rms_no u0 act: "": gold Some("refusal"), got None

## Throughput

109600 inputs in 1.089 s on one thread (100651 inputs/s, 9.9 µs per input incl. processor setup) · peak RSS of this process 4112 KiB
4 threads in parallel: 109600 inputs in 0.334 s (328482 inputs/s aggregate) · peak RSS 5672 KiB
```
