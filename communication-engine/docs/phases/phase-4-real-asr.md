# Phase 4 real-ASR baseline — sherpa-onnx streaming Zipformer (INT8)

Date: 2026-11-24 · Machine: cloud container, Linux x86_64, 4 vCPU Intel
Xeon @ 2.30 GHz, **no audio hardware** · Build: release, Rust 1.97.0 ·
Adapter: `adapters/ce-asr-sherpa` (ADR-0023).

## Evidence labels (read first)

| Label | What it means in this document |
|---|---|
| **REAL MODEL** | Transcripts are produced by the approved model, `sherpa-onnx-streaming-zipformer-en-2023-06-26`, INT8 encoder/decoder/joiner (70.9 MB), run through sherpa-onnx 2.0.3 / ONNX Runtime on the CPU with 1 thread and greedy search. These are genuine recognition results. |
| **SYNTHETIC speech** | The input is the 10 CMU Flite TTS fixtures (`fixtures/speech`), with generated noise, music, clipping, level and format variants. **No human speech was used**, because real speech corpora are not approved. TTS is cleaner and more regular than human speech in some ways, and its voices are unlike the model's training data (LibriSpeech read speech) in others. These numbers do **not** predict accuracy on people. |
| **REPLAY** | Blocks are fed in lock-step through the Phase 3 analyzer and the recognizer on one thread (`ce_speech::eval::replay`). Latencies are in **audio time**: they include the model's own look-ahead and chunking, Phase 3 decisions, partial cadence and finalization, but not scheduling or compute waiting. Processing times (RTF, load) are wall-clock on this machine. |
| **fake backend** | The two runtime tests run the real model inside the `Engine` on the test kit's fake audio backend. |
| **oracle** | Not used here except for the comparison lines quoted from [phase-4-evaluation.md](phase-4-evaluation.md). |

Small samples: dev = 4 utterances (32 words) and test = 6 utterances (66
words). One word is 1.5 points of test WER. Differences of a few words are
noise.

## Method

* **Unchanged pipeline.** The same `eval::build` / `replay` / `score` and
  the same scoring rules as [phase-4-evaluation.md](phase-4-evaluation.md)
  (normalized WER/CER at stream level, utterance mapping, audio-time
  latencies, revision metrics). The only change is the adapter.
* **Isolated decoding**: each fixture fed whole to the adapter in 100 ms
  chunks and then finalized, without the pipeline. This is the model's
  ceiling on this audio.
* **Tuning on dev only.** Two parameters were chosen on dev with the real
  model: `lead_in_ms` (new, see below) and the commit policy (kept). Test
  is only reported.
* **No casing/punctuation WER**: the model emits upper-case words without
  punctuation (declared `casing: false, punctuation: false`), so the
  case/punctuation-sensitive WER is not meaningful.
* Reproduce: `adapters/ce-asr-sherpa/scripts/fetch-model.sh`, then
  `CE_SHERPA_MODEL_DIR=… SHERPA_ONNX_ARCHIVE_DIR=… cargo run --release
  --example evaluate_real -- all` in `adapters/ce-asr-sherpa`.

## Finding and fix: lead-in before Phase 3 onsets

The first real-model run lost utterance-initial words in the pipeline
("TURN LEFT…" → "LEFT…", "HOW MUCH IS…" → "IS…"). Decoding the pipeline's
exact frame range directly reproduced the loss, so the feeding code was
correct. The cause was the utterance **start**: Phase 3's energy/spectral
onset (like the energy-based reference labels) fires after soft word
beginnings such as unvoiced /t/ and /h/. The oracle could not show this
because it ignores the audio.

Fix: `RecognitionConfig::lead_in_ms`, audio fed before each onset from the
pre-roll ring. The utterance's frame range includes it, so nothing is
shifted. The dev sweep (clean + white 20 dB + pink 10 dB, 96 words) is
below. With a 500 ms pre-roll, lead-in was also clamped by hint lag, so the
pre-roll default was raised to 1000 ms (≈ 64 KB per stream at 16 kHz) and
the sweep was re-run. **Chosen: 500 ms**, the smallest value on the
500–800 ms plateau. Before the fix, held-out clean pipeline WER was 0.288
(19/66); after it, 0.121 (8/66).

