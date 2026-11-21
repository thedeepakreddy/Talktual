# Phase 3 Test Report

All results were produced in this session on the final tree. Nothing is estimated.

## 1. Environment
Linux, 4 vCPU, no GPU · Node 22.23.5 · Vitest 5.1.1 + jsdom · `node:test` · headless Chromium 157.0.8103.78 (Playwright, fake media devices; the VoiceMirror E2E plays a **synthetic** espeak-ng voice as the fake microphone) · Python 3.13 venv for evaluation only (torch CPU, Resemblyzer 0.1.4, librosa 1.0.1, pocketsphinx 5.1.1; not app dependencies). No live provider calls.

## 2. Results (final tree)

| Suite | Command | Phase 2 baseline | Phase 3 final |
| --- | --- | --- | --- |
| Type-check | `npm run lint` | ✅ | ✅ 0 errors |
| Client | `npm test` | 207 | ✅ **261 / 261** (16 files) |
| Server | `npm run test:server` | 53 | ✅ **58 / 58** |
| Build | `npm run build` | 610.36 kB | ✅ 620.25 kB (+10 kB; VoiceMirror code included even when the flag is off) |
| Gate | `npm run test:all` | ✅ | ✅ exit 0 |
| Browser smoke (flag off) | `node e2e/smoke.mjs` | 17/17 | ✅ **17/17** |
| Browser recovery, relay-only (flag off) | `node e2e/recovery.mjs` | 8/8 | ✅ **8/8** |
| Browser compat, emulated (flag off) | `node e2e/compat.mjs` | 5/5 | ✅ **5/5** |
| **Browser VoiceMirror (flag on)** | `node e2e/voicemirror.mjs` | — | ✅ **8/8** |
| Phase 2 privacy probe | `node e2e/privacy-probe.mjs` | not running | ✅ not running |
| Dependency audit | `npm audit` ×2 | 0/0 | ✅ 0/0 |
| **Evaluation** | `python eval/voicemirror/run_eval.py` | — | ✅ two full runs (30 pairs each), see EVALUATION_REPORT |

Browser suites were run 2–3 times each during Phase 3; all runs on the final tree were green. The VoiceMirror E2E failed on its first run because of a **test design error, not a product defect**: a 3.5 s hold gives too little voiced speech for the confidence rule. I diagnosed it with in-page diagnostics and an offline analysis of the captured audio, then rewrote the test to reflect real use (several turns) and to assert that the first short turn is withheld.

## 3. New tests by category (spec M3.7)

| Category | Where | Count | Covers |
| --- | --- | --- | --- |
| Unit: analyzer | `voicemirror.test.ts` | 10 | pitch accuracy at 5 frequencies × 2 sample rates, tempo monotonicity, silence/noise fail closed, accumulation across turns, fixed memory, reset |
| Unit: descriptor/validator/hints | `voicemirror.test.ts` | 14 | quantization coarseness, clamping, tempo disabled, 7 hostile/malformed descriptors rejected, exhaustive render-hint clamps, register mapping |
| Unit: controller | `voicemirror.test.ts` | 10 | off by default, tap only during capture, withheld on low evidence, open/stop race, revocation, peer descriptor validation, reconnect resend, revocation re-delivery after outage, never-shared reconnect, dispose, tap failure |
| Integration / contract (session) | `session.voicemirror.test.ts` | 17 | flag off = Phase 2; opt-in + disclosure; decline; capture-consent prerequisite; segment fields unchanged; listener hints at playback start; revocation reaches queued segments; malformed messages; revocation mid-call; 4 session-end paths; mic off / peer away / reconnecting close the tap; reconnect resend; hands-free + playback pause; diagnostics keys |
| UI | `CallRoom.test.tsx` | +3 | hidden when off; disclosure text + toggle; peer indicator |
| Server | `api.test.js`, `config.test.js` | +5 | hint ignored when disabled; server-catalog voice + speed; no catalog; 6 malformed hints → 400 with no upstream call; env validation |
| Browser E2E | `e2e/voicemirror.mjs` | 8 steps | real mic → analyzer → peer → utterance pitch; fails closed on a short first turn; revocation; integrity; cleanup |
| Regression | all Phase 2 suites | 207 + 53 + 30 browser steps | unchanged and green (one server assertion updated for the additive `voiceHints` capability field) |

## 4. Do the tests detect defects? (mutation checks this phase)

| Mutation | Caught by |
| --- | --- |
| Hints snapshotted at enqueue (weakens revocation) | session test: revoke must reach queued segments |
| Opt-out keeps analyzer state | 2 tests (revocation; diagnostics) |
| Session writes a voice value into diagnostics | diagnostics key test |
| (Earlier attempt: calling a non-existent function) | not a valid mutation, so I redid it as above; noted for transparency |

## 5. Defects found by our own verification and fixed
See CHANGELOG "Fixed during Phase 3": precedence bug, syllable over-count, tempo constant drift, evaluation oracle mis-specification, revocation lost during outages, a flaky substring test, a `__pycache__` file nearly committed.

## 6. Not tested
Real devices, real human voices, Firefox/Safari/mobile VoiceMirror behaviour, provider (ElevenLabs) voices and speed, perceived naturalness/intelligibility/similarity (needs human listeners), `AudioWorklet` path (not implemented).

## 7. Reproduce
```bash
npm ci && (cd signaling-server && npm ci)
npm run test:all
node e2e/smoke.mjs && node e2e/recovery.mjs && node e2e/compat.mjs && node e2e/voicemirror.mjs   # needs espeak-ng for the last
python eval/voicemirror/run_eval.py ./out   # see EVALUATION_PLAN §7 for the venv
```
