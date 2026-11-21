# Phase 2 Privacy Report — Listening Modes, Consent, Disclosure, Stop

Addresses Phase 1 condition **P1** (spec §0, §3J).

## 1. What Phase 1 actually did (investigated before changing anything)

`CallRoom` called `recognizer.arm()` on mount; the Web Speech adapter started `SpeechRecognition` immediately (`continuous = true`) and restarted it whenever Chrome ended it. Results outside the button hold were discarded locally, **but the recognizer — and so the browser vendor's speech service — received audio for the entire call.** No consent or disclosure existed.

Reproduced in a real browser before any change (`e2e/privacy-probe.mjs`, idle connected call, button never pressed, 3 s):
```
baseline: {"recognizerRunning":true,"recognizerEvents":["start"]}
```

## 2. Listening contract now

| | Push-to-talk (default) | Hands-free (opt-in) |
| --- | --- | --- |
| Recognizer exists | created on first press after consent | created when the mode is switched on |
| Recognizer running | while held + 450 ms tail, then stopped (no restart) | while on and the call is connected; **paused while incoming speech plays** (half-duplex) |
| Results used | all (nothing captured is silently discarded) | all, sent at phrase boundaries |
| Server-STT browsers | recorder runs only while held; mic tracks disabled between pushes; 60 s cap | not offered (needs a streaming recognizer) |
| Visible state | button shows "On air", ripple, live caption | persistent green banner "Hands-free: listening continuously" with **Stop**; button labelled "Hands-free" |
| Persisted? | default | **never** — every call starts in push-to-talk |

Implementation: `AsrAdapter` contract (`engine/ports.ts`) — adapters never start themselves; `CommunicationSession` is the only caller of `start()`, and every path goes through `PrivacyPolicyController.mayCapture(route)`.

## 3. Consent and disclosure

* A disclosure sheet appears the first time someone who *can* talk is connected (and whenever they press without consent). It states, for the **actual route**:
  * Web Speech: "Your browser's speech-recognition service turns your voice into text. In Chrome and Edge that is a Google/Microsoft cloud service; in Safari, Apple's. Audio is sent to it only while you are talking."
  * Server STT: "Your voice is recorded only while you hold the button, then sent through the Talktual server to ElevenLabs for transcription."
  * Translation: names the configured provider (Gemini / DeepL / MyMemory) from `/api/capabilities`.
  * "Talktual doesn't store your audio or transcripts. Push-to-talk is the default; hands-free listening is optional and always shown on screen."
* Buttons: **Allow microphone** / **Not now**. "Not now" keeps the call usable for listening and is not re-shown automatically (only on a press).
* Consent is stored per route and version: `localStorage["talktual.consent.v1.<route>"]`. Changing the disclosure bumps the version and re-asks. Blocked storage → consent lasts for the page only.
* "How your voice is processed" link on the call screen re-opens the disclosure.
* **No `getUserMedia` and no `SpeechRecognition.start()` before consent** (tested at unit, engine and browser level).

## 4. Stop guarantees

Capture stops immediately, and does not come back by itself, on: mic off, leave, peer left, peer away, connection lost, dispose, device removed — in both modes (14 parameterized tests). Further guarantees:
* Results delivered by a recognizer after it was stopped are dropped by the session (`stale_asr_results_dropped`), independent of adapter correctness.
* Server-STT abort discards the recording and any in-flight upload result.
* Pending translations are aborted on leave; nothing finishing later is sent.
* Mic tracks are released on leave/mic-off (Phase 1 DEF-C03 epoch guard retained).

## 5. Verification

| Evidence | Result |
| --- | --- |
| Browser probe after (`e2e/privacy-probe.mjs`) | `{"recognizerRunning":false,"recognizerEvents":[]}` |
| E2E: before consent | disclosure shown; `__sr.length === 0` (no recognizer constructed) in both browsers |
| E2E: after consent, 1.5 s idle | recognizer not running |
| E2E: after release | recognizer stopped; log `start, stop` |
| Latency harness (60 pushes) | release → recognizer stop p50 452 ms, max 459 ms (= tail + flush) |
| E2E hands-free | banner visible while on; Stop ends capture |
| Engine tests | 31 in `session.privacy.test.ts` (consent, modes, half-duplex, stop matrix, failures) |
| Adapter tests | `speechToText.test.ts`: "creating the adapter starts nothing", "no auto-restart after release", "no results after abort" |
| Diagnostics redaction | full conversation → diagnostics contain no transcript, translation or names (test); allowlist redaction unit tests |
| Emulated server-STT route | disclosure names ElevenLabs (compat E2E) |

## 6. Remote-processing data flow (current)

| Data | Leaves the device to | When |
| --- | --- | --- |
| Microphone audio (Web Speech browsers) | Browser vendor speech service | **only while held (+≤0.46 s), or while hands-free is on and not paused** |
| Microphone audio (other browsers) | Talktual server → ElevenLabs Scribe | once per push, after release |
| Transcript text | Talktual server → translation provider; the peer | per finalized phrase |
| Translated text | Talktual server → ElevenLabs TTS (if configured); the peer | per phrase |
| Display name, language | the peer | on connect / change |
| Diagnostics | nowhere (in-memory; exposed only with `?debug=1`) | — |

Server logs: provider, language pair, latency, sizes — no text, names or session ids (Phase 1, unchanged).

## 7. Remaining privacy risks

| Risk | Severity | Note |
| --- | --- | --- |
| Not verified on real devices/browsers | P1 until done | Behaviour of real Chrome/Safari recognizers (e.g. ending early, permission UI) only emulated |
| Hands-free sends audio continuously while on | by design | Disclosed, opt-in, visible, not persisted; paused during playback |
| Browser mic indicator stays on after first push on server-STT browsers | P3 | The stream is kept (tracks disabled) to avoid re-prompting; released on leave/mic-off |
| Consent is per device/browser storage, not an account record | P3 | Clearing site data re-asks |
| Disclosure names provider *types*, not data-retention terms of each vendor | P3 | Link to vendor policies could be added in Phase 3 copy work |
| WebRTC exposes IPs to the peer and STUN provider; no frontend CSP; public TURN creds if configured | P2–P3 | Carried from Phase 1 SECURITY_REPORT |
