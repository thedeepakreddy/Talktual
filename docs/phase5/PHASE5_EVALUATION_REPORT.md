# Phase 5 Evaluation Report

Harness: `eval/intelligence/intelligence.eval.ts`. Results: `eval/intelligence/results/phase5-frozen.json`. Run with `npx vitest run --config vitest.eval.config.ts eval/intelligence`. Everything is **offline and synthetic**: no real speech, no real translator, no human raters.

## 1. Method
* **Two sets, authored separately.**
  * **dev** (43 cases): written by me with the lexicons in view and used for tuning. **Its results are optimistic by construction.**
  * **independent** (59 cases): written by a separate agent instance that was told not to read the repository. It was given only the case schema, the signal taxonomy and coverage targets, and wrote the cases before the implementation existed. I did not open the file until the implementation was **frozen at commit `6923dd3`**, and I did not tune on it afterwards.
  * Limitation: that author is the same model family, not a human, and the labels were not blind-reviewed by a second annotator.
* **Three modes**, so the effect of *context* can be separated from the effect of rules:
  * `off`: intelligence disabled (the Phase 4 baseline).
  * `nocontext`: ACI on, but context is reset just before the target turn.
  * `full`: ACI with session context.
* **Two devices.** Each case runs with two controllers, A (English speaker) and B (Spanish speaker). Each sees the dialogue as the real session would.
* **Hostile stand-in translator.** Used only to test whether names survive translation. It passes text through but mistranslates names that are also common words (Rose→rosa, Bill→factura…). It is **not** a translation model. Every other metric is measured on the decisions themselves.
* **Scoring rules (documented in the harness).**
  * A sense counts as correct when the expected gloss shares a 5-letter word prefix with our sense's gloss or id. "Ambiguous" is correct only if we flagged the word without choosing a sense.
  * A reference is correct if the antecedent strings contain one another. An expected `null` is correct if we did not guess.
  * Signals are counted at confidence ≥ 0.5 (micro-averaged), on the shared taxonomy only.

## 2. Results — independent set (held out, frozen implementation)
| Metric | off (baseline) | nocontext | full |
| --- | --- | --- | --- |
| Ambiguous words: resolved correctly (8 expected resolvable) | 0 | 4 | **5** |
| Ambiguous words: left ambiguous when context didn't decide (4) | 0 | 4 | **4/4** |
| **Overconfident sense guesses** (wrong sense, or resolved when it should stay ambiguous) | 0 | 0 | **0** |
| Ambiguous words not in lexicon (missed) | 12 | 3 | 3 |
| References resolved correctly (8 resolvable) | 0 | 0 | **2** |
| References correctly left unresolved (5 with no unique antecedent) | 5 | 5 | **5/5** |
| **Wrong reference guesses** | 0 | 0 | **0** |
| References not recognised (e.g. object pronouns "it", "la") | 8 | 3 | 3 |
| Names preserved through the hostile translator (13) | 6 | 10 | **10** |
| Idioms found (6) / false idiom notes | 0 / 0 | 3 / 0 | **3 / 0** |
| Signals precision / recall (46 labels) | — / 0 | 0.82 / 0.61 | **0.74 / 0.63** |
| Neutral cases with any note (5) | 0 | 0 | **0** |
| Sensitive strings stored (4) | 0 | 0 | **0** |
| Words added to MT input beyond lexicon edits | 0 | 0 | **0** |
| Decision time p50 / p95 / max (ms) | — | 0.10 / 0.66 / 1.27 | **0.09 / 0.15 / 0.61** |

**Per-class signals (independent, full):**

| Class | Precision | Recall |
| --- | --- | --- |
| question | 0.83 | 0.83 |
| urgency | 1.0 | 1.0 |
| misunderstanding | 1.0 | 1.0 |
| topic-change | 1.0 | 1.0 (n=1) |
| confirmation | 0.60 | 1.0 |
| clarification-request | 1.0 | 0.50 |
| request | 0.75 | 0.46 |
| offer | 1.0 | 0.40 |
| correction | 1.0 | 0.20 |
| **refusal** | **0.20** | 1.0 |

The main weakness is refusal: replies such as "No te preocupes…" and "No, tengo que…" fire it.

