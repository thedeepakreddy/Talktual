# Phase 5 Architecture — Autonomous Communication Intelligence (ACI)

ACI is an **advisory** layer in `src/intelligence/`. It is pure TypeScript, synchronous, uses an injected clock, and has no network and no storage. `CommunicationSession` calls it at two points. The session validates each decision and executes it, or ignores it. ACI never touches capture, transport, playback queues, credentials or rendering.

```
 my final phrase (Phase 4 commit / Phase 2 final)
   └─▶ aci.outbound(text) ──▶ OutboundDecision ──validate──▶ session:
         mtInput (names → QXnQ, context sense edits)    translate(mtInput) → restore names
         notes/signals for MY line, suggestions          fail → send ORIGINAL untranslated
         correction link                                 restore fails → 1 context-free retry
                                                         send {original (unchanged), translated, assist?}
 their final segment
   └─▶ aci.inbound(original, translated, assist) ─▶ InboundDecision ──validate──▶ session:
         notes (ambiguity/sense/reference/idiom/correction/assisted)   record.notes (shown, never spoken)
         signals (category+confidence)                                  record.signals
         speak? (concise, MY preference)                                playback text (transcript keeps faithful)
         correction → earlier segment                                   earlier record.correctedBy
         suggestions (to me)                                            snapshot.intelligence.suggestions
```
Provisional (Phase 4 draft) units are **never** analysed: ACI only sees finals, so it waits for Phase 4 finalization. Interruption, overlap, self-repair, segmentation, cancellation, reconnect and playback all stay with Phase 2/4.

## 1. Modules
| File | Role |
| --- | --- |
| `types.ts` | Versioned contracts (`ACI_VERSION = 1`, wire cap `aci1`): `OutboundDecision`, `InboundDecision`, `Findings`, `Signal`, `Note`, `Suggestion`, `ContextSummary`, `AciPrefs`, `AciLimits` |
| `lexicon.ts` | Everything ACI "knows": ambiguous words (en 14, es 10), idioms (en 22, es 22), signal cue regexes (en, es), filler lists (6 languages), reference words, place/person cues, days/months |
| `text.ts` | Normalisation; sensitive-identifier redaction (email, IBAN-like, 7+ digit runs) |
| `extract.ts` | Rule-based entities (person/place/name/date/time/number), each with a confidence; `protect` flag |
| `context.ts` | `SessionContext`: bounded turns, entities and topic; LRU eviction; user remove/pin; summary; clear |
| `analyze.ts` | Ambiguity (evidence rule), idioms, references (single antecedent only) |
| `signals.ts` | Signal detection plus overlap resolution between rule families |
| `protect.ts` | Deterministic MT-input construction; tolerant token restoration that detects malformed output |
| `policy.ts` | Faithful fallbacks; `validateOutbound`, `validateInbound`; concise filler removal |
| `controller.ts` | `IntelligenceController`: prefs, lifecycle, suggestions registry, decisions |

