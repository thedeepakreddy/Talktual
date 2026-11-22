# Phase 6 Evaluation Report

All measurements below are **real**, taken on the test hardware: Linux x86_64 container, 4 vCPU Intel Xeon (2.1–2.3 GHz), 16 GB RAM, no GPU, Node 22.23.5, Chromium 157 headless. The only simulations are marked as such. Raw data:
* `docs/phase6/probe-local-capabilities.json` (runtime probe)
* `docs/phase6/measurements.json` (resources)
* `eval/local/results/phrasebook-frozen.json` and `phrasebook-postfix.json` (quality)

## 1. Local runtime availability (real probe)
| Runtime | Result on test hardware |
| --- | --- |
| On-device speech recognition (`processLocally`) | **renderer crash** → disabled by default |
| Cloud Web Speech availability | `available` (cloud, not local) |
| Translator API | **no answer** within 3 s |
| LanguageDetector | `unavailable` |
| Speech voices | **0** (0 local) |
| WebGPU adapter | none |

Consequence: the **only local inference verified** on this hardware is the bundled phrasebook translator, plus Phase 5 assistance (already local). Typed input removes the need for ASR.

## 2. Phrasebook quality — independent blind set
**Method.**
* A separate agent session wrote 100 utterances (60 en, 40 es; 85 in scope, 15 out of scope) with 1–3 reference translations and the meaning-critical words for each item.
* It never saw the phrasebook.
* The set was run **once** on the frozen phrasebook (`9e9b5e0`).
* Limitation: the author is the same model family, and its references were not reviewed by a human.

| Metric | Frozen (blind) | After fix (no longer blind) |
| --- | --- | --- |
| In-scope utterances covered | **55 / 85 (65 %)** | 53 / 85 (62 %) |
| Out-of-scope utterances wrongly covered | **0 / 15** | 0 / 15 |
| Covered and matching a reference exactly | 42 / 55 | 47 / 53 |
| Covered and keeping every meaning-critical word | 54 / 55 | **53 / 53** |
| Covered with a **meaning change** (my review, not independent) | **3** | **0** |
| Coverage by language | en 33/60, es 22/40 | en 31/60, es 22/40 |

**Frozen-run meaning changes:**
* "where is the **nearest** pharmacy" → omitted "más cercana".
* "see you **tomorrow** at nine" → omitted "mañana".
* "**we** need a taxi to the hotel" → "Necesito un taxi, lléveme al hotel" (person changed, imperative added).

The cause was template variants that were treated as equivalent. The fix splits non-equivalent variants into separate templates and removes the `to the {place}` fragment. The two utterances that relied on it are now "not covered" instead of wrong.

**Remaining non-exact outputs (6):** tú vs usted, "cerca" vs "por aquí", "Can we" vs "Can I", "Excuse me" vs "Sorry", "The menu please" vs "Can we have the menu, please?". These are wording or register differences.

**What the numbers mean:** the phrasebook gives correct offline translation for roughly 6 in 10 typical travel utterances and **refuses** the rest rather than guessing. Coverage is not translation quality in general, and longer free speech is essentially never covered (0/15).

## 3. Resource measurements (real; `eval/local/measure.mjs`)
| Metric | Node 22 | Chromium 157 |
| --- | --- | --- |
| Module import | — | 13.1 ms |
| Phrasebook cold load (index build) | 3.3 ms | 5.2 ms |
| First translation after load | 0.77 ms | — |
| Translation latency p50 / p95 / max (n = 5000) | 0.019 / 0.16 / 3.4 ms | ≈0 / 0.2 / 2.6 ms (timer coarsened to 0.1 ms) |
| CPU per call | 0.056 ms | — |
| Heap when loaded | +278 KB | not measurable (`performance.memory` quantised → 0) |
| Heap after unload + GC | +383 KB (compiled code is retained; unload frees the index but no saving is measurable) | — |
| Policy plan decision | 0.15 µs | — |
| Local module size | 42 KB (31 KB minified) | — |
| App bundle (production build) | **756.88 kB** JS (Phase 5: 699.17 kB → **+58 kB**: phrasebook data, runtimes, policy, privacy UI). `measurements.json` recorded a stale pre-Phase-6 `dist/` (699 166 B); corrected here from the `npm run test:all` build | — |
| GPU / NPU | none present | none present |
| Battery / thermal | **not measurable** (container, no battery) | — |

**Not measured:** latency of real platform models (none available), mobile devices, provider latency (no live calls).

## 4. Behaviour tests on the wire (E2E, real browsers, stubbed providers)
| Check | Result |
| --- | --- |
| On-device only: press-to-talk creates **no** recognizer (no audio leaves the device) | ✅ |
| On-device only: covered phrase translated locally; uncovered sent as-is; **0** `/api/translate`, `/api/tts`, `/api/stt` requests from **either** page; provider never called | ✅ |
| Receiver honours `priv` (no cloud fix-up or TTS); speaks with an on-device voice (**simulated** voices) | ✅ |
| Hybrid: covered phrase stays local (0 requests); uncovered uses exactly 1 translate request | ✅ |
| Offline (Playwright network emulation): status shown; 0 requests while offline; message delivered exactly once after recovery | ✅ |
| Rollback build: no controls; Phase 5 path | ✅ |

## 5. Not done (needs approval or hardware)
* **Local neural models:** Whisper-tiny ASR, Opus-MT en↔es, Piper TTS via onnxruntime-web. These need model downloads (~40–300 MB) and a new dependency.
* **Real-device checks:** Chrome with on-device models, iOS/Android voices.
* **Cloud-vs-local quality comparison:** this needs live provider calls, so a cloud comparison is not reported.
