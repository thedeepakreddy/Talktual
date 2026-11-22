# Phase 4 Test Report

All runs: Node 22, Chromium 157 headless (Playwright), in this container, on the final Phase 4 code unless stated. No paid or live provider calls: the E2E server's provider `fetch` is stubbed.

## 1. Summary
| Suite | Before Phase 4 (`497713c`) | After Phase 4 | Command |
| --- | --- | --- | --- |
| Client unit/integration (Vitest) | 261 | **324 ✅** (+63) | `npm test` |
| Server (`node:test`) | 58 | **58 ✅** (unchanged) | `npm run test:server` |
| Typecheck + production build | ✅ 620.25 kB JS | ✅ 644.27 kB JS (+24 kB) | `npm run test:all` |
| Browser smoke / recovery / compat / VoiceMirror | 17 · 8 · 5 · 8 | **17 · 8 · 5 · 8 ✅** (layer on by default) | `node e2e/<suite>.mjs` |
| Browser conversation (new) | — | **8/8 ✅** (7 layer-on + 1 rollback build) | `node e2e/conversation.mjs` |
| Conversation simulator (13 scenarios) | baseline | **13/13 ran** (layer on and off) | `EVAL_LAYER=on EVAL_LABEL=phase4 npx vitest run --config vitest.eval.config.ts` |
| Latency harness (loopback, stubbed providers) | — | release → listener speech p50 13 ms, p95 24 ms (n=30) | `node e2e/latency.mjs` |

## 2. New tests
| File | Count | Covers |
| --- | --- | --- |
| `src/conversation/conversation.test.ts` | 41 (28 `it` incl. 13 table cases) | State transitions; completeness per language; fillers; repetition collapse incl. deliberate repeats kept; restart/correction vs continuation; adaptive hold bounds; segmenter: immediate commit, hold+merge, hold expiry, max-open under continuous partials, restart revision, release/flush bound, size cap, cancel/dispose clears timing, new-turn commit; listener policy: barge-in vs reply, stale floor, bounded overlap and interrupted memory; outbound merge/bounds; degraded hysteresis |
| `src/engine/session.conversation.test.ts` | 19 | Wire validation of new messages; capability negotiation; Phase 2/3 peer gets no Phase 4 traffic; provisional then one final with same id, never provisional after final; floor open/close; self-repair before send; filler/noise drop and uncertain flag; hands-free hold expiry and floor close; mic-off retract; listener provisional→final, stale revision ignored, resend deduped; retract never removes a final; barge-in suppression and new-turn playback; reply-after-release not an interruption; overlap hold then play with `notAfter`; reconnect resets floor; peer interrupt marks own turn; backpressure merge (6 phrases → 2 sends, 2 records); degraded stops provisionals; leave clears context and sends nothing more; diagnostics contain no transcript words; works with VoiceMirror absent |
| `src/components/CallRoom.test.tsx` | +2 | Provisional draft marker, final replacement, *Unclear audio*, *Interrupted — not played*, *is talking* chip, overlap *waits* chip then playback |
| `src/lib/playback.test.ts` | +1 | `PlaybackController` drops a waiting item past `notAfter`, plays the fresh one |
| `e2e/conversation.mjs` | 8 | Real two-browser calls: hesitation spoken once as one phrase (+ provisional translated); restart/repetition never spoken; interruption while floor open; overlap hold + chip; hands-free hold then bounded commit; no page errors, VoiceMirror off; **rollback build** reproduces Phase 3 split behaviour |

## 3. Phase 2/3 suites with the layer forced ON
`TALKTUAL_CONV_LAYER=1 npx vitest run src/engine src/components src/voicemirror` (186 tests, incl. the new Phase 4 files): **179 pass, 7 differ**. All 7 are intended differences, not regressions:

| Test | Difference | Why intended |
| --- | --- | --- |
| lifecycle: hello sent; pipeline: announces itself | `hello` has `caps:['conv1']` | capability negotiation |
| pipeline: 3 incoming-segment tests | playback items have `notAfter` | age bound |
| lifecycle: teardown cancels 1 playing | 0 cancelled | the segment arrived while the user held talk → overlap-held, not played |
| longrun: 2000 outbound sent | 400 sent | same-turn merge under backpressure (fake translator resolves asynchronously) |

The default engine behaviour (layer off unless the dependency is given) keeps all 261 Phase 2/3 tests unchanged and green. The production build turns the layer on, and all four Phase 1–3 browser suites pass against it.

## 4. Failures during development (disclosed)
* The first trailing-word lists were too broad: "you" in "how are you" caused a hold, and German separable particles were listed. Narrowed to articles, prepositions, conjunctions and possessives. Tests updated to assert the narrower policy.
* 2-gram collapse squashed runs of one word ("word word word word"). Fixed: those are left to the 1-gram rule, and content words are kept.
* The first forced-on run also failed the diagnostics-privacy test, because the `seg_final` diagnostics record was missing. It was restored for compatibility, and that test now passes with the layer on.
* Integration: `interrupt` was sent to a non-capable peer. Fixed: sent only with `conv1`.
* Before any Phase 4 code, the *interrupt-during-translation* scenario was revised so the speaker still holds the floor. A reply after release is a speaker change, not an interruption. The baseline was re-run on the unmodified engine, with the same failure result.
* The eval lag metric originally credited a merged unit to its newest phrase, which understated lag. Changed to per-phrase lag before reporting.

## 5. Not tested
Real devices and real speech; Safari/Firefox engines (only Chromium plus Phase 2 emulation); real translation providers; acoustic noise (noise is simulated as low-confidence recognizer results); languages beyond the six lexicons (they get no holds, by design); long real calls with the layer on (the Phase 2 long-run test ran with the layer forced on: bounded).
