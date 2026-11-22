# Phase 6 Test Report

Test hardware: Node 22.23.5, Chromium 157 headless, Linux x86_64 container (4 vCPU, 16 GB, no GPU). Provider `fetch` is stubbed: no live or paid calls.

## 1. Summary
| Suite | Before Phase 6 (`f393bde`) | After Phase 6 |
| --- | --- | --- |
| Client (Vitest) | 389 | **438 ✅** (+49) |
| Server | 58 | **58 ✅** (server unchanged) |
| Typecheck + production build | ✅ 699.17 kB | ✅ **756.88 kB** (+58 kB) |
| Browser: smoke / recovery / compat / VoiceMirror / conversation / intelligence | 17 · 8 · 5 · 8 · 8 · 7 | **17 · 8 · 5 · 8 · 8 · 7 ✅** (default build: Phase 4 + 5 + 6 on) |
| Browser: processing (new) | — | **7/7 ✅** (6 default + 1 rollback build) |
| Phase 4 simulator (13 scenarios × layer on/off) | ✅ | ✅ results identical (only timestamps differ) |
| Phase 5 evaluation (dev + independent, 3 modes) | — | ✅ **0 metric differences** from the Phase 5 frozen results |
| Phase 6 blind phrasebook set | — | run once frozen + once post-fix (EVALUATION_REPORT) |
| Real measurements | — | `node eval/local/measure.mjs` |

## 2. New tests
| File | Tests | Covers |
| --- | --- | --- |
| `src/local/local.test.ts` | 33 | phrasebook: 15 translations both ways, verbatim names and placeholder tokens, **never partial** (uncovered → null, unsupported language → null), runtime info and lifecycle; registry: output validation, **bounded concurrency/queue (busy refusal)**, per-call timeout, invalid output, cancellation, **LRU loaded bound**, idle unload, `unloadAll` (memory pressure), dispose, **hanging discovery time-bounded**; platform runtimes: Translator API uses only `available` pairs and **never calls `create()` for `downloadable`**; voices count only `localService`; on-device ASR disabled unless enabled; policy: mode semantics, **local-only never yields a cloud step for any task × context (64 combinations) and `beginRemote` throws**, offline, peer request, no-consent ASR, providers unreachable, **revocation aborts in-flight remote work**, enum-only bounded audit, unknown mode → cloud, preview not audited; network monitor states and cooldown |
| `src/engine/session.processing.test.ts` | 14 | off by default (Phase 5 parity), `priv1` negotiation; **local-only: no recognizer constructed**, typing works, **translator never called**, uncovered → untranslated + `priv`; inbound: no remote fix-up, **no remote captions**, `device-only` TTS; **mid-flight switch aborts the remote call, never resumed**, on-device takes over; cloud ASR stopped on switch; hybrid routing (1 cloud call); cloud: network failures → on-device fallback → providers-unreachable (no retry loop); **offline → only on-device; back online → no replay**; peer `priv` honoured and ignored without `priv1`; mode persisted, nothing else stored; `clearPrivateData` and session end clear caches and context; diagnostics without words; typed input bounded |
| `src/components/CallRoom.test.tsx` | +2 | privacy sheet shows where things run; On-device mode disables speech and enables typing; typed phrase translated on the device with markers; hidden when off |
| `e2e/processing.mjs` | 7 | **wire-level request audit on both pages**: on-device only makes 0 inference requests, starts no recognizer, honours the peer request, speaks only with on-device voices (*simulated*); hybrid makes exactly 1 cloud request; **offline emulation**: 0 requests, exactly-once delivery after recovery; no page errors; rollback build |

## 3. Feature-on parity (existing suites with processing forced on, cloud mode)
`TALKTUAL_PROC=1 npx vitest run src/engine src/components src/voicemirror` → 198 pass, **9 intended differences**:

| Count | Difference | Why |
| --- | --- | --- |
| 4 | `hello` carries `caps: ['priv1']` | capability negotiation |
| 3 | playback items carry `ttsRoute` | execution-time TTS routing |
| 2 | DEF-C02 and rate-limit: a failed **cloud** translation of "hello" is now translated **on the device** ("Hola") instead of being sent untranslated | cloud-assisted mode falls back on-device when the cloud fails (more private, keeps working) |

## 4. Development failures (disclosed)
* The first E2E run's on-device voice check failed. The fake voice objects can't be assigned to a real `SpeechSynthesisUtterance`. The app now treats a rejected voice as "no on-device voice" (fail closed), and the harness fakes the utterance class too, only in the simulated-voice context.
* The E2E offline step expected the wrong canonical rendering ("Thank you very much" for "muchas gracias"). The test phrase was changed.
* LRU bound bug: no eviction when the kept runtime sorted first. Fixed.
* The phrasebook threw on an unsupported language. It now returns "not covered".
* **The blind set found a meaning-loss bug** (template variants that aren't equivalent). Fixed after the frozen run. Both results are reported.
* A test-file type error was committed in `ab5628d` (commands chained with `;`) and fixed in `d980d1e`.
* The measurement script's first run was killed (worker restart). The re-run completed. It read a stale `dist/` for bundle size, and that figure is corrected in the report.

## 5. Not tested
Real devices; real on-device models (none available here); real providers; memory pressure from a real OS (simulated by `unloadAll`); battery and thermal (container).
