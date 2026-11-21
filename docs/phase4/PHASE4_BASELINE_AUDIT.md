# Phase 4 Baseline Audit — Human-Like Conversation

**Branch:** `claude/kind-archimedes-n6y11p`, restarted from `main` @ `497713c` (PR #3, Phase 3, merged by the owner). Working tree clean at start. VoiceMirror is off by default (both flags unset).

## 1. Test baseline (re-run on `497713c`)
| Suite | Result |
| --- | --- |
| `npm run test:all` | ✅ 261 client, 58 server, lint, build (620.25 kB JS) |
| Browser: smoke / relay-only recovery / emulated compat / VoiceMirror | ✅ 17/17 · 8/8 · 5/5 · 8/8 |

## 2. How conversation currently flows (code inspection)
| Concern | Current behaviour | Where |
| --- | --- | --- |
| Turn boundaries | **The recognizer decides.** Every Web Speech `final` is immediately translated and sent as one segment. A push-to-talk release only flushes the recognizer. No notion of a "turn" | `CommunicationSession.onLocalFinal` |
| Hesitation / pauses | None: a final at a mid-thought pause becomes its own segment | — |
| Provisional output | Only raw `partial` captions (interim text); no provisional translations; no revision semantics | `partial` peer message |
| Barge-in | Pressing while audio plays cancels playback + queue (Phase 2). **Segments of the interrupted turn that arrive later are still played** | `pressTalk` → `interrupt` |
| Overlap | Incoming segments play immediately even while the local user is talking (PTT) | `onPeerMessage` |
| Self-repair | None: restarts, abandoned starts and repetitions are sent verbatim | — |
| Ambiguity / noise | No confidence handling; recognized fillers ("uh") are sent as turns | `RecognizedSegment` has no confidence |
| Latency / load | Outgoing translations are serialized on an unbounded promise chain; no backpressure; playback queue bounded by count (12) but not by age | `sendChain`, `PlaybackController` |
| Speaker metadata | Each device = one speaker; segments carry `id`, `seq`. No turn IDs | `events.ts` |

## 3. Measured baseline (simulated scenarios, `eval/conversation/results/baseline.json`)
Two real sessions over a simulated network, scripted recognizer timing, 300 ms translation (2.5 s for the slow/interrupt scenarios), 60 ms network. **Simulated, not real speech.**

| Scenario (spec matrix) | Baseline behaviour |
| --- | --- |
| Normal alternating | ✅ 2/2 units; ~360 ms phrase-final → heard |
| Brief hesitation | ❌ one thought split into **2 fragments** |
| Long pause (hands-free) | ✅ committed (recognizer-driven) |
| Rapid speaker change | ✅ next turn not blocked |
| Barge-in during playback | ✅ playback stopped at once; ❌ **1 stale segment of the interrupted turn spoken afterwards** |
| Interrupt during translation | ❌ **stale segment spoken** after the listener interrupted |
| Overlapping voices | ❌ **played while the listener was talking** |
| Noise / speech rate | ❌ fillers "uh", "mm" **spoken as 2 extra turns** |
| Correction / restart / repetition | ❌ abandoned start **and** restart spoken; "can you can you" spoken |
| Network loss / recovery | ✅ no duplicates (Phase 2 dedupe) |
| Slow provider / saturation | ❌ **unbounded lag growth**: 12.1 s median, **19.7 s** max |
| Cancellation / session end | ✅ nothing spoken after leave |
| Privacy / revocation | ✅ no transcript phrases in diagnostics |

## 4. Constraints and risks
* **Recognizer-owned finals:** Web Speech decides phrase finals; Phase 4 can decide *how finals become turns/segments*, not where the recognizer pauses.
* **Mixed-version peers:** Phase 2/3 peers would speak any segment they receive, so new provisional/interrupt messages must be capability-negotiated.
* **No real speech fixtures:** no consented recordings; evaluation is a deterministic simulation of recognizer events.
* **Recognizer confidence** varies by browser (often absent or 0) and must be optional.
* **Phase 2/3 contracts** (lifecycle, consent, capture gates, segment IDs, dedupe, bounded buffers, VoiceMirror-off) must not regress.

## 5. Carry-forward conditions (open at entry)
| Condition | Status |
| --- | --- |
| Production settings `ALLOWED_ORIGINS` / `RATE_LIMIT_PER_MINUTE` / `TRUST_PROXY` | Open (owner). Phase 4 must not change policy |
| ElevenLabs key-exposure review/rotation | Open (owner) |
| Real-device compatibility + first-word latency | Open |
| Real-phone VoiceMirror second-microphone lifecycle test | Open; VoiceMirror stays off |
