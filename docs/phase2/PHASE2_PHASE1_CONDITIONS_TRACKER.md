# Phase 1 Conditions Tracker

Phase 1 closed as **PASS WITH CONDITIONS**. Each condition's disposition in Phase 2:

| # | Condition | Kind | Status | Evidence | Owner / action |
| --- | --- | --- | --- | --- | --- |
| C1 | **Privacy (P1):** Chrome speech recognition ran continuously during a call | Phase 2 item | ✅ **Resolved in code** (real-device confirmation pending, see C4) | Browser probe before: `{"recognizerRunning":true,"recognizerEvents":["start"]}` → after: `{"recognizerRunning":false,"recognizerEvents":[]}` (`e2e/privacy-probe.mjs`). E2E: no recognizer constructed before consent; none running after consent until held; stops ≤ 459 ms after release (p50 452). 31 engine privacy tests, 18 adapter tests. Details: [PHASE2_PRIVACY_REPORT](PHASE2_PRIVACY_REPORT.md) | Engineering: done. Owner: confirm on real Chrome/Safari devices |
| C2 | **Production settings:** origin allow-list, rate limit, trusted proxy disabled by default | Phase 2 (validation) + **deployment item (values)** | ✅ Validator delivered · ⏳ values pending | `validateEnv()`; `npm run check-config -- --production` exits 1 until `ALLOWED_ORIGINS` and `RATE_LIMIT_PER_MINUTE` are set; malformed values stop startup; 10 tests incl. CLI and startup gate. Defaults unchanged so the live deployment isn't broken | **Owner:** set `ALLOWED_ORIGINS=<frontend origin>`, `RATE_LIMIT_PER_MINUTE` (e.g. 600), `TRUST_PROXY=1` (behind Render-style proxy), then run the check as a deploy gate. Values not guessed by me |
| C3 | **Credential exposure / key rotation** | Release item | ⚠️ **Rotation recommended — not performed (needs your authorization)** | See below | **Owner:** decide; check Vercel |
| C4 | **Manual device testing** | Phase 2 (plan + emulation) + **release item (devices)** | ⏳ Plan delivered; emulated runs done; **no real device tested** | Automated: Chromium 157 headless (30 browser steps). Emulated: no-Web-Speech path with real MediaRecorder, no-STT degradation, Pixel 5 viewport. Staged real-device plan in [PHASE2_COMPATIBILITY_REPORT](PHASE2_COMPATIBILITY_REPORT.md) | **Owner/tester:** execute stages 1–4 on real devices |

## C3 — Key exposure evidence and decision

**Evidence reviewed (Git history, read-only):**

| Fact | Source |
| --- | --- |
| `e78f64c` (2026-07-11 00:05:34 +0200) added `import.meta.env.VITE_ELEVENLABS_API_KEY` reads in `src/lib/speechToText.ts` and `src/lib/textToSpeech.ts` (browser code) | `git grep VITE_ELEVENLABS_API_KEY e78f64c -- src` |
| `1c7412a` (00:18:24) still contained it | history |
| `c52e7c4` (00:22:09) moved calls to the server proxy; no browser reads remain | `git grep … c52e7c4 -- src` → none |
| All three commits are on `main` | `git branch -r --contains e78f64c` → `origin/main` |
| `vercel.json` existed since `7505b02` (2026-07-10), i.e. a Vercel project was set up before the window | `git log --diff-filter=A -- vercel.json` |
| No key value was ever committed (`.env*` ignored; history scanned in Phase 1) | Phase 1 SECURITY_REPORT |

**What I cannot determine from the repository:**
1. Whether `VITE_ELEVENLABS_API_KEY` was set in the Vercel project's environment. If it wasn't, no build contained a key.
2. Whether `e78f64c` or `1c7412a` were pushed (and so deployed) on their own. Vercel deploys the head of each push; if all three commits went up in one push, only `c52e7c4` (safe) was built.
3. Whether those deployments still exist. **Vercel keeps every deployment reachable at its own immutable URL** unless deleted, so an exposed bundle would still be public now, not only during the 17-minute window.

**Decision / recommendation:** treat it as potentially exposed until checked. In order:
1. In Vercel → Project → Settings → Environment Variables: check whether `VITE_ELEVENLABS_API_KEY` exists or existed.
2. In Vercel → Deployments: look for deployments of `e78f64c` / `1c7412a` (2026-07-11 ~00:05–00:22 +0200). If any exist, search their JS for the key prefix, then delete those deployments.
3. If either check is positive or can't be confirmed: **rotate the ElevenLabs key** in ElevenLabs, update `ELEVENLABS_API_KEY` on the server host, and remove any `VITE_ELEVENLABS_API_KEY` from Vercel.
4. Consider an ElevenLabs usage alert/cap.

**Status:** not rotated, no external system touched (spec §0/§8: no rotation without authorization). The server warns at boot, and `check-config` warns, if the key is still supplied under the `VITE_` name.