## Results (verbatim output of `evaluate_real all`)

Evidence: REAL MODEL recognition (ce.asr.sherpa-zipformer 0.1.0+sherpa-onnx-2.0.3, Some(("sherpa-onnx-streaming-zipformer-en-2023-06-26", Some("sha256sums:4c293cc8f1e3bfef"), Some(70913968)))) on SYNTHETIC TTS speech in REPLAY. Not human speech; no device.

#### Resources and determinism (this machine)

* model load (first, reused=false): 1964 ms
* second adapter load (reused=true): 7 µs
* peak RSS before load 23096 KiB, after load 151996 KiB
* test clean replay: audio 27.9 s, recognizer processing 1.94 s (RTF 0.069), analyzer 0.157 s
* determinism (num_threads = 1): digest e14d6627279b8c69 vs e14d6627279b8c69 → identical
* 4 concurrent replays sharing one model: 2.20 s wall for 4 × 27.9 s audio (aggregate 0.079 s of wall per s of audio per stream)
* peak RSS of the whole evaluation process so far: 199000 KiB

#### Isolated utterances (adapter only, clean, 100 ms chunks)

| split | id | reference | final hypothesis | WER | CER | first non-empty partial (audio ms) | partials | RTF |
|---|---|---|---|---|---|---|---|---|
| dev | dev_awb_yes | Yes | YES | 0.000 | 0.000 | – | 0 | 0.095 |
| dev | dev_kal16_station | Good morning, could you tell me where the train station is | GOOD MORNING COULD YOU TELL ME WHERE THE TRAIN STATION IS | 0.000 | 0.000 | 800 | 8 | 0.075 |
| dev | dev_rms_meeting | The meeting starts at nine thirty tomorrow | THE MEETING STARTS AT NINE THIRTY TO MO | 0.286 | 0.119 | 800 | 7 | 0.067 |
| dev | dev_slt_coffee | I would like a cup of coffee and a glass of water, please | I WOULD LIKE A CUP OF COFFEE AND A GLASS OF WATER PLEA | 0.077 | 0.036 | 800 | 9 | 0.068 |
| Dev | **total** | | | **0.094** (3/32) | **0.044** (7/158) | | | |
| test | test_awb_directions | Turn left at the second corner and walk straight for two blocks | TURN LEFT AT THE SECOND CORNER AND WALKED STRAIGHT FOR TWO BALL | 0.167 | 0.111 | 1200 | 9 | 0.065 |
| test | test_kal16_fast | How much does this cost, and do you accept cards | HOW MUCH IS THIS COST AND DULY ACCEPT CAR | 0.400 | 0.191 | 800 | 6 | 0.067 |
| test | test_kal16_slowly | Can you please speak a little more slowly, I am still learning | CAN YOU PLEASE SPEAK A LITTLE MORE SLOWLY I AM STILL LEAR | 0.083 | 0.066 | 1200 | 9 | 0.068 |
| test | test_rms_no | No |  | 1.000 | 1.000 | – | 0 | 0.064 |
| test | test_slt_long | We should leave early, because the traffic will be heavy in the evening, and the road near the bridge is closed for repairs | WE SHOULD LEAVE EARLY BECAUSE THE TRAFFIC WILL BE HEAVY IN THE EVENING AND THE ROAD NEAR THE BRIDGE IS CLOSED FOR REPA | 0.043 | 0.025 | 800 | 17 | 0.067 |
| test | test_slt_thanks | Thank you very much for your help today | THANK YOU VERY MUCH FOR YOUR HELP TO DAY | 0.250 | 0.026 | 1200 | 5 | 0.069 |
| Test | **total** | | | **0.167** (11/66) | **0.078** (26/333) | | | |

#### Partial transcript behaviour — first two test utterances (real model, clean)

