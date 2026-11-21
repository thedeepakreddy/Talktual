# Phase 2 Compatibility Report

Three evidence classes, never mixed:
* **Automated (real):** the actual browser engine ran the app.
* **Emulated:** Chromium ran the app with an API removed or a device profile applied. It exercises the code path another platform takes, but it is **not** that platform.
* **Real device:** a person on physical hardware. **None performed in Phase 2** (no devices in this environment).

## 1. Matrix

| Platform | Class | Version / profile | What ran | Result |
| --- | --- | --- | --- | --- |
| Chromium (headless, Linux) | Automated | 157.0.8103.78 | smoke 17, recovery 8 (relay-only), compat 5, privacy probe | ✅ Pass |
| "Firefox-like" (Chromium without Web Speech) | Emulated | Chromium 157 | consent names ElevenLabs route; **real MediaRecorder** recording of the fake mic → server STT (stubbed provider) → translated → heard by peer; hands-free not offered | ✅ Pass |
| No STT anywhere (no Web Speech, server without STT) | Emulated | Chromium 157 | up-front notice, talk disabled, no consent prompt, listening works | ✅ Pass |
| Phone viewport + touch | Emulated | Playwright "Pixel 5" (393×727, touch) in Chromium | home/invite/call without horizontal scroll; consent sheet's Allow button on screen and tappable; PTT target ≥ 44 px | ✅ Pass |
| jsdom (logic) | Automated (not a browser) | jsdom 26 | 207 tests | ✅ Pass |
| Chrome / Edge desktop (headed, real mic) | Real device | — | — | ⬜ **Not tested** |
| Firefox desktop | Real browser | — | — (not installed) | ⬜ **Not tested** |
| Safari macOS | Real browser | — | — (WebKit not installed) | ⬜ **Not tested** |
| iOS Safari | Real device | — | — | ⬜ **Not tested** |
| Android Chrome | Real device | — | — | ⬜ **Not tested** |
| Screen reader (VoiceOver/NVDA/TalkBack) | Real AT | — | — | ⬜ **Not tested** (roles/labels exercised by E2E locators only) |

## 2. What the emulation can't tell us (why real devices still matter)

| Platform | Risk to verify |
| --- | --- |
| Chrome/Edge real | Recognizer **start-up delay on press** (first-word clipping, now that it isn't pre-armed); `no-speech` ends during long holds; permission prompt flow on first press |
| Safari (macOS/iOS) | `webkitSpeechRecognition` behaviour per version; on iOS, recognition + `AudioContext` + `speechSynthesis` unlocks from the tap; behaviour when the screen locks or the app backgrounds mid-call (reconnect path) |
| Firefox | Real MediaRecorder mime type (`audio/ogg;codecs=opus` vs webm), `navigator.permissions` rejecting `"microphone"` (handled as `unknown`), ElevenLabs Scribe accuracy |
| Android Chrome | Network switch Wi-Fi ↔ cellular (reconnect within the 20 s grace); Bluetooth headset switching (device-lost path) |
| All | TURN/NAT on real networks ("Direct" vs "Relayed"); echo in hands-free mode on speakerphone (half-duplex pause relies on playback events) |

## 3. Staged real-device verification plan (Phase 1 condition C4)

Run on a staging deployment with real provider keys and `ALLOWED_ORIGINS` set. Record per row: device, OS, browser + version, network, result, notes. Mark anything not run as **Not run**, never as pass.

**Stage 1 — Primary path (blocker for release)**
| # | Device / browser | Steps |
| --- | --- | --- |
| 1.1 | Desktop Chrome (latest) ↔ Desktop Chrome | consent sheet → allow → first push: is the first word captured? → 10 turns each way → leave |
| 1.2 | Same | hands-free on → banner visible → speak → peer hears → peer replies (are you transcribed while they speak? expected: no) → Stop |
| 1.3 | Same | mic button off → press does nothing; on → works; `chrome://settings/content/microphone` block → retry path message |

**Stage 2 — Mobile (blocker for a mobile-facing release)**
| # | Device / browser | Steps |
| --- | --- | --- |
| 2.1 | iPhone, Safari (current + previous major) ↔ desktop Chrome | stage 1.1; also first incoming audio plays without an extra tap |
| 2.2 | iPhone | lock screen 10 s mid-call, unlock → reconnect or clear failure message |
| 2.3 | Android Chrome | Wi-Fi → cellular mid-call → reconnect within 20 s; no duplicated speech |
| 2.4 | Android Chrome | connect/disconnect Bluetooth headset mid-call → device-lost notice → mic on → works |

**Stage 3 — Secondary browsers**
| 3.1 | Firefox desktop, server with ElevenLabs key | batch transcription works; disclosure names ElevenLabs |
| 3.2 | Firefox desktop, server without key | "Speech recognition isn't available" notice; can still listen |
| 3.3 | Safari macOS | stage 1.1 |

**Stage 4 — Networks and accessibility**
| 4.1 | Two different networks (home Wi-Fi ↔ cellular) | transport label; call quality "Relayed" vs "Direct" |
| 4.2 | VoiceOver (iOS) or NVDA (Windows) | start → consent → talk → leave announced and operable |

## 4. Code-level compatibility decisions
* No `AbortSignal.any/timeout` in client code (older iOS Safari).
* `navigator.permissions.query({name:'microphone'})` failure → `unknown` (Firefox/older Safari).
* Hands-free is offered only where a streaming recognizer exists.
* Wire protocol v1 accepts unversioned Phase 1 peers; mixed-version calls work (see CHANGELOG).
