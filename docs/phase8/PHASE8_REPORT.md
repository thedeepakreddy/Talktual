# Phase 8 — Ambient translation ("Live translate") — completion report

Branch `claude/kind-archimedes-n6y11p`, fast-forwarded to `main` at `735f147` (PR #4 merge), then Phase 8 commits on top. No PR, merge or deploy, no paid calls, no model downloads, and Phase 9 not started.

## 1. Summary
Talktual now has a **Live translate** mode. You press Start; it listens to the people around you in the language you choose (default Hungarian); and it shows, and optionally speaks, the translation in your language. Nobody else needs the app.

It is built on the existing engine rather than beside it:
- recognition uses the same Web Speech adapter, in continuous mode
- translation goes through the same Phase 6 policy and routing, now one shared function
- speech goes through the same bounded playback queue
- the UI uses the same Phase 7 design system

**Verification:**
- **Simulated (done):** the end-to-end path, capture → recognition → translation → speech, is verified in desktop Chromium with a **scripted** recognizer, fake speech output and stubbed providers through the real server.
- **Real devices (not done):** no iPhone, AirPods, Android or café validation was performed. The precise plan is in `DEVICE_TEST_PLAN.md`.
- **Known platform blocker:** a web page cannot make iOS record from the phone's mic while playing to AirPods. Details are in §4.

## 2. Discovery and decisions (details and sources: `PHASE8_DISCOVERY.md`)
| Question | Finding | Status | Decision |
| --- | --- | --- | --- |
| Capture source | AirPods mic forces Bluetooth hands-free profile (mono, narrowband) for input **and** output | Reported | Recommend the phone's built-in mic. The web can't enforce it, so explain it in the UI. A native shell is the real fix |
| Pick the input on the web? | Web Speech takes no MediaStream; `navigator.audioSession` sets type only, not route | Documented / verified in code | Set `play-and-record` while listening and restore it after. Disclose the limit |
| iOS continuous recognition | Bug reports: hangs, no finals, flaky interims | Reported | Watchdog with bounded restarts, then a visible "stopped responding" state with Resume. Works with finals only |
| Language ID | None in Web Speech | Verified | User chooses "They speak". A text-level check *suggests* a switch and never acts by itself |
| Speaker separation | No browser API | Verified | Not implemented and not claimed. Lines are in the order heard |
| Hungarian on-device | No on-device hu recognizer or translator on the test hardware (Chrome Translator API probed if present) | Verified | On-device mode: listening **unavailable**, explained; never silently cloud |
| Firefox | No Web Speech; server STT is batch-only and paid | Verified | "This browser can't recognise speech", with no dead controls |

## 3. What was built
### Pipeline (`src/ambient/`)
- **`stablePrefix.ts`, the stable-prefix segmenter:**
  - It commits only words that every interim agreed on for 700 ms (at least 3 updates), in runs of at least 6 words, ending at a clause boundary where possible, or after 5 s of a run-on.
  - At the final, only the remainder after what was committed is used, so no word appears twice.
  - A recognizer rewrite of committed words is flagged as "Corrected by the recognizer", never silently changed.
  - Fillers, including new Hungarian "öö", and duplicates re-emitted after restarts are dropped. Per-utterance state is bounded.
- **`AmbientSession.ts`, the lifecycle and orchestration:**
  - **Lifecycle:** `idle → (consent) → starting → listening ⇄ paused → stopped`, plus `stalled` and `unavailable`, each with a way out.
  - **Explicit activation:** separate ambient consent (capturing *other people* differs from the call disclosure). Nothing is created before Allow.
  - **Order and bounds:** speech goes strictly in commit order. A failed or late phrase is shown but skipped, never played out of order or twice. Bounds: 2 translations in flight, 8 waiting (oldest skipped), 200 lines, 9 s speech-lag limit, 3 restarts per minute.
  - **No feedback loop without headphones:** capture is suspended while translated speech plays. Speech waits for the room to pause, so it never cuts a speaker off. An echo guard drops recognized text that matches what was just spoken.
  - **Stops:** Pause, leaving the screen, going offline and mic loss all abort the recognizer immediately and release the mic. Hidden stays paused; offline resumes on reconnect, because the user started it.
  - **Phase 6 end-to-end:**
    - recognition is planned and gated (`beginRemote`); a revoked gate pauses capture
    - translation uses the shared `privacy/routing.ts`
    - speech is routed per item at start
    - mode and offline changes abort cloud work in flight
  - **Diagnostics:** stage latencies and counters through the allowlist-redacted `Diagnostics`. A test asserts no text leaks into them.
- **`langGuess.ts`:** function-word scoring for hu/en/es/de/fr/it/pt. It suggests a switch only after a run of at least 12 words that clearly look like another language.
- **`disclosure.ts`:** per-mode text covering what is captured, who receives it, what is kept (nothing) and the bystander guidance.

### Engine changes, all backward compatible
- `privacy/routing.ts`: routed translation extracted from `CommunicationSession`, which now delegates to it. One implementation of the policy rules.
- `PlaybackItem.volume` (optional), applied to the server PCM gain, the encoded `<audio>` element and the browser voice. It is unset for calls, so call behaviour is unchanged.
- Filler lexicon: Hungarian hesitation sounds.
- hu↔en added to the browser Translator API probe list. It is used only if the browser reports it available.

### UI (Phase 7 system: `src/components/ambient/AmbientScreen.tsx`)
- **Home:** a quiet secondary entry, "Translate what's around you". Home keeps one primary action.
- **Setup:** They speak / You read and hear, Speak translations, "I'm listening through headphones" (explains half-duplex), and the microphone note.
- **Listening state, impossible to miss:**
  - a pulsing dot in the header and the "Listening" label
  - the tab title "● Listening — Talktual"
  - the amber Pause control and the status line
- **Feed** (`role=log`): translation first, original underneath, then quiet markers (Unclear audio, Corrected, Translated on this device, Shown not spoken). Live interim in italics. One "Play again" on the latest line.
- **Dock:** Speak on/off (text-only), Start/Pause/Resume, Clear.
- **One settings sheet:**
  - languages
  - speak and volume (Quiet/Medium/Full)
  - headphones
  - processing mode, with where each step runs
  - microphone facts (AirPods, no speaker labels)
  - disclosure and Clear
- **States:** consent, empty, listening, held for speech, paused (user, hidden, offline, mic, policy), stalled, unavailable (no recognition, On-device, permission), language hint. Each has one banner with the action that actually helps (Resume or Settings), and no dead buttons.

## 4. Verification: real versus simulated
| Evidence | Kind | Result |
| --- | --- | --- |
| Unit tests: segmenter, language check, session lifecycle, ordering, half-duplex, echo, text-only, late and backlog bounds, pause/stop/dispose never re-enable the mic, watchdog and stalled, permission, mic loss, audio session, line bound, policy (On-device unavailable, mode switch aborts cloud work, offline/online), no text in diagnostics or storage | Unit, fakes | 34 pass |
| Evaluation matrix: 10 recognizer-trace scenarios (`ambient.eval.test.ts` → `results/ambient-eval.json`) | Simulated traces | All invariants hold (table below) |
| Browser E2E (`e2e/ambient.mjs`): 13 steps in real Chromium against the real server | Simulated recognizer and voice, stubbed provider | 13/13 |
| Accessibility (`e2e/a11y.mjs`): Live translate targets ≥ 44 px; keyboard Start, consent focus, Pause, settings trap and return | Real browser | 11/11 overall |
| Screenshots: 10 states × phone/tablet/desktop (dark) + phone/desktop (light), inspected and iterated (`screens/`) | Real renders | Done |
| Regression: lint, 507 unit, 58 server, build; smoke 17, recovery 8, compat 5, conversation 8, intelligence 7, processing 7, VoiceMirror 8, a11y 11, ambient 13; reliability loop 10/10 | Mixed (as before) | All pass |
| iPhone, AirPods, Android, café, real latency, battery | **Real device** | **Not performed** |

### Evaluation matrix (simulated; deterministic recognizer traces)
| Scenario | Units | Early | Revised | Spoken | Out of order | Duplicate words | Missing words |
| --- | --- | --- | --- | --- | --- | --- | --- |
| clean | 2 | 0 | 0 | 2 | 0 | 0 | 0 |
| run-on (22 words, no pause) | 4 | 3 | 0 | 4 | 0 | 0 | 0 |
| revision of committed words | 2 | 1 | 1 | 2 | 0 | 0 | — |
| late final (+1.8 s) | 2 | 0 | 0 | 2 | 0 | 0 | 0 |
| missing final (promoted interim) | 2 | 0 | 0 | 2 | 0 | 0 | 0 |
| overlap (unstable hypothesis) | 1 | 0 | 0 | 1 | 0 | 0 | — |
| noise (fillers + re-emitted duplicate) | 1 | 0 | 0 | 1 | 0 | 0 | 0 |
| every 2nd translation fails | 6 | 0 | 0 | 3 | 0 | 0 | 0 |
| overload (4 s translation, a phrase every 1 s) | 14 | 0 | 0 | 6 (7 shown, not spoken) | 0 | 0 | 0 |
| no headphones (half-duplex) | 2 | 0 | 0 | 2 | 0 | 0 | 0 |

Metric notes:
- "Commit lag" in the JSON is measured from the **first word heard**, so it includes the time spent speaking.
- In the run-on case, early commits put the first text on screen about 2.5–3 s into a 6 s utterance, instead of after it.

### Latency
- **Simulated (`results/ambient-e2e.json`):** recognizer final → translated text p50 8 ms, p95 19 ms; final → speech start p50 8 ms, p95 20 ms, over 8 phrases. This covers only the app and loopback server.
- **Excluded:** mic capture, real recognizer delay (cloud ASR, typically hundreds of ms to seconds), real provider latency and Bluetooth output. **Real latency is unmeasured.**

## 5. Privacy and safety review
- **Deliberate activation:** listening starts only from Start after consent. It is always visible (header, tab title, control, status) and stops immediately.
- **Hidden tab:** the app pauses capture when the tab is hidden. It never listens in the background.
- **No retention:**
  - no audio is recorded or kept by Talktual
  - lines live in memory only, are bounded, and are cleared by Clear and on exit
  - storage holds only the consent flag and the mode preference
  - both points are asserted in unit and E2E tests
- **Logging:** diagnostics are allowlist-redacted (enums and numbers only), and tests assert no phrase text reaches them. No new console logging of content.
- **Processing modes enforced, not displayed:**
  - On-device never reaches the cloud (wire-level request audit in E2E)
  - switching modes aborts cloud work in flight
  - offline pauses
- **Third-party privacy and legal:** ambient capture involves people who didn't consent. Laws vary: recording, data-protection and eavesdropping rules differ by country. Talktual doesn't record, but recognition sends room audio to the browser vendor's cloud recognizer in Cloud and Hybrid modes. **No legal claim is made.** The disclosure tells users this and asks them to use it where listening is appropriate. Recommended next step: a legal review for the target markets (EU/GDPR and Hungary first).
- **Unresolved:** browser vendors' own retention of recognition audio is governed by their policies, not by Talktual.

## 6. Limitations
- iOS web cannot select the built-in mic while outputting to AirPods (§2). The flagship AirPods scenario may run on the AirPods mic at reduced quality. A native shell is needed for the optimal route.
- Recognition quality, noise robustness and distance behaviour depend on the browser's recognizer and are untested on real audio.
- No speaker separation or diarization; no automatic language ID.
- Firefox is unsupported for Live translate. On-device mode can't listen to Hungarian on the test hardware.
- Ducking other audio and true volume balancing against the room are not available to a web page. Volume steps, text-only mode and "Shown, not spoken" for late phrases are the mitigations.
- Pause stops listening; translations already queued may still be spoken. Speak off silences everything at once.

## 7. Files and commits
**New files:**
- `src/ambient/` — `AmbientSession.ts`, `stablePrefix.ts`, `langGuess.ts`, `disclosure.ts`, `useAmbientSession.ts`, `ambient.test.ts`, `ambient.eval.test.ts`
- `src/components/ambient/AmbientScreen.tsx`
- `src/privacy/routing.ts`
- `e2e/ambient.mjs`, `e2e/ambient-screens.mjs`
- `docs/phase8/*`

**Changed files:**
- `CommunicationSession.ts` (delegates routing)
- `ports.ts` and `textToSpeech.ts` (volume)
- `lexicon.ts` (fillers)
- `defaultDeps.ts` (hu↔en probe)
- `App.tsx` and `Home.tsx` (entry)
- `a11y.mjs` (ambient checks and a race fix)
- `package.json` (`test:e2e` includes ambient)

**Commits:** `b34f356` (pipeline), `1259df3` (UI and E2E), the UI iteration and accessibility commit, and this report's commit. Run `git log 735f147..`.

## 8. Rollback
- Live translate is additive: remove the Home entry (`onAmbient`) to hide it, or revert the Phase 8 commits.
- Shared changes are behaviour-preserving for calls (routing extraction, optional volume, filler lexicon), and the full call regression is green. Revert them individually if needed.
- No data migrations.

## 9. Remaining device tests and next conditions
1. Run `DEVICE_TEST_PLAN.md` configurations D1–D6. Above all, D1 (iPhone + AirPods): actual input route, recognition reliability in continuous mode, real latency.
2. Legal and privacy review of ambient capture for the target markets.
3. If D1 confirms the routing limit: a native iOS shell (`AVAudioSession` built-in mic → A2DP output, `SFSpeechRecognizer`). This is a separate project.
4. Tune the stable-prefix thresholds on real recognizer timing (current values are from simulated traces).

**Engineering completion:** done for the web platform scope, with all automated checks green.
**Production readiness for the café scenario:** not established, pending items 1–2.