| utterance | rev | status | input ms | committed | provisional | amended |
|---|---|---|---|---|---|---|
| 0 | 0 | Partial | 2410 |  | TURN LEFT |  |
| 0 | 1 | Partial | 2710 |  | TURN LEFT AT |  |
| 0 | 2 | Partial | 3010 | TURN LEFT | AT THE SEC |  |
| 0 | 3 | Partial | 3310 | TURN LEFT AT | THE SECOND |  |
| 0 | 4 | Partial | 3710 | TURN LEFT AT THE | SECOND CORNER |  |
| 0 | 5 | Partial | 4010 | TURN LEFT AT THE SECOND | CORNER AND |  |
| 0 | 6 | Partial | 4310 | TURN LEFT AT THE SECOND CORNER | AND WALKED STRA |  |
| 0 | 7 | Partial | 4610 | TURN LEFT AT THE SECOND CORNER AND | WALKED STRAIGHT |  |
| 0 | 8 | Partial | 4910 | TURN LEFT AT THE SECOND CORNER AND WALKED | STRAIGHT FOR TWO |  |
| 0 | 9 | Partial | 5310 | TURN LEFT AT THE SECOND CORNER AND WALKED STRAIGHT | FOR TWO BO |  |
| 0 | 10 | Partial | 5610 | TURN LEFT AT THE SECOND CORNER AND WALKED STRAIGHT FOR TWO | BOX |  |
| 0 | 11 | Final | 5870 | TURN LEFT AT THE SECOND CORNER AND WALKED STRAIGHT FOR TWO BOX |  | false |
| 1 | 0 | Partial | 7070 |  | HOW |  |
| 1 | 1 | Partial | 7370 |  | HOW MUCH DOES |  |
| 1 | 2 | Partial | 7670 | HOW | MUCH DOES THIS |  |
| 1 | 3 | Partial | 7970 | HOW MUCH DOES | THIS COST |  |
| 1 | 4 | Partial | 8270 | HOW MUCH DOES THIS | COST AND DO YOU |  |
| 1 | 5 | Partial | 8670 | HOW MUCH DOES THIS COST | AND DO YOU ACCE |  |
| 1 | 6 | Partial | 8970 | HOW MUCH DOES THIS COST AND DO YOU | ACCEPT CAR |  |
| 1 | 7 | Partial | 9270 | HOW MUCH DOES THIS COST AND DO YOU | ACCEPT CARDS |  |
| 1 | 8 | Final | 9420 | HOW MUCH DOES THIS COST AND DO YOU ACCEPT CARDS |  | false |

#### lead_in_ms sweep — Dev only (clean + white 20 dB + pink 10 dB)

| lead_in_ms | WER (edits/words) | CER | first partial p50 ms | start err p50 ms |
|---|---|---|---|---|
| 0 | 0.271 (26/96) | 0.190 | [800.0, 1220.0, 1310.0] | [10.0, 20.0, 30.0] |
| 100 | 0.188 (18/96) | 0.097 | [700.0, 730.0, 810.0] | [100.0, 80.0, 70.0] |
| 200 | 0.198 (19/96) | 0.106 | [610.0, 630.0, 710.0] | [200.0, 170.0, 90.0] |
| 300 | 0.135 (13/96) | 0.068 | [870.0, 920.0, 930.0] | [300.0, 270.0, 190.0] |
| 400 | 0.188 (18/96) | 0.091 | [790.0, 820.0, 830.0] | [400.0, 370.0, 290.0] |
| 500 | 0.115 (11/96) | 0.052 | [710.0, 730.0, 810.0] | [490.0, 470.0, 390.0] |
| 600 | 0.125 (12/96) | 0.060 | [620.0, 790.0, 790.0] | [580.0, 570.0, 410.0] |
| 700 | 0.115 (11/96) | 0.052 | [620.0, 820.0, 830.0] | [580.0, 670.0, 410.0] |
| 800 | 0.125 (12/96) | 0.062 | [620.0, 760.0, 790.0] | [580.0, 740.0, 410.0] |

#### Real model through the pipeline — Dev split (activity hints unless stated)