**Calibration (independent, full):**

| Confidence | Correct |
| --- | --- |
| 0.85–1.0 | 14/16 (88 %) |
| 0.70–0.84 | 13/19 (68 %) |
| 0.50–0.69 | 2/4 (50 %) |

Monotonic, but the 0.70–0.84 band is over-confident.

## 3. Results — developer set (tuned; optimistic)
| | off | nocontext | full |
| --- | --- | --- | --- |
| Senses correct (11) | 0 | 10 | 11 |
| Resolvable references (3) | 0 | 0 | 3 |
| Unresolvable references respected (4) | 4 | 4 | 4 |
| Names preserved (5) | 0 | 5 | 5 |
| Idioms (3) | 0 | 3 | 3 |
| Signals P / R | — / 0 | 1.0 / 0.96 | 1.0 / 1.0 |
| Wrong guesses | 0 | 0 | 0 |

## 4. Reading the results honestly
* **The benefit is real but modest, and comes mostly from rules, not context.**
  * Compared with the baseline, the held-out gains are names (6→10 of 13), idioms (0→3), signals (0→0.63 recall), and ambiguity surfaced instead of silently guessed (4/4).
  * *Context* specifically adds 1 sense resolution and 2 reference resolutions over `nocontext`.
  * Signal precision drops by 0.08 with context: refusal and confirmation fire more once a previous turn exists.
* **Safety held on unseen data.**
  * 0 overconfident senses and 0 wrong reference guesses.
  * 0 notes on neutral cases.
  * 0 added words in the MT input.
  * 0 sensitive strings stored.
  * The system mostly fails *quiet* (missed or left unresolved), not *wrong*.
* **Main failure causes on the held-out set**, which I won't patch silently:
  1. A sentence-initial name ("Bill said…", "Rose will meet us…") isn't recognised. This accounts for 3 of the 3 unpreserved names.
  2. Context cues are single-language. An English "cash" turn does not help resolve a Spanish "banco".
  3. Lexicon coverage is small: "sobre", "sick", "bill" (as invoice) and object pronouns ("it", "la") are absent.
  4. Refusal fires on any reply starting with "No…". Correction and offer phrasings vary more than the cue lists cover.
* **No claim of human-level understanding**, and no real-world validation.

## 5. Adversarial and edge cases covered
* **Covered by tests (unit/session/E2E):**
  * similar names (Ana/Anna → no guess)
  * contradictory context → stays ambiguous
  * explicit topic change breaks references
  * cross-turn corrections ("not 5 pm, 6 pm")
  * repeated clarification → suggestion offered once (no loop)
  * sarcasm ("Oh great, another delay. Just perfect." → no signal)
  * long session (600 turns): budgets hold, observed max 8 turns / 413 chars / 8 entities, p95 0.20 ms
  * context-budget eviction
  * reconnect keeps context within the call
  * provider failure → original sent untranslated, MT input never leaked
  * malformed provider output → one context-free retry
  * toggle off mid-session → immediate context-free fallback
  * input over 600 chars → faithful fallback
  * sensitive identifiers (phone/email/card/IBAN) never stored or pinned
* **Not tested:**
  * real recognizer output (capitalisation varies by browser and affects name detection)
  * real translators' handling of `QXnQ` tokens (only stubbed; the restoration fallback exists for this)
  * human-perceived usefulness or annoyance of notes and suggestions

## 6. Spec targets
| Target | Result |
| --- | --- |
| Measurable improvement vs baseline on a context-dependent set; guard non-contextual regression | Improvement on held-out (see §2). Feature-off parity: off mode = Phase 4 (tests + rollback E2E). Neutral cases unaffected |
| Report unsupported additions and meaning changes | 0 added words. The original is never altered (by construction, validated). The 10/59 changed MT inputs are name tokens or listed lexicon edits, labelled in the UI. Not human-reviewed |
| Per-class P/R/calibration; no human-level claim | §2 |
| Overhead p95 < 300 ms local | **p95 0.15–0.20 ms** (local rules). The provider-backed target is N/A (none built) |
| Explicit context budget and observed max | ARCHITECTURE §4 |
| Feature-off parity with documented UI differences | TEST_REPORT §3 |
