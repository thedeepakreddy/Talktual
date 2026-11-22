# Communication-Loop Reliability Report

Scope: why the deployed app shows the speaker's live transcription but delivers **no translated text and no translated audio**. This is an investigation and repair only. No Phase 7 work, no deployment, no PR, no live provider calls.

## 1. Executive summary
* **Root cause (confirmed by reproduction).** In the Web Speech adapter (`src/lib/speechToText.ts`, Phase 2+, so on `main` too), on push-to-talk release the user's words were **discarded whenever the cloud recognizer's final result arrived late (more than ~1.15 s after release) or never arrived**. The user sees their words as live *interim* text. When the engine ends without a final, or the 450 ms + 700 ms finalize bound fires first, the adapter finished without emitting anything. So nothing was translated, sent or spoken.
  * This matches the symptom exactly: "transcription appears to work" (interim captions), but no translated text and no audio, on either side.
  * Chrome on Android frequently ends without a final, and cloud recognizers often flush later than 1.15 s on mobile networks.
  * Every earlier browser test used a fake that finalizes instantly *before* release, so the defect was never exercised.
* **Secondary defect (fixed).** Server-streamed TTS (PCM) was played into an `AudioContext` even when the browser kept it **suspended**. A suspended context happens with no user gesture, a backgrounded tab, or an iOS interruption.
  * Short clips were "played" silently and reported as `ended`, with no fallback.
  * Clips longer than 10 s **blocked the playback queue forever** (backpressure loop on a frozen clock).
  * This only applies when the server has an ElevenLabs key.
* **Fixes.**
  * The adapter promotes the last interim text to a final when the engine ends or the finalize bound fires. An explicit abort (mic off, leave, mode change) still discards, so privacy behaviour is unchanged.
  * The finalize bound is now 1200 ms.
  * PCM playback runs only into a running `AudioContext` (resume bounded at 250 ms); otherwise the browser voice speaks.
* **Evidence.** In the two-participant reproduction, both directions fail on `main` and on the branch before the fix (6/10 scenarios lose everything). After the fix, all 10/10 deliver text and audio, on the branch and on `main` with the hotfix.
* **Not yet proven on real devices or real recognizers.** The recognizer timing is **simulated**, and I couldn't observe the live deployment (§2).

