# Phase 4 evaluation — speech pipeline and language identification

Date: 2026-11-24 · Machine: cloud container, Linux x86_64, 4 vCPU
(Intel Xeon @ 2.30 GHz), **no audio hardware** · Build: release, Rust 1.97.0.

## Evidence labels (read first)

| Label | Meaning here |
|---|---|
| **synthetic** | The speech is CMU Flite TTS (`fixtures/speech`, PROVENANCE.md); noise, music, clipping, level and format variants are generated with fixed seeds. No human recording was used. |
| **replay** | Blocks are fed to the Phase 3 analyzer and the recognizer in lock-step in one thread (`ce_speech::eval::replay`), exactly as the runtime wires them; no device, no wall-clock scheduling. |
| **oracle** | Transcripts come from the **reference adapter** (`ce.asr.reference`), which knows the reference words and their time spans and reveals them in proportion to the audio it is fed. Its WER measures what the **pipeline** loses (missed or truncated segments, gaps, finalization), **not** speech-recognition accuracy. |
| **fake backend** | Runtime/integration evidence uses the test kit's fake audio backend (manual or real-time driver thread). |
| **compile-only** | Cross-target evidence is `cargo check` only. |

**This document has no real-ASR result.** Real-model results (the
approved sherpa-onnx baseline, ADR-0023) are in
[phase-4-real-asr.md](phase-4-real-asr.md), clearly labelled; everything
here is oracle.

## Corpora, manifests and splits

* Speech: 10 TTS utterances (4 dev, 6 test), `fixtures/speech/manifest.json`
  (schema `ce.speech-manifest.v1`). Reference = the exact TTS input text
  (label method `tts-input-text`). Scenarios (Phase 3 builder): 1.2 s lead,
  1.0 s pauses, 1.5 s tail, −70 dBFS analog floor, condition and format
  applied to the whole stream. Reference word spans for the oracle = first
  to last energy-labelled 10 ms cell of each utterance (Phase 3 labels).
* Language: `fixtures/language/lid_corpus.json` (PROVENANCE.md), 42 dev and
  92 test items (84 single-language, 8 mixed).
* **Dev was used** to select the identifier, the evidence threshold
  (`min_letters = 10`), the split-half check and the commit policy; **test
  is held out** and only reported. The regression gate
  (`ce-speech/tests/evaluation.rs`) uses thresholds set below the reported
  test values. Leakage: see `fixtures/language/PROVENANCE.md`.

## Scoring rules

* **WER / CER**: stream level — all finals in order vs all references in
  order — after `normalize()` (lowercase; punctuation removed except
  word-internal apostrophes; whitespace collapsed); Levenshtein over words /
  characters. **Case- and punctuation-sensitive WER** (`wer_raw`, raw
  whitespace tokens) is implemented and unit-tested, but **not reported**:
  neither fake adapter produces punctuation or casing (they declare
  `punctuation: false, casing: false`), so the number would only measure
  the normalization of the reference.
* **Utterance mapping**: each utterance is assigned to the reference it
  overlaps most (device frames). *missed* = reference with no utterance,
  *split* = reference covered by > 1 utterance, *merged* = utterance
  overlapping > 1 reference. Latencies and boundary errors use only
  utterances that overlap exactly one reference (first utterance of that
  reference).
* **Latency (audio time)**: *first partial* = recognizer input position
  (`input_end_frame`) at the first partial − reference speech start;
  *first commit* = same at the first non-empty committed text; *final* =
  input position at the final − reference speech end. Values are audio
  time in a lock-step replay: they include Phase 3 decision latency,
  `close_silence`, `trailing_ms`, chunking and partial cadence, but no
  compute or scheduling delay. Shown as p50 / p95 / max (nearest rank).
* **Revisions**: *partials/s* = partial revisions per second of utterance
  audio; *rev/utt* = revisions of all statuses per utterance;
  *corrections/utt* = word-level Levenshtein between consecutive displayed
  texts minus pure appends (what a reader sees change); *amended* = finals
  that changed committed text.
