# Security & Privacy Report — Talktual Phase 1

## 1. Threat model

**Assets:** provider API keys (ElevenLabs, Gemini, DeepL) and the quota/billing behind them; conversation content (audio, transcripts, translations); call integrity (only the two intended people in a call); server availability.

**Actors:** (a) an anonymous Internet client that can reach the server; (b) another website a user visits; (c) the *other* participant of a call (untrusted input); (d) a third party who sees or guesses a room code.

**Trust boundaries:** browser ↔ Talktual server (HTTP + WebSocket); browser ↔ browser (WebRTC data channel or server relay); Talktual server ↔ providers; browser ↔ browser-vendor speech service (Web Speech).

| # | Scenario | Before Phase 1 | Now |
| --- | --- | --- | --- |
| T1 | Anonymous client crashes the server (DoS) | **One malformed frame killed the process** (DEF-S01) | Fixed; tested with invalid UTF-8, oversized frames, raw garbage |
| T2 | Memory/bandwidth exhaustion via signaling | 100 MiB frames relayed (DEF-S02) | 64 KiB cap |
| T3 | Spend the owner's provider quota from another site or script | Open: CORS `*`, no origin check, no rate limit, no length caps (DEF-S06, S08) | Length/type caps always on. Origin allowlist + rate limit **available but off until configured** (SEC-R2) |
| T4 | Join a call you weren't invited to | Missing session id paired strangers (DEF-S03); ghost seats (DEF-S04); codes from `Math.random`, sometimes short (DEF-C05) | Validated ids, CSPRNG 6-char codes, ghost seats dropped. Knowledge of the code still = access (SEC-R5) |
| T5 | Malicious peer sends crafted messages | Unvalidated; could inject server-type events (`peer-left`, `expired`) and crash handlers (DEF-S05, C06) | Relay allowlist; schema + length validation client-side; React escaping verified with HTML payloads |
| T6 | Credential leakage to the browser | Keys server-side only (verified); historic exposure window (SEC-08) | Bundle scan clean; warning if the key is supplied as `VITE_*` |
| T7 | Information disclosure | HTML stack traces with absolute paths (DEF-S07); raw provider errors to clients (DEF-S10) | Generic JSON errors; provider detail only in server logs |
| T8 | Injection | Language names interpolated into the Gemini prompt unvalidated | Languages restricted to the supported set. Utterance text still goes into the prompt (SEC-R6) |
| T9 | SSRF / unsafe file handling | No URL-fetch feature; STT upload kept in memory only | Upload ≤10 MB, 1 file, mimetype validated; filename not trusted |
| T10 | Privacy: audio captured when the user isn't talking | Web Speech armed for the whole call (SEC-R1); mic could stay on after leaving (DEF-C03) | DEF-C03 fixed. SEC-R1 **open — product decision** |

## 2. Findings and remediation

Defect details are in [DEFECT_REGISTER](DEFECT_REGISTER.md).

| ID | Sev | Finding | Status |
| --- | --- | --- | --- |
| SEC-01 | P0 | Server crash on malformed WS frame (DEF-S01) | ✅ Fixed |
| SEC-02 | P1 | Unbounded WS frames (DEF-S02) | ✅ Fixed |
| SEC-03 | P1 | Session pairing flaws (DEF-S03, S04) | ✅ Fixed |
| SEC-04 | P2 | Relay of arbitrary types / unvalidated peer input (DEF-S05, C06) | ✅ Fixed |
| SEC-05 | P1 | Open API proxy, no validation/limits (DEF-S06, S08) | ⚠️ Validation fixed; origin/rate limit need config |
| SEC-06 | P2 | Stack trace / upstream error disclosure (DEF-S07, S10) | ✅ Fixed |
| SEC-07 | P1 | Vulnerable dependencies: root 11 (1 critical: `proxy-addr` via unused `express`), server 4 (`multer` high, direct) | ✅ `npm audit` 0 / 0 |
| SEC-08 | P1* | **Historic key exposure window.** Between commits `e78f64c` and `c52e7c4` (both 2026-07-11) the frontend read `import.meta.env.VITE_ELEVENLABS_API_KEY` and called ElevenLabs from the browser, so any build deployed from that window contained the key in public JavaScript. No key value is in the Git history (scanned). | ⚠️ **Action: if a build from that window was ever deployed, rotate the ElevenLabs key.** I cannot verify deployment history. |
| SEC-09 | P2 | Mic left live after leaving (DEF-C03) | ✅ Fixed |
| SEC-10 | P3 | Weak room-code RNG (DEF-C05) | ✅ Fixed |