| scenario | refs | finals | missed / split / merged | WER | CER | partials/s | rev/utt | corrections/utt | amended | first partial ms p50/p95/max | first commit ms | final ms | start err ms | LID (utterance) | RTF rec / ana |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| clean | 4 | 4 | 0 / 0 / 0 | 0.062 (2/32) | 0.006 | 2.0 | 8.2 | 1.50 | 0 | 710 / 990 / 990 | 1300 / 1680 / 1680 | 770 / 790 / 790 | 490 / 510 / 510 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.064 / 0.0053 |
| white 20 dB | 4 | 4 | 0 / 0 / 0 | 0.125 (4/32) | 0.037 | 2.0 | 7.8 | 1.50 | 0 | 730 / 1020 / 1020 | 1320 / 1790 / 1790 | 600 / 670 / 670 | 470 / 480 / 480 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.060 / 0.0052 |
| white 10 dB | 4 | 4 | 0 / 0 / 0 | 0.500 (16/32) | 0.286 | 2.0 | 7.2 | 1.25 | 0 | 830 / 1140 / 1140 | 1340 / 1740 / 1740 | 510 / 580 / 580 | 370 / 460 / 460 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.056 / 0.0052 |
| white 5 dB | 4 | 3 | 1 / 0 / 0 | 0.625 (20/32) | 0.627 | 1.4 | 4.3 | 0.67 | 0 | 750 / 2800 / 2800 | 1350 / 2930 / 2930 | 530 / 570 / 570 | 450 / 1600 / 1600 | {"abstain:InsufficientEvidence": 2, "en": 1} | 0.030 / 0.0053 |
| pink 10 dB | 4 | 4 | 0 / 0 / 0 | 0.156 (5/32) | 0.112 | 1.8 | 7.5 | 1.25 | 0 | 810 / 1030 / 1030 | 1330 / 1790 / 1790 | 790 / 1350 / 1350 | 390 / 470 / 470 | {"abstain:Ambiguous": 1, "abstain:InsufficientEvidence": 1, "en": 2} | 0.067 / 0.0054 |
| music 10 dB | 4 | 4 | 1 / 1 / 3 | 0.344 (11/32) | 0.230 | 1.4 | 6.5 | 1.50 | 0 | 1630 / 1630 / 1630 | 2230 / 2230 / 2230 | 1620 / 1620 / 1620 | 430 / 430 / 430 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.077 / 0.0064 |
| clipped +20 dB | 4 | 4 | 0 / 0 / 0 | 0.219 (7/32) | 0.124 | 1.8 | 7.5 | 0.75 | 0 | 780 / 1070 / 1070 | 1330 / 1680 / 1680 | 820 / 820 / 820 | 470 / 730 / 730 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.064 / 0.0054 |
| quiet -30 dB | 4 | 4 | 0 / 0 / 0 | 0.281 (9/32) | 0.217 | 1.8 | 7.2 | 1.00 | 0 | 730 / 1020 / 1020 | 1320 / 2090 / 2090 | 600 / 670 / 670 | 470 / 480 / 480 | {"abstain:Ambiguous": 1, "abstain:InsufficientEvidence": 1, "en": 2} | 0.070 / 0.0070 |
| clean / continuous (model endpointing, no analysis) | 4 | 3 | 2 / 1 / 3 | 0.219 (7/32) | 0.186 | 1.5 | 9.0 | 2.00 | 0 | – | – | – | – | {"abstain:InsufficientEvidence": 1, "en": 2} | 0.068 / 0.0000 |

#### Real model through the pipeline — Test split (activity hints unless stated)

