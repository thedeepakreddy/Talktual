# Phase 3 Evaluation Plan — VoiceMirror

Harness: `eval/voicemirror/run_eval.py` (+ `extract.ts`, `corpus.py`). One command reproduces everything (§7).

## 1. Principles
* **Synthetic voices only** (espeak-ng 1.51). No real person's voice is used anywhere.
* **The product code is what gets evaluated.** Descriptors are produced by `src/voicemirror` running under Node, not by a Python re-implementation.
* **Calibration and evaluation use disjoint speakers.** Calibration: espeak timbres m2, m4, f2, f4. Evaluation: m1, m3, m7, f1, f3, f5. Product constants come only from calibration, and the harness flags a mismatch.
* **Each dimension is measured separately**, against a baseline (today's fixed voice) and upper bounds (oracles). No single metric is presented as "identity preservation".

## 2. Data
* **Speakers:** 6 evaluation timbres × 3 (pitch, tempo) settings = 18 synthetic speakers: espeak pitch 25/50/75, speed 140/175/205 wpm.
* **Languages:** en, es, de, hi, ja, ru, all 30 ordered pairs (source ≠ target). Sentences are self-written (`corpus.py`): 3 enrolment + 2 output per language; Japanese is kana-only.
* **Stock-voice catalog** (register path): low→m4, mid→m2, high→f2 (calibration timbres, disjoint from evaluation speakers).
* **Default voice** ("today"): m1, pitch 50, 175 wpm.

## 3. Conditions (listener-side output, target language)
| Condition | What the listener hears | Represents |
| --- | --- | --- |
| `baseline` | default voice, fixed pitch/tempo | Talktual today |
| `vm_prosody` | default timbre; pitch from the **descriptor** via a calibrated pitch map; tempo from the descriptor | VoiceMirror on an engine with pitch control (browser voice path) |
| `vm_register` | catalog voice chosen by the descriptor's register, neutral pitch; tempo (speed clamp 0.8–1.2) | VoiceMirror on the server TTS path (provider can't set pitch) |
| `oracle_prosody` | default timbre at the speaker's **measured** pitch and true tempo | Ceiling of any pitch+tempo approach |
| `oracle_select` | best **other** timbre (by embedding) at the speaker's pitch/tempo | Ceiling of stock-voice selection |
| `oracle_full` | the speaker's own timbre, pitch and tempo | Ceiling of perfect cloning |

## 4. Metrics by required dimension

| Spec dimension | Metric(s) | Method / reference | Limitations |
| --- | --- | --- | --- |
| Identity similarity | (a) GE2E cosine between output and speaker's enrolment; (b) top-1 identification and mean rank among 18 speakers; (c) different-speaker cosine as a reference | Resemblyzer 0.1.4 (Apache-2.0; bundled weights) | A proxy for perceived similarity; English-centric; synthetic voices |
| Selected characteristics | Pitch error: \|median F0(out) − median F0(source)\| in semitones (librosa pYIN, independent of the product analyzer); tempo error: \|rendered speed / true speed − 1\| | pYIN; known espeak speed | Tempo reference is espeak's wpm setting |
| Analyzer accuracy | Pitch error on exact synthetic harmonic tones (16/48 kHz); syllable-rate monotonicity | Generated signals with exact F0 | Tones are simpler than speech |
| Naturalness / prosody | **Not measured as perceived quality.** Reported proxies: pitch-range class preserved; render params always inside clamps | — | Needs a human MOS test (out of reach) |
| Intelligibility | pocketsphinx (bundled en-US model) WER on English-target outputs, per condition and at the parameter clamps | pocketsphinx 5.1.1 | High absolute WER on espeak speech; **relative comparison only**; English only |
| Cross-language consistency | All metrics per ordered language pair (30); languages not evaluated listed explicitly | — | 6 of Talktual's 31 languages |
| Streaming & latency | Analyzer ms per audio second (Node); descriptor size; where the work sits relative to the turn's critical path; browser E2E with real capture | `extract.ts` timing; `e2e/voicemirror.mjs` | Node/V8 on a server CPU, not a phone |
| Robustness | Descriptor rate and pitch-step stability under noise (20/10/0 dB SNR), short input (0.5/1/2 s), clipping, low level, 48 kHz, silence, noise-only | Deterministic seeds | Synthetic degradations |
| Privacy & security | Tests: opt-in, revocation, cleanup, no retention, diagnostics keys, validator, server hint validation | vitest + node:test | — |
| Resource footprint | Analyzer state bytes, CPU time, no GPU; server work = 0 beyond validation | — | — |

## 5. Expectations (written before the full 30-pair run, after the 6-pair quick run had been seen)
* `vm_prosody` pitch error should be clearly below baseline (target: median < 1.5 semitones). This is the characteristic the prototype claims to preserve.
* Tempo: report as measured. A descriptor that worsens tempo vs baseline would be a **finding to fix or disable**, not something to hide.
* Identity: expected *small* gains for `vm_*`, and far below `oracle_full`. That would confirm the research conclusion; it isn't a defect of the prototype.
* Robustness: no descriptor on silence, noise-only or very short input (fail closed); stable pitch step at 20 dB SNR.

## 6. Out of scope / not evaluated
Human listening tests (MOS, similarity), real human voices, real devices/microphones, provider TTS output (no paid calls), the 25 Talktual languages not in the corpus.

## 7. Reproduce
```bash
apt-get install -y espeak-ng
python3 -m venv .venv && .venv/bin/pip install torch resemblyzer librosa soundfile pocketsphinx "setuptools<81"
cd eval/voicemirror && ../../.venv/bin/python run_eval.py ./results          # full (~30 min on 4 vCPU)
../../.venv/bin/python run_eval.py ./results-quick --quick                   # 3 languages (~4 min)
```
Writes `voicemirror_eval.json` with metadata (tool versions, settings), calibration, and per-row results.
