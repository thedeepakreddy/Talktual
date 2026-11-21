# Conditions Tracker (Phases 1–4)

No condition has been closed without evidence. Items that need the owner stay open, with the owner named.

| # | Condition (origin) | Status after Phase 4 | Evidence / note | Owner |
| --- | --- | --- | --- | --- |
| C1 | Privacy: recognizer ran continuously (Phase 1) | ✅ Resolved in code (Phase 2). Real-device confirmation pending (C4). Phase 4 adds no capture path; the recognizer is still only started by press or explicit hands-free | Phase 2 privacy report; Phase 4 browser suites green | Owner: device confirmation |
| C2 | Production settings `ALLOWED_ORIGINS`, `RATE_LIMIT_PER_MINUTE`, `TRUST_PROXY` (Phase 1) | ⏳ **Open.** Validator delivered (Phase 2). Phase 4 **did not change** policy or the server | `git diff 497713c -- signaling-server` is empty | **Owner:** set values, run `check-config --production` |
| C3 | ElevenLabs key-exposure review / rotation (Phase 1) | ⚠️ **Open.** Not performed (needs authorization). Phase 4 touched no provider | Phase 2 tracker §C3 | **Owner** |
| C4 | Real-device compatibility + first-word latency (Phase 1/2) | ⏳ **Open.** Still Chromium headless + emulation only. Phase 4 adds timing policies that should be checked on real recognizers (where Safari/Chrome put finals, and how often the hold cues fire) | TEST_REPORT §5 | Owner/tester |
| C5 | VoiceMirror real-phone second-microphone lifecycle test (Phase 3) | ⏳ **Open.** VoiceMirror stays off by default. Phase 4 does not depend on it | Phase 3 handoff §5–§7 | Owner/tester |
| C6 | VoiceMirror human evaluation (intelligibility, naturalness, similarity) and the register-path decision (Phase 3) | ⏳ **Open.** Out of Phase 4 scope | Phase 3 handoff P3-R2 | Owner |
| C7 | Phase 3 accepted with conditions; Phase 4 must not depend on VoiceMirror or add cloning scope (owner, PR #3) | ✅ **Met** | No VoiceMirror import in `src/conversation/`; tests and E2E run with VoiceMirror absent | — |
| N1 | **New (Phase 4):** conversation policies validated only in simulation / scripted browsers | ⏳ **Open** | EVALUATION_REPORT §1, §4 | Owner/tester: real two-person calls, ideally an independent scenario set |
| N2 | **New (Phase 4):** processing disclosure doesn't mention provisional (early) translation | ⏳ **Open (owner decision)** | PRIVACY_REVIEW §3.1 | Owner: accept, reword the disclosure, or tune provisional off |
| N3 | **New (Phase 4):** completeness lexicons cover en/es/fr/de/pt/it only; lists unreviewed by native speakers | ⏳ Open | `src/conversation/lexicon.ts` | Owner: native-speaker review before claiming support |
