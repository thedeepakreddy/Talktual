# Phase 2 Test Report

Every number here comes from runs in this session. Nothing is estimated.

## 1. Environment
Linux container, 4 vCPU · Node 22.23.5 / npm 10.9.9 · Vitest 5.1.1 + jsdom 26 + Testing Library · `node:test` · headless Chromium 157.0.8103.78 (Playwright, fake media devices). **No live provider calls:** translation, STT and TTS are stubbed at the server's injected `fetch`. Chromium may send STUN binding requests to Google's public STUN servers during P2P setup (no user content).

## 2. Results (final tree)

| Suite | Command | Result |
| --- | --- | --- |
| Type-check | `npm run lint` | ✅ 0 errors (see §6 caveat) |
| Client unit/component/engine | `npm test` | ✅ **207 / 207** (15 files) |
| Server API/signaling/config | `npm run test:server` | ✅ **53 / 53** (3 files) |
| Build | `npm run build` | ✅ JS 610.08 kB (207.15 kB gzip), CSS 32.61 kB |
| Gate | `npm run test:all` | ✅ exit 0 |
| Browser smoke | `node e2e/smoke.mjs` | ✅ **17 / 17** — 2 green runs on the final tree, 3 more on the tree before the last engine fix; 0 failures |
| Browser recovery (relay-only) | `node e2e/recovery.mjs` | ✅ **8 / 8** — 2 runs on final tree (+3 green earlier); one earlier failure was a test bug (wrong peer selected), fixed |
| Browser compatibility (emulated) | `node e2e/compat.mjs` | ✅ **5 / 5** — 2 runs on final tree (+3 green earlier); one earlier failure was measuring during a slide-in animation |
| Privacy probe | `node e2e/privacy-probe.mjs` | ✅ recognizer not running in idle call |
| Dependency audit | `npm audit` (root + server) | ✅ 0 / 0 |

Phase 1 baseline at the start of Phase 2: 75 client + 37 server tests, 13 E2E steps (all green).

### Client tests by file
| File | Tests | Covers |
| --- | --- | --- |
| `engine/core.test.ts` | 19 | exhaustive transition tables, terminal states, event bus isolation/versioning, diagnostics redaction + bounds, segment dedupe/bounds, capability registry (incl. insecure context) |
| `engine/session.lifecycle.test.ts` | 21 | phases, idempotent start, immutable snapshot, all transport outcomes, teardown-once, leave races, late segment/translation after end, reconnect/peer-away, illegal transitions |
| `engine/session.pipeline.test.ts` | 20 | ported Phase 1 CallRoom behaviour + segment id/seq, ordering under out-of-order translation, dedupe, pending→translated, bounds, barge-in, capabilities, diagnostics leak check |
| `engine/session.privacy.test.ts` | 33 | consent gate, per-route/versioned consent, storage blocked, disclosure, PTT/continuous semantics, half-duplex, 14-case stop matrix, stale-result guard, failure handling, mic recovery |
| `engine/session.longrun.test.ts` | 1 | 20k inbound / 2k outbound soak, all buffers bounded |
| `lib/speechToText.test.ts` | 18 | ASR adapter contract (both engines) |
| `lib/translate.test.ts` | 20 | Phase 1 + `translateSegment` (success, passthrough, failure, 429, retry policy, cancel, malformed) |
| `lib/playback.test.ts` | 9 | PlaybackController status, bound, failure, cancel at each stage, PCM backpressure; capture device-lost/permission |
| `lib/textToSpeech.test.ts` | 7 | Phase 1 TTS tests, unchanged, passing on the new controller |
| `lib/webrtc.test.ts` | 32 | Phase 1 transport tests + versioning, leave, outbox, error mapping, backoff, 6 reconnect scenarios |
| `lib/audio.test.ts`, `lib/roomCode.test.ts` | 4, 6 | Phase 1, unchanged |
| `components/CallRoom.test.tsx` | 10 | view over the real engine with fake deps: consent flow, hands-free banner, mic off, device-lost recovery path, leave, peer-left, reconnecting, XSS, transcript |
| `components/JoinCall.test.tsx` | 7 | Phase 1, unchanged |

