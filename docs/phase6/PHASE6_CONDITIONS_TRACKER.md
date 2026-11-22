# Conditions Tracker (Phases 1–6)

Nothing is closed without evidence. Items that need the owner stay open, with the owner named.

| # | Condition (origin) | Status after Phase 6 | Evidence / note | Owner |
| --- | --- | --- | --- | --- |
| C2 | Production `ALLOWED_ORIGINS`, `RATE_LIMIT_PER_MINUTE`, `TRUST_PROXY` | ⏳ **Open** | Server unchanged in Phase 6 (`git diff f393bde -- signaling-server` empty) | **Owner** |
| C3 | ElevenLabs historical credential review / authorized rotation | ⚠️ **Open** | untouched; no credential changes | **Owner** |
| C1/C4 | Real-device microphone lifecycle, first-word clipping, real-browser compatibility | ⏳ **Open** | Chromium headless only. Phase 6 adds on-device runtimes that **must** be checked on real devices (Translator API, local voices, `processLocally`) | Owner/tester |
| C5 | Real conversation tests; VoiceMirror second-mic test (VoiceMirror off) | ⏳ **Open**; VoiceMirror off ✅ | E2E: no voice-match control in default builds | Owner/tester |
| N3 | Native-speaker review of Phase 4/5 rules | ⏳ **Open**, now also covers the **Phase 6 phrasebook** (`src/local/phrasebookData.ts`) | — | Owner |
| N4/P5-1 | Independent evaluation | 🟡 Partially addressed (blind agent-authored sets for Phase 5 and Phase 6, not human) | EVALUATION_REPORT §2 | Owner: human review |
| P5-2 | Real-provider placeholder handling | ⏳ **Open** (no live calls) | — | Owner |
| N2 | Consent and draft-processing disclosure review | 🟡 Wording implemented (Phase 5), owner sign-off pending; `CONSENT_VERSION` not bumped | Phase 6 adds a processing-mode explanation in the privacy sheet; the consent disclosure itself is unchanged | Owner |
| P6-1 | **New:** on-device speech recognition unavailable or unsafe on test hardware (renderer crash) | ⏳ Open; disabled by default | `probe-local-capabilities.json` | Owner/tester: verify on real Chrome; then consider enabling |
| P6-2 | **New:** Translator API and local voices unverified (none on test hardware) | ⏳ Open | same | Owner/tester |
| P6-3 | **New:** local neural models (Whisper/Opus-MT/Piper) need download and dependency approval | ⏳ Blocked by authorization | EVALUATION_REPORT §5 | **Owner decision** |
| P6-4 | **New:** server translation cache (2000 entries, memory) retains text across sessions | ⏳ Open (server not modified) | AUDIT §1 | Owner: approve a TTL or disable |
| P6-5 | **New:** `priv` request is honoured only by priv1 peers | Documented | ARCHITECTURE §4 | — |
