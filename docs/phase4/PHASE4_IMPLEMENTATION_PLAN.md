# Phase 4 Implementation Plan — Human-Like Conversation

Baseline: [PHASE4_BASELINE_AUDIT](PHASE4_BASELINE_AUDIT.md). Architecture: [PHASE4_ARCHITECTURE](PHASE4_ARCHITECTURE.md).

## 1. Approach
Add a **conversation layer** (`src/conversation/`) between the recognizer and the Phase 2 send/receive pipeline:
* **Pure, injectable policy modules** (turn/pause detection, segmentation, self-repair, interruption/overlap, latency adaptation). Each is unit-testable with a fake clock.
* **Integrated through the existing `CommunicationSession`.** No new transport, no rewrite.
* **Behind a rollback flag** (`VITE_CONVERSATION_LAYER=off` restores Phase 3 behaviour).
* **New wire features are capability-negotiated** (`hello.caps`), so Phase 2/3 peers keep working.
* **No dependency on VoiceMirror**, and no cloning scope.

## 2. Milestones → deliverables → acceptance mapping

| Milestone | Work | Acceptance criteria addressed |
| --- | --- | --- |
| M4.0 | Simulator + baseline (done), this plan, baseline audit | — |
| M4.1 | Typed unit/turn states, transitions, versioned events, turn IDs | provisional states explicit |
| M4.2 | Adaptive turn/pause detector: hesitation & incomplete-phrase cues, per-speaker gap statistics, bounded hold (min/max), hard maximum | turn policies bounded/tested; hesitation not finalized prematurely; turns don't stay open |
| M4.3 | Segmenter: hold/merge fragments, provisional → final with stable IDs and revisions, max unit size | provisional/revision/final states |
| M4.4 | Interruption: barge-in marks the peer's turn interrupted; later segments of that turn not played; `interrupt` message; no restart | barge-in cancels/supersedes stale output |
| M4.5 | Overlap: don't play over a talking listener; hold, then play or drop by age; uncertainty surfaced; confidence-based "uncertain" flag | overlap fallback; no separation claims |
| M4.6 | Self-repair within the unspoken window: restarts supersede, repetitions collapse, fillers dropped; never alters spoken content | no duplicate/stale playback from corrections |
| M4.7 | Latency/load: outbound backpressure (bounded pending, merge under pressure), playback age bound (`notAfter`), adaptive provisional pacing, degraded status | bounded queues under delay; degradation tested |
| M4.8 | UI: provisional vs final styling, interrupted/uncertain markers, overlap/degraded notices | — |
| M4.9 | Re-run the 13-scenario simulator (layer on / layer off), unit + integration tests, browser E2E | evaluation matrix; Phase 2/3 regressions |
| M4.10 | Privacy review, condition tracker, reports, handoff | diagnostics; context scope; rollback/flags |

## 3. Bounds (planned → shipped)
Shipped values are in `DEFAULT_CONVERSATION_CONFIG` (`src/conversation/types.ts`); changes from the plan are noted.
| Policy | Bound |
| --- | --- |
| Hold for an incomplete-looking fragment | adaptive, 500–2000 ms (default 900 ms) |
| Hard maximum before an open unit is committed | planned 2500 ms after last activity → **shipped: 6000 ms after the unit opened** (a fixed cap that continued speech can't extend; the 500–2000 ms hold still applies after each final) |
| Max merged unit | 400 characters |
| Pending outbound finals before merge-under-pressure | planned 3 → **shipped: merge as soon as one unit waits behind an in-flight translation (same turn)** |
| Playback item max age (not started) | 12 s |
| Overlap hold | until the local user stops talking, subject to the age bound |
| Provisional translation | ≤ 1 in flight, ≥ 700 ms apart, off when degraded |
| Interrupted-turn memory | 64 turn IDs (bounded set) |

## 4. Risks
| Risk | Mitigation |
| --- | --- |
| Holding fragments adds delay to genuine short turns | Hold only on explicit incompleteness cues; a release (PTT) always commits; bounded hold |
| Self-repair heuristics change meaning | Only within unspoken held text; conservative rules; per-language lists; fully tested; flag `repaired` |
| Old peers misinterpret new messages | `hello.caps` negotiation; provisional/interrupt only sent to capable peers |
| Phase 2/3 regressions | Additive integration; full suites per milestone; rollback flag tested |
| Simulation ≠ real speech | Results labelled simulated; real-device checks remain an open condition |
