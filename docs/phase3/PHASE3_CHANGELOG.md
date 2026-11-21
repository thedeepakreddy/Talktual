# Phase 3 Changelog — VoiceMirror

Branch `claude/kind-archimedes-n6y11p`, restarted from `main` @ `b5d365c` (Phase 2 merged by the owner).

| Commit | Milestone | Summary |
| --- | --- | --- |
| `adfb91c` | M3.0/M3.1 | Baseline audit, plan, feasibility probe |
| `969af78` | M3.2–M3.5 | Contracts, analyzer/descriptor/controller, browser tap, engine/playback/server/UI integration, evaluation harness, tests, VoiceMirror E2E |
| *(next)* | M3.6/M3.7 | Revocation re-delivery fix, evaluation-driven fixes, reports |

## New files
| Path | Purpose |
| --- | --- |
| `src/voicemirror/types.ts` | Contracts: `VoiceProfile` (on-device), `VoiceDescriptor` v1 (wire), `RenderHints`, `VoiceTier` |
| `src/voicemirror/analyzer.ts` | Streaming, fixed-memory pitch (YIN histogram) and tempo (envelope nuclei) analyzer |
| `src/voicemirror/descriptor.ts` | Quantize → descriptor; strict validator; clamped render hints; calibrated tempo reference |
| `src/voicemirror/controller.ts` | Per-call opt-in/out, tap lifecycle, publish-on-change, peer descriptor, reconnect resend (incl. revocation), dispose; enum-only diagnostics |
| `src/voicemirror/browserTap.ts` | Web Audio tap on the shared call mic |
| `src/voicemirror/disclosure.ts` | Purpose-specific disclosure text |
| `src/voicemirror/voicemirror.test.ts`, `src/engine/session.voicemirror.test.ts` | Unit + integration tests |
| `eval/voicemirror/*` | Feasibility probe; evaluation harness (`run_eval.py`, `extract.ts`, `corpus.py`) |
| `e2e/voicemirror.mjs` | Browser E2E with the flag on and a synthetic voice as the fake mic |
| `docs/phase3/*` | 12 required reports |

## Changed files (additive)
| Path | Change |
| --- | --- |
| `src/engine/CommunicationSession.ts` | Optional `voiceTap` dep; `voice` snapshot; commands; hooks at capture start/stop, connect, peer message, enqueue, teardown |
| `src/engine/events.ts` | `voice` peer message in wire v1 |
| `src/engine/ports.ts` | `PlaybackItem.voice?` resolver |
| `src/engine/defaultDeps.ts`, `src/vite-env.d.ts` | `VITE_VOICEMIRROR` flag |
| `src/lib/textToSpeech.ts` | Apply hints (browser pitch/rate; TTS `{register, speed}`) |
| `src/components/CallRoom.tsx`, `ConsentSheet.tsx` | Voice match toggle, disclosure, indicator; configurable accept label |
| `signaling-server/server.js` | `VOICEMIRROR_ENABLED`, `VOICEMIRROR_VOICE_CATALOG`, hint validation, `voiceHints` capability, config validation |
| `e2e/harness.mjs` | Build env, debug flag, records utterance pitch/rate |
| Tests | +server 5, +config 1; one Phase 2 assertion updated for the additive capability field |

## Decisions
1. **Build the "characteristics" tier only** (pitch register/liveliness/tempo + register-based stock voice). Cloning and conversion tiers are designed but not built (Research §5).
2. **Separate consent purpose**, off by default, per call, never persisted.
3. **Hints resolved at playback start**, so revocation reaches queued segments.
4. **Server owns voice IDs**; clients send only `{register, speed}`.
5. **Product constants from calibration speakers only**; the harness flags drift.

## Fixed during Phase 3 (found by our own verification)
* Operator-precedence bug in browser-pitch mapping (`2 ** x ** 2`).
* Syllable detector over-counted carrier ripple (tempo error was worse than baseline); a smoothed, prominence-gated detector raised the tempo rank correlation from 0.58 to 0.88 on calibration speakers.
* Product tempo reference (4.0) disagreed with calibration (5.94), set to 5.9.
* Evaluation oracles initially used espeak pitch *parameters* across different voices (not a real pitch oracle); corrected to measured pitch.
* Revocation issued during a network outage could be lost; now re-sent on reconnect.
* A flaky test (substring `f0` matched inside random UUIDs); replaced with a key-based leak check.
