# Phase 3 Integration Report — VoiceMirror × Phase 2 engine

VoiceMirror is integrated through Phase 2 extension points only. Each M3.5 item below has the change and the evidence.

## 1. What changed in Phase 2 code (all additive)

| Phase 2 contract | Change | Compatibility |
| --- | --- | --- |
| `SessionDeps` | optional `voiceTap?: VoiceTapPort` | Absent → feature off (default build) |
| `SessionSnapshot` | new `voice` section | Existing fields unchanged |
| `CommunicationSession` commands | `toggleVoiceMirror`, `acceptVoiceMirror`, `declineVoiceMirror`, `stopVoiceMirror`, `holdsVoiceData` | New methods only |
| Session hooks | Controller called at capture start (after ASR start resolves), capture stop (`onAsrEnd`, `stopAsr`), connect, peer `voice` message, enqueue, teardown | No new states or transitions |
| `PeerMessage` (wire v1) | `{t:'voice', descriptor \| revoked}` | Additive; Phase 2 peers reject unknown `t` |
| `PlaybackItem` | optional `voice?: () => RenderHints \| undefined` | Optional; absent = Phase 2 behaviour |
| `lib/textToSpeech.ts` | hints → browser `pitch`/`rate`; TTS body `voice:{register,speed}` | Body unchanged when no hints (Phase 2 servers ignore unknown fields anyway) |
| Server `/api/tts` | optional validated `voice` hint; `voiceHints` capability; 2 env vars | Default off; Phase 2 test updated for the new capability field |
| UI | Voice match toggle (flag builds only), separate disclosure, indicator | Invisible in default builds (test) |

## 2. M3.5 verification items

| Item | Evidence |
| --- | --- |
| **Segment identity** | Session test asserts the exact outgoing segment field set is unchanged with VoiceMirror on; browser E2E: every turn heard exactly once; Phase 2 dedupe/order tests unchanged and passing |
| **Cancellation** | Barge-in/cancel paths untouched; hints are resolved at playback start, so a cancelled segment never resolves them. Phase 2 cancellation tests pass |
| **Reconnect** | Descriptor (or revocation) re-sent on every (re)connect; tap closed while reconnecting; Phase 2 recovery E2E (relay-only) 8/8 with the new code |
| **Stale-session protection** | Controller disposed on every terminal phase; later voice messages are ignored (the session drops all peer messages after end; the controller ignores input after dispose) |
| **Buffer bounds** | No new queues; analyzer state fixed-size (test); playback queue/outbox bounds unchanged (Phase 2 tests) |
| **Streaming / stable segments** | Analysis is off the critical path (runs during capture; the descriptor is sent after capture stops); hints change only between segments, never mid-segment |
| **Consent / opt-in** | Off by default; separate disclosure; requires capture consent; per call |
| **Fallback** | See [FAILURE_AND_FALLBACK](PHASE3_FAILURE_AND_FALLBACK_REPORT.md); every failure → default voice |

## 3. Regression results (final tree)

| Suite | Result |
| --- | --- |
| Client unit/component/engine (`npm test`) | 261/261: all 207 Phase 2 tests unchanged and passing, plus 54 new |
| Server (`npm run test:server`) | 58/58: one Phase 2 assertion updated for the additive `voiceHints` capability, plus 5 new |
| Browser smoke / recovery / compat (flag off) | 17/17, 8/8, 5/5 |
| Browser VoiceMirror (flag on) | 8/8 |

## 4. Browser evidence (Chromium 157, flag on, `e2e/voicemirror.mjs`)
Real `getUserMedia` (Chromium fake device playing a **synthetic** espeak voice) → `ScriptProcessorNode` tap → product analyzer → descriptor over the data channel → listener `speechSynthesis` utterance with VoiceMirror pitch/rate.

| Step | Observed |
| --- | --- |
| Before opt-in | utterance pitch 1, rate 1 |
| Opt-in | disclosure shown; "Share voice style" turns it on |
| First short turn (3 s hold) | still default: the analyzer saw ~0.9 s of voiced speech, below the confidence rule (fails closed) |
| After more speech | listener renders at **pitch 0.69**; for an ~82 Hz source voice, the damped mapping predicts ≈ 0.71. Rate was 1.20 before tempo transfer was disabled and is 1 in the shipped configuration |
| Revocation | next segment back to pitch 1 / rate 1; "Their voice style matched" disappears |
| Integrity | each turn spoken exactly once; no page errors; server sessions 0 after leave |

**Finding from this run:** the synthetic voice spoke at 150 wpm (slower than the 175 wpm reference), yet the tempo hint was 1.20 (faster). The full evaluation confirmed tempo transfer was unreliable, so it was **disabled** (EVALUATION_REPORT §2).

## 5. Bundle impact
Default build JS: 610.36 kB → ≈620 kB (the VoiceMirror code is included even when the flag is off; the browser tap is only constructed when the flag is on). Tree-shaking it out of default builds is a follow-up if size matters.
