# Phase 5 Test Report

Container run: Node 22, Chromium 157 headless. Provider calls are stubbed. No live or paid calls.

## 1. Summary
| Suite | Before Phase 5 (`baedf69`) | After Phase 5 | Command |
| --- | --- | --- | --- |
| Client (Vitest) | 324 | **389 ✅** (+65) | `npm test` |
| Server | 58 | **58 ✅** (server unchanged) | `npm run test:server` |
| Typecheck + build | ✅ 644.27 kB | ✅ 699.17 kB (+55 kB: lexicons + ACI) | `npm run test:all` |
| Browser smoke / recovery / compat / VoiceMirror / conversation | 17 · 8 · 5 · 8 · 8 | **17 · 8 · 5 · 8 · 8 ✅** (default build: Phase 4 + ACI on) | `node e2e/<suite>.mjs` |
| Browser intelligence (new) | — | **7/7 ✅** (6 default + 1 rollback build) | `node e2e/intelligence.mjs` |
| Phase 4 conversation simulator | 13 | **13 ✅ both modes; results identical to Phase 4** (only timestamps differed) | `EVAL_LAYER=on …` |
| ACI evaluation | — | 7 runs (2 sets × 3 modes + stress) | see EVALUATION_REPORT |

## 2. New tests
| File | Tests | Covers |
| --- | --- | --- |
| `src/intelligence/intelligence.test.ts` | 44 | entity extraction (sentence-initial rule, places vs addresses, Spanish times); sensitive redaction and no storage; context eviction by count and chars, LRU with pinned survival, remove blocks re-learning, no sensitive pins, clear; ambiguity evidence rule, contradictory context, names vs words; idioms; 15 signal cases (en/es); refusal needs a prior turn; misunderstanding evidence; no emotion inference; unsupported language → none; protection build/restore/malformed; **6 validation mutation cases**; concise subsequence/bounds; controller: assist pre-edit, assist off, disable clears and doesn't learn, inbound notes, two candidates → no guess, topic break, suggestion lifecycle (dismiss, loop guard, rephrase, drop with segment, expiry, off), concise labelled, invalid mode coerced, too-long fallback, 200-decision time bound, reset |
| `src/engine/session.intelligence.test.ts` | 18 | `aci1` negotiation and assist validation; outbound protection and sense with original unchanged; no assist to non-aci peers; **failure never leaks MT input**; malformed restore → one retry; off switch mid-session; assist pref off; works with the Phase 4 layer; inbound notes/signals/cross-turn correction (earlier line annotated, all lines still spoken); concise speech vs faithful transcript; peer assist explanation; suggestions (nothing auto-sent, dismiss, no loop, expiry); lifecycle (reset, decline, leave; reconnect keeps context); remove/pin; diagnostics without words; disclosure points; drafts off |
| `src/components/CallRoom.test.tsx` | +3 | notes and tags under the line; assist sheet shows, forgets and resets; off switch; suggestion is a dismissible note; hidden when off |
| `e2e/intelligence.mjs` | 7 | disclosure points; **protected name never reaches the translator**, sense from context spoken, labels on both sides; reference note not spoken; misunderstanding suggestion with nothing sent on the user's behalf; inspector, off switch, context-free afterwards; no page errors, VoiceMirror off; **rollback build** (no UI, plain text to translator) |

## 3. Feature-off parity
With the engine default (intelligence off when the dependency is absent), all Phase 2–4 tests pass unchanged. Forcing ACI on for every existing suite (`TALKTUAL_ACI=1 npx vitest run src/engine src/components src/voicemirror`) gives 203 pass and **4 intended differences**:

| Difference | Why |
| --- | --- |
| 3 tests: `hello` now carries `caps: ['aci1']` | capability negotiation |
| 1 test asserts a plain session's disclosure has no assistance point | forcing ACI everywhere makes every session "not plain" (the test is about the off case) |

**UI differences when on:** "Assist" button, assistance sheet, suggestion chip, notes and tags under transcript lines, and one or two extra disclosure points. **When off:** none (rollback E2E).

## 4. Development failures (disclosed)
* A sentence-initial imperative merged into a name ("Tell Rose"). Fixed: common sentence-starting words are excluded.
* "can't make it" matched a correction cue (`make it`). Narrowed to "make it" followed by a number or day.
* "at 5" duplicated "5 pm" as two entities, which broke correction linking. Deduplicated; the last value of each kind is the corrected one.
* The concise bound was measured in characters (too strict). Changed to words.
* Dev-set tuning (before the freeze): theatre-type place words, third-person offers, polite-question overlap, "no, no puedo" read as a correction, refusal requiring a prior turn, low-confidence sentence-initial names (to avoid pronoun guesses).
* Test-side: `wait` collided with `waiting-for-peer` in the diagnostics leak check (word changed); the fake playback plays one item at a time (assertions include the queue).
* **After the freeze**, no implementation change was made in response to the independent set.

## 5. Not tested
Real devices, real speech, and real recognizers (capitalisation affects name detection); real translators' handling of placeholder tokens; human judgement of usefulness and annoyance; languages other than en/es for signals, ambiguity and references (fr/de/pt/it have only concise-mode filler lists).
