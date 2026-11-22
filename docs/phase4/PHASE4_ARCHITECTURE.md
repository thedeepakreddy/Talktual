# Phase 4 Architecture — Conversation Layer

The conversation layer sits between the recognizer and the Phase 2 send/receive pipeline. It does not replace the pipeline. It is pure TypeScript in `src/conversation/`, wired in through `CommunicationSession`, and turned on or off by one dependency (`SessionDeps.conversation`). It does not depend on VoiceMirror.

```
 LOCAL SPEAKER                                                          LISTENER
 recognizer ─partial/final(+confidence)─▶ Segmenter ──commit──▶ OutboundQueue ─▶ translate ─▶ segment{state:final}
             PTT release / ASR end ──cue─▶   │ hold ──▶ provisional translate ─▶ segment{state:provisional}
                                             │ discard ─▶ retract
 press/first speech ─▶ floor{open} … release/idle ─▶ floor{closed}
                                                                     ──▶ ListenerPolicy
                                                                         interrupted turn? → transcript only
                                                                         local user talking? → overlap hold
                                                                         else → playback (notAfter age bound)
 press while peer floor open / playing ─▶ interrupt{turn} ─────────────▶ speaker marks own turn interrupted
```

## 1. Modules (`src/conversation/`)
| File | Responsibility | State kept (all bounded, content-free except the one held unit) |
| --- | --- | --- |
| `types.ts` | Unit states and legal transitions, commit reasons, `ConversationConfig` + defaults, capability string `conv1` | — |
| `lexicon.ts` | Per-language (en/es/fr/de/pt/it) trailing-word lists, repeatable function words, correction markers; universal hesitation-sound regex | static |
| `repair.ts` | `looksIncomplete`, `isFillerOnly`, `collapseRepetitions`, `resolveRestart`. Pure functions | — |
| `pauseModel.ts` | `AdaptiveHold`: EMA of this speaker's continuation gaps → hold window clamped to [500, 2000] ms | 2 numbers |
| `segmenter.ts` | Turn/pause detection and segmentation for the local speaker; clock injected | the single open unit (text not yet sent as final) + 1 timer |
| `listener.ts` | `ListenerPolicy`: peer floor, interrupted turns (bounded set of 64), active segment→turn map (≤64), overlap-held items (≤12) | ids, timestamps, held playback items |
| `outbound.ts` | `OutboundQueue` (one in flight, same-turn merge behind it, ≤400 chars), `LatencyMonitor` (smoothed latency → degraded, with hysteresis) | waiting units, 1 number |

## 2. Unit state model
`speech-active → pause-candidate ⇄ provisional → turn-candidate → final → (interrupted)`, plus `superseded`, which is reachable from every non-final state. `final` is reachable only through `turn-candidate`. Nothing leaves `final` except `interrupted`, so **committed text is never revised**. `uncertain` is a flag, not a state. Illegal transitions are counted (`illegal_transitions`) and are never silently applied. A test asserts that the count stays 0.

## 3. Turn and pause policy (M4.1, M4.2)
| Situation | Decision | Bound |
| --- | --- | --- |
| Final looks complete (no trailing article/preposition/conjunction/possessive/filler; or terminal punctuation) | commit at once (no added delay) | 0 ms |
| Final looks unfinished ("…book a", "send it to") | hold as pause candidate | adaptive 500–2000 ms (default 900) |
| Partials arrive during the hold (still speaking) | extend the hold | never beyond `maxOpenMs` = 6 s after the unit opened |
| Next final arrives | merge (continuation) or replace (restart/correction, same id, rev+1) | unit ≤ 400 chars |
| Push-to-talk release | end-of-turn cue: commit on recognizer end, or after `releaseFlushMs` | 1200 ms |
| Recognizer ends | commit | 0 ms |
| Capture aborted (mic off, mode/language change, peer away, leave) | discard; send `retract` if a provisional went out | — |
| Hesitation sound only / confidence < 0.35 with ≤ 2 words | drop (counted) | — |
| Confidence < 0.5 | flag `uncertain` (shown in UI) | — |

**Silence alone never ends a thought early.** A complete-looking final ends the unit. A pause ends an unfinished-looking unit only after the adaptive hold. The "turn" (floor) is a separate notion: it opens on press (or on first speech in hands-free mode) and closes on release (or after 1.5 s of silence in hands-free mode).

