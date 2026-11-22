# Phase 5 Privacy Review and Data Flow

## 1. Data flow
```
 recognizer final (text) ──▶ ACI (this device, memory only)
                              ├─ context: ≤ 8 recent lines (redacted), ≤ 48 entities, ≤ 16 topic words
                              ├─ MT input: original with names → QXnQ, optional lexicon sense edit
                              │      └─▶ Talktual server ─▶ translator (MyMemory/Gemini/DeepL)   [same path as Phase 2]
                              └─ assist codes (e.g. "sense:bank:river", "protect:1") ─▶ peer (aci1 only)
 peer segment ──▶ ACI (listener's device): notes/signals/suggestions, never sent anywhere
```
| Data | Stored where | Lifetime | Leaves the device? |
| --- | --- | --- | --- |
| Recent lines (for context) | `SessionContext` memory; phone/email/card/IBAN-like strings replaced by `[number]` before storage | Until evicted (8 lines / 1600 chars), reset, disable, consent decline, or call end | No |
| Entities (names, places, dates, times, numbers) | memory; each marked *guessed* or *pinned* | same | No. Protected names are **withheld** from the translator (tokens) |
| Topic words | memory | same | No |
| Notes, signals | on segment records (transcript, memory) | transcript lifetime (Phase 2: memory, ≤ 500 records) | No |
| Suggestions, dismissed keys | memory | 45 s / session | No |
| Assist codes | sent to `aci1` peers | per segment | Yes: lexicon word + sense id, or a count. No context, no names |
| Prefs | memory | session (not persisted) | No |

**Minimum context to providers.** ACI sends the translator **less** than before: protected person names are replaced by tokens (E2E-verified, no name in the provider request). The only addition is a lexicon synonym when a sense was chosen. No context is sent: no earlier lines, entities or summaries. The server is unchanged.

## 2. Spec checks
| Requirement | Status | Evidence |
| --- | --- | --- |
| Ephemeral; cleared on session end, reset, disable, consent withdrawal | ✅ | `teardown` → `aci.reset()`; reset/disable/decline paths; session test "context cleared on reset, decline, and session end"; E2E off switch |
| Never silently persists across sessions | ✅ | no storage API in `src/intelligence/`; prefs not persisted |
| No audio, transcripts, payloads, secrets or summaries in logs | ✅ | diagnostics records `aci{segmentId, kind, reason, assisted, notes, suggestions}`, `aci_signal{segmentId, kind, confidence}`, counters, `aci_ms`. Test checks names, words and places never appear |
| No sensitive identifiers stored | ✅ | redaction before storage; pin refuses them; eval: 0/6 sensitive strings stored (dev 2 + independent 4); 600-turn stress: phone never stored |
| No emotion / identity / relationship / sensitive-attribute inference | ✅ | signals are interaction categories; no gender from names (two persons → no guess); urgency only from explicit words; test "no emotion inference" |
| Bounded memory, time, retries | ✅ | ARCHITECTURE §4; one context-free retry, only on malformed name restoration |
| Fallback to faithful / Phase 4 behaviour | ✅ | every error, invalid decision or too-long input → faithful; translation failure never leaks MT input (test) |
| Respect consent, cancellation, reconnect, rate limit, backpressure | ✅ | ACI runs inside the existing translate step (same AbortSignal, same Phase 4 queue). No extra calls except the bounded retry. Context kept across a reconnect within the same call (test) |
| VoiceMirror not a dependency, off by default | ✅ | no import; E2E checks no voice match control |

## 3. Residual concerns (owner review)
1. **Recent lines are held in memory for context** (up to 8 lines). They are visible to nothing except the rules, cleared as above, and never logged. They are already present in the Phase 2 transcript memory, so this adds no new exposure.
2. **Assist codes reveal which ambiguous word was used and the chosen sense** (e.g. `sense:bank:river`) to the peer, who already receives the words themselves.
3. **Disclosure wording** for drafts (Phase 4 N2) and assistance was added without bumping `CONSENT_VERSION`, so users who already consented are not re-asked. Bumping is a one-line owner decision (it re-prompts everyone).
