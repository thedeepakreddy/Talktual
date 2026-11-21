# Phase 2 Handoff — Communication Engine

**Branch:** `claude/kind-archimedes-n6y11p` (restarted from `main` @ `89ef81a`) · **Not merged, not deployed, no credentials touched, no paid calls.**

Reports: [ARCHITECTURE](PHASE2_ARCHITECTURE.md) · [PLAN](PHASE2_PLAN.md) · [CHANGELOG](PHASE2_CHANGELOG.md) · [TEST](PHASE2_TEST_REPORT.md) · [PERFORMANCE](PHASE2_PERFORMANCE_REPORT.md) · [PRIVACY](PHASE2_PRIVACY_REPORT.md) · [COMPATIBILITY](PHASE2_COMPATIBILITY_REPORT.md) · [FAILURE_RECOVERY](PHASE2_FAILURE_RECOVERY_REPORT.md) · [PHASE1_CONDITIONS](PHASE2_PHASE1_CONDITIONS_TRACKER.md)

## 1. Summary

The realtime logic that lived inside `CallRoom.tsx` is now a React-free engine (`src/engine/`). It has explicit state machines, versioned event and wire contracts, segment identity, privacy gates, bounded buffers, deterministic cancellation, reconnect with seat resume, and redacted diagnostics. The existing adapters were strengthened in place, not rewritten, and the UI kept its look. The Phase 1 privacy finding is fixed in code and verified in a browser.

| | Phase 1 end | Phase 2 end |
| --- | --- | --- |
| Client tests | 75 | **207** |
| Server tests | 37 | **53** |
| Browser E2E steps | 13 | **30** (smoke 17, relay-only recovery 8, emulated compat 5) |
| Idle call, recognizer running? | **yes** | **no** |
| Network blip | call lost | reconnects; nothing spoken twice |
| `npm audit` | 0 / 0 | 0 / 0 |

## 2. Milestone gates

| Milestone | Status | Evidence |
| --- | --- | --- |
| M2.0 Architecture & baseline | ✅ | Architecture + plan written before engine changes; baseline 75/37/13 green; baseline latency; P1 reproduced in browser |
| M2.1 Session core | ✅ | Exhaustive transition tests, 21 lifecycle/race tests, server resume tests |
| M2.2 Streaming audio | ✅ | Bounded playback queue, PCM backpressure, cancel-at-each-stage tests (bounds mutation-checked) |
| M2.3 Pipeline adapters | ✅ | ASR contract tests (18), `translateSegment` (no false success) |
| M2.4 Realtime integration | ✅ | Pipeline tests, CallRoom view tests, E2E 17/17 |
| M2.5 Privacy & recovery | ✅ | 33 privacy tests, relay-only recovery E2E, config validator, P1 probe after |
| M2.6 Verification & handoff | ✅ | Full regression, measurements, these reports |

## 3. Acceptance criteria (spec §7), one by one

| # | Criterion | Status | Evidence |
| --- | --- | --- | --- |
| 1 | Existing verified features intact, or intentional changes documented and approved | ✅ intact · ⚠️ **intentional changes need your approval** | Every Phase 1 E2E step and test is kept or ported 1:1. Intentional changes (CHANGELOG "Behaviour changes"): consent prompt on first use; PTT starts recognition on press; peer-left delayed 20 s for Phase 1 clients; mic button is the retry path. **Documented; awaiting your approval** |
| 2 | Session lifecycle explicit, tested, recoverable | ✅ | `sessionMachine.ts` tables (exhaustively tested); lifecycle tests; recovery E2E |
| 3 | Buffers bounded, cancellation deterministic | ✅ | Bounds table (PERFORMANCE §4) each test-backed; soak test; cancellation tests per stage |
| 4 | ASR, translation, TTS isolated behind maintainable contracts | ✅ | `engine/ports.ts`; adapters implement them; engine tested only through fakes |
| 5 | Segment identity and event contracts consistent | ✅ | id+seq end to end, dedupe, versioned `EngineEvent` v1 and wire v1 with validators |
| 6 | Listening understandable, consent-based, visible, controllable; P1 addressed in code and docs | ✅ in code/automation · ⏳ real-device confirmation | PRIVACY report; probe true → false; E2E consent/PTT/hands-free/stop steps |
| 7 | Failure and reconnect tested without unsupported guarantees | ✅ | FAILURE_RECOVERY matrix lists how each row was verified, plus explicit "not guaranteed" limits |
| 8 | Diagnostics avoid sensitive payloads by default | ✅ | Allowlist redaction; leak test over a full conversation; opt-in exposure (`?debug=1`) |
| 9 | Full tests run, evidence recorded; nothing fabricated | ✅ | TEST_REPORT (runs, counts, failures encountered and their causes, mutation checks) |
| 10 | Production config documented and safely validated; values not guessed | ✅ validator · ⏳ values are yours | `check-config [-- --production]`, startup gate on malformed values only, `.env.example` |
| 11 | Key-rotation exposure/status documented; no rotation without authorization | ✅ | CONDITIONS tracker C3: evidence, unknowns, decision = rotate unless Vercel checks clear it; **not rotated** |
| 12 | Manual device/browser matrix updated truthfully | ✅ | COMPATIBILITY: automated vs emulated vs real separated; **0 real devices tested**; staged plan |
| 13 | Architecture, implementation, test and known-risk reports delivered | ✅ | 10 documents in `docs/phase2/` |
| 14 | Evidence-based Phase 3 recommendation | ✅ | §6 below |

