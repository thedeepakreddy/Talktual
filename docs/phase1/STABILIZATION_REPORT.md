# Stabilization Report — Talktual Phase 1

**Branch:** `claude/kind-archimedes-n6y11p` · **Baseline:** `34a8400` · **Date:** 2026-11-21
**Not done (by instruction):** no merge, no deployment, no provider calls, no Talktual 2.0 work.

## 1. Executive summary

**Inspected:** the full repository (client ~2,000 LOC, server 467 LOC, config, deps, 20 commits of history). Every flow was traced: lobby → signaling → WebRTC/relay → STT → translation → TTS → playback → teardown.

**Found:** 23 confirmed defects with reproducible evidence — 1 P0, 9 P1, 9 P2, 4 P3 — plus 8 recorded observations. The most serious: **any anonymous client could crash the server with a 6-byte frame** (dropping every call), the API proxy was **open to any website** with no input limits (quota/billing abuse), **translation failures were disguised as translations and cached**, **barge-in didn't stop queued audio**, and a stalled provider could **freeze a call's outgoing speech indefinitely**.

**Repaired:** 22 of 23 fully; the 23rd (open proxy) has its mitigation shipped but **switched off by default** to avoid breaking the unknown production setup — it must be configured. No rewrite: one structural change (server exported as a factory so it can be tested); the UI is visually unchanged apart from new notices.

**Verified:** 0 → **112 automated tests** (75 client, 37 server) + a **13-step two-browser E2E** in Chromium, all green and repeatable. The 7 client defect tests that could run against baseline code fail there and pass now. `npm audit` 0/0 (from 11 + 4).

**Remaining:** one open **privacy decision** (always-on speech recognition), production configuration (origins, rate limit), a possible **key rotation** for a historic exposure window, and **no real-device / non-Chromium testing** yet.

## 2. Release gates

| Gate | Status | Evidence |
| --- | --- | --- |
| **G0 Baseline** | ✅ Pass | Baseline commit, clean tree, install/lint/build results, audit counts and 6 live-probe failures recorded (ENGINEERING_AUDIT §1). |
| **G1 Core functionality** | ✅ Pass *in the available environment* | E2E in Chromium: start/join, both directions translated and spoken, multi-segment ordering, language switch, transcript, leave. Speech APIs and providers faked — real STT/TTS/provider behaviour unverified. |
| **G2 Reliability** | ✅ Pass, with one documented gap | Session lifecycle, expiry, cancellation (barge-in, aborts), cleanup (mic, sockets, audio nodes, sessions → 0), provider failure/timeout paths all tested. **Gap:** no auto-reconnect (OBS-03) — now reported as "Disconnected" instead of failing silently. |
| **G3 Security** | ⚠️ Conditional | P0 and all fixable high findings fixed; no new high/critical introduced; audits clean; secrets reviewed. **Open:** SEC-R1 (privacy decision), SEC-R2 (prod config), SEC-R3 (key rotation if the July build was deployed). |
| **G4 Quality checks** | ✅ Pass | `npm run test:all` exit 0 (lint, 75 client, 37 server, build). Only exception: pre-existing Vite >500 kB chunk *warning*. |
| **G5 Compatibility** | ⚠️ Partial | Chromium 157 headless: pass. Firefox, Safari/WebKit, iOS, Android, headed desktop browsers: **unverified** (not available here). |
| **G6 Documentation** | ✅ Pass | 8 reports in `docs/phase1/`, README (verified commands, test matrix, configuration, limitations, troubleshooting), `.env.example` updated. |
| **G7 Stabilization** | ✅ Pass | Final diff reviewed (§4), full regression suite re-run on the final tree, residual-risk register complete (SECURITY_REPORT §3, §5 below). |

## 3. Change summary

| Area | Changes |
| --- | --- |
| Server | Crash fix; frame/payload/text/upload limits; join/session validation; relay allowlist; JSON errors + headers; provider timeouts and upstream cancellation; explicit translation failure codes; opt-in origin allowlist and rate limiting; `createServer()` factory; deps patched. |
| Client | Cancellable TTS queue; honest translation failures end-to-end; mic release race; join timeout/cleanup; CSPRNG room codes; inbound message validation; transport-loss detection; Scribe races; STT-unavailable notice; accessibility semantics; dead code and 10 unused deps removed. |
| Tests / tooling | Vitest + Testing Library; `node:test` server suite; Playwright E2E; perf benchmark; `test`, `test:server`, `test:e2e`, `test:all` scripts. |
| Docs | README, `.env.example`, `docs/phase1/*`. |

