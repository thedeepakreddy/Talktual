# Repair Log — Talktual Phase 1

Chronological record of changes made on branch `claude/kind-archimedes-n6y11p` from baseline `34a8400`. Each entry names the defects it addresses ([DEFECT_REGISTER](DEFECT_REGISTER.md)), files, rationale, risk and the tests that verify it.

## R-01 · Server: testable factory + signaling/API repairs
**Commit:** `49f3a51` (together with R-02…R-10)
**Defects:** DEF-S01 – S10
**Files:** `signaling-server/server.js`, `signaling-server/test/{helpers,api.test,signaling.test}.js`, `signaling-server/package.json`

| Change | Rationale |
| --- | --- |
| `createServer(config)` factory + `loadConfig(env)`; listen only when run directly; `dotenv` loaded only in that branch | The file listened on import, so it could not be tested; tests must never read a developer `.env` with real keys. `node server.js` behaviour is unchanged. |
| `fetch` injectable | Provider calls are stubbed in every test — no live or paid calls. |
| `ws.on('error')` | DEF-S01 (process crash). |
| `maxPayload` 64 KiB; binary frames ignored | DEF-S02. |
| session-id regex, `already_joined`, ghost-peer filtering, membership check on relay/close, expired sockets detached | DEF-S03, DEF-S04. |
| relay allowlist (`offer`/`answer`/`ice`/`app`) | DEF-S05. |
| `ALLOWED_ORIGINS`, `RATE_LIMIT_PER_MINUTE`, `TRUST_PROXY` (opt-in) + boot warnings | DEF-S06 without breaking the unknown production topology. |
| JSON error handler, `x-powered-by` off, `nosniff`, `Referrer-Policy` | DEF-S07. |
| text type/length checks, supported-language checks, JSON 64 kB, STT 10 MB / 1 file, mimetype + language hint validation | DEF-S08. |
| provider timeouts; TTS upstream aborted on client disconnect | DEF-S09. |
| translation failure → 502/504 with generic codes; STT returns only `{text, language_code}` | DEF-C02 (server half), DEF-S10. |
| session ids no longer logged | Room codes are bearer secrets for a call. |

**Risk:** moderate surface (one file rewritten in place) but logic preserved line-for-line where not repaired; covered by 37 server tests + E2E. **Behaviour changes visible to clients:** translate failures are now non-2xx (older clients treat that as a failure and fall back to source text — same visible result as before); TTS upstream errors are 502 instead of mirroring the provider status (client falls back either way).

## R-02 · Server dependencies
**Defects:** DEF-S11 · **Files:** `signaling-server/package*.json`
`multer` 2.2.0 → 2.5.0; `npm audit fix` (express transitive: `proxy-addr` 2.0.8, `body-parser` 2.4.0, `qs` 6.17.1). In-range, non-breaking. **Verify:** `npm audit` → 0, server suite green.

## R-03 · TTS queue cancellation and node cleanup
**Defects:** DEF-C01, DEF-C10 · **Files:** `src/lib/textToSpeech.ts`, `src/lib/textToSpeech.test.ts`
Generation counter checked by each queued item; `AbortController` per segment aborted on barge-in; no browser-voice fallback for aborted segments; per-segment `GainNode` disconnected on completion. **Risk:** low — queue semantics unchanged for the non-barge-in path (ordering test passes on baseline and now). **Tests:** 7 (4 fail on baseline).

## R-04 · Honest translation failures
**Defects:** DEF-C02, DEF-S09 (client) · **Files:** `src/lib/translate.ts`, `src/components/CallRoom.tsx`, `src/lib/webrtc.ts`, `src/types.ts`, tests
`TranslationError` (http / timeout / network / malformed); failures never cached; 10 s timeout implemented with plain `AbortController` + `setTimeout` (not `AbortSignal.any/timeout`, which older iOS Safari lacks); caller aborts (superseded live captions) stay `AbortError` and don't share in-flight promises. `CallRoom`: sender marks `untranslated`, notices on both sides, receiver re-translates or plays the original in the source-language voice, transcript label. New optional protocol field `segment.untranslated` (ignored by older peers). **Risk:** low–moderate (touches the core turn path); covered by unit, component and E2E tests.

## R-05 · Mic release race
**Defects:** DEF-C03 · **Files:** `src/lib/audio.ts`, `src/lib/audio.test.ts`
Release epoch; late streams stopped; stale `finally` can't clobber a newer request. **Tests:** 4 (1 fails on baseline).

## R-06 · Scribe recognizer races; Web Speech timer cleanup
**Defects:** DEF-C08 · **Files:** `src/lib/speechToText.ts`, `src/lib/speechToText.test.ts`
Per-push `MediaRecorder` and chunk list; `holding` re-checked after the async mic wait; no upload/emit after `destroy()`; Web Speech release timer cleared on destroy. **Tests:** 9 (2 fail on baseline; Web Speech behaviour tests pass on both).

## R-07 · Join flow
**Defects:** DEF-C04, DEF-C05, DEF-C09 (part) · **Files:** `src/components/JoinCall.tsx`, `src/components/StartCall.tsx`, `src/App.tsx`, `src/lib/roomCode.ts`, tests
12 s join timeout; socket + handlers torn down on finish/unmount; distinct messages per server error code; CSPRNG 6-char codes; shared normalization for typed, scanned and `?join=` codes; both screens use `SIGNALING_URL` from config (was duplicated `import.meta.env` reads); labelled code input; `role="alert"` errors.

## R-08 · Transport validation and loss detection
**Defects:** DEF-C06, DEF-C07 · **Files:** `src/lib/webrtc.ts`, `src/lib/webrtc.test.ts`
`isCallMessage()` on every inbound path; negotiation in a caught async function; ICE candidate shape check; `disconnected` when the data channel closes after the socket is gone.

## R-09 · Accessibility semantics
**Defects:** DEF-C09 · **Files:** `CallRoom.tsx`, `LanguagePicker.tsx`, `JoinCall.tsx`
Dialog roles/labels, live regions, labelled controls, `aria-pressed`. No visual change.

## R-10 · Dead code and client dependencies
**Defects:** DEF-C12 · **Files:** `src/SocketContext.tsx` (deleted), `package.json`, `package-lock.json`
Removed 10 unused packages; added test deps (`vitest@5.1.1`, `jsdom`, `@testing-library/react`, `@testing-library/dom`); package renamed `react-example` → `talktual`; scripts `test`, `test:server`, `test:e2e`, `test:all`.

## R-11 · STT-unavailable notice; E2E; perf
**Commit:** `062e444`
**Defects:** DEF-C11 · **Files:** `src/components/CallRoom.tsx`, `src/components/CallRoom.test.tsx`, `src/test/setup.ts`, `e2e/smoke.mjs`, `e2e/perf.mjs`, `.gitignore`
Persistent notice + disabled PTT when neither Web Speech nor server STT is available. Removed a redundant `sr-only` status element I had added in R-09 (it duplicated the `aria-live` hint and would be announced twice — found by the E2E run).

## R-12 · Documentation
**Files:** `README.md`, `.env.example`, `docs/phase1/*.md`
Verified commands, test matrix, new server variables, limitations/troubleshooting, removed unused `APP_URL` from `.env.example`.

## Not changed (deliberately)
UI design/visuals, provider selection logic, PTT/segmentation timing constants, Web Speech always-armed behaviour (OBS-01), DeepL code mapping (OBS-02), reconnect (OBS-03), bundle splitting (OBS-04), colours (OBS-05).