## 2. Decision rules (all deterministic)
* **Sense evidence.** A cue word in the same utterance scores 2; a cue in recent context scores 1. A sense is chosen only if its score is ≥ 2 and at least twice the runner-up's. Otherwise the word stays **ambiguous**: the listener gets a note and the speaker a dismissible suggestion. Contradictory context therefore leaves it ambiguous. The MT input is pre-edited only when the chosen sense has a listed unambiguous replacement (e.g. `bank → riverbank`). The edit is labelled on both sides.
* **Names.** Person names and unknown proper names, plus street-type addresses, are replaced by `QXnQ` tokens in the MT input and restored afterwards. Place names are not protected, because many have exonyms (London → Londres). A lone capitalized word at the start of a sentence is not treated as a protected name (known limitation).
* **References.** `there/allí/ahí/allá` → places; `he/she/him/her/él/ella` → persons or names. Resolution needs **exactly one** candidate in the last 4 context turns, and an explicit topic change clears the candidates. No gender is inferred from names, so two persons always means no guess.
* **Corrections across turns.** A correction signal plus a new value of the same kind (time/date/number/place/name) supersedes the speaker's previous value. The earlier line is annotated "corrected later" and is never rewritten (Phase 4's never-revise-committed rule).
* **Signals.** Each is a category with a fixed rule confidence. The UI and diagnostics use ≥ 0.5. Misunderstanding needs observable evidence (explicit "I don't understand", or a clarification request right after the other person's turn). Urgency needs explicit words. Refusal needs a preceding turn by the other person. There is no emotion, personality or sensitive-attribute inference.
* **Suggestions.** At most one per decision and 3 active. Each has an id, a de-duplication key and a 45 s TTL. A key is never offered twice in a session (loop guard). A suggestion is dropped when its segment is interrupted. A newer suggestion of the same kind supersedes the older one. Suggestions are informational only. **Nothing is ever sent automatically.**

## 3. Validation (policy enforcement)
`validateOutbound` rejects a decision unless all of these hold. Any rejection → faithful fallback (`fallback: 'invalid'`, counted). Decisions are never partially applied.
* The MT input equals the deterministic construction from the original.
* Every protected value occurs in the original.
* Every edit is a lexicon sense edit of a word that was spoken.
* The assist codes match `sense:<word>:<id>` or `protect:<n>`.
* Notes, signals and suggestions are within bounds.

`validateInbound` requires that concise speech is an in-order subsequence of the faithful words, removes at most half the words, and keeps at least two. It also bounds notes and signals. Mutation tests cover each rule (`intelligence.test.ts`).

## 4. Bounds
| Item | Bound | Observed max (600-turn stress) |
| --- | --- | --- |
| Recent turns | 8, ≤ 1600 chars (oldest evicted first) | 8 turns, 413 chars |
| Entities | 48 (LRU; pinned evicted last) | 8 |
| Topic words | 16 | — |
| Active suggestions / dismissed keys | 3 / 64 | — |
| Notes per segment | 3 | — |
| Analysed input | 600 chars (longer → faithful fallback, `too-long`) | — |
| Decision time | synchronous rules; p95 0.20 ms (stress), 0.15–0.19 ms (eval sets) | max 2.2 ms |
| Provider calls added by ACI | **0** (one context-free retry only when protected names come back malformed) | — |

## 5. Interfaces to the rest of the app
* `SessionDeps.intelligence?: boolean | { prefs?, limits? }`. The engine is **off when absent**, giving exact Phase 4 behaviour.
* Session commands: `setIntelligenceEnabled`, `setIntelligencePrefs({mode, assist, notes, suggestions})`, `resetIntelligenceContext`, `removeContextItem`, `pinContextTerm`, `dismissSuggestion`, and, for Phase 4 condition N2, `setDraftsEnabled`.
* Snapshot: `intelligence {available, prefs, suggestions, context: ContextSummary, peerCapable}` and `conversation.drafts`. `SegmentRecord`: `notes`, `signals`, `assisted`, `correctedBy`.
* Wire: `hello.caps` includes `aci1`. Segments may carry `assist: string[]` (validated, ≤ 8 codes), sent only to `aci1` peers. Older peers see no change.
* Disclosure: when the respective layers are on, points are added for Phase 4 drafts and for on-device assistance.

## 6. Flags and rollback
| Flag | Default | Off → |
| --- | --- | --- |
| `VITE_COMMUNICATION_INTELLIGENCE` (build) | on | exact Phase 4 behaviour: no assistance UI, no disclosure point, translator receives plain text (rollback E2E) |
| `SessionDeps.intelligence` (engine) | off when absent | — |
| In-call "Assistance" switch | on | context cleared immediately; context-free from the next phrase (tests + E2E) |
| `VITE_CONVERSATION_LAYER`, `VITE_VOICEMIRROR` | unchanged | — |

## 7. Deviations from the spec's conceptual pipeline
* **No provider-backed decisions.** The server exposes no context API, and adding one would mean paid or keyed calls. All intelligence is local rules. The spec's 1.5 s provider-latency target is therefore not applicable.
* **"Outcome feedback"** is user feedback only (dismiss, forget, pin, reset), applied to the current session. There is no automatic learning.
* **Context assistance is applied by the sender** (it shapes the MT input). The listener's ACI independently computes notes from the original text and its own context, so no context is ever sent over the wire. Only the `assist` explanation codes are.