* **Start error** = |utterance start − reference speech start|.
* **LID (utterance)** = utterance-scope decisions on the finals.
* **RTF rec / ana** = processing time / audio time for the recognizer
  (with the oracle: CE's own overhead only) and the Phase 3 analyzer, in
  this run on this machine.

## Results

The tables below are the verbatim output, re-generated after the
real-ASR baseline changed two defaults (`preroll_ms` 1000, `lead_in_ms`
500; see [phase-4-real-asr.md](phase-4-real-asr.md)), of
`cargo run --release -p ce-speech --example evaluate_speech -- all`.

#### ce.lid.whichlang 1.0.0+whichlang-0.1.1 — default (min_letters 10, split check on)

| split | length | n | decided | coverage | accuracy (decided) |
|---|---|---|---|---|---|
| dev | all | 42 | 35 | 0.83 | 1.000 |
| dev | long | 4 | 4 | 1.00 | 1.000 |
| dev | medium | 24 | 23 | 0.96 | 1.000 |
| dev | short | 14 | 8 | 0.57 | 1.000 |
| test | all | 84 | 61 | 0.73 | 0.984 |
| test | long | 25 | 24 | 0.96 | 1.000 |
| test | medium | 31 | 24 | 0.77 | 1.000 |
| test | short | 28 | 13 | 0.46 | 0.923 |

Mixed (test): mix-test-00: en; mix-test-01: abstain:Ambiguous; mix-test-02: abstain:Ambiguous; mix-test-03: abstain:Ambiguous; mix-test-04: abstain:Ambiguous; mix-test-05: abstain:Ambiguous; mix-test-06: abstain:Ambiguous; mix-test-07: abstain:Ambiguous

Confusion (test, truth → outcome):

* de → abstain:Ambiguous: 1
* de → abstain:InsufficientEvidence: 1
* en → abstain:Ambiguous: 2
* en → abstain:InsufficientEvidence: 3
* es → abstain:Ambiguous: 1
* es → abstain:InsufficientEvidence: 2
* fr → abstain:InsufficientEvidence: 1
* it → abstain:Ambiguous: 1
* it → abstain:InsufficientEvidence: 3
* it → de: 1
* nl → abstain:Ambiguous: 1
* nl → abstain:InsufficientEvidence: 2
* pt → abstain:Ambiguous: 2
* pt → abstain:InsufficientEvidence: 3

#### ce.lid.whichlang 1.0.0+whichlang-0.1.1 — split check off

| split | length | n | decided | coverage | accuracy (decided) |
|---|---|---|---|---|---|
| dev | all | 42 | 36 | 0.86 | 1.000 |
| dev | long | 4 | 4 | 1.00 | 1.000 |
| dev | medium | 24 | 24 | 1.00 | 1.000 |
| dev | short | 14 | 8 | 0.57 | 1.000 |
| test | all | 84 | 69 | 0.82 | 0.986 |
| test | long | 25 | 25 | 1.00 | 1.000 |
| test | medium | 31 | 31 | 1.00 | 1.000 |
| test | short | 28 | 13 | 0.46 | 0.923 |

Mixed (test): mix-test-00: en; mix-test-01: en; mix-test-02: en; mix-test-03: es; mix-test-04: fr; mix-test-05: it; mix-test-06: en; mix-test-07: en

#### ce.lid.whichlang 1.0.0+whichlang-0.1.1 — no evidence gate (min_letters 0, split check off)

| split | length | n | decided | coverage | accuracy (decided) |
|---|---|---|---|---|---|
| dev | all | 42 | 42 | 1.00 | 0.952 |
| dev | long | 4 | 4 | 1.00 | 1.000 |
| dev | medium | 24 | 24 | 1.00 | 1.000 |
| dev | short | 14 | 14 | 1.00 | 0.857 |
| test | all | 84 | 84 | 1.00 | 0.964 |
| test | long | 25 | 25 | 1.00 | 1.000 |
| test | medium | 31 | 31 | 1.00 | 1.000 |
| test | short | 28 | 28 | 1.00 | 0.893 |

Mixed (test): mix-test-00: en; mix-test-01: en; mix-test-02: en; mix-test-03: es; mix-test-04: fr; mix-test-05: it; mix-test-06: en; mix-test-07: en

#### Oracle pipeline — Dev split

| scenario | refs | finals | missed / split / merged | WER | CER | partials/s | rev/utt | corrections/utt | amended | first partial ms p50/p95/max | first commit ms | final ms | start err ms | LID (utterance) | RTF rec / ana |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| clean / hints | 4 | 4 | 0 / 0 / 0 | 0.000 | 0.000 | 2.2 | 9.0 | 0.00 | 0 | 380 / 590 / 590 | 880 / 1290 / 1290 | 770 / 790 / 790 | 490 / 510 / 510 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.0001 / 0.0073 |
| clean / continuous | 4 | 1 | 3 / 0 / 1 | 0.000 | 0.000 | 2.0 | 33.0 | 0.00 | 0 | – | – | – | – | {"en": 1} | 0.0001 / 0.0073 |
| white 20 dB / hints | 4 | 4 | 0 / 0 / 0 | 0.000 | 0.000 | 2.4 | 9.0 | 0.00 | 0 | 320 / 520 / 520 | 890 / 1160 / 1160 | 600 / 670 / 670 | 470 / 480 / 480 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.0001 / 0.0072 |
| white 10 dB / hints | 4 | 4 | 0 / 0 / 0 | 0.000 | 0.000 | 2.5 | 9.0 | 0.00 | 0 | 430 / 550 / 550 | 940 / 1030 / 1030 | 510 / 580 / 580 | 370 / 460 / 460 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.0001 / 0.0070 |
| white 5 dB / hints | 4 | 3 | 1 / 0 / 0 | 0.469 | 0.497 | 2.4 | 6.7 | 0.00 | 0 | 570 / 2180 / 2180 | 980 / 2400 / 2400 | 530 / 570 / 570 | 450 / 1600 / 1600 | {"abstain:InsufficientEvidence": 1, "en": 2} | 0.0001 / 0.0072 |
| pink 10 dB / hints | 4 | 4 | 0 / 0 / 0 | 0.000 | 0.000 | 2.2 | 9.0 | 0.00 | 0 | 330 / 530 / 530 | 890 / 1310 / 1310 | 790 / 1350 / 1350 | 390 / 470 / 470 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.0001 / 0.0052 |
| music 10 dB / hints | 4 | 1 | 3 / 0 / 1 | 0.000 | 0.000 | 2.0 | 33.0 | 0.00 | 0 | – | – | – | – | {"en": 1} | 0.0001 / 0.0051 |
| clipped +20 dB / hints | 4 | 4 | 0 / 0 / 0 | 0.000 | 0.000 | 2.2 | 9.0 | 0.00 | 0 | 380 / 570 / 570 | 880 / 1290 / 1290 | 820 / 820 / 820 | 470 / 730 / 730 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.0001 / 0.0076 |
| quiet -30 dB / hints | 4 | 4 | 0 / 0 / 0 | 0.000 | 0.000 | 2.4 | 9.0 | 0.00 | 0 | 320 / 520 / 520 | 890 / 1160 / 1160 | 600 / 670 / 670 | 470 / 480 / 480 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.0001 / 0.0073 |

#### Oracle pipeline — Test split

| scenario | refs | finals | missed / split / merged | WER | CER | partials/s | rev/utt | corrections/utt | amended | first partial ms p50/p95/max | first commit ms | final ms | start err ms | LID (utterance) | RTF rec / ana |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| clean / hints | 6 | 6 | 0 / 0 / 0 | 0.000 | 0.000 | 2.6 | 12.0 | 0.00 | 0 | 300 / 400 / 400 | 900 / 1170 / 1170 | 770 / 790 / 790 | 500 / 540 / 540 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.0001 / 0.0055 |
| clean / continuous | 6 | 2 | 4 / 0 / 2 | 0.000 | 0.000 | 2.4 | 34.0 | 0.00 | 0 | – | – | – | – | {"en": 2} | 0.0001 / 0.0054 |
| white 20 dB / hints | 6 | 6 | 0 / 0 / 0 | 0.000 | 0.000 | 2.7 | 12.0 | 0.00 | 0 | 320 / 440 / 440 | 820 / 1010 / 1010 | 630 / 690 / 690 | 460 / 490 / 490 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.0001 / 0.0053 |
| white 10 dB / hints | 6 | 6 | 1 / 1 / 0 | 0.197 | 0.195 | 2.8 | 9.8 | 0.00 | 0 | 340 / 840 / 840 | 940 / 980 / 980 | 540 / 680 / 680 | 460 / 470 / 470 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.0001 / 0.0054 |
| white 5 dB / hints | 6 | 3 | 3 / 0 / 0 | 0.348 | 0.334 | 2.9 | 15.3 | 0.00 | 0 | 340 / 350 / 350 | 840 / 950 / 950 | 530 / 670 / 670 | 460 / 460 / 460 | {"en": 3} | 0.0001 / 0.0070 |
| pink 10 dB / hints | 6 | 6 | 0 / 0 / 0 | 0.000 | 0.000 | 2.5 | 12.0 | 0.00 | 0 | 320 / 450 / 450 | 850 / 1700 / 1700 | 980 / 1310 / 1310 | 380 / 490 / 490 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.0001 / 0.0051 |
| music 10 dB / hints | 6 | 2 | 4 / 0 / 2 | 0.000 | 0.000 | 2.4 | 34.0 | 0.00 | 0 | – | – | – | – | {"en": 2} | 0.0001 / 0.0051 |
| clipped +20 dB / hints | 6 | 6 | 0 / 0 / 0 | 0.000 | 0.000 | 2.5 | 12.0 | 0.00 | 0 | 280 / 450 / 450 | 880 / 1230 / 1230 | 810 / 910 / 910 | 520 / 750 / 750 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.0002 / 0.0061 |
| quiet -30 dB / hints | 6 | 6 | 0 / 0 / 0 | 0.000 | 0.000 | 2.7 | 12.0 | 0.00 | 0 | 320 / 430 / 430 | 820 / 1010 / 1010 | 640 / 680 / 680 | 470 / 490 / 490 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.0001 / 0.0057 |

#### Commit policies with revision noise (truncate 0.3, rewrite 0.15) — Dev

| scenario | refs | finals | missed / split / merged | WER | CER | partials/s | rev/utt | corrections/utt | amended | first partial ms p50/p95/max | first commit ms | final ms | start err ms | LID (utterance) | RTF rec / ana |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| default 3 hyp / 800 ms / hold 1 | 4 | 4 | 0 / 0 / 0 | 0.000 | 0.000 | 2.8 | 11.0 | 5.50 | 0 | 380 / 590 / 590 | 810 / 1290 / 1290 | 770 / 790 / 790 | 490 / 510 / 510 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.0001 / 0.0052 |
| 2 hyp / off / hold 1 | 4 | 4 | 0 / 0 / 0 | 0.000 | 0.000 | 2.9 | 11.2 | 5.50 | 0 | 380 / 590 / 590 | 580 / 1290 / 1290 | 770 / 790 / 790 | 490 / 510 / 510 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.0001 / 0.0054 |
| 3 hyp / off / hold 1 | 4 | 4 | 0 / 0 / 0 | 0.000 | 0.000 | 2.8 | 11.0 | 5.50 | 0 | 380 / 590 / 590 | 810 / 1290 / 1290 | 770 / 790 / 790 | 490 / 510 / 510 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.0001 / 0.0051 |
| 5 hyp / off / hold 1 | 4 | 4 | 0 / 0 / 0 | 0.000 | 0.000 | 3.0 | 11.8 | 6.50 | 0 | 380 / 590 / 590 | 1210 / 1480 / 1480 | 770 / 790 / 790 | 490 / 510 / 510 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.0001 / 0.0051 |
| off / 400 ms / hold 0 | 4 | 4 | 0 / 0 / 0 | 0.000 | 0.000 | 2.8 | 11.0 | 5.50 | 0 | 380 / 590 / 590 | 810 / 1290 / 1290 | 770 / 790 / 790 | 490 / 510 / 510 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.0001 / 0.0051 |
| off / 800 ms / hold 1 | 4 | 4 | 0 / 0 / 0 | 0.000 | 0.000 | 3.0 | 11.8 | 6.50 | 0 | 380 / 590 / 590 | 1210 / 1480 / 1480 | 770 / 790 / 790 | 490 / 510 / 510 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.0001 / 0.0053 |
| 2 hyp / off / hold 0 | 4 | 4 | 0 / 0 / 0 | 0.000 | 0.000 | 3.1 | 12.0 | 6.00 | 1 | 380 / 590 / 590 | 580 / 1290 / 1290 | 770 / 790 / 790 | 490 / 510 / 510 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.0001 / 0.0052 |

#### Commit policies with revision noise (truncate 0.3, rewrite 0.15) — Test

| scenario | refs | finals | missed / split / merged | WER | CER | partials/s | rev/utt | corrections/utt | amended | first partial ms p50/p95/max | first commit ms | final ms | start err ms | LID (utterance) | RTF rec / ana |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| default 3 hyp / 800 ms / hold 1 | 6 | 6 | 0 / 0 / 0 | 0.000 | 0.000 | 3.1 | 14.3 | 7.83 | 0 | 300 / 400 / 400 | 900 / 1170 / 1170 | 770 / 790 / 790 | 500 / 540 / 540 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.0001 / 0.0058 |

#### Robustness — test split, oracle

| scenario | refs | finals | missed / split / merged | WER | CER | partials/s | rev/utt | corrections/utt | amended | first partial ms p50/p95/max | first commit ms | final ms | start err ms | LID (utterance) | RTF rec / ana |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 16 kHz mono 10 ms | 6 | 6 | 0 / 0 / 0 | 0.000 | 0.000 | 2.6 | 12.0 | 0.00 | 0 | 300 / 400 / 400 | 900 / 1170 / 1170 | 770 / 790 / 790 | 500 / 540 / 540 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.0001 / 0.0052 |
| 48 kHz stereo 10 ms | 6 | 6 | 0 / 0 / 0 | 0.000 | 0.000 | 2.6 | 12.0 | 0.00 | 0 | 310 / 410 / 410 | 910 / 1150 / 1150 | 750 / 780 / 780 | 500 / 580 / 580 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.0009 / 0.0055 |
| 44.1 kHz mono 512-frame blocks | 6 | 6 | 0 / 0 / 0 | 0.000 | 0.000 | 2.6 | 12.0 | 0.00 | 0 | 300 / 407 / 407 | 904 / 1156 / 1156 | 766 / 801 / 801 | 500 / 700 / 700 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.0017 / 0.0059 |
| 8 kHz mono 20 ms | 6 | 6 | 0 / 0 / 0 | 0.000 | 0.000 | 2.6 | 12.0 | 0.00 | 0 | 308 / 418 / 418 | 908 / 1178 / 1178 | 778 / 798 / 798 | 500 / 540 / 540 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.0012 / 0.0026 |
| 3 × 100 ms recognizer drops | 6 | 8 | 0 / 2 / 0 | 0.015 | 0.015 | 2.6 | 9.1 | 0.00 | 0 | 300 / 400 / 400 | 790 / 1170 / 1170 | 360 / 790 / 790 | 500 / 540 / 540 | {"abstain:InsufficientEvidence": 2, "en": 6} | 0.0001 / 0.0051 |
| throttled (forced load 2.0) | 6 | 6 | 0 / 0 / 0 | 0.000 | 0.000 | 0.0 | 1.0 | 0.00 | 0 | – | 3100 / 6920 / 6920 | 770 / 790 / 790 | 500 / 540 / 540 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.0000 / 0.0052 |
| no analysis, continuous, 5 s max | 6 | 6 | 1 / 1 / 4 | 0.000 | 0.000 | 2.4 | 12.0 | 0.00 | 0 | 350 / 1050 / 1050 | 950 / 1650 / 1650 | -100 / 1710 / 1710 | 850 / 1450 / 1450 | {"en": 6} | 0.0000 / 0.0000 |

Replay determinism: digest 778427eea7270385 vs 778427eea7270385 → identical

## Reading the results

**Language identification (held-out test, whichlang + CE policy).**
Accuracy on decided items 0.984 (60/61) at 0.73 coverage; long 1.00 at
0.96, medium 1.00 at 0.77, short 0.92 at 0.46 coverage. Without the
evidence gate the identifier always answers and is wrong on 11 % of short
items (dev 0.857, test 0.893). The split-half check abstains as
`Ambiguous` on 7 of 8 mixed-language items (the eighth is labelled `en`)
and costs 8 of 84 single-language test items in coverage (1 of 42 on dev).
One single-language error remains (an Italian sentence identified as
German). These are **text** results on authored sentences, not on ASR
output.

**Oracle pipeline (held-out test).** Clean, white 20 dB, pink 10 dB,
clipped and quiet −30 dB: 6/6 finals, nothing missed, split or merged,
oracle WER 0.000, first partial p50 ≈ 300–340 ms after speech start,
finals p50 ≈ 630–980 ms after speech end (the Phase 3 default
`close_silence` plus `trailing_ms` dominate). Utterances start ≈ 500 ms
before the labelled speech start: that is the `lead_in_ms` default, chosen
with the real model, and the frame range reports it exactly. White 10 dB: 1 missed and 1 split reference (WER
0.197); white 5 dB: 3 of 6 missed (WER 0.348); music 10 dB: utterances
merge into 2 finals (activity never ends) — these are **inherited Phase 3
detector limits** (Phase 3 report: music 0/6 endpoints, white 5 dB recall
0.60), and the recognizer cannot recover speech the detector does not
delimit. `Continuous` with the oracle (no endpointing) merges everything
until the 20 s maximum, as designed; with a 5 s maximum and no analysis
the stream is cut every 5 s (4 merges, 1 split).

**Commit policy (dev; ADR-0021).** With synthetic revision noise
(30 % truncated last word, 15 % rewritten word 3 back), every policy keeps
WER 0 (corrections are never suppressed). Count-only policies commit
earlier (2 hypotheses: first commit p50 580 ms vs 810 ms for the default)
with the same visible corrections (5.5 per utterance); hold-back 0 needed
1 amending final; 5 hypotheses or an 800 ms time rule show more
corrections (6.5). **Decision: keep the default (3 hypotheses / 800 ms /
hold-back 1)**: it never amended on dev or test, and the earlier commit of
the 2-hypothesis policy rests on 4 dev utterances. The dev comparison was
repeated with the real model (ADR-0023): identical transcripts, the
default 0 amendments, 2 hypotheses committing ≈ 290 ms earlier, so the
default was kept. Test (held out, oracle noise): default policy 0
amendments, first commit p50 900 ms, 7.8 corrections per utterance.

**Robustness (test).** 16 kHz mono, 48 kHz stereo, 44.1 kHz in 512-frame
blocks and 8 kHz: identical finals and WER 0; latencies differ by ≤ 20 ms
(block granularity). Three 100 ms
recognizer-only drops: each affected utterance is split at the gap (never
bridged), the utterance resumes after the gap, 1 word of 66 lost (WER
0.015). Forced overload (`Throttled`): no partials, all finals intact, first
commit only at the final. Replay is deterministic (identical digests).

## Resource and runtime evidence (fake backend; not hardware)

| Measurement | Value | Label |
|---|---|---|
| CE recognizer overhead per audio second (oracle adapter, replay) | RTF ≈ 0.0001 (16 kHz) to 0.0015 (44.1 kHz, resampling) | replay, this machine |
| Phase 3 analyzer in the same replays | RTF ≈ 0.005 | replay, this machine |
| `speech_smoke` (release): 2 capture streams on the real-time fake driver, 1 analysis, 2 recognitions (scripted adapter), 5 s | 0.17 s user + 0.12 s sys CPU over 5.03 s wall (≈ 6 % of one core for the whole process); peak RSS 12.7 MiB | fake backend |
| `evaluate_speech robustness` (release) | 1.28 s CPU; peak RSS 27.3 MiB | replay |
| Per-recognition bounded memory | tap ≤ `queue_blocks` blocks; hint queue ≤ `hint_queue`; pre-roll ring ≤ `preroll_ms` (+ the open utterance's unfed audio); ≤ 64 boundaries, ≤ 64 deferred hints; latency reservoirs ≤ 1024 values | code + `memory_stays_bounded_over_long_streams_and_hint_floods` (10 min of audio) |

Model load time, model RTF, model RAM and warm-up cost are **unknown**
until a real adapter is approved and measured.

## Proposed targets (for the first real adapter, from this evidence)

These are proposals to verify, not results:

* First partial ≤ 500 ms p95 after speech start and final ≤ 1000 ms p95
  after speech end on the clean test scenarios (the oracle pipeline already
  spends ≈ 400 ms and ≈ 790 ms; a streaming model adds its own look-ahead).
* Zero merged/split references on clean and 20 dB scenarios (pipeline
  achieves this).
* Model RTF ≤ 0.3 on a 4-vCPU x86 container for one stream, so that four
  concurrent streams stay below one core-second per second.
* Commit amendments ≤ 5 % of finals under the default policy.
