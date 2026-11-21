# Compatibility Report — Talktual Phase 1

Rule applied: a platform is marked **Pass** only if a test actually ran on it in this session. Everything else is **Unverified** — not "passed".

## 1. Browser / device matrix

| Platform | Version | Environment | What ran | Result |
| --- | --- | --- | --- | --- |
| Chromium (headless) | 157.0.8103.78 | Linux container, Playwright, fake media devices | 13-step two-peer E2E against the production build, ×3 | ✅ **Pass** (with faked speech APIs, see §2) |
| jsdom (unit/component) | jsdom 26 | Node 22 | 75 tests | ✅ Pass — not a browser; logic only |
| Chrome / Edge (desktop, headed) | — | — | — | ⬜ Unverified |
| Firefox | — | not installed | — | ⬜ Unverified |
| Safari (macOS) / WebKit | — | not installed | — | ⬜ Unverified |
| iOS Safari | — | no device | — | ⬜ Unverified |
| Android Chrome | — | no device | — | ⬜ Unverified |

Only Chromium is available in this environment (`/opt/pw-browsers` contains Chromium builds only). No physical devices were available.

## 2. What the Chromium pass does and does not cover

**Covered (real):** React UI and routing, QR/link join flow, WebSocket signaling, WebRTC data-channel negotiation (P2P established, reported "Direct"), language handshake, mid-call language switch, translation pipeline through the real server, notices, transcript dialog, leave/peer-left teardown, unknown-code timeout, absence of page errors.

**Faked:** `webkitSpeechRecognition` (scripted phrases), `speechSynthesis` (recorded), translation provider (stub), microphone (`--use-fake-device-for-media-stream`, permission pre-granted). ElevenLabs PCM streaming was not exercised in a browser (no key) — the browser-voice fallback path ran.

## 3. Expected behaviour by platform (from code + platform knowledge — unverified)

| Platform | STT engine the code will pick | Notes / risks to test |
| --- | --- | --- |
| Chrome / Edge desktop & Android | Web Speech (streaming) | Primary path. Audio goes to the vendor's speech service (SEC-R1). |
| Safari macOS / iOS | `webkitSpeechRecognition` if exposed (Safari 14.1+) | iOS needs the autoplay unlock in `unlockAudio()` (runs from the start/join tap); speech recognition and backgrounding behaviour must be tested on device. `AbortSignal.any/timeout` deliberately avoided for older iOS. |
| Firefox | Scribe (batch) via server | **Requires `ELEVENLABS_API_KEY`**; without it the call screen now warns and disables talk (DEF-C11) — listening still works. MediaRecorder `audio/ogg;codecs=opus` or webm candidates. |
| Any, non-localhost over HTTP | — | Mic and speech APIs require a secure context (HTTPS). |

## 4. Accessibility checks (Chromium + jsdom)

| Check | Result |
| --- | --- |
| Interactive controls reachable by role/label (E2E uses role/label locators throughout) | ✅ |
| Sheets exposed as labelled modal dialogs | ✅ (fixed, DEF-C09) |
| Status / notices announced (`role="status"`, `aria-live`, `role="alert"`) | ✅ (fixed) |
| Code input focusable *and* exposed to assistive tech | ✅ (fixed) |
| Keyboard PTT (Space, ignored in text fields), Escape closes picker | ✅ by code; Space verified by code review only |
| Focus trap / focus return in dialogs | ⚠️ Not implemented (observation) |
| Text contrast: `--text-1` 18.38:1, `--text-2` 6.70:1, **`--text-3` 2.52:1** (computed, WCAG formula) | ⚠️ `--text-3` fails AA for small text (OBS-05) |
| Screen reader run (VoiceOver/NVDA/TalkBack) | ⬜ Unverified |

## 5. Recommended manual test plan before a production release

1. iOS Safari (latest and one prior major): start + join, first-utterance audio without extra tap, lock/unlock screen mid-call, AirPods switch.
2. Android Chrome: same, plus switching between Wi-Fi and mobile data (expect "Disconnected"; no auto-reconnect).
3. Firefox desktop with and without `ELEVENLABS_API_KEY` (expect batch transcription vs. the up-front warning).
4. Two different networks (e.g. home Wi-Fi ↔ mobile carrier) to observe "Direct" vs "Relayed".
5. One screen-reader pass of start → join → talk → leave.
