# Phase 8 — Discovery and feasibility decisions

Status labels used throughout Phase 8:
- **Verified (here):** shown by code or tests in this repository.
- **Documented:** a vendor or standards source says so; not tested here.
- **Reported:** developer forums or bug trackers; plausible, but not authoritative.
- **Assumption:** reasoned, not yet verified.

Nothing here was tested on a real iPhone, AirPods or in a café.

## 1. What the existing engine already provides (verified by code audit)
| Need | Existing piece | Reuse |
| --- | --- | --- |
| Streaming speech recognition with partial/final, bounded restart | `lib/speechToText.ts` `createAsr(lang, 'continuous', …)`: Web Speech `continuous` + `interimResults`, auto-restart when the engine ends itself, finalize timeout, abort discards | As-is, through the `AsrFactory` port |
| Translation with retries, timeout, cancellation | `lib/translate.ts` `translateSegment` via the Talktual server (Gemini / MyMemory / DeepL, chosen server-side) | As-is, through the `Translator` port |
| Phase 6 policy | `privacy/policy.ts` (`plan`, `beginRemote` execution gate, revocation on `local-only` and offline), `local/registry.ts` (on-device runtimes), `privacy/network.ts` | As-is. Routed translation **extracted to `privacy/routing.ts`** and now shared by the call and ambient sessions (one implementation, not a copy) |
| Bounded, cancellable speech output | `lib/textToSpeech.ts` `PlaybackController`: FIFO, max queue, `notAfter` stale drop, `cancelAll`, server PCM → browser voice fallback, `ttsRoute` (device-only / device-first / cloud-first) | As-is, plus **per-item `volume`** (new, opt-in) so speech can stay below the room |
| Consent / disclosure pattern | `engine/privacy.ts` (per route, per version, local storage) | Same pattern, **separate key** for ambient: capturing *other people's* speech is a different act from talking yourself |
| Diagnostics without content | `engine/diagnostics.ts` (enum/number fields only) | As-is |
| Phase 4 segmenter | `conversation/segmenter.ts`: tuned to *one local speaker's turns* (push-to-talk release, en/es incomplete-phrase lexicon) | Not suitable for a continuous third-party stream. Ambient gets a **stable-prefix segmenter**, reusing the `repair.ts` helpers (filler and repetition) |

**Not present:** speaker diarization, audio-level language identification, input-device selection for Web Speech, and Hungarian on-device translation (the bundled phrasebook is en↔es).

