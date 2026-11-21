# Phase 3 Handoff — VoiceMirror

**Branch:** `claude/kind-archimedes-n6y11p` (restarted from `main` @ `b5d365c`). **Not merged, not deployed, no paid calls, no credential changes, no real-person voice data.**

Reports: [BASELINE_AUDIT](PHASE3_BASELINE_AUDIT.md) · [PLAN](PHASE3_PLAN.md) · [RESEARCH_AND_FEASIBILITY](PHASE3_RESEARCH_AND_FEASIBILITY.md) · [ARCHITECTURE](PHASE3_ARCHITECTURE.md) · [CHANGELOG](PHASE3_CHANGELOG.md) · [EVALUATION_PLAN](PHASE3_EVALUATION_PLAN.md) · [EVALUATION_REPORT](PHASE3_EVALUATION_REPORT.md) · [PRIVACY_AND_CONSENT](PHASE3_PRIVACY_AND_CONSENT_REPORT.md) · [INTEGRATION](PHASE3_INTEGRATION_REPORT.md) · [TEST](PHASE3_TEST_REPORT.md) · [FAILURE_AND_FALLBACK](PHASE3_FAILURE_AND_FALLBACK_REPORT.md)

## 1. Verdict

**Phase 3 development acceptance: PASS WITH CONDITIONS.**
**Production readiness: NOT READY** (separate assessment, §7).

## 2. What was done
* **Research:** measured on synthetic voices that **timbre dominates identity**. A perfect pitch+tempo match only reaches different-speaker similarity; only cloning reaches identity (cosine 0.94, 12/12). Compared 5 approach families against licensing (verified from primary sources), latency, consent risk, deployment and cost. The cloning/conversion approaches are infeasible here (no models, no GPU, no paid calls, no consented data).
* **Decision:** build the safe, measurable **characteristics tier** (pitch register + register-based stock voice) and the integration seam that higher tiers need. Design cloning/selection tiers; don't build or activate them.
* **Prototype** (`src/voicemirror/`): on-device streaming analyzer (fixed memory, ~2 ms per audio second), coarse 58-byte descriptor, strict validator, clamped hints, per-call controller with revocation and cleanup. Behind `VITE_VOICEMIRROR=1` **and** a per-call opt-in.
* **Integration** through Phase 2 contracts, all additive: optional dep, additive wire message, playback resolver at segment start, validated server hint with server-owned voice IDs, UI toggle and separate disclosure.
* **Evaluation harness:** reproducible; calibration/evaluation speaker split; product code evaluated under Node; two full runs (18 speakers × 30 language pairs).
* **Evaluation-driven change:** tempo transfer **disabled** after run 1 showed it was no better than baseline (worse tail).

## 3. Results (shipped configuration, run 2)

| Measure | Today | VoiceMirror (browser path) | Ceiling: perfect clone |
| --- | --- | --- | --- |
| Output pitch error (median) | 6.70 st | **1.10 st** (all 30 pairs improve) | 0.50 st |
| Speaker-encoder cosine (different-speaker reference 0.765) | 0.741 | 0.800 | 0.898 |
| Identified as the right speaker | 5.6% | 13.7% | 85.7% |
| Tempo error | 14.6% | 14.6% (transfer off) | 0 |
| Server path (register voice) pitch error | 6.70 st | 5.00 st | — |
| Intelligibility proxy (WER, relative) | 0.66 | 0.75 (register path 0.90) | 0.76 |

**Plain-language reading:** VoiceMirror makes the translated voice follow the speaker's **pitch level**, reliably, across all evaluated languages. It doesn't make it sound like *them*. It must not be marketed or described as voice cloning or identity preservation.

## 4. Acceptance criteria

