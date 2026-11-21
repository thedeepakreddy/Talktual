# Phase 3 Failure & Fallback Report — VoiceMirror

**Rule:** every VoiceMirror failure falls back to exactly the Phase 2 behaviour (the default voice) and never blocks, delays or alters a turn's words. "Verified" names the test; nothing here is claimed for real devices.

| Failure | Behaviour | Verified by |
| --- | --- | --- |
| Feature flag off (default build) | No controller; `voice` messages ignored and counted; no UI; identical Phase 2 behaviour | flag-off session test; CallRoom "hidden" test; full Phase 2 suites (258/58 tests, 30 Phase 2 browser steps) |
| Peer runs Phase 2 | Their validator drops `t:'voice'`; they hear default voices; our side works normally | `isPeerMessage` default branch (Phase 2 tests) + additive wire design |
| Mic tap can't open (permission, device busy) | `tap-failed` status; no descriptor; capture/ASR unaffected | controller test "tap failure is reported and nothing is held" |
| Capture ends before the tap opens (quick tap) | Tap closed as soon as it opens; nothing analysed | controller race test |
| Too little speech / low confidence | Descriptor withheld (`descriptor-withheld`); default voice; evidence accumulates across turns | controller + analyzer tests; browser E2E "first short turn … still default" |
| Silence, noise-only, very short or 0 dB SNR input | No descriptor (fails closed) | analyzer tests; evaluation robustness table |
| Malformed / hostile voice message | Rejected by the strict validator; the previous good descriptor is kept; counted | 7 validator cases, controller + session tests |
| Peer revokes | Descriptor deleted; queued, not-yet-started segments revert to default | session test; browser E2E |
| Revocation sent during an outage | Re-sent on reconnect | controller test (fixed in Phase 3) |
| Reconnect after a blip | Current descriptor re-sent (the outbox keeps only segments + hello) | controller + session reconnect tests |
| Hint resolver throws | Caught (`safeHints`); segment plays with the default voice | code path in `textToSpeech.ts` (by inspection) |
| Server has VoiceMirror disabled | Hint ignored; fixed voice, speed 1.0 | server test |
| Server enabled without a catalog | Default voice; speed applied | server test |
| Malformed hint to server | 400 before any provider call; client falls back to the browser voice (Phase 2 path), still with pitch/rate hints | server test (6 cases); Phase 2 fallback tests |
| Server TTS fails | Phase 2 fallback to the browser voice; hints applied there (pitch/rate) | Phase 2 playback tests + hint plumbing |
| Mic off / peer away / reconnecting / playback pause in hands-free | Tap closes with capture | session test (3 cases) + hands-free test |
| Session ends any way | All voice data cleared; tap closed; later input ignored | 4 parameterized session tests; controller dispose test |
| Extreme descriptor values | Hints clamped (pitch 0.6–1.6, rate 0.8–1.25, speed 0.8–1.2) | exhaustive clamp test |

## Phase 2 contracts under failure
None of the paths above changes segment IDs, ordering, dedupe, the playback queue bound, the outbox bound, or any Phase 2 state transition. Voice messages travel *beside* segments, never inside them (session test asserts the exact segment field set).

## Known gaps
* Behaviour on real devices with two microphone consumers (Web Speech + tap) is untested (Phase 2 condition C4 still open).
* `ScriptProcessorNode` is deprecated in the Web Audio spec but still broadly supported. An `AudioWorklet` path is a follow-up if browsers remove it.
