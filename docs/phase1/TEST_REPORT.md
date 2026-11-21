# Test Report — Talktual Phase 1

All results below were produced in this session. Nothing here is estimated.

## 1. Environment

| Item | Value |
| --- | --- |
| OS | Linux 6.18 (cloud container), 4 vCPU |
| Node / npm | 22.23.5 / 10.9.9 |
| Browser | Headless Chromium 157.0.8103.78 via preinstalled Playwright (launch flags `--use-fake-ui-for-media-stream --use-fake-device-for-media-stream`) |
| Test frameworks | Vitest 5.1.1 + jsdom 26 + Testing Library (client); `node:test` (server; no extra deps) |
| External services | **None called.** Translation/STT/TTS providers are stubbed at the server's injected `fetch`. Chromium may emit ICE/STUN traffic to the hard-coded Google STUN servers during E2E; no user content is in those packets. |

## 2. Commands and results (final state, commit after `062e444`)

| Gate | Command | Result |
| --- | --- | --- |
| Type-check | `npm run lint` | ✅ 0 errors |
| Client unit + component | `npm test` | ✅ **75 / 75** passed (8 files, ~3 s) |
| Server API + signaling | `npm run test:server` | ✅ **37 / 37** passed (8 suites, ~1 s) |
| Production build | `npm run build` | ✅ JS 575.05 kB (196.88 kB gzip); Vite >500 kB chunk warning (pre-existing) |
| All of the above | `npm run test:all` | ✅ exit 0 |
| Browser E2E | `npm run test:e2e` | ✅ **13 / 13** steps, run 3× consecutively, 3/3 green |
| Dependency audit | `npm audit` (root, server) | ✅ 0 / 0 vulnerabilities |
| Performance | `node e2e/perf.mjs [server.js] [port]` | ✅ see PERFORMANCE_REPORT |

Server suite stability: run 3× consecutively, 37/37 each time. One test was corrected during development (session-expiry test asserted an exact message count, but the second peer's own 50 ms test session legitimately expired too); the assertion now checks the actual property (no relay/peer-left from the expired socket).

## 3. Do the regression tests detect the original defects?

Client modules whose public API did not change were swapped back to the baseline (`git show 34a8400:…`) and the suites re-run:

| Suite (baseline module) | Tests | Fail on baseline | Pass on baseline |
| --- | --- | --- | --- |
| `textToSpeech.test.ts` | 7 | 4 — DEF-C01 ×3, DEF-C10 | ordering, empty text, PCM decode/scheduling |
| `audio.test.ts` | 4 | 1 — DEF-C03 | reuse, release, gating |
| `speechToText.test.ts` | 9 | 2 — DEF-C08 ×2 | Scribe happy path, back-to-back pushes, error surfacing, Web Speech gating/dedupe/errors/restart |
| **Total** | **20** | **7 (exactly the defect-tagged tests)** | **13 (behaviour-preservation)** |

During this check two tests initially failed on baseline for the wrong reason (one asserted on *which* recorder instance records; one bundled the GainNode-leak check into the PCM decode test). Both were rewritten so each asserts one claim; the numbers above are after that correction.

`translate.ts`, `webrtc.ts`, `JoinCall.tsx`, `roomCode.ts` and the server gained new exports, so their tests can't be pointed at baseline files. For those, defects were reproduced independently: server defects by live probes on the unmodified server (outputs in DEFECT_REGISTER), client ones by code-path trace. `DEF-C11` was checked by disabling its fix: the test fails, then passes when restored.

## 4. Coverage of the Phase 1 testing matrix

| Matrix item (spec §5) | Where | Status |
| --- | --- | --- |
| Build, lint, typecheck, existing tests | `test:all` | ✅ (no tests existed before) |
| Backend route + schema/validation | `api.test.js` (25) | ✅ |
| Frontend interaction + state transitions | `CallRoom.test.tsx` (11), `JoinCall.test.tsx` (7), E2E | ✅ |
| Session lifecycle: create/join/leave, invalid id, expiry, disconnect, teardown | `signaling.test.js` (12), E2E | ✅ · reconnect: **n/a — not implemented** (OBS-03) |
| Realtime signalling: valid, invalid, oversized, malformed, unexpected | `signaling.test.js`, `webrtc.test.ts` | ✅ |
| STT / translation / TTS mocked providers: success, timeout, error, unsupported, cancellation, malformed | `api.test.js`, `translate.test.ts`, `textToSpeech.test.ts`, `speechToText.test.ts` | ✅ |
| Audio lifecycle + playback cancellation | `audio.test.ts`, `textToSpeech.test.ts` | ✅ (fakes) |
| Browser smoke on available environments | `e2e/smoke.mjs` | ✅ Chromium only |
| Security checks: audit, secrets scan, validation, origin, logs, threat scenarios | tests + SECURITY_REPORT | ✅ |
| Performance: long session, repeated join/leave | `e2e/perf.mjs` | ✅ server-side |

### E2E steps (`e2e/smoke.mjs`)
Two separate browser contexts (en-US host, es-ES guest) against the **production build** (`vite build` + `vite preview`) and an in-process instance of the real server:
1. host starts a channel, gets a 6-char code · 2. guest opens `?join=` link; both reach the call; language handshake visible · 3. transport reported (**"Direct" — P2P data channel established on localhost**) · 4. EN → ES: caption + speech in `es-ES` · 5. ES → EN · 6. multi-segment turn arrives in spoken order · 7. translation failure surfaced on both sides, original spoken in `en-US` voice · 8. mid-call language switch retargets the next turn (fr-FR) without reconnecting · 9. transcript dialog shows both speakers and the "Not translated" label · 10. host leaves → guest notified, returned home · 11. server has 0 sessions afterwards · 12. unknown code → error after timeout, lobby seat released · 13. no uncaught page errors.

## 5. Fakes, skips and limitations

* **Speech recognition** in E2E is a scripted fake (`window.__say`), not a real microphone or Google's recognizer. Real recognition accuracy, latency and permission UX are **unverified**.
* **Speech synthesis** in E2E is recorded, not played. Real audio output, autoplay unlock and ElevenLabs PCM streaming in a real browser are **unverified** (PCM decode/scheduling is unit-tested with an `AudioContext` fake).
* **Providers** (Gemini, DeepL, MyMemory, ElevenLabs) are stubbed in their documented response shapes. No live call was made, so real provider error formats/quotas are **unverified**.
* **Network:** both peers on localhost. NAT traversal, TURN and lossy-network behaviour are **unverified**. The relay fallback is covered by unit tests only.
* **Browsers:** only Chromium. See COMPATIBILITY_REPORT.
* No coverage tool is configured; no coverage percentage is claimed.

## 6. Reproducing

```bash
npm ci && (cd signaling-server && npm ci)
npm run test:all          # lint + client tests + server tests + build
npm run test:e2e          # needs `playwright` resolvable and Chromium installed
node e2e/perf.mjs         # optional benchmark
```
`e2e/smoke.mjs` uses ports 8899 (API) and 4199 (web) by default (`E2E_API_PORT`, `E2E_WEB_PORT`). Playwright is not a project dependency (this environment ships it globally); add `playwright` as a dev dependency to run E2E in CI.