| scenario | refs | finals | missed / split / merged | WER | CER | partials/s | rev/utt | corrections/utt | amended | first partial ms p50/p95/max | first commit ms | final ms | start err ms | LID (utterance) | RTF rec / ana |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| clean | 6 | 6 | 0 / 0 / 0 | 0.121 (8/66) | 0.044 | 2.2 | 10.5 | 2.67 | 0 | 750 / 1000 / 1000 | 1300 / 1590 / 1590 | 770 / 790 / 790 | 500 / 540 / 540 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.077 / 0.0065 |
| white 20 dB | 6 | 6 | 0 / 0 / 0 | 0.288 (19/66) | 0.136 | 2.3 | 10.3 | 3.00 | 0 | 760 / 1040 / 1040 | 1350 / 1640 / 1640 | 630 / 690 / 690 | 460 / 490 / 490 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.072 / 0.0065 |
| white 10 dB | 6 | 6 | 1 / 1 / 0 | 0.652 (43/66) | 0.462 | 1.9 | 7.0 | 2.00 | 0 | 780 / 1040 / 1040 | 1600 / 1640 / 1640 | 540 / 680 / 680 | 460 / 470 / 470 | {"abstain:InsufficientEvidence": 3, "en": 3} | 0.051 / 0.0057 |
| white 5 dB | 6 | 3 | 3 / 0 / 0 | 0.894 (59/66) | 0.675 | 2.2 | 12.0 | 4.00 | 0 | 740 / 750 / 750 | 1640 / 1650 / 1650 | 530 / 670 / 670 | 460 / 460 / 460 | {"en": 3} | 0.039 / 0.0054 |
| pink 10 dB | 6 | 6 | 0 / 0 / 0 | 0.591 (39/66) | 0.305 | 1.9 | 9.3 | 1.50 | 0 | 780 / 1050 / 1050 | 1420 / 1910 / 1910 | 980 / 1310 / 1310 | 380 / 490 / 490 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.068 / 0.0064 |
| music 10 dB | 6 | 4 | 2 / 0 / 3 | 0.773 (51/66) | 0.509 | 1.9 | 14.5 | 4.50 | 0 | 1650 / 1650 / 1650 | 2550 / 2550 / 2550 | 1710 / 1710 / 1710 | 450 / 450 / 450 | {"en": 4} | 0.077 / 0.0059 |
| clipped +20 dB | 6 | 6 | 0 / 0 / 0 | 0.439 (29/66) | 0.263 | 2.0 | 10.0 | 2.83 | 0 | 750 / 1050 / 1050 | 1480 / 1880 / 1880 | 810 / 910 / 910 | 520 / 750 / 750 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.070 / 0.0063 |
| quiet -30 dB | 6 | 8 | 0 / 2 / 0 | 0.591 (39/66) | 0.521 | 1.2 | 4.6 | 0.75 | 0 | 1010 / 1060 / 1060 | 1620 / 2360 / 2360 | 260 / 680 / 680 | 470 / 490 / 490 | {"abstain:InsufficientEvidence": 5, "en": 3} | 0.066 / 0.0057 |
| clean / continuous (model endpointing, no analysis) | 6 | 4 | 2 / 0 / 4 | 0.182 (12/66) | 0.107 | 1.9 | 14.0 | 3.25 | 0 | – | – | – | – | {"en": 4} | 0.083 / 0.0000 |

#### Commit policies with the real model — Dev (clean)

| scenario | refs | finals | missed / split / merged | WER | CER | partials/s | rev/utt | corrections/utt | amended | first partial ms p50/p95/max | first commit ms | final ms | start err ms | LID (utterance) | RTF rec / ana |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| default 3 hyp / 800 ms / hold 1 | 4 | 4 | 0 / 0 / 0 | 0.062 (2/32) | 0.006 | 2.0 | 8.2 | 1.50 | 0 | 710 / 990 / 990 | 1300 / 1680 / 1680 | 770 / 790 / 790 | 490 / 510 / 510 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.066 / 0.0056 |
| 2 hyp / off / hold 1 | 4 | 4 | 0 / 0 / 0 | 0.062 (2/32) | 0.006 | 2.0 | 8.2 | 1.50 | 0 | 710 / 990 / 990 | 1010 / 1290 / 1290 | 770 / 790 / 790 | 490 / 510 / 510 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.061 / 0.0052 |
| 5 hyp / off / hold 1 | 4 | 4 | 0 / 0 / 0 | 0.062 (2/32) | 0.006 | 2.0 | 8.2 | 1.50 | 0 | 710 / 990 / 990 | 1900 / 2380 / 2380 | 770 / 790 / 790 | 490 / 510 / 510 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.065 / 0.0053 |
| off / 400 ms / hold 0 | 4 | 4 | 0 / 0 / 0 | 0.062 (2/32) | 0.006 | 2.0 | 8.2 | 1.50 | 0 | 710 / 990 / 990 | 1300 / 1680 / 1680 | 770 / 790 / 790 | 490 / 510 / 510 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.058 / 0.0057 |
| 2 hyp / off / hold 0 | 4 | 4 | 0 / 0 / 0 | 0.062 (2/32) | 0.006 | 2.0 | 8.2 | 1.50 | 0 | 710 / 990 / 990 | 1010 / 1290 / 1290 | 770 / 790 / 790 | 490 / 510 / 510 | {"abstain:InsufficientEvidence": 1, "en": 3} | 0.063 / 0.0056 |

