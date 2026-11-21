# Phase 3 Plan — VoiceMirror

Governing spec: *Talktual 2.0 Phase 3 — VoiceMirror: Voice Identity Research & Engineering*. Baseline: [PHASE3_BASELINE_AUDIT](PHASE3_BASELINE_AUDIT.md).

## 1. Approach in one paragraph

Research first: compare voice-preservation approaches against this repository's real constraints (browser-first app, no GPU, blocked model hub, no paid calls, no real voice data). Then build a **repeatable evaluation harness on synthetic voices** that measures what any approach actually preserves: pitch, tempo and speaker-embedding similarity, separately and against baselines. Implement the narrowest approach that is feasible and consent-safe here as an isolated module behind a build flag and a per-call opt-in. Integrate it through Phase 2 contracts without changing their behaviour when the feature is off. Design higher-fidelity approaches (provider cloning, voice conversion) behind the same interface, but don't activate them without approval.

## 2. Milestones

| Milestone | Deliverables | Gate evidence |
| --- | --- | --- |
| **M3.0** Recon & baseline | BASELINE_AUDIT, PLAN, risk register, evaluation design (EVALUATION_PLAN) | Baseline suites green (207/53/17) |
| **M3.1** Research & feasibility | RESEARCH_AND_FEASIBILITY: approaches, licensing, decision matrix, recommendation; a **measured feasibility probe** on synthetic voices | Decision backed by documented sources and the probe |
| **M3.2** Architecture & contracts | ARCHITECTURE; typed contracts in `src/voicemirror/` (profile, render hints, events, renderer interface) | Contract tests |
| **M3.3** Evaluation harness | `eval/voicemirror/`: synthetic speaker generation, metrics (pitch/tempo error, speaker-embedding cosine, intelligibility proxy, robustness, latency/resources), baselines + oracle | Harness runs end to end from one command; results written to JSON |
| **M3.4** Controlled prototype | On-device voice-profile extractor + quantized descriptor + render mapping; `VITE_VOICEMIRROR` flag | Unit tests; harness results |
| **M3.5** Phase 2 integration | Voice opt-in in session, `voice` peer message (wire v1 additive), playback hints, server TTS hint (validated) | All Phase 2 suites unchanged and green; integration tests; E2E with the flag on and off |
| **M3.6** Privacy & safety | Separate consent, revocation, session-end cleanup, no retention, logging review | Privacy tests incl. diagnostics leak test |
| **M3.7** Verification & handoff | Remaining reports; verdict | Full regression + evaluation re-run |

## 3. Evaluation design (summary; full version in [PHASE3_EVALUATION_PLAN](PHASE3_EVALUATION_PLAN.md))
Separate measurement for each spec dimension: identity (speaker-embedding cosine plus a profile distance, each against a no-preservation baseline and an oracle upper bound), naturalness/prosody (proxy only — a human MOS is required and is out of reach), intelligibility (proxy plus parameter-range safety; ASR-based scoring is unavailable offline), cross-language (every supported language pair evaluated or marked unevaluated), latency/streaming (component timings in Node and Chromium), robustness (short, noisy, silent, clipped, interrupted inputs), privacy (tests), resources (CPU time, memory, payload bytes).

## 4. Risks
See the BASELINE_AUDIT risk register (R1–R8), updated in the HANDOFF.

## 5. Non-goals (spec)
Production release; UI redesign; Phase 4–7 scope; provider credential changes; paid usage; real-person voice data without authorization and consent.
