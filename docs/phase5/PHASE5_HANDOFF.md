# Phase 5 Handoff — Autonomous Communication Intelligence

Branch `claude/kind-archimedes-n6y11p`, from `baedf69` (Phase 4). **Not merged, not deployed, no PR opened.** No paid calls, credential changes, server or policy changes. Phase 6+ not started.

Documents: [BASELINE_AUDIT_AND_PLAN](PHASE5_BASELINE_AUDIT_AND_PLAN.md) · [ARCHITECTURE](PHASE5_ARCHITECTURE.md) (contracts, bounds, flags) · [TEST_REPORT](PHASE5_TEST_REPORT.md) · [EVALUATION_REPORT](PHASE5_EVALUATION_REPORT.md) · [PRIVACY_REVIEW](PHASE5_PRIVACY_REVIEW.md) (data flow) · [CONDITIONS_TRACKER](PHASE5_CONDITIONS_TRACKER.md)

## 1. What was built
An advisory, on-device intelligence layer (`src/intelligence/`, behind `VITE_COMMUNICATION_INTELLIGENCE`, on by default):
* **Bounded session context.** 8 lines / 1600 chars, 48 entities, 16 topic words. Sensitive identifiers are redacted. The context can be inspected, corrected ("Forget"), pinned and reset, and is cleared on call end, reset, disable or consent decline.
* **Context-aware translation input.**
  * Person names and addresses are kept verbatim (and withheld from the translator).
  * Ambiguous words are pre-edited only with context evidence. Otherwise the ambiguity is preserved and surfaced.
  * Always labelled.
* **Listener notes**, shown and never spoken: ambiguity, likely sense, reference ("allí → Plaza Mayor"), idiom meaning, cross-turn correction ("Corrects earlier: cinco → seis").
* **Signals** (category + confidence): question, request, offer, confirmation, refusal, correction, clarification-request, misunderstanding, explicit urgency, topic change/continuation, unresolved reference.
* **Policies.** Faithful by default; concise speech is user-chosen. Suggestions are non-blocking, dismissible, de-duplicated and never act on their own.
* **Validation.** Typed, versioned decisions are validated by the session. Anything invalid falls back to faithful, context-free translation.
* **UI.** "Assist" button and sheet (toggles, context inspector, reset), suggestion chip, notes and tags under transcript lines.
* **Phase 4 carry-forward (N2).** Disclosure point for drafts plus a "Send drafts while I pause" toggle.

## 2. Acceptance criteria
| Criterion | Status | Evidence |
| --- | --- | --- |
| Isolated behind a flag; disable keeps the Phase 2/4 baseline | ✅ | Engine off when absent (all Phase 2–4 tests unchanged); rollback build E2E; in-call off switch |
| Context bounded, session-scoped, inspectable, cleared at lifecycle boundaries | ✅ | ARCHITECTURE §4; session lifecycle tests; inspector UI test + E2E |
| Typed/versioned interfaces, validation, policy enforcement | ✅ | `types.ts` v1; 6 mutation tests; invalid → fallback |
| Demonstrable improvement on a context-dependent set without unacceptable meaning drift | ✅ **modest** | Held-out set: names 6→10/13, idioms 0→3/6, senses 0→5/8 plus 4/4 ambiguities surfaced, references 0→2/8, signals recall 0→0.63 (P 0.74). 0 added words. Context-specific gain is small (+1 sense, +2 references) |
| Ambiguity preserved or surfaced, not invented | ✅ | 0 overconfident senses, 0 wrong reference guesses on the held-out set |
| Suggestions non-blocking, dismissible, no external actions | ✅ | Tests + E2E (no request sent on the user's behalf) |
| Cancellation, interruption, reconnect, stale-result reliability preserved | ✅ | ACI is synchronous inside the existing translate step; all Phase 2–4 suites pass with it on; failure and malformed-output paths tested |
| No sensitive content in diagnostics or persistent storage | ✅ | Diagnostics leak test; no storage API; 0/6 sensitive strings stored |
| Automated tests and honest limitations | ✅ | TEST_REPORT, EVALUATION_REPORT §4–5 |
| Phase 2–4 conditions tracked, not closed without evidence | ✅ | CONDITIONS_TRACKER |

## 3. Verdict
**Phase 5 development: PASS WITH CONDITIONS.** The layer is isolated, bounded, privacy-preserving and reversible, and it fails quietly rather than wrongly on unseen data. Its accuracy is limited, though:
* Signal recall is 0.63 and refusal precision is 0.20.
* Lexicon coverage is small.
* Cross-language context and sentence-initial names are known gaps.

Everything was evaluated offline, with no real speech, real translator or human raters.

## 4. Rollback
* **Build:** `VITE_COMMUNICATION_INTELLIGENCE=0` restores Phase 4 behaviour, with no code or server change (verified by E2E).
* **Per call:** the "Assistance" switch clears context immediately.
* **Code:** revert the Phase 5 commits (`bd0b597..HEAD`). The wire stays compatible because `aci1` and `assist` are optional.

## 5. Precise remaining work
1. **Accuracy** (engineering, on new data rather than the held-out set):
   * a sentence-initial name heuristic (or casing from the recognizer)
   * cross-language sense cues
   * refusal precision ("No te preocupes" is not a refusal)
   * more correction, offer and request phrasings
   * object-pronoun references, with care
   * lexicon growth with native-speaker review (N3)
2. **Real-translator check** of `QXnQ` token survival (one authorized call per provider; P5-2).
3. **Human evaluation:** usefulness and annoyance of notes and suggestions; meaning-preservation review of the context-assisted edits (P5-4); human-authored or blind-reviewed scenarios (N4).
4. **Owner decisions:** disclosure wording and whether to bump `CONSENT_VERSION` (N2); C2 production config; C3 key review.
5. **Real devices** (C1, C4, C5, N1).

## 6. Production readiness (separate)
**Not ready.** The pre-existing blockers are unchanged (C2, C3, C1/C4 real devices, N1). Phase 5 adds P5-1 to P5-4. The layer does not worsen readiness: it is on-device, server-neutral, flag-controlled and sends the translator less identifying text than before.

## 7. Files changed (since `baedf69`)
* **New:**
  * `src/intelligence/*`: 10 modules + tests
  * `src/components/AssistPanel.tsx`
  * `src/engine/session.intelligence.test.ts`
  * `eval/intelligence/*`: harness, `cases/dev.json`, `cases/independent.json`, `results/phase5-frozen.json`
  * `e2e/intelligence.mjs`
  * `docs/phase5/*`
* **Modified:**
  * `src/engine/CommunicationSession.ts` (hooks, controls, snapshot)
  * `events.ts` (record fields, `assist` validation)
  * `privacy.ts` (disclosure extras)
  * `defaultDeps.ts` (flag)
  * `src/components/CallRoom.tsx` (+ test)
  * `src/test/engineFakes.ts` (opt-in env)
  * `src/vite-env.d.ts`
  * `vitest.eval.config.ts`
  * `package.json` (`test:e2e` includes the new suite)
  * `.gitignore`
* **Unchanged:** `signaling-server/`, `src/voicemirror/`, `src/conversation/`.