| # | Criterion | Status | Evidence |
| --- | --- | --- | --- |
| 1 | Baseline audit and plan completed | ✅ | BASELINE_AUDIT (re-run baseline, contracts, constraints, risk register), PLAN |
| 2 | Research comparison and architecture decision include constraints, evidence, licensing and trade-offs | ✅ | RESEARCH_AND_FEASIBILITY: measured probe, 5 approaches, verified licences (unverifiable items marked), decision matrix, recommendation; ARCHITECTURE |
| 3 | Repeatable evaluation harness; metrics and test data documented | ✅ | `eval/voicemirror/` (one command), EVALUATION_PLAN, committed results JSON + generated summaries for two runs |
| 4 | Bounded prototype behind a controlled flag/seam; scope and limitations clear | ✅ | `VITE_VOICEMIRROR` + per-call opt-in; fixed-memory analyzer; limitations in EVALUATION_REPORT §9 and UI copy |
| 5 | Integration does not regress Phase 2 lifecycle, cancellation, consent, reconnect, IDs or buffer limits | ✅ | All 207 Phase 2 client tests, 53 server tests and 30 browser steps unchanged and green; integration tests for segment fields, cancellation, reconnect, cleanup, bounds |
| 6 | Opt-in explicit; revocation and cleanup verified; no voice data retained by default | ✅ | Separate disclosure, off by default, per call; revocation (local + peer + queued segments + outage re-delivery) and 4 session-end paths tested; no storage writes |
| 7 | No sensitive voice content, embeddings, names or transcripts in diagnostics/logs | ✅ | Enum-only `voicemirror` diagnostics (exact-keys test + mutation check); server logs unchanged; no embeddings are computed in the product |
| 8 | Regression, contract, privacy and reliability tests run and reported honestly | ✅ | TEST_REPORT (incl. the failed first E2E attempt and its cause; an invalid mutation attempt disclosed) |
| 9 | No unauthorized paid usage, deployment, merging, key changes, irreversible operations or real-person voice data | ✅ | None performed. Evaluation used synthetic espeak voices only; sources fetched read-only |
| 10 | Handoff explains risks and gives a clear Phase 4 recommendation | ✅ | §6–§8 |

**Why "with conditions":** the dimensions the spec asks about are measured, but several are only proxies or unmeasured. Perceived naturalness, intelligibility and similarity need human listeners; real voices and devices are untested; the register (server) path shows a proxy intelligibility drop. These don't block development acceptance, but they do block any user-facing claim or release.

## 5. Phase 2 carry-forward conditions
| Condition | Status after Phase 3 |
| --- | --- |
| Real-device compatibility / first-word latency | **Still open.** VoiceMirror adds a second mic consumer on Web Speech browsers, so the device check matters more |
| Production config (`ALLOWED_ORIGINS`, `RATE_LIMIT_PER_MINUTE`, `TRUST_PROXY`) | Still owner's task. New vars `VOICEMIRROR_ENABLED`, `VOICEMIRROR_VOICE_CATALOG` validated by `check-config`; defaults off |
| ElevenLabs key exposure review | Still owner's task; Phase 3 touched no provider |
| Preserve Phase 2 contracts; no merge/deploy | Met |

## 6. Open risks
| ID | Risk | Severity | Mitigation / owner action |
| --- | --- | --- | --- |
| P3-R1 | Users misread VoiceMirror as voice cloning | High | UI copy says "does not copy or clone your voice"; keep it; no marketing claims |
| P3-R2 | Perceived intelligibility/naturalness not measured; register path scored worst on the proxy | Med | Human listening test before any release; consider shipping only the pitch (browser) path |
| P3-R3 | Synthetic-only evaluation may not transfer to human voices | Med | Consented, licensed human voice set (owner decision) |
| P3-R4 | Second mic stream on Web Speech browsers (real devices) | Med | Real-device check (iOS Safari, Android Chrome) |
| P3-R5 | Render voice pitch range limits high-pitched speakers (~8 st tail) | Med | Pair register selection with pitch control; calibrate catalog voices |
| P3-R6 | `ScriptProcessorNode` deprecation | Low | `AudioWorklet` path follow-up |
| P3-R7 | +10 kB bundle in default builds | Low | Tree-shake behind the flag |
| P3-R8 | Higher tiers (cloning) carry impersonation and retention risk | High (if built) | Not built; gated on consent-verification design, retention/deletion terms, budget and legal review |

## 7. Production readiness (separate)
**Not ready**, independent of Phase 3 acceptance. Needed first: Phase 2 conditions (real devices, production config, key review); human evaluation of VoiceMirror (intelligibility, naturalness, perceived similarity) on consented voices; real-device check with two mic consumers; and a decision on whether to expose the register/server path at all, given the proxy result. VoiceMirror should stay flag-off in any production build until then.

## 8. Recommendation on Phase 4
**Phase 4 may begin**, on these conditions:
1. Phase 4 builds on the Phase 2 engine; VoiceMirror stays behind its flag (default off) and isn't a Phase 4 dependency.
2. The owner decides whether VoiceMirror continues toward a higher tier. That needs budget/hosting for zero-shot cloning (OpenVoice V2 is MIT-licensed; a provider is the alternative), a consent-verification and deletion design, and a consented human evaluation set. **None of that is Phase 4 work unless the owner says so.**
3. The Phase 2 release conditions remain open and are unaffected by Phase 4 starting.

## 9. Files and commits
Commits since `b5d365c`: `adfb91c` (M3.0/M3.1), `969af78` (M3.2–M3.5), `5881d9f` (M3.6: revocation re-delivery + reports), `705fd95` (tempo disabled + run-1 evidence), and the final M3.7 commit (run-2 evidence, evaluation/test/handoff reports). The file list is in [CHANGELOG](PHASE3_CHANGELOG.md).
