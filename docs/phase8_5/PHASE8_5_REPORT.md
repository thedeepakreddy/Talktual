# Phase 8.5 — Real-world ambient validation — completion report

## Verdict: **BLOCKED**
Phase 8 could **not** be validated in the physical world in this phase. No iPhone, AirPods, Android device, café or native-speaker reviewer was available to Claude, and none of the required tests can be simulated meaningfully (see `DEVICE_TEST_PROTOCOL.md`, "What cannot be simulated").

All software-side checks pass. Everything the owner needs to produce real evidence is now in place:
- a repeatable protocol and result template
- privacy-safe, opt-in test instrumentation with export
- a scoring rubric and review plan
- a battery protocol
- a decision rule for native iOS

The verdict should be re-assessed after the owner runs tests A, B, G, H, I and J.

What the evidence **does** establish:
- In real Chromium (desktop, phone emulation), against the real Talktual server, with a scripted recognizer and stubbed providers, the ambient pipeline behaves as specified: ordering, bounds, stop semantics, policy enforcement and no content in storage, logs or exports.
- Research shows that on iPhone the web app **cannot choose the microphone** for recognition. The flagship AirPods route therefore depends on what iOS does, which is likely the AirPods mic, with reduced quality (reported, not verified).

## 1. Branch, commits, working tree
- **Phase 8 merge:** at the owner's request, PR #5 merged Phase 8 into `main` as `41cdb8f` (first-parent `735f147`).
- **Branch:** `claude/kind-archimedes-n6y11p`, fast-forwarded to `41cdb8f` (no force push).
- **Phase 8.5 commits:** `77ac21f` (instrumentation + server fixes), `39959f2` (protocols, research, audit, disclosure fix), plus this report's commit. The working tree is clean after the final commit.
- **No PR for 8.5, no deploy, no credentials changes, no paid calls, no downloads, no native code.** Phase 9 not started.

## 2. Baseline audit and capability matrix
Full detail is in `BASELINE_AUDIT.md`. Key points:
- **Traced end to end, every stage gated or bounded:** capture → recognition → segmentation → translation → ordering → queue → playback, plus teardown.
- **Real vs mocked:**
  - recognizer: mocked (unit) or scripted (E2E)
  - microphone and Bluetooth: never exercised
  - providers: stubbed
  - speech output: faked
  - server, policy and UI: real
- **Claims re-checked:**
  - two server-side gaps against "never saved / never logged" were found and fixed (§4)
  - one Phase 8 research claim was corrected: desktop Chrome *can* recognize from a chosen audio track; iPhone and Android cannot

