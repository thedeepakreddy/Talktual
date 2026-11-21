# Defect Register — Talktual Phase 1

Severity: **P0** critical security / data loss / session compromise · **P1** major reliability, security or core function · **P2** significant UX / performance / compatibility · **P3** minor.

Only items with reproducible evidence are listed as defects. Concerns without such evidence are in §3 (observations) and the [SECURITY_REPORT](SECURITY_REPORT.md) risk register.

"Baseline" = commit `34a8400`. Test names refer to `signaling-server/test/*.test.js` (server) and `src/**/*.test.ts(x)` (client).

## 1. Summary

| ID | Sev | Area | Title | Status |
| --- | --- | --- | --- | --- |
| DEF-S01 | **P0** | Signaling | Malformed WebSocket frame crashes the server process | ✅ Fixed, verified |
| DEF-S02 | P1 | Signaling | 100 MiB frames accepted and relayed (memory/bandwidth DoS) | ✅ Fixed, verified |
| DEF-S03 | P1 | Sessions | `join` without a valid session id pairs strangers | ✅ Fixed, verified |
| DEF-S04 | P1 | Sessions | Double join self-pairs; ghost peers hold seats | ✅ Fixed, verified |
| DEF-S05 | P2 | Signaling | Any message type relayed (peer can inject server events) | ✅ Fixed, verified |
| DEF-S06 | P1 | API | API proxy usable from any origin, no rate limit (provider-quota abuse) | ⚠️ Mitigation shipped, **off by default — needs prod config** |
| DEF-S07 | P2 | API | HTML error page with stack trace and absolute paths | ✅ Fixed, verified |
| DEF-S08 | P1 | API | No type/length validation on paid endpoints | ✅ Fixed, verified |
| DEF-S09 | P1 | API | No upstream timeouts; one stall blocks all later turns | ✅ Fixed, verified |
| DEF-S10 | P3 | API | Upstream provider error text returned to clients | ✅ Fixed, verified |
| DEF-S11 | P1 | Deps | Known-vulnerable dependencies (server) | ✅ Fixed (`npm audit`: 0) |
| DEF-C01 | P1 | TTS | Barge-in doesn't cancel queued segments | ✅ Fixed, verified |
| DEF-C02 | P1 | Translation | Failures disguised as translations and cached | ✅ Fixed, verified |
| DEF-C03 | P2 | Audio/Privacy | Mic left live if prompt answered after leaving | ✅ Fixed, verified |
| DEF-C04 | P2 | Join | Unknown code spins forever; lobby socket leaks | ✅ Fixed, verified |
| DEF-C05 | P2 | Sessions | Room codes from `Math.random`, can be < 6 chars | ✅ Fixed, verified |
| DEF-C06 | P2 | Transport | Inbound peer messages not validated | ✅ Fixed, verified |
| DEF-C07 | P2 | Transport | Silent message loss once both transports are gone | ✅ Fixed, verified |
| DEF-C08 | P2 | STT (Scribe) | Runaway recording / upload after leave / shared buffer | ✅ Fixed, verified |
| DEF-C09 | P3 | A11y | Focused `aria-hidden` input; no dialog/live semantics | ✅ Fixed, verified |
| DEF-C10 | P3 | Audio | GainNode leaked per TTS segment | ✅ Fixed, verified |
| DEF-C11 | P2 | STT | No warning when no STT is available at all | ✅ Fixed, verified |
| DEF-C12 | P3 | Deps | Dead code and unused (vulnerable) dependencies in client | ✅ Fixed (`npm audit`: 0) |

## 2. Details

### DEF-S01 — Malformed WebSocket frame crashes the server · P0
* **Evidence (baseline):** sent a 6-byte masked text frame containing invalid UTF-8 (`C3 28`) to `node server.js`. Server log: `node:events:497 throw er; // Unhandled 'error' event … WS_ERR_INVALID_UTF8`, process exit code 1, `/health` unreachable afterwards.
* **Reproduction:** any client — no session or auth needed. Also triggered by oversized frames once a size limit exists, bad opcodes, etc.
* **Root cause:** `ws` emits `'error'` on the socket for protocol violations; no listener was registered, so Node throws.
* **Impact:** one anonymous request drops every active call on the instance.
* **Fix:** `ws.on("error", …)` logs the code; `ws` already closes the socket (1007/1009).
* **Verification:** `DEF-S01: a malformed frame (invalid UTF-8) does not crash the process`, `DEF-S02: oversized frames are refused and the server survives`, `garbage on the raw TCP port doesn't crash the process`.

