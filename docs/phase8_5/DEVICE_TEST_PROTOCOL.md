# Phase 8.5 — Owner-run real-device test protocol (A–J)

**Status: not executed.** No physical device, AirPods, café or native speaker was available to Claude in this phase. Every test below **requires physical hardware and people**, and none of it can be simulated meaningfully. Record results in `DEVICE_RESULTS_TEMPLATE.md`, one copy per device and configuration.

## 0. Common setup (all tests)
- **Build:** current `main`, served over **https**. Web Speech and the mic need a secure context.
- **Open** `https://<host>/?validate=1`. This shows the **Test mode** strip on Live translate: *Mark end of speech* · *Export* · *Clear*. Test mode records **no speech content**: only timings, enums and counts (`talktual.ambient.validation.v1`).
- **Record the device context in the template:**
  - device model, iOS/Android version, browser and version
  - AirPods model and firmware (Settings › Bluetooth › ⓘ)
  - network (Wi-Fi or cellular)
  - room type and approximate noise level (a phone sound-meter app is acceptable; say which)
- **Speakers:** only consenting people. Use the scripted phrases in `ACOUSTIC_EVALUATION.md`. **Do not record bystanders.** Talktual doesn't record; the tester must not run a separate recorder unless every speaker has consented.
- **Each trial:**
  1. Press *Clear* in test mode, then Start.
  2. Run the steps.
  3. Tap *Mark end of speech* the moment each speaker finishes a phrase.
  4. Press *Export* (JSON download) and attach the file to the results.
- **Pass/fail:** each test lists its criteria. Anything else counts as **Fail**, with a note.

## A. iPhone mic + AirPods playback (target route)
- **Preconditions:** AirPods connected; iPhone Safari; headphones switch **on** in Live translate settings.
- **Steps:**
  1. Open Control Centre › audio output: confirm AirPods.
  2. Start Live translate, then check Control Centre's mic indicator and Settings › Accessibility › Audio and Visual (if available) for the *input* in use.
  3. A speaker at 1 m reads phrases P1–P10 (Hungarian). The phone faces the speaker.
- **Expected:** text and English speech in the AirPods. Note which mic iOS actually used.
- **Pass:** at least 8/10 phrases produce a translated line; spoken audio is intelligible (rubric ≥ 3); input route documented.
- **Evidence:** export JSON; route observation (text only); rubric sheet.
- *A web page cannot force the phone mic. If iOS uses the AirPods mic, record "A not achievable on web" and continue with B. This is the key input to DR-8.5-1.*

## B. AirPods mic + AirPods output
- **Preconditions:** same as A; AirPods in both ears.
- **Steps:** same as A, plus a playback-quality comparison. Play the same music track before Start, during listening, and after Pause.
- **Expected:** possible switch to call-quality audio while listening.
- **Pass:** recognition at least 8/10. Record playback quality during and after listening (normal / degraded / recovers after Pause / recovers only after Back).
- **Evidence:** export JSON; quality notes.

## C. iPhone mic + iPhone speaker (no headphones), controlled room
- **Preconditions:** AirPods disconnected; headphones switch **off**.
- **Steps:**
  1. Read P1–P10 at 1 m, in a quiet room.
  2. Then read P11 (a long run-on) and P12 (two people alternating quickly).
- **Expected:** capture pauses while the translation plays (status "Listening resumes after the translation"). No translation of Talktual's own speech.
- **Pass:** `echo_dropped` stays low and no English self-translation appears in the feed; for every phrase, capture resumes after speech; no phrase is played twice.
- **Evidence:** export JSON (`reliability.half_duplex_holds`, `reliability.echo_dropped`); notes.

## D. Wired or USB-C headset (if available)
- **Preconditions:** headset connected; headphones switch **on**.
- **Steps:** same as A.
- **Pass:** as A. Record which mic is used (headset or phone).

## E. Route connect and disconnect while listening
- **Steps:** while listening:
  1. Connect AirPods.
  2. Remove one AirPod.
  3. Put the AirPods back in the case.
  4. Reconnect.
  5. Toggle Bluetooth off and on.
- **Expected:** a short notice "An audio device was connected or removed…" (Chrome/Android: `devicechange`; iOS: may not fire). Listening either continues or pauses with a clear message; it never silently stops while still showing "Listening".
- **Pass:** for every transition, the UI state matches reality (if the header says Listening, the recognizer is still producing text when spoken to).
- **Evidence:** export JSON (`reliability.route_changes`, `asr_restarts`, `interruptions`); a step log.

## F. Permission denial, revocation, background, foreground, lock
- **Steps:**
  1. First run: deny the mic → expect "Microphone access is blocked…".
  2. Allow it via settings → Start.
  3. While listening: switch app, then return → expect **Paused**, with no auto-resume.
  4. Lock the screen, then unlock → expect Paused.
  5. Revoke the mic in Safari site settings while listening → expect an error or Paused state.
- **Pass:** in no case is the listening indicator shown while capture is impossible; nothing resumes without a tap.
- **Evidence:** export JSON (`interruptions.hidden`, `asr_errors`); a step log.

## G. Pause, resume, stop, navigation, offline, cancellation during playback
- **Steps:**
  1. While a translation is playing: Pause (expect capture off immediately; the already queued speech may finish).
  2. Speak translations off (expect silence immediately).
  3. Back (expect everything stops).
  4. Airplane mode while listening (expect "You're offline…", then automatic resume when the network returns).
- **Pass:** each action takes effect in under 1 s (stopwatch); no stale phrase is spoken after Speak off, Back or offline; no duplicate after reconnect.
- **Evidence:** export JSON (`cancels`, `queue_drops`, `playback_dropped`); a step log.

## H. Speech conditions
- **Steps:** a scripted set from `ACOUSTIC_EVALUATION.md`:
  - silence (60 s)
  - short (1–3 words)
  - long (> 20 words)
  - conversational pace
  - rapid speech
  - self-corrections
  - two people overlapping
  - café noise played at about 65–70 dBA
- **Measure:** per phrase, tap *Mark end of speech*.
- **Pass (proposed):**
  - human-marked end → audio **p95 ≤ 4 s** and end → text **p95 ≤ 3 s** on Wi-Fi
  - no duplicated or out-of-order speech
  - silence produces no lines
- **Evidence:** export JSON (`humanMarked.*`, `engineReported.*`, `measured.*`); rubric sheets.

## I. 20–30 minute resource session
Follow `BATTERY_PROTOCOL.md`.

## J. Hungarian → English with native-speaker review
Follow `ACOUSTIC_EVALUATION.md` §5. A native Hungarian speaker who is fluent in English scores each line (adequacy, omissions, additions) from the on-screen text. The tester copies the text by hand into the review sheet **with the speakers' consent**. Talktual itself never saves it.

## What cannot be simulated (and why)
| Test | Needs |
| --- | --- |
| A, B, D, E | Real Bluetooth stack and iOS routing; WebKit behaviour on device |
| C | Real speaker-to-mic acoustic coupling (echo) |
| F, G | iOS lifecycle (lock, background, permission UI) |
| H, J | Real voices, rooms, noise, the real recognizer and translator |
| I | Real battery and thermal behaviour |
