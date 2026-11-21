# Phase 4 Handoff — Human-Like Conversation

Branch `claude/kind-archimedes-n6y11p`, from `main` @ `497713c`. **Not merged, not deployed, no PR opened** (none requested). No paid calls, credential changes, policy changes or VoiceMirror changes.

Documents: [BASELINE_AUDIT](PHASE4_BASELINE_AUDIT.md) · [IMPLEMENTATION_PLAN](PHASE4_IMPLEMENTATION_PLAN.md) · [ARCHITECTURE](PHASE4_ARCHITECTURE.md) · [TEST_REPORT](PHASE4_TEST_REPORT.md) · [EVALUATION_REPORT](PHASE4_EVALUATION_REPORT.md) · [PRIVACY_REVIEW](PHASE4_PRIVACY_REVIEW.md) · [CONDITIONS_TRACKER](PHASE4_CONDITIONS_TRACKER.md)

## 1. What was built
A conversation layer (`src/conversation/`), integrated into `CommunicationSession` behind `VITE_CONVERSATION_LAYER` (on by default; `0` rolls back):
* **Turn/pause detection.** Unfinished-sounding phrases are held for an adaptive, bounded window. Complete phrases go out immediately. Release, recognizer end and a 6 s hard cap stop turns from staying open.
* **Segmentation with explicit states.** Units have stable ids and move provisional → final, with revisions. Provisional translations are shown, never spoken. Retraction applies when capture is aborted.
* **Self-repair.** Before anything is sent: restarts supersede abandoned starts, repetitions collapse, hesitation sounds and low-confidence noise are dropped, and uncertain speech is flagged.
* **Interruption.** Cutting in while the peer holds the floor, or is being played, interrupts that turn. Later parts stay in the transcript and are never spoken, and the speaker sees *Interrupted*.
* **Overlap fallback.** Incoming speech waits while you talk, then plays if it is less than 12 s old.
* **Load adaptation.** One translation in flight, same-turn merge behind it, a degraded mode that pauses provisionals, and a "slow translation" notice.
* **Capability negotiation** (`hello.caps: ['conv1']`), so Phase 2/3 peers keep working unchanged.
* **UI.** Draft/interrupted/unclear/waiting/skipped markers, and peer-talking/waiting/slow chips.

## 2. Acceptance criteria
| Criterion | Status | Evidence |
| --- | --- | --- |
| Turn/pause/segmentation policies bounded and tested | ✅ | Bounds table (ARCHITECTURE §3); 41 unit tests |
| Hesitations don't finalize prematurely; turns don't stay open | ✅ (simulated + scripted browser) | brief-hesitation 2 fragments → 1 unit; hold-expiry, max-open, release-flush tests; E2E hesitation and hands-free steps |
| Explicit provisional / final states | ✅ | Typed state model; wire `state`/`rev`; UI draft marker; tests |
| Barge-in cancels stale output; never restarts or duplicates | ✅ | Stale spoken 2 → 0 (simulated); integration + E2E interruption tests; Phase 2 dedupe kept (per final id) |
| Queues bounded under delay | ✅ | Outbound ≤ 1 + 1 per turn; slow-provider max lag 19.7 s → 8.4 s; playback age bound |
| Overlap has a fallback | ✅ | Hold-then-play, age-bounded; 1 → 0 played over the listener; no separation claim |
| Corrections cause no duplicate playback | ✅ | correction scenario; E2E self-repair; held unit revised before send, finals never revised |
| Context session-scoped and cleared | ✅ | teardown clears everything; test |
| Phase 2 regressions pass | ✅ | 261 → 324 client, 58 server, 38 Phase 1–3 browser steps, all green with the layer on in the production build. Forced-on engine run: 7 intended differences (TEST_REPORT §3) |
| Works with VoiceMirror disabled | ✅ | Default; tests + E2E |
| Diagnostics exclude sensitive content | ✅ | Allowlist redactor; leak tests |
| Performance claims labelled simulated | ✅ | EVALUATION_REPORT header and §3 |
| Rollback and flags documented | ✅ | ARCHITECTURE §7; layer-off results identical to baseline; rollback browser build test |

## 3. Verdict
**Phase 4 development: PASS WITH CONDITIONS.** Every acceptance criterion is met in code and verified by deterministic simulation, unit/integration tests and scripted two-browser E2E. The conditions are that nothing has been validated on real speech, real recognizers or real people (N1, C4), the lexicons are unreviewed by native speakers (N3), and the owner has a disclosure decision on provisional translation (N2).

## 4. Open risks
| ID | Risk | Severity | Mitigation / owner action |
| --- | --- | --- | --- |
| P4-R1 | Real recognizers place finals differently from the scripts; holds may fire too often or too rarely | Med | Real-device calls (C4/N1); bounds are config (`SessionDeps.conversation`) |
| P4-R2 | False "unfinished" cue delays a complete sentence ≤ 2 s | Low–Med | Conservative lists; adaptive window; measure on real calls |
| P4-R3 | Restart heuristic replaces a held fragment that was actually a continuation sharing its first two words | Low | Applies only to held unfinished fragments; flagged `repaired`; tune with real data |
| P4-R4 | Provisional translations consume rate limit / provider quota | Low–Med | Paced, off when degraded; owner can raise the interval |
| P4-R5 | Lost `floor` close (outage) could misclassify a reply as an interruption | Low | 8 s staleness + reset on reconnect (tested) |
| P4-R6 | Overfitting: same author wrote scenarios and policies | Med | Independent scenario set / human-scripted calls before Phase 5 claims |
| P4-R7 | +24 kB bundle | Low | Acceptable; tree-shake if needed |

## 5. Rollback
* Build with `VITE_CONVERSATION_LAYER=0`. This gives Phase 3 behaviour, with no code change and no server change (verified).
* Or revert the Phase 4 commits (`7e25832..HEAD`). The server and wire stay compatible, because older peers simply don't advertise `conv1`.

## 6. Production readiness (separate assessment)
**Not ready.** The blockers predate Phase 4 and are unchanged: C2 production config, C3 key review, C4 real devices. Phase 4 adds N1 (real conversation validation) and N2 (disclosure decision). VoiceMirror stays flag-off (C5, C6). Phase 4 does not make production readiness worse: it is flag-controlled, server-neutral and backward compatible.

## 7. Phase 5 recommendation
Phase 5 may begin once the owner has reviewed this handoff. These items should come first, or run alongside:
1. **Real-device conversation check** (C4 + N1): two people, Chrome Android and iOS Safari, scripted prompts covering the 14 matrix rows. Record only counters, timings and markers (no audio or transcripts) unless consent is explicitly given.
2. Owner decision on N2 (provisional translation disclosure) and native-speaker review of the lexicons (N3).
3. Phase 2 release conditions (C2, C3) remain open, whatever is decided about Phase 5.
I have not started Phase 5 scope.

## 8. Commits
`7e25832` simulator + baseline · `433cf75` audit/plan · `3720ff1` scenario revision + re-baseline · `b34bd78` conversation modules · `0a2f30c` session integration · `5d0d78f` UI · `a2711c0` browser E2E · final commit: reports, source-language fix, tracker.
