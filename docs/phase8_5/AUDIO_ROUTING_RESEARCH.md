# Phase 8.5 — Audio routing, speech recognition and platform limits (research record)

All sources were accessed on **2026-11-23**. Evidence grades:
- **A:** vendor or standards documentation, or a vendor release note.
- **B:** a vendor engineer statement, or a vendor-run bug tracker entry with a reproducible description.
- **C:** a developer forum or third-party article (**unverified unless corroborated**).

Nothing here was tested on a physical device in this phase.

## 1. Can a web page choose the microphone used for speech recognition?
| Platform | Finding | Grade | Source |
| --- | --- | --- | --- |
| Chrome desktop | `SpeechRecognition.start(audioTrack)` accepts a live audio `MediaStreamTrack`; without it, the default mic is used | A | MDN `SpeechRecognition.start()` — https://developer.mozilla.org/en-US/docs/Web/API/SpeechRecognition/start |
| Chrome desktop | Supported from Chrome 135; **Chrome for Android: not supported; Safari, Firefox: not supported** | A− (compat data) | caniuse — https://caniuse.com/mdn-api_speechrecognition_start_audiotrack |
| Chrome | Intent to Ship: "Add MediaStreamTrack support to the Web Speech API" (motivation: captioning other audio sources) | A | blink-dev — https://groups.google.com/a/chromium.org/g/blink-dev/c/4ibjEVQ-i0s |
| Spec | Issue still open for the spec text | A | https://github.com/w3c/speech-api/issues/66 |
| Safari iOS | Web Speech recognition since Safari 14.1, "powered by the same speech engine as Siri" | A | WebKit blog — https://webkit.org/blog/11648/new-webkit-features-in-safari-14-1 |
| Safari iOS | Not enabled for third-party WKWebView embedders | B | WebKit bug 239816 — https://bugs.webkit.org/show_bug.cgi?id=239816 |
| WebKit | `navigator.audioSession.type` (e.g. `play-and-record`): sets category-like intent only, **no input/route selection** | A | MDN Audio Session API — https://developer.mozilla.org/en-US/docs/Web/API/Audio_Session_API ; W3C — https://www.w3.org/TR/audio-session/ |

**Conclusion.**
- **iPhone (Safari, any iOS browser, since all use WebKit) and Chrome for Android:** the web page cannot choose the recognition input. Recognition uses the system's current route.
- **Desktop Chrome:** it *can* choose, by passing a `getUserMedia({deviceId})` track. This is an **unimplemented option**, recorded as a possible improvement for desktop users only. It is not built in 8.5, because the spec excludes scope growth.

## 2. AirPods: what happens to quality when the AirPods mic is used?
| Finding | Grade | Source |
| --- | --- | --- |
| Bluetooth headsets switch from A2DP (stereo, one direction) to the hands-free profile when the mic is active; playback quality drops | C (consistent across vendors; not Apple documentation) | daily.co — https://www.daily.co/blog/why-bluetooth-audio-quality-suffers-in-video-calls/ ; Gcore — https://gcore.com/docs/streaming-platform/troubleshooting/real-time-video-issues/audio-quality-is-reduced-when-using-bluetooth-headsets-in-video-call-apps |
| iPhone users report media through Bluetooth headphones "severely degraded" while the mic is active | C | Apple Community — https://discussions.apple.com/thread/256287820 |
| Apple reportedly improved AirPods call quality via firmware (2024) | C (press report citing social media) | https://www.headphonesty.com/2024/07/apple-solves-bad-airpods-call-quality/ |
| Apple documents a `bluetoothHighQualityRecording` category option (newer OS) | A (existence) | https://developer.apple.com/documentation/avfaudio/avaudiosession/categoryoptions-swift.struct/bluetoothhighqualityrecording |

**Conclusion (unverified until device test B):** expect reduced capture and playback quality when the AirPods mic is the input. The size of the effect on current iOS and AirPods firmware is **unknown**. That is the purpose of device tests A and B.