## 2. Deployment identity and phase inventory
| Fact | Evidence | Verified? |
| --- | --- | --- |
| Frontend host is Vercel (SPA rewrite), backend a Render-style Node service | `vercel.json`; Phase 1 audit | config only |
| Deployed URL / project | **not in the repository**; GitHub shows **no deployment integration** (no commit statuses, check runs or Vercel/Render comments on PRs #1–#3) | ❌ **cannot verify from here** |
| Code that would be deployed from `main` | `origin/main` = **`497713c`** (Merge PR #3, 2026-11-21 23:43 +0100) = Phases 1–3 (merges `89ef81a`, `b5d365c`, `497713c`) | ✅ |
| Phase 4/5/6 on `main` | **absent**: `src/conversation`, `src/intelligence`, `src/local`, `src/privacy` don't exist on `main`; their flags (`VITE_CONVERSATION_LAYER`, `VITE_COMMUNICATION_INTELLIGENCE`, `VITE_PROCESSING_MODES`) are not read | ✅ |
| Flags on `main` | `VITE_VOICEMIRROR` (off unless `1`), `VITE_SIGNALING_URL` (required; defaults to `ws://localhost:8080`) | ✅ (code) |
| Phase 5 placeholder protection in the deployed path | not present on `main` (ruled out as a cause) | ✅ |

**Conclusion.** The belief "deployed through Phase 3" is consistent with all repository evidence, but the live build hash can't be confirmed without the deployment URL or dashboard. The defect exists in **every** tree from Phase 2 onward, so the diagnosis doesn't depend on which of them is live.

## 3. End-to-end trace and failure location
| Step | `main` behaviour | Status |
| --- | --- | --- |
| Mic capture → Web Speech `start()` | press → capture | ✅ |
| Interim results → `onPartial` → live caption | shown | ✅ ("transcription appears to work") |
| Release → `stop()` after a 450 ms tail → engine flushes | **final late or missing** | ⚠️ |
| `onend` (stopping) / finalize timer (450 + 700 ms) | `finish()` **without emitting the pending interim**; the timer path also aborts and detaches, so a later final is ignored | ❌ **failure point** |
| `onFinal` → `seg_final` → translate → `segment` → peer → TTS | never reached | ❌ (consequence) |

Safe correlation on the branch, using diagnostics with no text:
* `asr_interim_promoted` (count): an interim was rescued by the fix.
* `asr_release_no_text` (count) and `asr_turn {kind, n}`: a press produced no text.
* The existing `seg_final`, `seg_translated`, `seg_sent`, `seg_received` and `playback` records are keyed by segment id.

On a deployed build, the defect shows up as a release with no `seg_final` record.

**Hypotheses excluded with evidence:**
* **Frontend/backend version skew.** The `main` frontend was run against the Phase 1 server (`89ef81a`) and the original pre-Phase-1 server, each with provider `fetch` stubbed. Text and audio were delivered in both directions, and the relay message types are compatible.
* **A translation request hanging the serial chain.** The 10 s client bound covers both fetch and parse.
* **Phase 5 placeholders.** Not deployed.
* **Audio unlock on entry.** `App.tsx` unlocks on the start/join taps.

**Not testable here:** the live server's environment (keys, `ALLOWED_ORIGINS`), provider quotas, and real recognizer timing.

## 4. Reproduction results (`e2e/repro-loop.mjs`)
Setup: two real Chromium pages, the real signaling server, and the real app build. Providers are stubbed; the recognizer is a **scripted fake with realistic timing**. Raw data: `docs/reliability/repro-*.json`.

| Scenario (recognizer behaviour after release) | `main` (deployed code) | branch before fix | **branch after fix** | **`main` + hotfix** |
| --- | --- | --- | --- | --- |
| final before release (old test assumption) | ✅ both directions | ✅ | ✅ | ✅ |
| final 300 ms after stop | ✅ | ✅ | ✅ | ✅ |
| final 900 ms after stop | ❌ nothing delivered (A→B and B→A) | ❌ | ✅ text + audio | ✅ |
| final 1600 ms after stop | ❌ | ❌ | ✅ | ✅ |
| no final (onend only, Chrome Android style) | ❌ | ❌ | ✅ | ✅ |
| server TTS path (stubbed PCM) → **real Chromium Web Audio** playback at the listener | — | — | ✅ 4/4 (both directions, incl. no-final) | (unit-tested) |

**Disclosed mistake:** one earlier run set was invalid. A leftover `vite preview` from the first run kept serving the `main` build on the same port, so the first "branch before fix" and "branch after fix" runs actually tested `main`. The harness now spawns `vite` directly, so `stop()` really stops it, and it refuses to start on an already-served port. All results above are from clean runs (0 leftover processes).

## 5. Changes
* **Branch `claude/kind-archimedes-n6y11p`:**
  * `src/lib/speechToText.ts`: interim promotion and abort-discards.
  * `src/lib/config.ts`: finalize bound 700 → 1200 ms.
  * `src/lib/textToSpeech.ts`: PCM only into a running `AudioContext`; mid-stream suspension aborts and falls back.
  * `src/engine/ports.ts`: `promoted?` on `RecognizedSegment`.
  * `src/engine/CommunicationSession.ts`: sanitized loop counters.
  * Tests in `src/lib/speechToText.test.ts` (+5), `src/lib/playback.test.ts` (+3), `src/engine/session.pipeline.test.ts` (+1).
  * `e2e/repro-loop.mjs` (new): realistic recognizer, version skew, server TTS, assert mode.
  * `e2e/harness.mjs`: process and port hygiene.
  * `package.json`: `test:e2e` now also runs `processing` (missed in Phase 6) and the loop suite.
* **Hotfix for `main`:** `docs/reliability/hotfix-main.patch` contains the adapter, finalize-bound and playback fixes plus their 8 tests (5 files, against `497713c`). It was verified in a local worktree of `main` (not pushed): 269 client tests, 58 server tests, smoke 17/17, recovery 8/8, compat 5/5, VoiceMirror 8/8, loop 10/10.

## 5b. Test results (final, branch)
| Suite | Result |
| --- | --- |
| Client (Vitest) | **447 ✅** (Phase 6: 438; +9 reliability tests) |
| Server | **58 ✅** |
| Typecheck + build | ✅ 757.76 kB |
| Browser smoke / recovery / compat / VoiceMirror / conversation / intelligence / processing | 17 · 8 · 5 · 8 · 8 · 7 · 7 ✅ |
| Loop suite (assert), browser-voice path | **10/10 ✅** (5 recognizer behaviours × 2 directions) |
| Loop suite (assert), server-TTS path → real Web Audio | **10/10 ✅** |
| Phase 4 simulator (layer on/off) | 13 + 13 ✅ (unchanged) |
| New tests proven to fail without the fix | the adapter tests (late/missing final) and the 2 suspended-AudioContext tests fail against the unfixed files |
| `main` + hotfix (local worktree, not pushed) | 269 client, 58 server, smoke 17/17, recovery 8/8, compat 5/5, VoiceMirror 8/8, loop 10/10 ✅; the patch applies cleanly to `497713c` |

## 6. Real vs simulated
| Real | Simulated / stubbed |
| --- | --- |
| Chromium 157 pages, the real app build, the real signaling server, WebSocket/data-channel delivery, real Web Audio scheduling of server PCM, real DOM | the **speech recognizer** (timing scripted from known engine behaviour), **providers** (translation and ElevenLabs stubbed), speech synthesis in the browser-voice path, network |

**Not done:** real devices, real voices, real recognizers, live providers, the live deployment.

## 7. Deployment instructions (not performed)
1. **Recommended (deployed line):** apply `docs/reliability/hotfix-main.patch` to `main`:
   ```
   git checkout main && git apply docs/reliability/hotfix-main.patch
   npm test && npm run test:server
   ```
   Open a PR, review and merge. Vercel rebuilds the frontend. **No server change is required.** Keep `VITE_SIGNALING_URL` as today.
2. **Verify on real phones:** Chrome Android and iOS Safari, with two people speaking in both directions. Use `?debug=1` to check that each release produces `seg_final` → `seg_sent` → `seg_received` → `playback`.
3. **Alternative:** deploying this branch ships Phases 4–6 as well (on by default). That needs its own review; it's not recommended as a hotfix.

## 8. Rollback
* Revert the hotfix commit on `main` (frontend only).
* On the branch, revert `aeded8d` and the follow-up commit.
* There's no data or server state to undo.

## 9. Carry-forward conditions
* **Unchanged:** production config (C2), the ElevenLabs credential review (C3), real-device mic/browser tests and real conversations (C1/C4), the VoiceMirror second-mic test (VoiceMirror stays off), native-speaker review (N3), real-provider placeholder validation (P5-2), and the consent version decision (N2).
* **New:**
  * R1: confirm the fix on real devices and recognizers after deployment.
  * R2: the live deployment URL/hash wasn't verifiable from here.
* **Server translation cache (reviewed, not changed):**
  * Holds a `Map` of up to 2000 entries.
  * The key is `from>to:<source text>`; the value is the translated text.
  * It's **global across all calls and users** (no per-session isolation), with FIFO eviction and **no TTL**. A repeated phrase from a different call is answered from the cache, which is why repeated phrases showed `translateCalls: 0`.
  * Recommendation for owner approval: a configurable TTL (e.g. 10 min) plus a size cap, or disabling the cache. Never log keys.

## 10. Verdict
* **Engineering: PASS WITH CONDITIONS.** The root cause was found, fixed and demonstrated in a realistic two-participant reproduction in both directions, with a secondary audio defect fixed and all regression suites green. The condition is that the recognizer timing that triggers the bug is simulated, and the live deployment couldn't be inspected.
* **Production readiness: NOT APPROVED.** It needs real-device confirmation of the loop after redeploy, plus the carried conditions.
