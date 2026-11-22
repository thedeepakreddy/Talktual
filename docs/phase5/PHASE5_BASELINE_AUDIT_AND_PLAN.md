# Phase 5 — M5.0 Baseline Audit and Plan

**Branch:** `claude/kind-archimedes-n6y11p` @ `baedf69` (Phase 4 complete, not merged). Clean tree. Baseline: 324 client tests, 58 server tests and the 5 browser suites green.

## 1. Verified from code (not from reports)
| Area | What the code does today | Where |
| --- | --- | --- |
| Translation lifecycle | Local final → (Phase 4) segmenter commit → `OutboundQueue` → `deps.translate({text, from, to, signal})` → `segment` sent. Legacy (layer off): serial `sendChain`. **On failure `translateSegment` returns the input text** as `text`, and the session sends it as `translated` with `untranslated: true` | `CommunicationSession.pumpOutbound/finishOutbound`, `lib/translate.ts` |
| Context | **None.** Each segment is translated independently. The server `/api/translate` accepts `{text, from, to}` only and caches by exact text. The client keeps a transcript (≤ 500 records) for display only | `server.js:427`, `segments.ts` |
| Provider constraints | MyMemory (keyless, default), Gemini or DeepL (owner keys). No context or glossary parameter is exposed. Rate limit is owner-set (`RATE_LIMIT_PER_MINUTE`). Any provider-backed "intelligence" would need new server code plus paid or keyed calls → **not done in Phase 5** (no paid calls). ACI is local and rule-based | `server.js` |
| Interruptions / corrections | Phase 4: barge-in marks turns interrupted; restarts are repaired before send; committed text is never revised; `retract` for discarded provisionals | `src/conversation` |
| Cancellation / reconnect | `work` AbortController on teardown; outbound queue cleared; ListenerPolicy floor reset on reconnect; Phase 2 resume tokens | session |
| Consent UI | Disclosure names the recognizer and translator. **It does not mention Phase 4 draft (provisional) translation, and there is no control to turn drafts off** (Phase 4 condition N2) | `privacy.ts`, `ConsentSheet` |
| Diagnostics | Allowlist redactor (enum strings under known keys, numbers, booleans) | `diagnostics.ts` |
| Latency budget | Loopback release → speech p50 13 ms. Every Phase 5 decision must be local, synchronous, and well under the spec's 300 ms p95 | Phase 4 latency harness |

**Baseline behaviour for Phase 5 purposes:** a context-free, faithful translation of each segment. No entity protection, no ambiguity handling, no reference notes, no signals, no suggestions. Measured on the Phase 5 evaluation set with ACI off (see EVALUATION_REPORT).

## 2. Plan
A separate module `src/intelligence/` (ACI). It is pure, synchronous, has its clock injected, and its typed decision objects are versioned (`ACI_VERSION = 1`). The session asks it for decisions and validates and executes them. ACI never touches capture, transport, playback queues or UI directly.

| Milestone | Deliverable |
| --- | --- |
| M5.1 | `SessionContext`: recent turns (≤ 8, ≤ 1600 chars), entities (≤ 48, LRU), pinned terms (≤ 24), unresolved references (≤ 8), topic keywords (≤ 16), language pair, preferences. Deterministic eviction; reset; inspectable summary; no sensitive identifiers (phone/email/card-like strings are never stored) |
| M5.2 | Context-aware translation: protect established names/places/terms through MT (placeholders, validated restore, fallback); sense pre-editing for ambiguous words **only with context evidence** (labelled "context-assisted"); otherwise preserve ambiguity plus a listener note; reference notes ("there → Plaza Mayor") that never alter the translation |
| M5.3 | Signals: question, request, offer, confirmation, refusal, correction, clarification-request, topic change/continuation, unresolved reference, explicit urgency, misunderstanding. Each is a category with a confidence |
| M5.4 | Policy: faithful (default) / concise (user-controlled, closed list of discourse fillers, shown); direct delivery vs clarification suggestion; provisional units are never processed (Phase 4 finalization is awaited); term consistency |
| M5.5 | Correction linking ("no, Tuesday" updates context and marks the earlier line); suggestions with ids, supersession, dismissal, and a no-repeat loop guard |
| M5.6 | UI: intelligence toggle, mode/assist/suggestion controls, context inspector with reset and per-item removal, "context-assisted" explanation, dismissible suggestion chip; Phase 4 draft disclosure and toggle (N2) |
| M5.7 | Privacy lifecycle (end, reset, disable, consent decline), bounded time and memory, deterministic fallbacks, enum-only diagnostics |
| M5.8 | Evaluation: developer set + **independently authored blind subset** (written by a separate agent that never sees the implementation), adversarial cases, feature-off parity, latency/size, lifecycle |

Flag: `VITE_COMMUNICATION_INTELLIGENCE` (build; on by default, `0` = off) → `SessionDeps.intelligence` (engine; off when absent, so Phase 2–4 tests keep exact parity).

**Deviations from the spec's conceptual pipeline:** no provider-backed decisions (no paid calls; the server exposes no context API). The "outcome feedback" step is user feedback only (dismiss, remove or pin context items, reset), applied to the current session.