## 3. Native iOS: can an app record from the iPhone mic while playing to AirPods?
| Finding | Grade | Source |
| --- | --- | --- |
| `AVAudioSession` category options decide Bluetooth availability per category | A | https://developer.apple.com/documentation/avfaudio/avaudiosession/categoryoptions-swift.struct |
| `setPreferredInput(_:)`, `preferredInput` and `currentRoute` select and verify the input port; the built-in mic is found via `availableInputs` | A | https://developer.apple.com/documentation/avfaudio/audio-routing ; QA1799 https://developer.apple.com/library/ios/qa/qa1799/_index.html |
| `playAndRecord` with `.allowBluetoothA2DP` **and without `.allowBluetooth` (HFP)** gave built-in mic in + AirPods out (WWDC lab answer) | C (forum, attributed to a WWDC lab) | https://developer.apple.com/forums/thread/741513 |
| Bluetooth routing regressions reported on iOS 17 for `playAndRecord` | C | https://developer.apple.com/forums/thread/742524 ; https://developer.apple.com/forums/thread/736814 |

**Conclusion:** the desired route is supported by documented APIs. Whether it behaves as intended on current iOS with current AirPods is **unverified**; that is native spike S1 in `NATIVE_FEASIBILITY_DECISION.md`.

## 4. Native speech recognition on iOS
| Finding | Grade | Source |
| --- | --- | --- |
| `SFSpeechRecognizer`: plan for a **one-minute** limit per task for server-based recognition | A | https://developer.apple.com/documentation/speech/sfspeechrecognizer ; QA1951 https://developer.apple.com/library/archive/qa/qa1951/_index.html |
| On-device recognition: `supportsOnDeviceRecognition` / `requiresOnDeviceRecognition`; server limits don't apply on device | A | https://developer.apple.com/documentation/speech/sfspeechrecognitionrequest/requiresondevicerecognition ; WWDC19-256 https://developer.apple.com/videos/play/wwdc2019/256 |
| `SpeechAnalyzer` / `SpeechTranscriber` (iOS 26) for long-form on-device transcription; **locale coverage must be checked at runtime** (Hungarian not confirmed) | A (existence) / C (language list) | https://developer.apple.com/documentation/speech/speechanalyzer ; https://developer.apple.com/documentation/speech/speechtranscriber ; WWDC25-277 https://developer.apple.com/videos/play/wwdc2025/277/ |
| Apple Translation framework: on-device models shared with the Translate app; `LanguageAvailability` reports supported pairs at runtime (Hungarian **not confirmed**) | A (API) / unknown (hu) | https://developer.apple.com/documentation/translation/languageavailability |

## 5. Lifecycle limits on iOS web
| Finding | Grade | Source |
| --- | --- | --- |
| Audio capture track can end after Safari is backgrounded (more than 30 s) | B | WebKit bug 204681 — https://bugs.webkit.org/show_bug.cgi?id=204681 |
| Mic stops after screen auto-lock (iOS 15.5) | B | WebKit bug 241400 — https://bugs.webkit.org/show_bug.cgi?id=241400 |
| Home Screen web apps lose the mic on background or lock | B | WebKit bugs 226620 / 239602 — https://bugs.webkit.org/show_bug.cgi?id=226620 |
| iOS `continuous` recognition can hang or deliver nothing; `interimResults` unreliable | C | https://developer.apple.com/forums/thread/699881 ; https://developer.apple.com/forums/thread/775699 |

Talktual pauses on `visibilitychange: hidden` by design and never listens in the background. These platform limits therefore match the product rule. The open question is whether **foreground** continuous recognition on iOS is reliable enough (device test G/H).

## 6. On-device recognition in Chrome
| Finding | Grade | Source |
| --- | --- | --- |
| `processLocally` and `SpeechRecognition.available()/install()` for on-device language packs (Chrome/Edge 139+) | A | MDN — https://developer.mozilla.org/docs/Web/API/SpeechRecognition/processLocally ; https://developer.mozilla.org/docs/Web/API/SpeechRecognition/available_static ; Edge — https://learn.microsoft.com/en-us/microsoft-edge/web-platform/speech-recognition-api |
| Language list not published as a stable table; check `available({langs, processLocally:true})` at runtime | A | same |

Talktual's on-device recognition probe is off by default (`VITE_ONDEVICE_ASR`, Phase 6), because the earlier availability check crashed the tab on the test hardware. Hungarian on-device recognition is **unknown** and must be checked at runtime on the target device.

## 7. Uncertainties that only physical tests can resolve
1. The actual input route on iPhone + AirPods for Safari Web Speech (device test A/B).
2. Playback quality while listening, and recovery after Pause (A/B).
3. Foreground continuous-recognition reliability on iOS Safari for hu-HU (G/H).
4. Real end-of-speech → audio latency (H; human-marked and engine-reported via test mode).
5. Battery and thermal over 20–30 minutes (I).
