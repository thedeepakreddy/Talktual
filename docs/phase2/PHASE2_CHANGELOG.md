# Phase 2 Changelog — Communication Engine

Branch `claude/kind-archimedes-n6y11p`, restarted from `main` @ `89ef81a` (Phase 1 merged). Commits in order:

| Commit | Milestone | Summary |
| --- | --- | --- |
| `2f2b56f` | M2.0 | Architecture + plan; shared E2E harness; baseline latency and privacy probes |
| `e6afb4c` | M2.1 | Engine contracts, state machines, session core; transport resume/leave/outbox; server seat hold + resume tokens |
| `517bea0` | M2.2 | Bounded, status-reporting `PlaybackController`; PCM backpressure; capture permission + device-lost |
| `bce2b3c` | M2.3+M2.4 | ASR contract (P1 fix at adapter level), normalized translation adapter, full pipeline in `CommunicationSession`, `CallRoom` as a view, consent UI |
| `7f2b303` | M2.5 | Privacy tests and stale-result guard; relay-only recovery E2E; config validator (`check-config`) |
| `8628565` | M2.6 | Emulated compatibility E2E, latency + soak measurements |
| *(next)* | M2.6 | Mic recovery fix after device loss / permission denial; no retry on timeouts |
| *(next)* | M2.6 | Reports and README |

## Significant changes and rationale

### New: `src/engine/` (React-free)
| File | What | Why |
| --- | --- | --- |
| `CommunicationSession.ts` | Owns one call: lifecycle, pipeline, privacy gates, cancellation, teardown-once, diagnostics; `getSnapshot/subscribe` + commands | Phase 1's `CallRoom.tsx` *was* the engine (≈800 lines of refs/effects); untestable without mocking every module and not reusable by a future UI (spec A, K) |
| `sessionMachine.ts` | Explicit `CallPhase` and `Activity` transition tables | Spec B; illegal transitions are refused + counted, terminal states have no exits (exhaustively tested) |
| `events.ts`, `eventBus.ts` | Versioned internal events (v1), versioned peer wire protocol (v1, accepts unversioned Phase 1 peers), validators | Spec G |
| `segments.ts` | Segment id/seq, bounded record store and dedupe | Spec E/G; resend after reconnect is spoken once |
| `privacy.ts` | Listening modes, versioned per-route consent, disclosure text | Spec J, Phase 1 P1 |
| `capabilities.ts` | Browser + server capabilities → features + degradation reasons | Spec H (kept outside the state machine) |
| `diagnostics.ts` | Allowlist-redacted ring buffer, counters, latency samples | Spec L |
| `ports.ts`, `defaultDeps.ts`, `useCommunicationSession.ts` | Adapter interfaces, production wiring, React seam | Testability; stable UI integration point |

### Changed adapters (kept, strengthened in place)
| File | Change | Why |
| --- | --- | --- |
| `lib/speechToText.ts` | New `AsrAdapter` contract: listens only between `start()` and `stop()`/`abort()`; PTT stop = 450 ms tail → flush → end (no restart); continuous restarts while active; coded errors; `RecognizedSegment` with timestamps; server-STT per-push recorder, 60 s cap, mic gated between pushes, abort discards uploads | **P1**: Phase 1 armed Web Speech for the whole call. API `createRecognizer/arm/hold/release` → `createAsr/start/stop/abort` (internal API; only `CallRoom` used it) |
| `lib/translate.ts` | `translateSegment()` normalized result (translated/passthrough/failed/cancelled, attempts, error code incl. `rate-limited`); one retry for network errors only | Spec E; never fake success; bounded retries without stretching serialized turns |
| `lib/textToSpeech.ts` | `PlaybackController`: bounded queue (12, drops oldest waiting), per-segment status incl. `failed` (Phase 1 reported success when no voice could play), PCM schedule-ahead ≤ 10 s with reader backpressure; Phase 1 functions kept as wrappers | Spec C/F |
| `lib/audio.ts` | `micPermission()`, device-lost notifications, `captureAdapter` | Spec I |
| `lib/webrtc.ts` | Wire version stamp; resume token + reconnect with backoff (0.5→8 s, 6 attempts) + P2P renegotiation; intentional `leave`; peer-away/back; bounded outbox; no relaying into an empty room | Spec I |
| `components/CallRoom.tsx` | 809 → 543 lines; pure view over the hook; adds hands-free control + indicator, disclosure link, reconnect/failed states | Spec K; visual design otherwise unchanged |
| `components/ConsentSheet.tsx` | New disclosure/consent dialog | Spec J |

### Server (`signaling-server/server.js`)
* **Seats + resume:** an unexpected socket drop holds the seat for `RESUME_GRACE_MS` (20 s); reclaimable only with a 128-bit per-seat token; `leave` frees immediately; lobbies never hold seats; `peer-away`/`peer-back`/`ready{resumed}`.
* **`validateEnv()` + `npm run check-config [-- --production]`:** malformed values stop startup; missing production settings warn at runtime and fail the explicit production check. Secrets never printed.
* Phase 1 behaviour otherwise unchanged.

### Behaviour changes users will notice
1. **First use asks for consent** before any microphone use, naming where audio goes.
2. **Push-to-talk now starts recognition on press** (Phase 1 pre-armed it for the whole call). Real-device first-word latency impact is unmeasured (see Performance report).
3. **Hands-free mode** (opt-in, Web Speech browsers only) with a persistent banner.
4. Network blips reconnect instead of ending the call; the other side sees "lost connection — waiting…".
5. After device loss / permission denial, the mic button is the retry path.

### Compatibility with Phase 1 deployments
* Phase 1 peer ↔ Phase 2 peer: interoperable (wire v1 superset).
* Phase 1 client ↔ Phase 2 server: works; a Phase 1 client never sends `leave`, so its peer learns it left after the 20 s grace instead of immediately.
* Phase 2 client ↔ Phase 1 server: works; no resume token is issued, so reconnect is disabled (drop = disconnected, as in Phase 1).

### Found and fixed during Phase 2 verification
* Mic toggle was a no-op after device loss / permission denial → user stuck until leaving the call (engine + UI regression tests added).
* Timeout retries would have doubled stalls on the serialized send path.
* E2E tests that picked the "host" by server role were wrong when the guest joined first (passed by luck once); now each page cuts its own sockets.
* Config gate initially refused to start under `NODE_ENV=production` without an allow-list — would have risked taking a live deployment down; now warning-only at runtime.
