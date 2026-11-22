# Phase 4 Privacy Review

Scope: everything the conversation layer adds (`src/conversation/`, the session integration, wire messages, UI markers, evaluation tooling).

## 1. Data handled
| Data | Where | Lifetime | Leaves the device? |
| --- | --- | --- | --- |
| The one held (unfinished) phrase | `Segmenter` memory | Until commit/discard; cleared on mic off, mode/language change, peer away, leave/dispose | Only as a provisional translation to a `conv1` peer (and the translation provider). Withdrawn with `retract` if discarded |
| Learned pause timing (EMA of gaps, ms) | `AdaptiveHold` | Session; reset on dispose | No |
| Recognizer confidence | used once per final to decide drop/flag | Not stored. Only an `uncertain` boolean remains on the segment | The boolean goes to a `conv1` peer |
| Turn ids, interrupted turns, segment→turn map | `ListenerPolicy` | Session; bounded (64); cleared on teardown | Turn ids are random UUIDs (no identity) |
| Overlap-held playback items (translated text) | `ListenerPolicy` | Until played/dropped (≤ 12 s, ≤ 12 items); cleared on teardown | No |
| Translation latency estimate | `LatencyMonitor` | Session | No |

Nothing is written to storage. No new server endpoints, server code or logs.

## 2. Spec checks
| Requirement | Status | Evidence |
| --- | --- | --- |
| No raw audio / transcript logging for evaluation | ✅ | The simulator uses scripted synthetic phrases only. Result JSON holds the synthetic scenario phrases, which are test fixtures and not user data. No recordings |
| Diagnostics exclude sensitive content | ✅ | New records are `conv_unit{segmentId, state, reason, ms, rev, repaired, uncertain}`, `conv_suppressed{segmentId, reason}`, `conv_degraded{status}`, plus counters. They pass the Phase 2 allowlist redactor. Tests: session test "diagnostics carry ids, enums and timings — no transcript words", and the simulator leak check (0 word pairs across 13 scenarios) |
| Confidence values not logged | ✅ | Only booleans are recorded |
| Context session-scoped and cleared | ✅ | `teardown()` clears segmenter (incl. learned timing), listener policy, outbound queue, latency, provisional state. Test "leave clears every piece of conversation context; nothing is sent afterwards" |
| No emotion / identity / relationship / sensitive-attribute inference | ✅ | Only lexical completeness, repetitions and recognizer confidence are used. No speaker modelling beyond pause timing, and none of it leaves the device |
| VoiceMirror off; no cloning or identity expansion | ✅ | Phase 4 does not import or change VoiceMirror. It works with `voiceTap` absent (test + E2E) |
| No policy changes | ✅ | `ALLOWED_ORIGINS`, `RATE_LIMIT_PER_MINUTE`, `TRUST_PROXY` untouched; server unchanged |

## 3. New exposure to review
1. **Provisional translations send partial thoughts to the translation provider earlier, and once more.** The provider already receives the final text, and the provisional is a prefix of the same utterance, so no new category of data leaves the device. But the provider and the peer see an unfinished phrase that the speaker might abandon. Mitigations: provisional output goes only to `conv1` peers, is paced (≥ 700 ms, one in flight), stops when degraded, and is retracted on discard. **Owner decision:** keep it on, or set `provisionalMinIntervalMs` very high or disable it by config if the processing disclosure should not cover "text may be translated before you finish". The current disclosure says speech is translated by the named provider and does not mention timing.
2. **Extra calls against the server rate limit** (`RATE_LIMIT_PER_MINUTE`, owner-set). At most one provisional per held fragment.
3. **Interrupted content is still shown** in the listener's transcript, labelled *Interrupted — not played*. It is not deleted. This keeps the record honest, but it means words spoken after the listener cut in remain readable. That matches Phase 2 transcript semantics: the speaker chose to send them.

## 4. Verdict
No new storage, logging or identity processing. One disclosure-scope question (item 1) for the owner. Nothing blocks development acceptance.