### DEF-S02 — Unbounded frame size · P1
* **Evidence (baseline):** a 5 MiB `app` frame was accepted and relayed in full to the peer (`5MB relay received bytes: [5242942]`). `ws` default `maxPayload` is 100 MiB.
* **Fix:** `maxPayload` 64 KiB (configurable `MAX_WS_PAYLOAD_BYTES`); SDP and captions are far below this.
* **Verification:** oversized frame → close code 1009, nothing relayed, server healthy.

### DEF-S03 — Strangers paired via missing session id · P1
* **Evidence (baseline):** two sockets each sending `{"type":"join"}` (no `sessionId`) both received `role` + `ready` — paired into session `undefined`.
* **Root cause:** `msg.sessionId` used as a Map key without validation.
* **Fix:** require `^[A-Z0-9]{4,12}(_signal)?$`; reply `{type:"error",code:"invalid_session"}`.
* **Verification:** `DEF-S03: join without / with malformed sessionId is rejected`.

### DEF-S04 — Double join / ghost seats · P1
* **Evidence (baseline):** one socket sending `join` twice got `offerer`, `answerer`, `ready`, `ready` (paired with itself). A socket that joined `CCCCCC` then `DDDDDD` and closed stayed in `CCCCCC`; the next real joiner of `CCCCCC` got `answerer` + `ready` — paired with a dead socket, so the call never connects.
* **Root cause:** no "already joined" guard; close handler only cleans `ws.sessionId` (the last session).
* **Fix:** reject a second join (`already_joined`); filter non-OPEN peers when seating; close handler only acts if the socket is actually in the session (also stops spurious `peer-left` from expired sockets).
* **Verification:** two `DEF-S04` tests, session-expiry test.

### DEF-S05 — Arbitrary types relayed · P2
* **Evidence (baseline):** `{"type":"whatever","evil":true}` was forwarded verbatim. A peer could send `{"type":"peer-left"}` or `{"type":"expired"}` and the other client would treat it as a server event.
* **Fix:** relay only `offer`, `answer`, `ice`, `app`; ignore binary frames.
* **Verification:** `DEF-S05: only signaling types are relayed`.

### DEF-S06 — Open API proxy · P1 (mitigation needs configuration)
* **Evidence (baseline):** `Origin: https://evil.example` → `Access-Control-Allow-Origin: *`; no origin check on WS upgrade; no rate limiting. Any site (or script) can drive the owner's Gemini/DeepL/ElevenLabs quota.
* **Fix:** `ALLOWED_ORIGINS` (enforced on `/api/*` with 403 and on WS upgrade), per-IP `RATE_LIMIT_PER_MINUTE`, `TRUST_PROXY`. **Defaults keep the previous permissive behaviour** because the production origin and proxy topology are unknown to me and a wrong default would break the live app; the server prints warnings at boot when unset.
* **Verification:** `DEF-S06` tests (API 403, WS refused, allowed origin works), rate-limiter test, default-permissive test.
* **Residual:** open until configured in production — see SECURITY_REPORT SEC-R2.

### DEF-S07 — Stack-trace disclosure · P2
* **Evidence (baseline):** `POST /api/translate` with body `{bad` returned an HTML page containing `SyntaxError … at JSON.parse … /home/user/Talktual/signaling-server/node_modules/body-parser/…`.
* **Fix:** JSON error handler (`invalid_json`, `payload_too_large`, `file_too_large`, generic `internal_error`); `x-powered-by` disabled; `nosniff` and `Referrer-Policy` set.
* **Verification:** `DEF-S07: malformed JSON returns JSON 400 without a stack trace or paths`, security-headers test.

### DEF-S08 — No input validation on paid endpoints · P1
* **Evidence (baseline):** `{"text":{"a":1}}` returned 200 with the object echoed; a 200,000-character TTS body was accepted by the JSON parser (256 kB limit) and would be forwarded to ElevenLabs when a key is configured (no length check in code). `from`/`to` were interpolated into the Gemini prompt unvalidated.
* **Fix:** `text` must be a non-empty string ≤ `MAX_TEXT_CHARS` (2000) → 400/413; languages must be in the supported set; JSON limit 64 kB; STT upload ≤ 10 MB, 1 file; STT mimetype and language hint validated.
* **Verification:** `DEF-S08` tests (translate and TTS; no upstream call made).

