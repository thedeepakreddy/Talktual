# Phase 8.5 — Latency, backlog and reliability analysis

## 1. What can be measured, and where
| Stage | Start event | End event | Where measurable | Credibility |
| --- | --- | --- | --- | --- |
| Recognizer endpointing | speech actually ends | recognizer final | **Device only** | Human-marked (tester tap) or engine-reported (`speechend`), never measured |
| Commit | recognizer final / stable interim | unit committed | Here (simulated) and on device | Measured (app events) |
| Translation | request | response | Here (stubbed) and on device | Measured; provider latency only real on device |
| Queue wait | queued | playback start | Here and on device | Measured |
| Output route | playback start | sound in the ear | **Device only** | Not measurable by the app (Bluetooth buffer); stopwatch only |
| **End to end** | end of speech | audio start | **Device only** | Only as human-marked or engine-reported (test mode export) |

The test-mode export (`talktual.ambient.validation.v1`) reports three groups and never mixes them: **measured**, **engine-reported** and **human-marked**.

## 2. Simulated results so far (do not extrapolate)
- **Phase 8 E2E** (loopback, stubbed provider, fake voice): recognizer final → text p50 8 ms, p95 19 ms. This only shows that the app adds little overhead. **It says nothing about real latency**, which is dominated by cloud recognition endpointing (typically hundreds of ms to seconds), the provider round trip, TTS and Bluetooth.
- **Phase 8 evaluation:** 10 deterministic traces. All invariants hold: no out-of-order speech, no duplicated or lost words with headphones, speech lag ≤ the 9 s bound + 4 s queue margin. The overload case sheds 7 of 14 phrases to text only.

## 3. Behaviours verified by tests (simulated)
| Behaviour | Test |
| --- | --- |
| Stale / drop policy: late phrases shown, not spoken; queue `notAfter` | `ambient.test.ts` "late translations…", `ambient.eval.test.ts` overload |
| Ordering under out-of-order translation completion | `ambient.test.ts` "heard → translated → spoken, strictly in order" |
| Partial stability and corrections | `StablePrefixSegmenter` tests; evaluation "revision", "overlap-unstable" |
| Filler suppression (incl. Hungarian "öö") | evaluation "noise" |
| Echo / self-translation prevention | `ambient.test.ts` echo guard; half-duplex tests |
| Stop behaviour (pause/stop/dispose never re-enable the mic) | `ambient.test.ts` |
| Watchdog and recovery (bounded restarts → stalled → Resume) | `ambient.test.ts` watchdog |
| Provider failure | evaluation "translation-failures"; server tests (failure not cached, no body logging) |
| Provider timeout | server test (`translateTimeoutMs`) → client shows "Not translated" |
| Network transitions (offline → pause → resume, no duplicates) | `ambient.test.ts` policy suite; Phase 6 processing E2E |
| Cancellation (Speak off, mode switch aborts in-flight) | `ambient.test.ts` |

## 4. Risks only real devices can confirm
1. **iOS endpointing:** if Safari delivers finals late or not at all in continuous mode, early stable-prefix commits carry the experience. Thresholds (700 ms stable, 6 words) were tuned on synthetic timing and may need adjusting. The export's `first_partial_to_commit_ms` and `utterances_without_final` show this.
2. **Watchdog cost:** in a silent room, the recognizer restarts about every 25 s (bounded at 3 per minute). On iOS this may cause audible route flicker or battery cost. Check `asr_restarts_per_min` in test I.
3. **Half-duplex gaps (speaker mode):** while a translation plays, the room isn't heard. In fast conversation this loses words by design. This is the trade-off against a feedback loop. Test C measures it.
4. **Bluetooth output latency:** not visible to the app. Use a stopwatch in A/B.