#### Commit policies with the real model — Test (clean)

| scenario | refs | finals | missed / split / merged | WER | CER | partials/s | rev/utt | corrections/utt | amended | first partial ms p50/p95/max | first commit ms | final ms | start err ms | LID (utterance) | RTF rec / ana |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| default 3 hyp / 800 ms / hold 1 | 6 | 6 | 0 / 0 / 0 | 0.121 (8/66) | 0.044 | 2.2 | 10.5 | 2.67 | 0 | 750 / 1000 / 1000 | 1300 / 1590 / 1590 | 770 / 790 / 790 | 500 / 540 / 540 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.059 / 0.0053 |

#### Formats, drops and throttling — test split, real model

| scenario | refs | finals | missed / split / merged | WER | CER | partials/s | rev/utt | corrections/utt | amended | first partial ms p50/p95/max | first commit ms | final ms | start err ms | LID (utterance) | RTF rec / ana |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 16 kHz mono 10 ms | 6 | 6 | 0 / 0 / 0 | 0.121 (8/66) | 0.044 | 2.2 | 10.5 | 2.67 | 0 | 750 / 1000 / 1000 | 1300 / 1590 / 1590 | 770 / 790 / 790 | 500 / 540 / 540 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.078 / 0.0064 |
| 48 kHz stereo 10 ms | 6 | 6 | 0 / 0 / 0 | 0.167 (11/66) | 0.056 | 2.2 | 10.5 | 2.83 | 0 | 730 / 1010 / 1010 | 1310 / 1600 / 1600 | 750 / 780 / 780 | 500 / 580 / 580 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.070 / 0.0060 |
| 44.1 kHz mono 512-frame blocks | 6 | 6 | 0 / 0 / 0 | 0.136 (9/66) | 0.041 | 2.2 | 10.7 | 2.50 | 0 | 762 / 1005 / 1005 | 1310 / 1601 / 1601 | 766 / 801 / 801 | 500 / 700 / 700 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.069 / 0.0058 |
| 8 kHz mono 20 ms | 6 | 6 | 0 / 0 / 0 | 0.227 (15/66) | 0.086 | 2.2 | 10.5 | 2.33 | 0 | 758 / 1018 / 1018 | 1318 / 1608 / 1608 | 778 / 798 / 798 | 500 / 540 / 540 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.072 / 0.0030 |
| 3 × 100 ms recognizer drops | 6 | 8 | 0 / 2 / 0 | 0.242 (16/66) | 0.121 | 2.0 | 7.4 | 1.75 | 0 | 750 / 1000 / 1000 | 1300 / 1590 / 1590 | 360 / 790 / 790 | 500 / 540 / 540 | {"abstain:InsufficientEvidence": 3, "en": 5} | 0.071 / 0.0059 |
| throttled (forced load 2.0) | 6 | 6 | 0 / 0 / 0 | 0.121 (8/66) | 0.044 | 0.0 | 1.0 | 0.00 | 0 | – | 3100 / 6920 / 6920 | 770 / 790 / 790 | 500 / 540 / 540 | {"abstain:InsufficientEvidence": 1, "en": 5} | 0.071 / 0.0059 |

## Summary of genuine model results (held-out test, synthetic TTS)