### Server tests
`api.test.js` 25 (Phase 1, unchanged) · `signaling.test.js` 18 (12 Phase 1 with assertions updated for the new `joined` message and grace hold, + 6 resume/leave) · `config.test.js` 10 (validator, CLI, startup gate).

## 3. Spec §6 required verification → where

| Requirement | Evidence |
| --- | --- |
| Unit: state machine, transitions, cancellation, queue limits, event validation, segment identity, retries, cleanup, privacy modes | core, lifecycle, privacy, playback, translate, webrtc tests |
| Integration: capture → ASR → translate → TTS → playback; provider failure/recovery; stop/interruption per stage; reconnect, stale session | session.pipeline/privacy (fakes), E2E smoke + recovery (real browser, real server) |
| Regression: Phase 1 features | all Phase 1 tests kept or ported 1:1 (§4); E2E keeps every Phase 1 step |
| Privacy: no capture before consent; PTT release; continuous indication; stop; disclosure; redaction | privacy tests, E2E steps 2/3/6/10, probe |
| Performance | PHASE2_PERFORMANCE_REPORT |
| Compatibility: automated vs emulated vs real | PHASE2_COMPATIBILITY_REPORT |
| Failure: denied mic, device removal, network interruption, provider timeout, malformed event, rate limit, unsupported capability, playback failure, teardown | PHASE2_FAILURE_RECOVERY_REPORT matrix |
| Condition tracking | PHASE2_PHASE1_CONDITIONS_TRACKER |

## 4. Regression handling of Phase 1 tests
* Kept unchanged and passing: `textToSpeech`, `audio`, `roomCode`, `JoinCall`, `api`, all Phase 1 `translate` and `webrtc` cases (2 `webrtc` assertions updated for the `v: 1` stamp).
* **Rewritten because the behaviour changed intentionally:** `speechToText.test.ts`. The Phase 1 test "emits finalized phrases only while held" encoded the always-on recognizer that P1 removes. Each Phase 1 case has an equivalent under the new contract (tail, duplicate suppression, permission/routine errors, restart rules, DEF-C08 races).
* **Moved:** Phase 1 `CallRoom.test.tsx` cases now live in `session.pipeline.test.ts` (behaviour) and the new `CallRoom.test.tsx` (view), marked "(ported)".
* `signaling.test.js`: expected-message assertions filter the new `joined` message; the peer-left test runs with resume disabled. Both changes are deliberate protocol changes.

## 5. Do the tests detect defects? (mutation checks run this phase)
| Mutation | Caught by |
| --- | --- |
| Don't resend hello on reconnect | lifecycle (1 fail) |
| Skip `call.close()` in teardown | lifecycle (3 fail) |
| Remove PCM backpressure loop | playback backpressure test |
| Remove playback queue cap | bounded-queue test |
| Accept callbacks from stopped recognizers | stale-result privacy test |
| Revert the mic-recovery fix | 2 engine + 1 view test |
| Remove the inbound terminal-phase guard | **not caught** — a second guard in `deliver()` keeps the behaviour (defense in depth, documented) |

## 6. Limitations and caveats
* **Speech, TTS and providers are fakes** in every automated test. Real recognizer start-up, accuracy, permission prompts, audio output and provider behaviour are unverified.
* **Only Chromium.** Firefox/Safari/mobile results are emulated (compat suite) or untested.
* **TypeScript caveat:** the project has no `@types/react` and `tsconfig` isn't `strict`, so React is typed as `any`. `tsc` passing gives little assurance for component props. Found this phase; not changed (scope). Recommended for Phase 3 prep.
* No coverage tool; no coverage % is claimed.
* Playwright isn't a project dependency (preinstalled here). Add it as a dev dependency for CI.

## 7. Reproduce
```bash
npm ci && (cd signaling-server && npm ci)
npm run test:all                      # lint + client + server + build
npm run test:e2e                      # smoke + recovery + compat (needs Playwright/Chromium)
node e2e/privacy-probe.mjs            # P1 probe
node e2e/latency.mjs . 30             # emulated latency
node e2e/perf.mjs [server.js] [port]  # server benchmark
(cd signaling-server && npm run check-config -- --production)
```
