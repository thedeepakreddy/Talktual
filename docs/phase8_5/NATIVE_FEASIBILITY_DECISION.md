# Decision record DR-8.5-1 — Web-first, thin native shell, or other?

- **Status:** **Proposed** — the decision is deferred until device tests A, B, G, H and I are run (owner).
- **Date:** 2026-11-23.
- **No native code is built in Phase 8.5.**

## Context
The flagship scenario is: iPhone, AirPods, Budapest café, Hungarian → English in the ear. Phase 8 built it as a web feature. Research (`AUDIO_ROUTING_RESEARCH.md`) shows that on iPhone:
- a web page cannot choose the recognition input
- WebKit's Web Speech uses the system route, which is likely the AirPods mic (hands-free profile) when AirPods are connected
- continuous recognition has reported reliability problems
- background capture is not possible for web pages, and Talktual doesn't want it anyway

A native app can request the phone mic plus A2DP AirPods output through documented `AVAudioSession` APIs. That route's behaviour on current iOS is unverified.

## Options
| | A. Web-first (current) | B. Thin native iOS shell | C. Hybrid: web UI + native audio/ASR bridge | D. Server-side streaming ASR |
| --- | --- | --- | --- | --- |
| **Input routing** | System decides; can't choose on iOS | `setPreferredInput(builtInMic)` + A2DP output (documented; iOS 17 regressions reported) | Same as B | Same as A for capture on iOS web |
| **Recognition** | Safari Web Speech (Apple engine); reliability unverified | `SFSpeechRecognizer` (1-min server limit; on-device per locale) or `SpeechAnalyzer` (iOS 26; hu unconfirmed) | Same as B | A streaming provider (cost and credentials needed; not approved) |
| **Latency** | Unmeasured | Likely lower (no web layers); unmeasured | Like B, plus a bridge hop | Network + provider; unmeasured |
| **Lifecycle** | Foreground only (by design) | Foreground (can mirror web rule); background possible but not wanted | Like B | Like A |
| **Privacy** | Browser vendor processes audio | Apple processes (server) or on device | Like B | New processor (provider) and new data flow → counsel review |
| **Maintenance** | One codebase | + Swift app, App Store review, device matrix | + bridge contract between two runtimes | + server streaming infrastructure |
| **Effort (rough)** | Done | Spike 1–2 weeks; product several weeks | Spike 1–2 weeks; more integration risk | Weeks, plus paid provider |
| **Key constraint** | Can't fix the AirPods mic route on iOS | WKWebView has no Web Speech; ASR must be native | Two lifecycles to keep in sync | Credentials, cost, data flow |

## Decision rule (evidence-based; to apply after device tests)
1. **Stay web-first (A)**, adding desktop-Chrome mic selection via `start(audioTrack)` as a small improvement, **if all of the following hold**:
   - Test A/B shows acceptable capture and playback quality with AirPods connected (rubric ≥ 3 on intelligibility, playback not judged "call quality")
   - Test G/H shows foreground continuous recognition producing finals for at least 90 % of scripted phrases
   - Test H gives human-marked end-of-speech → audio p95 ≤ 4 s
2. **Run native spike S1 (B or C)** if A/B confirms degraded quality *or* G/H shows unreliable iOS recognition. Spike S1 scope (time-boxed, separate approval):
   - `AVAudioSession` `.playAndRecord` + `.allowBluetoothA2DP`, preferred input built-in mic; verify `currentRoute` on iOS 17/18/26 with AirPods Pro 2
   - `SFSpeechRecognizer` hu-HU: availability, on-device support, a 1-minute continuous strategy; check `SpeechAnalyzer` hu-HU support
   - The same Talktual server and policy semantics; no background listening
   - Measure the same validation schema (`talktual.ambient.validation.v1`) so results compare like for like
3. **Do not pursue D** without a cost and privacy decision by the owner and counsel.

## Consequences
- Until the device tests are run, the AirPods scenario is **unvalidated**. The product should describe Live translate as working best with the phone held toward speakers and headphones for listening, as the UI already does.
- The validation instrumentation (`src/ambient/validation.ts`) and test mode let the owner produce the evidence this decision needs, without any speech content.