## 4. Provisional / final protocol (M4.3)
Capabilities are negotiated in `hello.caps: ['conv1']`. A peer without it (Phase 2/3) **never** receives `floor`, `interrupt`, `retract`, provisional segments, or Phase 4 fields on segments. Old peers would ignore the message types, but they would play provisional segments, which is why capability negotiation is required.

| Message | Meaning |
| --- | --- |
| `segment{state:'provisional', id, rev, turn}` | Translated held text; shown, never spoken. At most one in flight, ≥ 700 ms apart. Not sent when degraded, and never after the unit's final (guarded by id+rev) |
| `segment{state:'final', id, rev, turn, uncertain?, repaired?}` | Committed unit. Same id as its provisional |
| `retract{id}` | The held unit was discarded. The receiver removes the provisional (never a final) |
| `floor{turn, open}` | Speaker turn open/closed |
| `interrupt{turn}` | "I cut into your turn". The speaker marks its own records interrupted |
| `partial.turn` | Optional turn tag on interim captions |

On the receiving side, a provisional never replaces a final, and an older revision never replaces a newer one. Finals are still deduplicated by id (Phase 2 ledger), so a provisional followed by its final is not treated as a duplicate.

## 5. Interruption, overlap, latency (M4.4, M4.5, M4.7)
* **Barge-in:** pressing to talk while the peer's floor is open, or while their speech is queued or playing, marks those turns interrupted. Phase 2 cancellation still stops audio at once. Later segments of those turns are added to the transcript as *Interrupted — not played* and are never enqueued, so stale output cannot restart. A reply after the peer released is **not** an interruption. A floor that has been silent for 8 s, or one held across a reconnect, is treated as closed (lost close message).
* **Overlap:** a segment that arrives while the local user is talking is held (≤ 12 items, oldest evicted) and played when they stop. Each playback item carries `notAfter = received + 12 s`. `PlaybackController` drops stale items instead of playing them late. **No speaker separation is attempted or claimed.** The fallback is turn-taking.
* **Backpressure:** while a translation is in flight, later units of the same turn merge into one waiting unit. Queue depth is therefore at most one in flight plus one waiting per turn. Smoothed translation latency above 2 s sets `degraded`: provisional translations stop and a "slow translation" chip and notice appear. It clears below 1.2 s.

## 6. Context lifetime
All conversation state is per session, in memory only: the open unit, learned hold timing, interrupted turns, overlap-held items, the outbound queue and the latency estimate. `teardown()` (leave, peer-left, failure, dispose) clears all of it. Nothing is persisted. An abort (mic off and similar) discards the held unit.

## 7. Feature flags and rollback
| Flag | Default | Effect |
| --- | --- | --- |
| `VITE_CONVERSATION_LAYER` (build) | on | `0` / `off` / `false` → `SessionDeps.conversation = false` → exact Phase 3 behaviour (verified: layer-off simulator results are identical to the pre-Phase-4 baseline; rollback browser build passes) |
| `SessionDeps.conversation` (engine) | off when absent | Tests and embedders opt in. Accepts a partial `ConversationConfig` to tune bounds |
| `VITE_VOICEMIRROR` | off | unchanged; independent of Phase 4 |

Rollback options: rebuild with `VITE_CONVERSATION_LAYER=0` (no code change), or revert the Phase 4 commits. The server is unchanged.

## 8. Files changed outside `src/conversation/`
`src/engine/CommunicationSession.ts` (integration), `events.ts` (wire fields/messages and validator, record fields, `conversation.unit` event), `ports.ts` (`confidence?`, `notAfter?`), `segments.ts` (`hasSeenPeer`, `remove`), `defaultDeps.ts` (flag), `src/lib/textToSpeech.ts` (age bound), `src/lib/speechToText.ts` (forwards confidence), `src/components/CallRoom.tsx` (markers/chips), `src/vite-env.d.ts`, `src/test/engineFakes.ts` (opt-in env for regression runs). **No server, no policy (`ALLOWED_ORIGINS`/`RATE_LIMIT_PER_MINUTE`/`TRUST_PROXY`) and no VoiceMirror changes.**
