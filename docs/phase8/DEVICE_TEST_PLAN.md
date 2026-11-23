# Phase 8 — Real-device test plan (not yet performed)

Nothing in this plan has been run. Everything Phase 8 verified so far is simulated: desktop Chromium, scripted recognizer, fake speech output, stubbed providers. This plan is what separates "the orchestration works" from "it works in a Budapest café with AirPods".

## Devices and configurations
| # | Device | Browser | Output | Input (expected) | Why |
| --- | --- | --- | --- | --- | --- |
| D1 | iPhone (iOS 17 and current) | Safari | AirPods Pro, transparency on | AirPods mic (system default) | Flagship scenario as shipped |
| D2 | iPhone | Safari | AirPods disconnected; phone speaker | Built-in mic | Half-duplex and echo path |
| D3 | iPhone | Safari | Wired or USB-C headphones | Built-in mic | Best web capture case on iOS |
| D4 | Pixel or mid-range Android | Chrome | Bluetooth earbuds | System default | Chrome cloud recognizer for hu-HU |
| D5 | Laptop | Chrome / Edge | Headphones | Built-in mic | Desktop baseline |
| D6 | Laptop | Firefox | — | — | Must show "can't recognise speech" (no Web Speech) |

## What to measure (per configuration)
Record **numbers and enums only**: no recordings of bystanders, and no transcripts kept after the session. Diagnostics are available with `?debug=1` via `window.__talktualAmbientDiagnostics()`. They contain stage latencies, counters and routes, never text.

1. **Route.** Which mic is actually used (iOS Control Centre, audio output picker). Does output quality drop to call quality (mono, muffled) while listening? Does it recover after Pause, and after Back?
2. **Recognition.** In continuous mode, is a Hungarian final produced at all on iOS Safari (known bug reports)? How often does the watchdog restart (`asr_restarts`)? Does the stalled state ever appear?
3. **Latency (real).** `amb_commit_ms`, `amb_text_ms`, `amb_e2e_speech_ms`, p50 and p95 over 30+ phrases. Separately, a stopwatch from the end of a spoken phrase to the start of the translation in the ear.
4. **Quality.** Using scripted phrases read aloud by a consenting Hungarian speaker at 1 m, 2 m and 4 m, in a quiet room and with café noise played back at about 65–70 dBA: word error rate by inspection, translation adequacy (1–5), and the share of lines marked "Unclear audio".
5. **Half-duplex (D2).** Are translations ever re-recognized (`echo_dropped`)? Does capture resume after speech (`half_duplex_holds`)?
6. **Interruptions.** Phone call, Siri, notification sound, AirPods removed or inserted, screen lock, app switch. Expected: listening pauses (`hidden` or `mic`), is shown, and never resumes silently. The one exception is reconnection after the app went offline, which resumes because the user had started listening.
7. **Battery and thermal.** 30 minutes of continuous listening on D1 and D4: battery %, and whether the device gets warm. Compare with screen-on idle.
8. **Ethics and consent.** Only consenting speakers. No recording. Bystander-facing guidance in the disclosure is shown before first use.

## Pass criteria (proposed)
- A Hungarian final is recognized and spoken in English on at least one of D1 or D3, with p95 end-of-phrase → audio under 4 s on a good network.
- No speech played out of order or twice. No listening after Pause, Back or screen lock.
- On iOS with AirPods (D1): document the actual input route and quality. If HFP degradation is confirmed, it confirms the need for the native-shell option below.

## If D1 confirms the web routing limit
Build a thin native iOS shell (WKWebView is **not** enough: Web Speech is disabled for third-party WKWebViews, per WebKit bug 239816). Use:
- `AVAudioSession` `.playAndRecord` with `.allowBluetoothA2DP` (no `.allowBluetooth`), preferred input = built-in mic
- `SFSpeechRecognizer` for recognition
- the same translation server and the same policy semantics

That is a separate project and is not started.