## 3. Instrumentation (new)
- **`src/ambient/validation.ts`** (`ValidationRecorder`):
  - **Opt-in:** `?validate=1` or `?debug=1`.
  - **Clock and bounds:** monotonic, bounded (3,000 events, 500 units), clearable; nothing persisted.
  - **Content-free by construction:** the API takes only ids, enums and numbers.
  - **Events:** recognizer start, stop, restart, error, speech start/end, first interim, final, commit, translation request/response, queue admit/drop, playback start/end, interruptions, route changes (device-kind counts only), cancellations, teardown, and safety mechanisms (echo drop, half-duplex holds).
  - **Summary:** p50/p90/p95/max, in three separate groups:
    - **measured** (app events on one clock)
    - **engine-reported** (the recognizer's speech-end as the start)
    - **human-marked** (the tester taps *Mark end of speech*)

    End-to-end speech→audio is reported only from the last two. Rates include missing finals, finals without interims, restarts per minute, drops, failures and the maximum queue depth.
- **Test mode strip** on Live translate (only when opted in): *Mark end of speech* · *Export* (JSON download, clipboard fallback) · *Clear*.
- **Device changes:** `devicechange` is recorded as counts, and a brief notice is shown while listening.
- **Tests:**
  - unit tests: percentiles, groups, rates, bounds, opt-in, session integration with a scan for content leakage
  - `e2e/validation.mjs`: absent by default; export contains timings and no phrase words; download works; nothing persisted

## 4. Safe fixes (each with regression tests)
| # | Bug | Fix | Test |
| --- | --- | --- | --- |
| F1 | Server translation cache kept source text indefinitely, shared across users, and advertised hits (a cross-user side-channel showing whether a phrase was recently translated) | TTL (default 2 min, `TRANSLATION_CACHE_TTL_MS`); `TRANSLATION_CACHE_MAX=0` disables; hits reported as the original provider | server: "expires after the TTL; size 0 disables"; updated cache-hit test |
| F2 | Provider and ElevenLabs error bodies logged (up to 300 characters; could echo user text) | log provider and status only (`errKind`) | server: "provider error bodies are never logged" |
| F3 | Ambient disclosure didn't name the speech-output processor | Names ElevenLabs (server TTS) or the device/browser voice per mode; ambient consent version → 2 (re-asks) | unit: disclosure names processor; consent key v2 |
| F4 | No visibility of audio device changes while listening | `devicechange` recorded (counts); 8 s info notice | unit: route change notice + count |

Not changed (recorded as findings): the call disclosure's TTS processor omission (P4, outside Phase 8 scope), and bystander notice (P5, a counsel question).

## 5. Real-device test matrix
`DEVICE_TEST_PROTOCOL.md` (A–J) plus `DEVICE_RESULTS_TEMPLATE.md`. Every test has preconditions, steps, expected results, pass/fail criteria and evidence (export JSON plus notes).

**Status: all A–J not executed (BLOCKED — need physical devices and people).**

## 6. Audio routing research and native feasibility
`AUDIO_ROUTING_RESEARCH.md` grades every source (A: vendor or standards docs; B: vendor bug tracker; C: forum or third party, unverified) and gives access dates.

`NATIVE_FEASIBILITY_DECISION.md` (DR-8.5-1):
- **Status:** **Proposed** — stay web-first if device tests A/B/G/H pass the stated thresholds.
- **Otherwise:** a time-boxed native spike S1 (`AVAudioSession` built-in mic + A2DP AirPods; `SFSpeechRecognizer` / `SpeechAnalyzer` hu-HU availability), under separate approval.
- Server-side streaming ASR is not pursued without a cost and privacy decision.
- No native code was written.

## 7. Acoustic and translation evaluation
`ACOUSTIC_EVALUATION.md` contains:
- the dimensions and a conditions matrix (quiet, distance, reverberation, played-back and real café noise, overlap, rapid turns, fillers and corrections, fragments, ambiguity)
- authored phrase sets and a 1–5 rubric (intelligibility, adequacy, chunking), with omission, addition, repetition, clipping and late tallies
- a blind, calibrated native-speaker review plan
- an aggregation rule: report n; don't average across conditions

**No scores are claimed.**

## 8. Latency, backlog and reliability
`LATENCY_BACKLOG_ANALYSIS.md` covers:
- **What can be measured where.** End to end is only on device, human-marked or engine-reported.
- **The simulated 8 ms result:** app overhead only, **not to be extrapolated**.
- **Simulated invariants (evaluation, 10 scenarios):** no out-of-order speech, no duplicated or lost words, bounded lag, overload sheds to text.
- **Behaviours verified in simulation:** watchdog, recovery, provider failure and timeout, offline, cancellation.
- **Risks only devices can confirm:** iOS endpointing, watchdog cost in silence, half-duplex gaps, Bluetooth output delay.

## 9. Battery, thermal, resources
`BATTERY_PROTOCOL.md`: a 10-minute baseline plus a 20–30 minute session, 5-minute checkpoints, the data template, and allowed versus disallowed claims. **Not executed.**

## 10. Privacy audit and counsel brief
- **`PRIVACY_AUDIT.md`:**
  - activation and indicators
  - every store with its lifetime and clearability (client storage, memory, both caches, logs, diagnostics, validation, audio: none stored)
  - third-party recipients per mode, findings P1–P6
  - the "no retention" re-tests
- **`COUNSEL_BRIEF.md`** (not legal advice):
  - facts and data flows
  - references for counsel to assess: GDPR household exemption and Ryneš C-212/13; roles; lawful basis; transparency; minimisation; transfers; EDPB VVA guidelines 02/2021; Hungarian Civil Code §2:48 (voice recordings), Infotv. and Criminal Code §422; ePrivacy; AI Act classification
  - 10 questions and the unresolved facts

## 11. Tests and regressions
| Suite | Result |
| --- | --- |
| `npm run test:all` (lint, 515 unit, 60 server, build) | pass |
| smoke 17 · recovery 8 · compat 5 · conversation 8 · intelligence 7 · processing 7 · VoiceMirror 8 · a11y 11 · ambient 13 · validation 2 | pass |
| ambient evaluation (10 scenarios) | pass |
| reliability loop (`REPRO_ASSERT=1`) 10/10 | pass |

All of these are simulated or emulated. None is real-device evidence.

## 12. Limitations
- Nothing physical was tested. All conclusions about iPhone, AirPods, cafés, latency, accuracy and battery are research-based or pending.
- Some research inputs are grade C (forums) and are marked unverified. Apple has no published hu-HU coverage for `SpeechAnalyzer` or the Translation framework; these must be checked at runtime.
- Residual timing side-channel within the 2-minute server cache TTL (disable via `TRANSLATION_CACHE_MAX=0` for zero retention).

## 13. Rollback
| Change | How to undo |
| --- | --- |
| Instrumentation | Inert unless opted in; revert `77ac21f` to remove |
| Server cache | `TRANSLATION_CACHE_TTL_MS` (very large = old behaviour without hit disclosure) or revert |
| Log sanitizing | Revert (not recommended) |
| Disclosure v2 | Revert `39959f2`'s `disclosure.ts` change (users would not be re-asked) |
| Phase 8 itself | `git revert -m 1 41cdb8f` |

## 14. Next steps (prioritised by evidence)
1. **Owner runs device tests A and B** (iPhone + AirPods), with `?validate=1`, using the template. This decides DR-8.5-1.
2. **Tests G and H** (iOS foreground reliability and real latency, human-marked) on iPhone Safari and an Android Chrome device.
3. **Counsel review** of `COUNSEL_BRIEF.md` (bystander notice, roles, Hungarian §2:48) before any public launch of Live translate.
4. **Test I** (battery) and **J** (native-speaker review).
5. Depending on 1–2: native spike S1 (separate approval) or desktop-Chrome mic selection via `start(audioTrack)`.
6. The owner decides P4 (name the TTS processor in the call disclosure) and whether to disable the server cache for zero retention.
