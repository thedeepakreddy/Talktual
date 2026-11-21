# Phase 2 Failure & Recovery Report

Each row states the behaviour **and how it was verified**. "Unit/engine" = fakes in Vitest; "server" = real server with ws clients; "browser" = Chromium E2E with real server. Nothing here is claimed for real devices or real provider outages.

## 1. Failure matrix

| Failure | Behaviour | Duplicate / revived work? | Verified by |
| --- | --- | --- | --- |
| **Microphone permission denied** (Web Speech `not-allowed` or `NotAllowedError`) | Capture stopped, `permission=denied`, sticky notice, mic shown off; listening to peer continues; no prompt loop. Mic button → re-checks permission; if now granted, back to ready | — | engine (3 tests), adapter (both engines) |
| **Microphone missing** (`audio-capture`, `NotFoundError`) | `capture-unavailable` notice, stopped; mic button retries | — | adapter tests |
| **Device removed mid-call** (track `ended`) | Capture stopped, notice, mic off; mic on + press re-acquires | — | capture-adapter test, engine test, **view test** (the UI recovery path) |
| **Signaling socket lost, P2P up** | Call continues on P2P; signaling reconnects in the background with the resume token | No — same seat, no resend needed | transport unit test; browser smoke ("recovery" step, Direct) |
| **Signaling socket lost, relay only** | Host: `reconnecting` (+ "Reconnecting…"), backoff 0.5→8 s; peer: "lost connection — waiting…"; capture paused while alone; on resume both get `ready{resumed}`, P2P renegotiated, held segments resent | **No:** next turn heard exactly once; earlier turn not replayed (receiver dedupes by segment id) | transport tests (6), engine tests, **browser recovery A** (relay-only Chromium) |
| **Outage longer than the seat hold (20 s)** | Peer: `peer-left` after the grace → returns home. Returning user: resume refused → "The call ended while you were offline." (failed, no retries) | — | server test, transport test, **browser recovery B** (offline emulation) |
| **Reconnect attempts exhausted** (server unreachable) | After 6 attempts (~23.5 s): `disconnected` → failed with "Disconnected. Leave and start a new channel" | — | transport test |
| **Server restart / stale session** | Token unknown → `resume_failed` → failed message | — | server test (wrong/reused/non-string tokens) |
| **Intentional leave** | `leave` frame → peer told immediately → both sessions cleaned | — | server test, browser smoke |
| **Phase 1 client leaves (no `leave` frame)** | Peer sees "lost connection" then `peer-left` after 20 s | — | server tests (grace expiry) |
| **Translation provider timeout** | Failed after one 10 s budget (not retried — the send path is serialized); original sent marked `untranslated`; both users told | — | translate tests (timeout not retried), engine test |
| **Translation provider error / malformed** | Server returns 502; client `failed` with code; untranslated path | — | server + client tests, browser smoke |
| **Rate limited (429)** | `rate-limited`; not retried; "Too many requests" notice; untranslated path | — | translate + engine tests |
| **Transient network error to API** | One retry after 300 ms; then failed | — | translate tests |
| **TTS server failure** | Browser voice fallback (`playback.status` engine `browser`) | — | Phase 1 + playback tests; browser runs use this path (no TTS key) |
| **No voice can play** | `playback.status failed`; text still shown in transcript; call continues | — | playback tests (synthesis error; no synthesis API), engine test |
| **Playback backlog** | Queue ≤ 12; oldest waiting dropped (`dropped`) | — | playback test |
| **Malformed / oversized / wrong-version peer message** | Dropped before reaching the engine; counted | — | event/transport validation tests (Phase 1 + v2 version gate) |
| **Malformed signaling / frames** | Server ignores/closes (Phase 1 DEF-S01..S05 still covered) | — | server tests |
| **Unsupported capability** (no Web Speech + no server STT; insecure context) | Notice up front, talk disabled, no consent prompt, listening works | — | engine tests, **browser compat 2** |
| **Late results after stop** | Recognizer callbacks after stop/abort are dropped by the session; server-STT abort discards upload result | **No** | privacy test (stale guard), adapter tests |
| **User stop during each stage** | During capture: aborted. During translation: aborted on leave; not sent. During playback: cancelled, queue dropped | **No** revived work | lifecycle, privacy and playback tests |
| **Session teardown** | Exactly once: transport closed (with `leave`), recognizer aborted + disposed, capture released, playback cancelled, timers cleared, no events afterwards | — | lifecycle tests |
| **Invalid config at startup** | Server refuses to start, names the variable (never its value) | — | config tests (spawned server) |

## 2. Guarantees and limits

**Guaranteed by design and tests**
* Recovery never re-plays a segment the receiver already accepted (id dedupe, 500-id window).
* No captured audio is processed after a stop.
* Terminal states are final: late transport events and translations are ignored.

**Not guaranteed / known limits**
* A segment produced locally *while reconnecting* can't be produced: talking is disabled until reconnected. The outbox only covers translations that finish during a drop.
* Resends older than the 500-id dedupe window would be accepted (not reachable in practice: the outbox holds ≤ 20).
* Seat hold (20 s) and client attempts (~23.5 s) are tuned for blips, not long outages. Longer outages end the call by design.
* Hands-free echo protection depends on playback events. Browser `speechSynthesis` end events are unreliable, so a capped wait is used and capture may resume slightly early or late.
* None of this was exercised on real mobile networks, real provider outages or real devices.