Diff (excluding lockfiles): 43 files; see `git diff 34a8400 --stat`.

## 4. Final diff review

Reviewed `git diff 34a8400..HEAD` for scope creep, secrets, accidental files and unexplained changes:
* **Scope:** no new product features, modes or redesigns. New user-visible elements are limited to failure notices (untranslated turn, STT unavailable, join timeout/error messages) and a transcript label — each tied to a defect.
* **Secrets:** none added; test fixtures use placeholder keys (`"k"`, `"el-secret"`, `"gm-secret-test"`) only to assert they are *not* leaked.
* **Accidental files:** none; `e2e/.dist/` (E2E build output) is git-ignored; no `.env`, logs or build output committed.
* **Behavioural changes for existing deployments:** translate failures now non-2xx (older clients degrade the same way as before); TTS upstream errors → 502; JSON body limit 256 kB → 64 kB; session ids must match `[A-Z0-9]{4,12}(_signal)?` (all codes the app generates do); peer messages over the length caps are dropped; new optional `untranslated` field in the peer protocol (ignored by old clients).

## 5. Known issues and residual risks

| ID | Sev | Issue | Practical mitigation |
| --- | --- | --- | --- |
| SEC-R1 | P1 privacy | Web Speech recognizer listens for the whole call; audio goes to the browser vendor even when not holding PTT. | Decide: start-on-press (+~200–500 ms first word) vs. disclose vs. idle timeout. Mic-off toggle stops it today. |
| SEC-R2 | P1 | Origin allowlist / rate limit off by default. | Set `ALLOWED_ORIGINS`, `RATE_LIMIT_PER_MINUTE=600`, `TRUST_PROXY=1` in production; set provider-side spend caps. |
| SEC-R3 | P1* | ElevenLabs key may have shipped in a July 2026 build. | Rotate the key if that build was deployed. |
| G5 | P2 | No real-device or non-Chromium testing. | Run COMPATIBILITY_REPORT §5 manual plan. |
| OBS-03 | P2 | No auto-reconnect after network change. | User sees "Disconnected" and restarts; Phase 2 candidate. |
| SEC-R4/R5 | P2 | Public TURN creds if set; code-only room access. | Ephemeral TURN creds; host approval (Phase 2). |
| OBS-02 | P3 | DeepL `NO`/`EN`/`PT` codes. | Falls back to MyMemory; verify with a DeepL key. |
| OBS-04/05/06 | P3 | 575 kB bundle; `--text-3` contrast 2.52:1; unbounded transcript. | See PERFORMANCE_REPORT §5. |
| SEC-R6/R7/R8 | P3 | Prompt injection (self-limited), no frontend CSP, WebRTC IP exposure. | See SECURITY_REPORT §3. |

## 6. Recommendation

**Talktual is ready to move to Phase 2 planning and development — conditionally.** The codebase now has a reproducible baseline, a working regression suite and CI-ready commands, and every confirmed defect that Phase 1 could safely repair is repaired and verified. Phase 2 work can build on this branch with confidence that regressions in the core flows will be caught.

**Before any production release** (of this branch or Phase 2), close these four items — none requires code from me, but they need an owner's decision or access I don't have:
1. Decide SEC-R1 (always-on recognition) — the only open item that affects users' privacy expectations.
2. Configure `ALLOWED_ORIGINS`, `RATE_LIMIT_PER_MINUTE`, `TRUST_PROXY` (SEC-R2).
3. Rotate the ElevenLabs key if the July 2026 browser-key build was ever deployed (SEC-R3).
4. Run the manual device/browser plan (iOS Safari, Android Chrome, Firefox, two real networks).

## 7. Next steps

1. Owner review of this branch; merge when satisfied (I have not merged or opened a PR).
2. Add a CI workflow running `npm ci && (cd signaling-server && npm ci) && npm run test:all` (commands are ready; no workflow file was added to avoid changing remote repository behaviour without approval).
3. Execute the manual compatibility plan and record results in COMPATIBILITY_REPORT.
4. Measure real release-to-first-audio latency on devices with real providers (PERFORMANCE_REPORT §5.1).
