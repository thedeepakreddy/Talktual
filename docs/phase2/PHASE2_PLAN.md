# Talktual 2.0 — Phase 2 Plan

Governing spec: *Talktual 2.0 Phase 2 — Next-Generation Communication Engine (updated with Phase 1 conditions)*. Architecture: [PHASE2_ARCHITECTURE.md](PHASE2_ARCHITECTURE.md).

## 1. Approach

Smallest viable refactoring path: keep every Phase 1 adapter (`src/lib/*`) and the server, strengthen their contracts in place, and move orchestration out of `CallRoom.tsx` into a React-free `src/engine/` layer. `CallRoom` becomes a view over a `useCommunicationSession()` hook. Each milestone ends with lint + all unit/server tests; M2.4 onward also runs the browser E2E. One commit (or more) per milestone.

## 2. Milestones

| Milestone | Work | Gate evidence |
| --- | --- | --- |
| **M2.0 Architecture & baseline** | Read Phase 1 reports; restart branch from `main`; baseline tests (75 + 37 + E2E 13/13); baseline latency (`e2e/latency.mjs`); P1 reproduced in browser (`e2e/privacy-probe.mjs`); key-exposure evidence gathered; these two documents. | This document set; `PHASE2_PHASE1_CONDITIONS_TRACKER.md` opened. |
| **M2.1 Session core** | `events.ts`, `eventBus.ts`, `sessionMachine.ts`, `segments.ts`, `diagnostics.ts`; `CommunicationSession` with transport + phase handling; server seats with resume tokens, `leave`, `peer-away/back`. | Transition-table tests, illegal-transition tests, teardown-once tests, race tests (leave during connect, double start, events after end), server resume tests. |
| **M2.2 Streaming audio foundations** | `PlaybackController` with bounded queue + status events; PCM schedule-ahead bound; capture adapter permission/device-lost; recording cap; bounded transport outbox. | Queue-limit, overflow-drop, cancellation-during-each-stage, backpressure, device-lost tests. |
| **M2.3 Pipeline adapters** | ASR interface (modes, `RecognizedSegment`, error codes, `stop`); `translateSegment()` with normalized result + bounded retry; synthesis adapter status. | Adapter contract tests; mocked provider success/timeout/error/429/malformed/cancel; no false success. |
| **M2.4 Realtime integration** | `CommunicationSession` runs the full pipeline; `useCommunicationSession`; `CallRoom` converted to a view; wire v1 with `seq`, dedupe; capability registry. | Engine integration tests (capture → ASR → translate → send → receive → playback); all Phase 1 tests still pass or are migrated 1:1; E2E green. |
| **M2.5 Privacy & recovery** | Listening modes, consent sheet, disclosure, indicators, stop guarantees; reconnect/backoff/resume; failure handling; config validator (`check-config`); P1 closed. | Privacy tests (no capture before consent, PTT release, continuous indication, stop, redaction); browser privacy probe; reconnect E2E; failure tests. |
| **M2.6 Verification & handoff** | Full regression, latency re-measure vs baseline, perf benchmark, compatibility matrix (incl. emulated no-Web-Speech and mobile viewport runs), all reports. | `PHASE2_*` deliverables; acceptance criteria one by one. |

## 3. Phase 1 conditions (carried forward)

| Condition | Handling in Phase 2 | Kind |
| --- | --- | --- |
| P1 privacy | Fixed in code (M2.5) + disclosure | Phase 2 item |
| Production settings | Startup validation + `npm run check-config -- --production`; values stay deployment tasks | Phase 2 (validator) + deployment item (values) |
| Key exposure / rotation | Evidence reviewed, decision recorded, rotation recommended; **not rotated** (needs owner) | Release item |
| Manual device testing | Staged plan; automated/emulated runs clearly separated from real devices | Phase 2 (plan, emulation) + release item (devices) |

## 4. Risks

| Risk | Mitigation |
| --- | --- |
| Moving logic out of `CallRoom` regresses verified behaviour | Port Phase 1 CallRoom tests to the engine 1:1 before deleting component logic; E2E unchanged in intent |
| Push-to-talk start-on-press clips first words on real devices | Documented; measured only emulated; continuous mode remains for hands-free; device check in release plan |
| Reconnect duplicates or revives speech | Segment-id dedupe, generation checks on cancellation, terminal states drop late results; tests per stage |
| Server seat-hold delays `peer-left` for Phase 1 clients | Bounded 20 s grace; new clients send `leave` |
| Scope creep into Phase 3 / redesign | Only consent sheet, mode control, banners added; no new modes beyond PTT/continuous; no new providers |
| Fake browser APIs hide real-device behaviour | Every report separates automated/emulated from real-device evidence |

## 5. Out of scope (spec §8)
Voice cloning, agents, redesign, offline/native, group calls, Pluto/ARC, provider migration, deployment, paid calls, credential rotation.