| Measure | Value |
|---|---|
| WER / CER, isolated utterances (model alone) | **0.167** (11/66) / **0.078** (26/333) |
| WER / CER, full pipeline, clean (activity hints, defaults) | **0.121** (8/66) / **0.044** |
| WER, pipeline: white 20 dB / pink 10 dB / clipped +20 dB / quiet −30 dB | 0.288 / 0.591 / 0.439 / 0.591 |
| WER, pipeline: white 10 dB / white 5 dB / music 10 dB | 0.652 / 0.894 / 0.773 (Phase 3 segmentation also fails here) |
| WER, pipeline: 48 kHz stereo / 44.1 kHz 512-frame / 8 kHz | 0.167 / 0.136 / 0.227 |
| WER, 3 × 100 ms recognizer drops | 0.242 (vs 0.121 without drops) |
| WER, model endpointing only (no analysis, continuous) | 0.182 (2 missed, 4 merged references) |
| First partial after speech start (audio time) | p50 750 ms, p95 1000 ms |
| First committed text after speech start | p50 1300 ms, p95 1590 ms |
| Final after speech end | p50 770 ms, p95 790 ms (Phase 3 segment close + 200 ms trailing) |
| Partial revisions | 2.2 per second of speech; 10.5 revisions per utterance |
| Visible corrections (non-append word edits) | 2.67 per utterance; committed text was never amended by a final (0 amendments, dev and test) |
| Model load (first / shared reuse) | ≈ 2.0 s (1964 ms in the final run; 2.6–2.9 s in earlier runs) / < 10 µs |
| Memory | peak RSS 23 MB → 152 MB after loading the model (+≈ 129 MB); evaluation process peak 199 MB |
| Compute (1 thread) | RTF 0.06–0.09 for the recognizer (model + CE) on one stream; Phase 3 analyzer 0.005–0.007 |
| 4 concurrent streams, one shared model | 2.20 s wall for 4 × 27.9 s of audio (no slowdown observed on 4 vCPU) |
| Determinism (1 thread) | identical event digests across runs |
| Utterance language (whichlang on real transcripts) | 5 × `en`, 1 abstention ("No") on clean test |

**Partial behaviour.** Hypotheses grow word by word every ≈ 300 ms of
audio. The last word is often a fragment that the next revision completes
("SEC" → "SECOND", "STRA" → "STRAIGHT"). Committed text trails the
hypothesis by about one word, and corrections stay in the provisional
tail. **Commit policy (dev)**: every policy gave the same transcripts. A
2-hypothesis rule committed first text 290 ms earlier (p50 1010 vs
1300 ms), with the same corrections and no amendments. The default (3
hypotheses / 800 ms / hold-back 1) is kept: the gain rests on 4 dev
utterances, and the default never amended on dev or test.

## Error analysis (test, clean pipeline)

The 8 word errors in the six held-out finals (reference → model): "walk" →
WALKED, "blocks" → BOX, "speak" → PEOPLE, "learning" → EARNING, "bridge
is" → BRIDGES (substitution + deletion), "today" → TO DAY (substitution +
insertion). The other two utterances ("How much does this cost, and do
you accept cards"; "No") are exact. In the partials of the first utterance the
last word grows from the fragment "BO" to "BOX"; such completions happen
in the provisional tail. Isolated
decoding (no Phase 3, whole file) differs: it misses "No" entirely and
makes different errors on kal16's fast voice ("DOES" → IS, "DO YOU" →
DULY). Segmentation and lead-in change what the model sees. Noise hurts
strongly: white 20 dB more than doubles WER. These figures come from
synthetic noise mixed at fixed SNRs.

## Limitations

* **Synthetic speech only; no human recordings.** Real speech corpora are
  not approved, and the sample WAVs in the model archive were deliberately
  not extracted. Accuracy on people, accents, spontaneous speech,
  far-field microphones or real noise is **unknown**.
* Tiny test set (66 words); WER confidence intervals are wide.
* English only. No punctuation, casing, word timestamps or scores, so the
  timing error of word timestamps cannot be measured. Token start times
  exist in sherpa-onnx results but are not exposed, because word end
  times would have to be invented.
* CPU on a shared cloud VM. RTF and load time vary between runs (load
  ≈ 2.0–2.9 s observed). No GPU, no mobile and no hardware numbers.
* Linux x86_64 only. The adapter has not been built for macOS, Windows,
  iOS or Android (native libraries per target; ADR-0023).
* The adapter needs Rust ≥ 1.89 (dependencies of `sherpa-onnx-sys`). CE
  itself stays at 1.85.

## Regressions

No CE gate regressed (see the Phase 4 report, section 10). Two defaults
changed, and the oracle tables were re-generated with them:
`preroll_ms` 500 → 1000 and `lead_in_ms` 0 → 500. The visible effect on the
oracle pipeline is the reported start offset, now ≈ 500 ms before the
labelled speech start (the lead-in, by design). Oracle WER, finals and
latencies are unchanged.