## 2. Platform findings
### Capture source and AirPods
- **Reported:** when the AirPods microphone is active, the Bluetooth link switches from A2DP (stereo AAC) to the hands-free profile (mono, narrowband), degrading *both* capture and playback quality ([daily.co](https://www.daily.co/blog/why-bluetooth-audio-quality-suffers-in-video-calls/), [Apple Community](https://discussions.apple.com/thread/256287820), [Gcore](https://gcore.com/docs/streaming-platform/troubleshooting/real-time-video-issues/audio-quality-is-reduced-when-using-bluetooth-headsets-in-video-call-apps)). Apple reportedly improved this in 2024 AirPods firmware ([Headphonesty](https://www.headphonesty.com/2024/07/apple-solves-bad-airpods-call-quality/)); unverified, and model- and OS-dependent.
- **Reported (Apple Developer Forums, including a WWDC-lab answer):** a *native* app can record from the iPhone's built-in mic while playing to AirPods over A2DP: `AVAudioSession` `.playAndRecord` with `.allowBluetoothA2DP` (without `.allowBluetooth`/HFP), and preferred input = built-in mic ([thread 741513](https://developer.apple.com/forums/thread/741513), [784318](https://developer.apple.com/forums/thread/784318)). iOS 17 regressions were reported ([742524](https://developer.apple.com/forums/thread/742524), [745993](https://developer.apple.com/forums/thread/745993)).
- **Documented:** the web exposes `navigator.audioSession.type` (`'play-and-record'`, etc.; experimental, WebKit only) ([MDN](https://developer.mozilla.org/en-US/docs/Web/API/Audio_Session_API), [W3C](https://www.w3.org/TR/audio-session/)). It has **no way to pick the input route**. **Reported:** Safari forces `play-and-record` while the mic is in use, and it can stick ([WebKit 271305](https://bugs.webkit.org/show_bug.cgi?id=271305), [264473](https://bugs.webkit.org/show_bug.cgi?id=264473)).
- **Verified (code) / Documented:** Web Speech `SpeechRecognition` takes **no MediaStream input**. It always listens to the system's current input, so a web page cannot choose "phone mic in, AirPods out" for recognition.
  - **Correction (Phase 8.5, 2026-11-23):** this was too broad. Chrome desktop 135+ supports `SpeechRecognition.start(audioTrack)`, so a page can recognize from a chosen `getUserMedia` track (MDN; caniuse `mdn-api_speechrecognition_start_audiotrack`). Safari and Chrome for Android do **not**, so the iPhone conclusion is unchanged. See `docs/phase8_5/AUDIO_ROUTING_RESEARCH.md`.

**Decision.** The best capture source for the café scenario is the phone's built-in mic, with AirPods used for listening only. A web app cannot enforce that. On iOS, with AirPods connected and Safari using the mic, the system will likely route input to the AirPods (HFP).

Implemented:
- the setup screen explains the trade-off honestly
- a "Listening through headphones" switch that controls echo protection
- `navigator.audioSession.type = 'play-and-record'` set on start and restored on stop, where present
- a precise remaining-work item: a native iOS shell (`AVAudioSession` as above) is the real fix (**platform blocker for web**)

### Recognition support
- **Documented:** Safari has supported Web Speech recognition since 14.1, using Apple's recognizer, with Siri enabled ([caniuse](https://caniuse.com/mdn-api_speechrecognition_continuous)). Not enabled in third-party WKWebViews ([WebKit 239816](https://bugs.webkit.org/show_bug.cgi?id=239816)).
- **Reported:** on iOS, `continuous = true` can hang or deliver nothing; `interimResults` is unreliable ([Apple forums 699881](https://developer.apple.com/forums/thread/699881), [775699](https://developer.apple.com/forums/thread/775699), [WebKit 252936](https://bugs.webkit.org/show_bug.cgi?id=252936)).
- Chrome/Edge (desktop, Android): Web Speech goes to a cloud recognizer; continuous mode ends periodically and the adapter restarts it (verified in code and tests). `hu-HU` is accepted by Chrome's recognizer (**documented** by Google's language list; not tested on a device here).

**Decision.** Ambient mode uses the existing continuous Web Speech adapter.

Implemented safeguards:
- a **watchdog**: no recognizer activity for 25 s while listening → recognizer restart (bounded: 3 tries per minute, then a visible "Listening stalled" state with Retry)
- interim-independent finalization: if a platform only delivers finals, the pipeline still works, just with higher latency

Not supported, and disclosed: Firefox (no Web Speech) — an ambient recognizer there would need streaming server STT, which this server does not have. Its STT is batch-only after push-to-talk, and not free.

### Language identification
Web Speech needs the language up front; there is no audio language ID. **Implemented:** the user chooses "They speak" (default Hungarian). A text-level check flags when recognized words look like a different language ("This sounds like English — change?"). It never switches by itself.

### Speaker separation
No browser API provides diarization or source separation. **Not implemented, and not claimed.** Lines are labelled by time, not by speaker. The UI says "Speaker labels aren't available" in the help text, rather than inventing labels.

### Providers, cost and data exposure
| Stage | Route | Cost | Data exposure |
| --- | --- | --- | --- |
| Recognition | Web Speech (Chrome: Google; Safari: Apple) | Free to the app | Audio to the browser vendor's recognizer while listening |
| Translation | Talktual server → MyMemory (free) / Gemini or DeepL if keys are configured | Free tier / operator's keys | Text only |
| On-device translation | Phrasebook (en↔es only); Chrome Translator API where present | Free | None |
| Speech | Server TTS if configured, else the browser voice; on-device voice in On-device mode | Operator's keys / free | Translated text |

No paid calls are made in tests: providers are stubbed. No models were downloaded.

### Hungarian → English in each processing mode (verified by policy and registry code)
- **Cloud:** works (cloud recognition, cloud translation).
- **Hybrid:** on-device steps are tried first. There is no hu→en on-device translator unless the browser's Translator API offers it, so translation falls back to the cloud.
- **On-device only:** recognition is on-device only if the browser supports `processLocally` for hu (not on the tested hardware). With no on-device recognition, ambient listening is **unavailable**. The UI says so and never silently uses the cloud.

## 3. Risk list (carried into the device-test plan)
1. **iOS input route** (AirPods HFP versus phone mic) and output quality while listening.
2. **iOS continuous recognition reliability**, and behaviour on screen lock or when backgrounded. The page stops capturing when hidden; this is a **decision**, so capture never continues without a visible indicator.
3. **Feedback loop:** translated speech re-captured by the phone mic when output is the speaker. Mitigated by half-duplex unless headphones are declared, plus an echo filter.
4. **Real latency, accuracy in café noise, battery and thermal behaviour:** not measurable here.
