# Conditions Tracker (Phases 1–5)

No condition is closed without evidence. Items that need the owner stay open, with the owner named.

| # | Condition (origin) | Status after Phase 5 | Evidence / note | Owner |
| --- | --- | --- | --- | --- |
| C1 | Phase 2: real-device microphone lifecycle and first-word clipping | ⏳ **Open** | Chromium headless + emulation only. Phase 5 adds no capture path | Owner/tester |
| C2 | Production `ALLOWED_ORIGINS`, `RATE_LIMIT_PER_MINUTE`, `TRUST_PROXY` | ⏳ **Open** | Phase 5 did not change the server or policy (`git diff baedf69 -- signaling-server` empty). ACI adds no provider calls except one bounded retry on malformed name restoration | **Owner** |
| C3 | ElevenLabs historical key exposure review / rotation | ⚠️ **Open** (needs authorization) | untouched | **Owner** |
| C4 | Phase 2 real-device compatibility | ⏳ **Open** | name detection depends on recognizer capitalisation: check on Chrome/Safari | Owner/tester |
| C5 | Phase 3: second-mic access, revocation and cleanup on real phones; VoiceMirror off by default | ⏳ **Open**; off by default ✅ | E2E: no voice-match control in the default build | Owner/tester |
| C6 | Phase 3: VoiceMirror human evaluation / register-path decision | ⏳ Open (out of scope) | — | Owner |
| N1 | Phase 4: real-conversation evaluation | ⏳ **Open** | still simulated / scripted | Owner/tester |
| N2 | Phase 4: disclosure for draft translation before phrase completion, plus a disable control | 🟡 **Implemented, owner sign-off pending** | Disclosure point added when the layer is on; "Send drafts while I pause" toggle (`setDraftsEnabled`); tests (session + E2E disclosure). `CONSENT_VERSION` **not** bumped, so existing consents aren't re-asked (owner decision) | Owner: approve wording; decide whether to bump the consent version |
| N3 | Phase 4: native-speaker validation of word lists | ⏳ **Open**, now also covers Phase 5 lexicons | `src/conversation/lexicon.ts`, `src/intelligence/lexicon.ts` | Owner |
| N4 | Phase 4: independent scenarios | 🟡 **Partially addressed for Phase 5** | Independent, implementation-blind ACI set (59 cases, agent-authored, not human). Phase 4 turn-taking scenarios are still self-authored | Owner: human-authored or blind-reviewed set |
| P5-1 | **New:** ACI validated only offline (synthetic dialogue, stand-in translator) | ⏳ Open | EVALUATION_REPORT §1, §5 | Owner/tester |
| P5-2 | **New:** placeholder-token behaviour of real translators (MyMemory/Gemini/DeepL) unverified | ⏳ Open | restoration fallback exists and is tested with simulated malformed output | Owner: run one authorized check |
| P5-3 | **New:** known accuracy gaps (sentence-initial names, cross-language cues, refusal over-firing, lexicon coverage) | ⏳ Open | EVALUATION_REPORT §4 | Engineering |
| P5-4 | **New:** human review of notes and suggestions for usefulness vs annoyance, and of meaning preservation | ⏳ Open | not performed | Owner |