\*Severity assumes the window was deployed.

### Checks performed
* **Secrets in Git:** full-history search for `.env` files (only `.env.example` ever added) and key-shaped strings (Google `AIza…`, ElevenLabs, DeepL `:fx`, `xi-api-key` literals): none found.
* **Frontend bundle:** `dist/assets/*.js` contains no provider names, key variable names, provider hosts or `VITE_*` names other than inlined values. The only `VITE_*` inputs are the signaling URL and optional TURN settings.
* **Headers:** API responses carry `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`; `X-Powered-By` removed (tested).
* **Logs:** server logs no utterance text (asserted in tests), no session ids, no IPs, no keys. Provider error bodies are truncated to 200–300 chars in logs.
* **XSS:** all peer-supplied strings are rendered as React text nodes; no `dangerouslySetInnerHTML` anywhere; test renders `<img onerror>`/`<b>` payloads and asserts no elements are created.

## 3. Residual risk register

| ID | Sev | Risk | Mitigation / recommendation |
| --- | --- | --- | --- |
| SEC-R1 | **P1 (privacy)** | **Always-on recognition.** On Web Speech browsers the recognizer runs continuously for the whole call; Chrome sends that audio to Google's speech service even while the user is *not* holding the button. Results are discarded locally, but the audio has left the device. The UI ("Hold to talk", mic toggle) does not communicate this. | **Owner decision required.** Options: (a) start recognition on press and accept ~200–500 ms first-word latency; (b) keep the design and disclose it clearly in-app; (c) keep it but stop recognition after N seconds idle. Not changed in Phase 1 because it alters the product's latency behaviour. The existing mic-off toggle does stop recognition. |
| SEC-R2 | P1 | Origin allowlist and rate limiting are off by default. | Set `ALLOWED_ORIGINS=<frontend origin>`, `RATE_LIMIT_PER_MINUTE` (e.g. 600), and `TRUST_PROXY=1` on Render-style hosts. Server warns at boot. Note: origin checks stop other *websites*, not scripts; the rate limit and provider-side spend caps are the real backstop. |
| SEC-R3 | P1* | Historic key exposure (SEC-08). | Rotate if applicable. |
| SEC-R4 | P2 | TURN credentials, if configured, are `VITE_*` and therefore public in the bundle. | Use short-lived TURN credentials minted by the server (feature work). |
| SEC-R5 | P2 | No authentication; possession of a 6-char code (≈2.2 × 10⁹ space) = access. A third party who sees the QR before the intended guest scans it can take the seat. | Acceptable for the current product; Phase 2 could add host approval or one-time tokens. Rate limiting (SEC-R2) also limits WS brute-force only indirectly — WS joins are not rate-limited. |
| SEC-R6 | P3 | Prompt injection via spoken text into Gemini. Impact is limited to the translation the *speaker's own peer* receives. | Accept; the speaker already controls what the peer hears. |
| SEC-R7 | P3 | No CSP / security headers on the static frontend (Vercel). | Add headers in `vercel.json` once the exact origins (fonts, signaling host) are confirmed. Not changed: a wrong CSP breaks the deployed app and I can't verify the production host. |
| SEC-R8 | P3 | WebRTC exposes both peers' IP addresses to each other and to the STUN provider (Google). | Inherent to P2P; document in a privacy notice. Relay-only mode would avoid it at a latency cost. |

## 4. Data flows and retention

| Data | Recipient | Retention by Talktual |
| --- | --- | --- |
| Microphone audio — Web Speech browsers | Browser vendor's speech service, continuously during a call (SEC-R1) | none |
| Microphone audio — other browsers | Talktual server → ElevenLabs Scribe, once per push | In memory for the request only |
| Transcripts | Server → Gemini / DeepL / MyMemory; the peer | Server translation cache: ≤2000 phrase pairs in memory, no TTL, lost on restart. Client: ≤500 per tab |
| Translations | Server → ElevenLabs TTS; the peer | not stored |
| Display name, language | The peer | not stored |
| Operator email (`MYMEMORY_EMAIL`) | MyMemory (query parameter) | — |

Recommendation (not implemented): give the server translation cache a TTL (e.g. 10 min) if conversation-text retention in memory is a concern; it currently exists only to save provider calls.