## 4. Remaining defects and known limitations

| ID | Severity | Item | Mitigation / next step |
| --- | --- | --- | --- |
| L1 | P1 (release) | **No real-device or non-Chromium testing.** All speech/TTS is faked in automation | Run COMPATIBILITY §3 stages 1–2 before any release |
| L2 | P1 (UX, unmeasured) | **First-word clipping risk:** PTT now starts the recognizer on press (P1 fix); real start-up delay unknown | Measure in stage 1.1; if material, design a privacy-preserving pre-roll in Phase 3 |
| L3 | P1 (security, release) | Possible historic ElevenLabs key exposure (C3) | Vercel checks → rotate if positive or unknown |
| L4 | P1 (deployment) | `ALLOWED_ORIGINS` / `RATE_LIMIT_PER_MINUTE` / `TRUST_PROXY` still unset in production (C2) | Set values; run `check-config -- --production` as a deploy gate |
| L5 | P2 | No `@types/react` and non-strict TS: React components are effectively untyped, so `lint` gives weak assurance for UI code | Add `@types/react`/`@types/react-dom`, fix the fallout, consider `strict` (Phase 3 prep) |
| L6 | P2 | Hands-free is half-duplex (can't cut in by voice while the peer speaks); echo pause relies on playback events, which are imprecise for browser voices | Document in UI copy; evaluate with real speakers |
| L7 | P2 | Phase 1 clients don't send `leave`; their peers wait 20 s for `peer-left` | Resolves once everyone loads the new frontend |
| L8 | P3 | Bundle 610 kB (+35 kB) single chunk | Lazy-load the QR scanner |
| L9 | P3 | Server-STT browsers keep the mic stream (tracks disabled) between pushes, so the browser indicator stays on | Optionally release after idle |
| L10 | P3 | Carried from Phase 1: `--text-3` contrast 2.52:1, no frontend CSP, public TURN creds if configured, WebRTC IP exposure | See Phase 1 SECURITY_REPORT |

No open defect is known to break a verified Phase 1 feature.

## 5. Phase 1 conditions

| Condition | Disposition |
| --- | --- |
| C1 Privacy (P1) | **Resolved in code**, verified in automated browser runs; real-device confirmation pending (L1) |
| C2 Production settings | Validator delivered; **values = deployment task (owner)** |
| C3 Key rotation | Evidence reviewed; **rotation recommended pending Vercel check; not performed** |
| C4 Manual devices | Staged plan delivered; emulated runs pass; **real devices not run** |

## 6. Recommendation for Phase 3

**Proceed to Phase 3 engineering — conditionally. Do not do a production release of Phase 2 until L1–L4 are closed.**

Evidence for proceeding:
* The engine is the stable seam Phase 3 UI and product work need. React consumes one hook, and pipeline behaviour is covered by 207 + 53 tests that run without browsers or providers.
* The core flows (talk/translate/hear, language switch, failure honesty, leave, reconnect, privacy gating) pass in a real browser against the real server, on both P2P and relay-only transports.
* No measurable latency or server-performance regression on the emulated path.

Conditions:
1. **Before Phase 3 builds on push-to-talk UX:** measure real first-word latency on Chrome and iOS Safari (L1/L2). It's the one Phase 2 trade-off that could change the product feel, and it can't be measured here.
2. **Before any release:** close C2 (production values) and C3 (Vercel check / rotation), and run stages 1–2 of the device plan.
3. **Early in Phase 3:** add React type definitions (L5). UI work will grow, and the current type-check doesn't cover components.
4. Please review and approve (or reject) the intentional behaviour changes in acceptance criterion 1.

## 7. Commands

```bash
npm ci && (cd signaling-server && npm ci)
npm run test:all                                   # lint + 207 client + 53 server + build
npm run test:e2e                                   # smoke + relay-only recovery + emulated compat
node e2e/privacy-probe.mjs                         # P1 probe
node e2e/latency.mjs . 30 ; node e2e/perf.mjs      # measurements
(cd signaling-server && npm run check-config -- --production)
```
