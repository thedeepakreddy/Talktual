# Phase 3 Baseline Audit — VoiceMirror

**Date:** 2026-11-21 · **Branch:** `claude/kind-archimedes-n6y11p`, restarted from `main` @ `b5d365c` (PR #2, Phase 2, merged by the owner) · working tree clean.

## 1. Repository and test baseline (re-run, not copied from Phase 2 reports)

| Check | Result |
| --- | --- |
| `npm run test:all` | ✅ exit 0: lint, **207/207** client, **53/53** server, build |
| `node e2e/smoke.mjs` | ✅ **17/17** (Chromium 157) |
| Bundle | 610.36 kB JS (207.20 kB gzip) |
| Phase 2 handoff | PASS WITH CONDITIONS for development; production release **not** approved |

## 2. How speech output works today (the thing VoiceMirror changes)

| Path | Where | Voice used |
| --- | --- | --- |
| Server TTS (ElevenLabs, PCM stream) | `signaling-server/server.js` `/api/tts` | **One fixed voice** for everyone: `ELEVENLABS_VOICE_ID` (default "Rachel"); `voice_settings.speed` fixed at 1.0; model `eleven_flash_v2_5` |
| Browser fallback | `src/lib/textToSpeech.ts` `browserSpeak()` | First system voice matching the language; `pitch`/`rate` left at defaults |
| Playback | `PlaybackController` (bounded, per-segment status) | Gets only `{segmentId, text, lang}`; no speaker information |

**Every listener hears the same synthetic voice regardless of who spoke.** There is no notion of speaker voice anywhere in the pipeline.

## 3. Phase 2 contracts VoiceMirror must preserve

| Contract | Location | Constraint for Phase 3 |
| --- | --- | --- |
| Call lifecycle + teardown-once | `CommunicationSession`, `sessionMachine.ts` | Voice state must be created/cleared inside the same lifecycle; nothing survives `ended`/`failed` |
| Consent per processing route, versioned | `engine/privacy.ts` | Voice profiling is a **new purpose** and needs its own separate opt-in; existing consent doesn't cover it |
| PTT/continuous capture gates | `AsrAdapter` + session | Any voice analysis may only run when capture is already allowed and active |
| Peer wire v1 + validator | `engine/events.ts` `isPeerMessage` | New message must be additive and validated; old peers drop unknown types (verified: `default: return false`) |
| Segment identity + dedupe | `segments.ts` | Voice hints must not change segment ids or ordering |
| Bounded buffers | playback 12, outbox 20, PCM ≤10 s | No new unbounded buffers (audio analysis must be streaming, O(1) memory) |
| Diagnostics allowlist | `diagnostics.ts` `redact()` | **Risk found:** `redact()` keeps *every* numeric field. Voice features are numbers (e.g. pitch in Hz), so they would pass redaction. Phase 3 must keep voice values out of diagnostics by construction |
| Reconnect / outbox | `lib/webrtc.ts` | Outbox keeps only segments + latest hello; a voice profile must either be re-sent after reconnect or be stateless per segment |

## 4. Capture constraint discovered in the code

On Web Speech browsers (the primary path) **the app never sees microphone samples**: `SpeechRecognition` owns the mic internally. Any on-device voice analysis therefore needs a **separate** `getUserMedia` stream opened in parallel with recognition. It must run only while the button is held (or hands-free is active) and must be released afterwards. On server-STT browsers the recorder's stream already exists.

## 5. Environment constraints (measured)

| Resource | Status | Consequence |
| --- | --- | --- |
| GPU | none (4 vCPU, 15 GB RAM) | No neural TTS/VC inference at realistic speed |
| Hugging Face | **blocked** (no connection) | Cannot download XTTS, OpenVoice, F5-TTS, CosyVoice, kNN-VC/WavLM, SpeechBrain ECAPA |
| PyPI | reachable | `resemblyzer` (bundled GE2E speaker-encoder weights, 17 MB) and `librosa` installed in a scratch venv **for evaluation only**; nothing added to the app |
| espeak-ng 1.51 | installed (apt) | Local, deterministic **synthetic** speech in 12+ app languages with timbre variants and pitch/speed control. Synthetic voices are not real people |
| Paid providers (ElevenLabs cloning, etc.) | not authorized | Provider cloning can only be designed and mock-tested |
| Real voice data | none authorized | Evaluation uses synthetic voices only |

## 6. Carry-forward conditions from Phase 2 (status at start of Phase 3)

| Condition | Status | Phase 3 handling |
| --- | --- | --- |
| Real-device compatibility + first-word latency unverified | Open | VoiceMirror adds a second mic consumer on Web Speech browsers, which **raises** the real-device risk. Track it; can't be closed here |
| Production config (`ALLOWED_ORIGINS`, `RATE_LIMIT_PER_MINUTE`, `TRUST_PROXY`) | Open (owner) | Unchanged; any new server env vars are validated by `check-config` |
| ElevenLabs key exposure review | Open (owner) | Unchanged; Phase 3 makes no provider calls |
| Preserve Phase 2 contracts; no merge/deploy | Binding | Adapter/extension only; full Phase 2 regression at every milestone |

## 7. Risk register (initial)

| ID | Risk | Likelihood | Impact | Mitigation |
| --- | --- | --- | --- | --- |
| R1 | Voice data is biometric; misuse or impersonation | Med | High | No voice cloning shipped; only coarse, non-invertible descriptors; opt-in per call; no storage; never logged |
| R2 | Overclaiming identity preservation | High | High | Report metrics separately; a speaker-encoder baseline and an oracle upper bound; explicit "not cloning" wording |
| R3 | Second mic stream breaks or slows Web Speech on real devices | Med | Med | Behind a flag and opt-in; fail closed (no profile → default voice); real-device test item |
| R4 | Voice params harm intelligibility (extreme pitch/rate) | Med | Med | Clamp ranges; evaluate intelligibility proxy; human listening test recommended |
| R5 | Evaluation on synthetic voices doesn't transfer to humans | High | Med | Stated as a limitation; human-data evaluation needs consented data (owner decision) |
| R6 | Diagnostics leak voice features (numbers pass redaction) | Med | High | Voice module never passes values to diagnostics; leak test |
| R7 | Provider-cloning path tempting but paid and high-risk | — | High | Design-only behind an interface; requires explicit approval and a consent/deletion design |
| R8 | Regression of Phase 2 contracts | Low | High | Additive integration; full Phase 2 suites at each milestone |
