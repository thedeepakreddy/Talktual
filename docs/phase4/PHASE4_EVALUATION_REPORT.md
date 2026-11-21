# Phase 4 Evaluation Report

> **All numbers below are SIMULATED.** Two real `CommunicationSession`s run over a simulated network (60 ms one-way). Recognizer timing is scripted, translation has a fixed latency (300 ms; 900 ms for the rapid-change scenario; 1500 ms for cancellation; 2500 ms for the slow and interrupt scenarios), and playback lasts 45 ms per character. No real speech, no human listeners, no real devices, no provider. They show how the policies behave on scripted conversations. They say nothing about recognition quality or perceived naturalness.

Sources: `eval/conversation/` (`sim.ts`, `scenarios.ts`, `conversation.eval.ts`). Results: `results/baseline.json` (unmodified pre-Phase-4 engine), `results/phase4-layer-off.json` (Phase 4 code with the flag off), `results/phase4.json` (layer on).

## 1. Method and ground truth
Each scenario declares an `ideal` list (what an ideal listener hears, in order) and a `stale` list (content that must not be spoken, such as speech after the listener interrupted). Scenarios were written against the baseline **before** any Phase 4 code. One was revised before coding (the speaker in *interrupt-during-translation* now still holds the floor), and the baseline was re-run. Disclosed in TEST_REPORT §4. Overfitting risk: the same author wrote the policies and the scenarios. The browser E2E and unit tests use different phrases, but an independent, human-scripted or recorded set is still needed.

## 2. Results (evaluation matrix)
`cov` = ideal units heard (exactly or inside a merged unit). `frag` = premature fragments spoken. `stale` = stale units spoken. `unexp` = unexpected units spoken (fillers, abandoned starts). Lag = finalized phrase → listener starts hearing it (median / max ms).

| Matrix row | Baseline (= layer off) | Phase 4 (layer on) |
| --- | --- | --- |
| Normal alternating | cov 2/2, lag 360/360 | cov 2/2, lag 360/360 (**no added delay**) |
| Brief hesitation | ❌ **2 fragments**, cov 0/1 | ✅ one unit, cov 1/1. Provisional sent 300 ms after the fragment. Lag from the *fragment's* final 1660 ms, **360 ms from the thought's end** |
| Long pause / completed thought (hands-free) | cov 2/2 | cov 2/2 (no premature merge across a 6 s pause) |
| Rapid speaker change | cov 1/1, lag 960 | cov 1/1, lag 960 (not blocked; not treated as an interruption) |
| Barge-in during playback | ❌ **1 stale spoken**; stop 0 ms | ✅ 0 stale; stop 0 ms (simulated cancel is synchronous) |
| Interrupt during translation | ❌ **1 stale spoken** (2560 ms late) | ✅ 0 stale |
| Overlapping voices | ❌ **1 played while listener talked** | ✅ 0 during overlap; played after release (lag 1200) |
| Noise / speech rate | ❌ **2 unexpected** (fillers) | ✅ 0 unexpected; 3/3 heard; translation calls 5 → 3 |
| Correction / restart / repetition | ❌ cov 1/2, 1 fragment + 1 unexpected ("can you can you…") | ✅ cov 2/2, 0 fragments, 0 unexpected |
| Network loss / recovery | cov 3/3, 0 duplicates | cov 3/3, 0 duplicates |
| Slow provider / saturation (10 phrases, 2.5 s translation) | lag **12060 / 19660** (grows with talk length), 10 calls | ✅ lag **4625 / 8390** (per phrase, incl. merged), cov 10/10, 4 calls, outbound depth ≤ 2 |
| Cancellation / session end | nothing spoken after leave | nothing spoken after leave |
| VoiceMirror disabled | all scenarios ran with VoiceMirror absent (the default) | same |
| Privacy / revocation | 0 transcript word pairs in diagnostics | 0 (all 13 scenarios) |

**Rollback check:** `phase4-layer-off.json` is identical to `baseline.json` on every scenario metric above.

## 3. Metrics requested by the spec
| Metric | Result (simulated unless noted) |
| --- | --- |
| Turn-finalization delay | Complete-looking phrase: 0 ms added (normal-alternating lag unchanged; browser latency harness p50 13 ms release → speech). Unfinished-looking phrase: adaptive hold 500–2000 ms, extended by continued speech up to a hard 6 s. PTT release: ≤ 1200 ms |
| Time to first provisional | 300 ms after the fragment (= simulated translation latency) |
| Time to final | = hold (if held) + translation; examples above |
| Interruption-stop delay | 0 ms in simulation (synchronous cancel). Real stop latency depends on the TTS engine. Phase 2 measured cancel as immediate on Chromium's fake synthesis only |
| Revision rate | 1 revision in correction-restart (restart superseding "i want to"); 0 elsewhere |
| Duplicate / stale output rate | 0 duplicates and 0 stale units in all 13 scenarios (baseline: 2 stale) |
| Queue depth / age | Listener playback queue max 2; outbound ≤ 1 in flight + 1 waiting per turn; playback age bound 12 s (unit-tested drop) |
| Memory / buffer bounds | interrupted turns ≤ 64, active map ≤ 64, overlap hold ≤ 12, unit ≤ 400 chars, one open unit; Phase 2 long-run test (20 000 in / 2 000 out) stays bounded with the layer on |
| Cancellation completion | leave/dispose: no sends, no playback after; held unit discarded; retract sent when a provisional was out (tests) |
| Failure / recovery | network loss: 0 duplicates; reconnect resets a stale floor; failed provisional translation is silently skipped; failed final keeps Phase 2 "untranslated" behaviour |

## 4. Trade-offs and limitations
* **Holding costs time when the cue is wrong.** A complete sentence that ends in a listed word ("…what are you looking for") waits for the hold (≤ 2 s; 900 ms until the speaker's own pause rhythm is learned). The lists are conservative to keep this rare. Not measured on real speech.
* **Merging under load** produces longer units, so the listener hears fewer, longer utterances. That is the intended trade: bounded lag instead of a growing backlog.
* **Provisional translations add translation calls** (one per held fragment, paced and off when degraded). This counts against provider rate limits (see PRIVACY_REVIEW).
* **Heuristics are lexical,** not acoustic: no prosody or VAD. Web Speech still decides where finals fall. Only six languages have word lists. Others pass through unchanged.
* **Overlap = turn-taking fallback.** There is no source separation, and none is claimed.
* **Hands-free stays half-duplex** (Phase 2). No barge-in by voice.
