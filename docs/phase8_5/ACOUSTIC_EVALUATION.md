# Phase 8.5 — Acoustic and translation evaluation: methodology, rubric and review plan

**No scores are reported in this phase.** No real audio, speakers or reviewers were available. This document defines how to produce scores that can be trusted.

## 1. What is evaluated
| Dimension | Question | Measured by |
| --- | --- | --- |
| Recognition coverage | Did each spoken phrase produce a line? | Count of lines vs phrases (export `reliability.units`) |
| Intelligibility | Is the translated audio understandable in the ear? | Listener rubric I (1–5) |
| Adequacy | Does the English convey the Hungarian meaning? | Native-speaker rubric A (1–5) |
| Omissions / additions | Missing or invented content? | Count per phrase (native speaker) |
| Repetition | Any phrase or words played twice? | Tester tally + export (no duplicate `playback_start` per unit) |
| Clipping | Sentence cut at the start or end? | Tester tally (A/B against the script) |
| Stale speech | Speech played long after it was said? | Export `humanMarked.mark_to_audio_start_ms`, `queue_drops` |
| Chunk timing | Early pieces natural, or fragmenting meaning? | Native-speaker rubric C (1–3) for lines marked *early* |

## 2. Conditions matrix
Run each phrase set under each condition. Distances are speaker-to-phone in metres.

| Code | Condition | Setup |
| --- | --- | --- |
| Q1 | Quiet, conversational | Quiet room, 1 m, normal pace |
| Q2 | Distance | 2 m and 4 m |
| Q3 | Reverberation | Tiled kitchen or stairwell, 2 m |
| N1 | Café noise | Café babble played from a separate speaker at about 65–70 dBA at the phone (meter app); speaker at 1 m |
| N2 | Real café | Consenting speakers at a table; **no bystander recording**; Talktual doesn't record |
| O1 | Overlap | Two speakers overlap for about 30 % of each phrase |
| R1 | Rapid changes | Two speakers alternating every 1–2 s |
| F1 | Fillers and corrections | Scripted "öö", "hát", self-corrections ("tíz… nem, tizenegy órakor") |
| I1 | Fragments | Incomplete phrases ("Szerintem a…") |
| A1 | Ambiguity | Words with several senses ("bank", "ég") in context |

## 3. Phrase sets (authored for this project; no third-party corpus)
- **P1–P10:** short travel and café phrases (3–8 words), e.g.:
  - "Jó napot kívánok"
  - "Hol van a pályaudvar?"
  - "Egy kávét kérek tejjel"
  - "Mennyibe kerül a jegy Bécsbe?"
  - "Elnézést, szabad ez a hely?"
  - "Mikor indul az utolsó metró?"
  - "Kártyával lehet fizetni?"
  - "Kérem a számlát"
  - "Holnap esni fog"
  - "Köszönöm szépen a segítséget"
- **P11:** a run-on (25+ words, no pause), e.g. the Phase 8 evaluation run-on sentence.
- **P12:** a two-speaker exchange (8 turns).
- **F-set:** 5 phrases with fillers and self-corrections. **I-set:** 5 fragments. **A-set:** 5 ambiguity phrases.
- **Other supported pairs** (spot check, 5 phrases each): es→en, de→en, fr→en, en→hu (the reverse direction).

Phrases are read by consenting speakers. The written script is the reference. **No audio is retained.** If a recording is wanted for later re-scoring, it requires the explicit written consent of every voice in it and must be stored outside Talktual.

## 4. Scoring rubric
| Score | I — Intelligibility (listener, in ear) | A — Adequacy (native speaker, from text) | C — Chunking (early lines) |
| --- | --- | --- | --- |
| 5 | Effortless | Full meaning | — |
| 4 | Clear with minor effort | Meaning intact, small loss | — |
| 3 | Understandable with effort | Gist preserved | Natural chunks |
| 2 | Partly understandable | Partial meaning, misleading parts | Fragmented but recoverable |
| 1 | Not understandable | Wrong or no meaning | Fragments mislead |

Per phrase, also record:
- omissions (count of content words missing)
- additions (count invented)
- repetition (Y/N)
- clipping (start / end / none)
- late (spoken > 4 s after the mark, Y/N, from the export)

**Aggregate:** per condition, the median I and A, the share of phrases with A ≥ 4, total omissions and additions per 100 words, and the repetition and clipping counts. **Report sample sizes. Don't average across conditions.**

## 5. Native-speaker review plan (test J)
- **Reviewer:** a native Hungarian speaker with C1+ English, not on the development team.
- **Material:** the script plus the on-screen English lines, copied by hand by the tester with speakers' consent. They are not exported by Talktual, which never stores text.
- **Blind order:** the reviewer scores lines in randomised order, without knowing the condition.
- **Calibration:** 5 practice phrases with agreed scores first. Re-score a random 10 % to check consistency (report agreement).
- **Output:** a rubric sheet per condition, plus free-text notes on systematic errors (names, numbers, word senses).

## 6. Limitations
- Recognition quality belongs to the browser vendor's engine and can change without notice. Results are tied to the OS and browser versions recorded.
- Played-back café noise is not a real café: report N1 and N2 separately.
- Small samples (10–20 phrases per condition) give direction, not statistical certainty.
