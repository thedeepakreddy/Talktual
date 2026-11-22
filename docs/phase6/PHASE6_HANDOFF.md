# Phase 6 Handoff — Privacy, Local Intelligence and Offline

Branch `claude/kind-archimedes-n6y11p`, from `f393bde` (Phase 5). **Not merged, not deployed, no PR.** No paid calls, credential or server changes, model or dependency downloads, or machine-configuration changes. Phase 7 not started.

Documents: [AUDIT_AND_FEASIBILITY](PHASE6_AUDIT_AND_FEASIBILITY.md) · [ARCHITECTURE](PHASE6_ARCHITECTURE.md) (contracts, mode semantics, capability matrix, data flow, threat model, rollback) · [TEST_REPORT](PHASE6_TEST_REPORT.md) · [EVALUATION_REPORT](PHASE6_EVALUATION_REPORT.md) (quality and real measurements) · [CONDITIONS_TRACKER](PHASE6_CONDITIONS_TRACKER.md) · evidence: `probe-local-capabilities.json`, `measurements.json`

## 1. What was built
* **Processing modes**: Cloud-assisted (default), Hybrid, On-device only, plus derived Offline, Providers-unreachable and Degraded states.
* **A central policy** (`src/privacy/policy.ts`):
  * per-task plans (ASR, translation, captions, TTS);
  * an execution-time remote gate;
  * revocation of in-flight remote work on mode or network change;
  * an enum-only audit.
  * On-device only fails closed and never routes to the cloud.
* **A local runtime abstraction** (`src/local/`): typed contracts, plus a bounded registry with discovery, LRU loading, concurrency/queue/timeout limits, idle unload, memory-pressure unload and output validation.
* **Verified local capability:** a bundled **offline phrasebook translator (en↔es)**. It never translates partially: an utterance is either covered or left as is. Measured on a blind set and on real hardware.
* **Gated platform runtimes**, used only when the platform reports them ready:
  * Chrome Translator API (never triggers downloads);
  * on-device OS voices (`localService` only);
  * Web Speech `processLocally`, **disabled by default** because it crashed the renderer on the test hardware.
* **Typed input**, so local-only and offline users can still communicate without ASR.
* **Peer privacy request** (`priv1`): a local-only sender's words stay on-device on the receiving side too.
* **Offline and degraded:** browser offline events, provider-failure detection with cooldown, accurate status, no replay of revoked work.
* **Privacy controls:** a privacy sheet (mode, where each function runs, what leaves the device, on-device engines, "Clear this call's data"), status chips and transcript markers. The client translation cache is now cleared on session end and mode change. Only the mode choice persists, as a setting.

## 2. Acceptance criteria
| Criterion | Status | Evidence |
| --- | --- | --- |
| Precise mode semantics, centrally enforced policy | ✅ | ARCHITECTURE §1; policy unit tests |
| Local-only demonstrably blocks remote inference | ✅ | 64-combination unit test; session tests (translator never called, no recognizer); **E2E wire audit: 0 inference requests from either page** |
| Offline/degraded status accurate, no misrepresentation | ✅ | capability rows from the policy preview; offline E2E; "unavailable" shown, never "on-device" for cloud |
| Capability discovery reports honestly | ✅ | `RuntimeInfo.verification`; real probe; time-bounded discovery |
| ≥ 1 meaningful verified local capability, or substantiated blockers | ✅ **phrasebook + typed input** (blockers for ASR/TTS/neural MT documented with evidence) | EVALUATION §1–3 |
| Hybrid respects consent; no silent cloud fallback from local-only | ✅ | policy + tests; ASR cloud requires consent |
| Lifecycle, cancellation, reconnect, stale results tested | ✅ | TEST_REPORT §2 |
| Actual resource measurements | ✅ (container; battery/thermal not measurable) | EVALUATION §3 |
| UI/data-flow disclosures reflect the implementation | ✅ | sheet text driven by the capability matrix; UI tests |
| Previous conditions tracked, not closed without evidence | ✅ | CONDITIONS_TRACKER |
| Rollback preserves the Phase 5/4 baseline | ✅ | `VITE_PROCESSING_MODES=0` E2E; engine off when absent; Phase 4/5 evaluations identical |

## 3. Verdict
**Phase 6 development: PASS WITH CONDITIONS.** The privacy architecture (modes, central policy, fail-closed local-only, offline handling) is implemented and demonstrated on the wire.

Local inference is honestly limited:
* **Verified:** on-device translation for common en↔es phrases only (53/85 in-scope blind utterances; 0 false coverage of free speech).
* **Not verified on any device:** on-device speech recognition and speech output (none on the test hardware), and neural local translation (needs an authorized model download).
* **Not possible:** calls without network (signaling).

## 4. Rollback
* **Build:** `VITE_PROCESSING_MODES=0` restores Phase 5 behaviour (E2E-verified).
* **Per call:** "Cloud-assisted" mode.
* **Code:** revert `ab5628d..HEAD`. The wire stays compatible because `priv1`/`priv` are optional.

## 5. Production readiness (separate)
**Not ready.** The carried blockers are unchanged:
* C2 production config;
* C3 key review;
* real devices;
* real conversations.

New items from Phase 6:
* P6-1/P6-2: on-device runtimes are unverified on real devices;
* P6-4: the server translation cache retains text (owner decision).

Phase 6 does not weaken readiness: it is flag-controlled, server-neutral, and in local-only mode sends providers nothing.

## 6. Remaining work (precise)
1. **Owner decisions.**
   * Authorize evaluating local neural models (Whisper-tiny / Opus-MT en↔es / Piper via onnxruntime-web). This involves downloads and a new dependency.
   * Server translation-cache TTL (P6-4).
   * Consent version bump (N2).
2. **Real-device verification.** Chrome with on-device Translator models and `processLocally` (enable `VITE_ONDEVICE_ASR=1` only on test builds); iOS/Android local voices. Then update the capability matrix.
3. **Phrasebook.** Native-speaker review (N3); coverage growth driven by new blind data, not the current set.
4. **Carried conditions** C1–C5, N3, P5-2.