### DEF-S09 — No provider timeouts · P1
* **Evidence:** code path — `fetch` to providers had no signal. On the client, outgoing segments are chained (`sendChain`) to preserve order, so one hung translation blocks every later turn of the call indefinitely. Reproduced in tests with a never-resolving stub.
* **Fix:** server 8 s translate / 10 s TTS time-to-headers / 30 s STT (504 on timeout); TTS upstream aborted if the browser disconnects; client translation timeout 10 s.
* **Verification:** `DEF-S09` translate + TTS tests; client `a stalled request times out with reason "timeout"`.

### DEF-S10 — Upstream error text exposed · P3
* **Evidence:** baseline returned `{ text, provider:"none", error: err.message }` where `err.message` contained the raw provider response body.
* **Fix:** generic error codes to clients; provider details only in server logs (truncated).
* **Verification:** `Gemini failure falls back … upstream error text is not exposed`.

### DEF-S11 — Vulnerable server dependencies · P1
* **Evidence:** `npm audit`: `proxy-addr` (critical, via express), `multer` 2.2.0 (high, direct: several DoS advisories), `body-parser`, `qs`.
* **Fix:** `multer` → 2.5.x, `npm audit fix` (in-range). **Verification:** `npm audit` → 0; server suite green.

### DEF-C01 — Barge-in doesn't cancel the queue · P1
* **Evidence:** test `stopSpeaking drops segments already queued behind the current one` fails on baseline `textToSpeech.ts` (B and C are fetched and spoken after A is cut off); `segments enqueued after a barge-in…` and `aborts the in-flight TTS request…` also fail on baseline.
* **Root cause:** `stopSpeaking()` reset `queueTail` but the already-chained `.then` callbacks still ran.
* **Fix:** generation counter — queued items from before a barge-in are skipped; in-flight TTS fetch aborted via `AbortController`.
* **Verification:** 3 tests (fail on baseline, pass now).

### DEF-C02 — Translation failure masked and cached · P1
* **Evidence:** baseline server returned HTTP 200 `{ text: <source>, provider: "none", error }`; baseline client used `data.text || trimmed` and called `remember()` on it, so the source text was cached as the "translation" of that phrase for the tab's lifetime. Network errors also returned source text. The listener's TTS then read e.g. English text with a Spanish voice, with no indication anything failed.
* **Fix:** server returns 502/504; client throws `TranslationError`, never caches failures; sender sends the original marked `untranslated: true` and sees a notice; receiver tries its own translation, otherwise plays the original **in the source language's voice** with a notice; transcript labels it.
* **Verification:** 7 client translate tests, 3 CallRoom tests, server test, E2E step "translation failure is surfaced, not disguised".

### DEF-C03 — Mic left on after leaving · P2 (privacy)
* **Evidence:** test fails on baseline `audio.ts`: `getMic()` pending → `releaseMic()` → prompt resolves → baseline stores the stream and never stops its tracks (browser mic indicator stays on until the next call).
* **Fix:** release epoch; a late stream is stopped and the request rejects with `AbortError`.
* **Verification:** `DEF-C03…` (fails on baseline, passes now).

### DEF-C04 — Join hangs forever / socket leak · P2
* **Evidence:** code path — `JoinCall` had no timeout; a wrong code waits in a 1-person lobby indefinitely with the Join button disabled. The socket was not closed on unmount, so after "Back" it kept a lobby seat and a later `ready` called `onJoined`, navigating the user into a call from another screen. `ws.onclose` reset the spinner without explaining why.
* **Fix:** 12 s timeout with a clear message; socket closed and handlers detached on unmount/finish; error messages distinguish full / invalid / unreachable / lost.
* **Verification:** 5 JoinCall tests; E2E "joining an unknown code fails with a message" (and server has 0 sessions afterwards).

### DEF-C05 — Weak / short room codes · P2
* **Evidence:** `Math.random().toString(36).substring(2, 8)` — not a CSPRNG, and for values with short base-36 expansions (e.g. `0.5` → `"0.i"`) yields codes shorter than 6 characters, which `JoinCall` (exactly 6 required) cannot join.
* **Fix:** `generateRoomCode()` — `crypto.getRandomValues`, rejection sampling, always 6 of `[A-Z0-9]`; shared `normalizeRoomCode`/`codeFromScan` also sanitize `?join=` input.
* **Verification:** `roomCode.test.ts` (2000 samples, bias rejection, server-rule match).

