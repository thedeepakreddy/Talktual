# Phase 8.5 — Privacy audit of Live translate (code- and behaviour-verified)

Audit date: 2026-11-23. Scope: ambient capture (Phase 8) and the server paths it uses. Each row cites the evidence: code location and/or test.

## 1. Activation and indicators
| Check | Result | Evidence |
| --- | --- | --- |
| Never listens without an explicit Start + ambient consent | ✔ | `AmbientSession.start/grantConsent`; unit "first use asks for consent"; E2E step 2 |
| Listening always visible | ✔ | header dot + "Listening", tab title "● Listening — Talktual", amber Pause control, status line (`AmbientScreen.tsx`); E2E checks title and meta |
| Stops immediately on Pause, Back, hidden, offline, mic loss, policy change | ✔ | `pauseFor/stopCapture` (abort + release); unit tests; E2E pause and hidden |
| Never re-enables the mic as a side effect | ✔ | unit "stop, pause or dispose while speech holds capture…" |
| No background listening | ✔ by design (pauses on `visibilitychange`) | unit + E2E "leaving the page" |

## 2. Persistence, caches, logs, analytics, diagnostics
| Store | What | Lifetime | Clearable | Evidence |
| --- | --- | --- | --- | --- |
| `localStorage` | `talktual.ambient.consent.v2.browser-speech=granted`, `talktual.processing.mode.v1` | until browser data is cleared | browser settings | grep of `setItem` (only these + call consent); E2E storage audit |
| Transcript lines | original + translation in memory | until Clear / Back / tab close; max 200 | Clear | `AmbientSession` (`maxLines`); unit "lines are bounded", "clear drops all lines" |
| Client translation cache | source → translation, in memory, bounded | page lifetime | cleared on Clear, mode change and dispose (`clearTranslationCache`) | `lib/translate.ts`; `AmbientSession.clear/dispose` |
| **Server translation cache** | source → translation, in memory, shared process-wide | **Before 8.5:** process lifetime, up to 2,000 entries, hits advertised. **Now:** TTL 2 min default (`TRANSLATION_CACHE_TTL_MS`), `TRANSLATION_CACHE_MAX=0` disables, hits not advertised | expires; restart | `server.js` cache functions; server tests "expires after the TTL", "not advertised" |
| Server logs | language pair, provider, latency, status codes | stdout (operator's hosting) | operator | **Before 8.5:** provider error bodies logged (could echo text). **Now:** provider and status only. Server test "provider error bodies are never logged" |
| Client console | error objects (no content) | DevTools only | — | grep of `console.*` in `src` |
| Analytics / telemetry | **none** | — | — | grep: no analytics, beacon or third-party SDK |
| Diagnostics (`Diagnostics`) | enums and numbers (allowlist redaction) | in memory; `?debug=1` exposes them on `window` | dispose | `engine/diagnostics.ts`; unit "no text in diagnostics" |
| Validation recorder (8.5) | ids, enums, timings | in memory; **only with `?validate=1`/`?debug=1`**; export is an explicit user action | Clear / dispose | `ambient/validation.ts`; unit + E2E "no content in export" |
| Audio | **none stored by Talktual**; the browser's recognizer receives audio while listening | — | — | no audio buffers in ambient code; the Web Speech API streams to the browser vendor |

## 3. Third-party processing and cloud routing (per processing mode)
| Data | Recipient | Cloud | Hybrid | On-device | Notes |
| --- | --- | --- | --- | --- | --- |
| Room audio | Browser vendor's recognizer (Chrome/Edge: Google/Microsoft; Safari: Apple) | ✔ | ✔ (no on-device hu recognizer on tested hardware) | ✘ listening unavailable unless on-device recognition exists | outside Talktual's control; the vendor's policy applies |
| Recognised text | Talktual server → Gemini / DeepL / MyMemory (operator configuration) | ✔ | ✔ if not translatable on device | ✘ | the server is in the operator's chosen hosting region; providers' regions vary |
| Translated text (speech on) | Talktual server → **ElevenLabs** if server TTS is configured; else the browser voice | ✔ | ✔ | device voice only | **8.5 fix:** now named in the ambient disclosure (v2) |
| IP address | Talktual server (rate-limit buckets, in memory) | ✔ | ✔ | ✔ (signalling not used by ambient; capability fetch is) | standard web-server exposure |

**Policy enforcement is end to end, not just displayed:**
- the recognition gate uses `beginRemote('asr')`
- translation goes through `routedTranslate` with execution-time re-checks and revocation
- the speech route is resolved per item at start
- E2E shows no inference request after switching to On-device

## 4. Findings and actions
| # | Finding | Severity | Action |
| --- | --- | --- | --- |
| P1 | Server translation cache kept source text with no expiry, shared across users, and advertised hits (membership side-channel: anyone could test whether a phrase was recently translated) | High (privacy) | **Fixed:** TTL default 2 min, configurable or disable; hits not advertised; tests added. *Residual:* response-time differences can still hint at a cache hit within the TTL. Operators wanting zero retention set `TRANSLATION_CACHE_MAX=0` |
| P2 | Provider and ElevenLabs error bodies logged (≤ 300 characters; could echo user text) | Medium | **Fixed:** log provider and status only; test added |
| P3 | Ambient disclosure didn't name the speech-output processor (ElevenLabs) | Medium (transparency) | **Fixed:** named per mode and server capability; consent version bumped to v2 so users see it again |
| P4 | The call disclosure (Phase 2–6, `engine/privacy.ts`) also doesn't name the TTS processor | Medium (transparency) | **Not changed** (outside Phase 8 scope; the owner should decide). Recommended wording is as in `ambient/disclosure.ts` |
| P5 | Bystanders get no notice (no visible signal to people nearby) | Open | Product and legal question → counsel brief Q4 |
| P6 | Browser vendors' own retention of recognition audio | Open | Outside Talktual's control; document in the privacy notice → counsel brief Q6 |

## 5. Re-test of the "no retention" claims
| Claim | Test |
| --- | --- |
| Nothing from the room in browser storage | E2E "Clear removes every line; nothing from the room is in browser storage" |
| Diagnostics and validation exports contain no text | unit (`JSON.stringify` vs phrases) + E2E validation (word scan of export) |
| Server logs contain no user text | server tests (success path and provider-error path) |
| Server doesn't keep text beyond the TTL | server test "expires after the TTL" |