### DEF-C06 — Unvalidated peer messages · P2
* **Evidence:** baseline `webrtc.ts` passed `msg.payload` straight to `onMessage`; `{type:"app"}` with no payload made `CallRoom` read `msg.t` of `undefined` inside an `async` `onmessage` → unhandled rejection. Malformed `segment`s (`targetLang` missing) threw at `.split`.
* **Fix:** `isCallMessage()` schema check (types + length caps) on both data channel and relay; negotiation errors caught and logged.
* **Verification:** `isCallMessage` table tests; "drops malformed ones without throwing" (asserts no unhandled rejection); negotiation-failure test.

### DEF-C07 — Silent message loss · P2
* **Evidence:** code path — with the data channel open, a signaling-socket drop correctly doesn't end the call; but if the channel then closes, baseline only switched the transport label. `send()` then queued every later message into an outbox that is never flushed, while the UI still showed "connected".
* **Fix:** data-channel close with the socket already gone → status `disconnected`.
* **Verification:** `DEF-C07…` test.

### DEF-C08 — Scribe recognizer races · P2
* **Evidence:** 2 tests fail on baseline `speechToText.ts`: (a) hold + release before the mic resolved → recorder started afterwards and recorded until the next release; (b) `destroy()` mid-push → `onstop` uploaded the audio and emitted a segment after the call ended. (c) A single shared `chunks` array was reset by the next `hold()`.
* **Fix:** one recorder + chunk list per push; `holding` flag checked after the async mic wait; uploads/results suppressed after destroy.
* **Verification:** 4 scribe tests.

### DEF-C09 — Accessibility · P3
* **Evidence:** the channel-code `<input>` that receives focus was `aria-hidden="true"`; language picker and transcript sheets had no `role="dialog"`/label; notices and status changes were not in a live region.
* **Fix:** labelled input; `role="dialog" aria-modal` + labels; `role="status"`/`aria-live="polite"`; `role="alert"` for join errors; `aria-pressed` on language options.
* **Verification:** JoinCall label test; E2E locates dialogs/inputs by role and label.
* **Not fixed:** `--text-3` contrast 2.52:1 (OBS-05).

### DEF-C10 — GainNode leak · P3
* **Evidence:** test fails on baseline — the per-segment `GainNode` was only disconnected on barge-in, never on normal completion.
* **Fix:** disconnect on finish. **Verification:** `DEF-C10…`.

### DEF-C11 — No STT, no warning · P2
* **Evidence:** code path — `createRecognizer` picks the Scribe engine when Web Speech is missing, but `CallRoom` never consulted `capabilities.stt`. With no ElevenLabs key the user could hold-to-talk and only got "Transcription failed (503)" after each attempt.
* **Fix:** persistent notice and PTT disabled when `engine === 'scribe' && !caps.stt`; listening still works.
* **Verification:** CallRoom test (fails with the fix disabled, passes with it) + positive control.

### DEF-C12 — Dead code / unused vulnerable deps · P3
* **Evidence:** `SocketContext.tsx` never imported; 10 unused packages (grep + build). Root `npm audit` 11 advisories (1 critical) mostly via unused `express`/`socket.io`, plus `vite`.
* **Fix:** removed; `vite` patched in range; vitest pinned to 5.1.1 (3.x carried advisories). **Verification:** `npm audit` 0; build green.

## 3. Observations (not defects / not changed)

| ID | Observation | Why not changed |
| --- | --- | --- |
| OBS-01 | Web Speech recognition is armed for the whole call (audio streamed to the browser vendor even when not holding PTT). | Deliberate latency design; changing it changes product behaviour → owner decision (SEC-R1). |
| OBS-02 | DeepL language codes: `NO` invalid (should be `NB`), `EN`/`PT` targets legacy. | Unverified without paid calls; failure falls back to MyMemory. |
| OBS-03 | No automatic reconnect after signaling loss. | New feature, not a repair. Now surfaced as "Disconnected". |
| OBS-04 | Bundle is one 575 kB chunk (197 kB gzip). | Optimization, not a defect; see PERFORMANCE_REPORT. |
| OBS-05 | `--text-3` text colour contrast 2.52:1 on the background (WCAG AA needs 4.5:1 for small text). | Visual design change → owner decision. |
| OBS-06 | Transcript `messages` grows without bound in very long calls. | Text only; no measured problem. |
| OBS-07 | Room access = knowledge of a 6-char code (36⁶ ≈ 2.2 × 10⁹); no accounts. | By design for the QR flow; see SECURITY_REPORT. |
| OBS-08 | `createLevelMeter` in `audio.ts` is unused. | Harmless; left to avoid churn. |
