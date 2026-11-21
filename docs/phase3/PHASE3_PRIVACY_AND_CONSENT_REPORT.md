# Phase 3 Privacy & Consent Report — VoiceMirror

Voice is biometric-adjacent data. The prototype is designed so that the least sensitive form possible leaves the device, only by explicit choice, for one call at a time.

## 1. Consent model

| Aspect | Implementation | Evidence |
| --- | --- | --- |
| Separate purpose | VoiceMirror has its **own** disclosure and opt-in, distinct from Phase 2 microphone/speech consent. Microphone consent doesn't imply VoiceMirror | `session.voicemirror.test.ts` "off by default", "explicit opt-in" |
| Default | **Off.** Also unavailable unless the build sets `VITE_VOICEMIRROR=1` | flag-off test; CallRoom test "hidden when the feature is off" |
| Explicit action | Toggle → disclosure ("Share your voice style?") → "Share voice style". "Not now" changes nothing | session + CallRoom + browser E2E |
| Scope | **This call only.** Never persisted; a new call starts off | No storage writes in `src/voicemirror` (by inspection; storage is only used by Phase 2 consent) |
| Prerequisite | Analysis needs active capture, which needs Phase 2 microphone consent and a held button / hands-free | "without microphone consent there is no capture, so no analysis either" |
| Disclosure text | States: what changes (pitch level and speaking speed), that it doesn't copy or clone the voice, that analysis is on-device with no recording or upload, that only three coarse settings are sent, that nothing is stored, that it can be turned off and the peer deletes the data, and that it's experimental | `src/voicemirror/disclosure.ts`; CallRoom and E2E assert key phrases |

## 2. Purpose limitation
The descriptor is used for exactly one thing: setting playback parameters for that speaker's translated segments on the other participant's device during the same call. It isn't sent to the server. The server sees only `{register, speed}` inside individual TTS requests when `VOICEMIRROR_ENABLED=1`; it isn't stored, logged or linked to a person.

## 3. Data inventory and retention

| Data | Where | Retention | Deleted by |
| --- | --- | --- | --- |
| Raw microphone audio for analysis | Speaker's browser, one 40 ms frame at a time | none (streaming) | — |
| `VoiceProfile` (pitch Hz, rates) | Speaker's browser, analyzer histogram | until opt-out or call end | `optOut()`, `dispose()` → analyzer dropped |
| Sent descriptor | Speaker's browser (to detect changes) | until opt-out or call end | same |
| Peer descriptor | Listener's browser, memory | until peer revokes or call end | `revoked` message, `dispose()` |
| Render hints | Computed when a segment starts playing; not stored | transient | — |
| Server TTS hint | In-flight request only | none | — |
| Diagnostics | Status enums only (`opt-in`, `descriptor-sent`, `peer-revoked`, …) | ring buffer, page lifetime | — |

**No voice data is retained by default or ever written to storage, logs or the server.**

## 4. Revocation
* **Local:** turning VoiceMirror off closes the tap, discards the analyzer and the sent descriptor, and sends `{t:'voice', revoked:true}`.
* **Remote effect:** the listener deletes the descriptor on receipt. Because hints are resolved **when a segment starts playing**, segments already queued but not started also revert to the default voice. A segment already playing finishes as it started (cutting it off would lose words).
* **Delivery during outages:** the Phase 2 outbox deliberately keeps only segments and hello, so a revocation sent while the transport is down could be lost. Found while writing this report and **fixed**: on every (re)connect the controller re-sends its *current* state, a descriptor if sharing, or `revoked` if it ever shared during this call (test "revocation issued during an outage is re-delivered on reconnect").

Verified by: controller and session revocation tests (local deletion, peer message, queued-segment reversion), a mutation check (keeping the analyzer after opt-out fails 2 tests), and the browser E2E (the next segment after revocation plays at default pitch/rate).

## 5. Session-end cleanup
On every terminal path (leave, peer left, failed connection, dispose), `controller.dispose()` closes the tap and clears the analyzer, sent descriptor and peer descriptor; later input is ignored. Verified by four parameterized tests (`holdsVoiceData()` is false afterwards; tap closed) and by the E2E's server session cleanup check.

## 6. Security boundaries
| Threat | Control |
| --- | --- |
| Malicious peer sends crafted voice data (extra fields, out-of-range values, embeddings) | Strict validator: exact key set, enums, integer steps, ranges; rejected messages are counted and never applied (7 validator cases + session test) |
| Peer makes the listener's playback extreme | Render hints are clamped for **every** possible descriptor (exhaustive test over all 23×10 values) |
| Client selects arbitrary provider voices (e.g. someone's cloned voice ID) | Server never accepts voice IDs from clients; IDs come only from server config; hint has a strict schema (6 malformed cases → 400, no upstream call) |
| Server misconfiguration | `check-config` validates `VOICEMIRROR_ENABLED` and the catalog JSON (IDs `[A-Za-z0-9]{8,64}`, registers only) |
| Leakage via diagnostics | Controller emits status enums only; session test asserts exact keys and no voice-field keys; a mutation that adds a voice value is caught |
| Re-identification from the descriptor | 2-semitone × 3 × 10 = coarse classes; many people share each descriptor. **Not** an anonymity guarantee: combined with other context it narrows who's speaking, which is why it's opt-in and per call |

## 7. Disclosure of remote processing
VoiceMirror adds **no** new remote processor. Analysis is local; the descriptor goes peer-to-peer (or via the Phase 2 relay, which forwards it without storing it). When server TTS is used, the provider receives the same text as before plus a speed value and the server-chosen voice ID.

## 8. Residual privacy risks
| Risk | Severity | Mitigation / next step |
| --- | --- | --- |
| A second microphone consumer on Web Speech browsers (the tap) while the button is held | P2 | Same gate as capture; tracks disabled after; real-device check needed |
| Revocation sent during an outage reaches the peer only after reconnect (or never, if the call ends first, in which case the peer's copy is deleted at call end anyway) | P3 | Re-sent on reconnect (fixed in Phase 3) |
| Descriptor plus context could help re-identify within a small group | P3 | Opt-in, per call, coarse; don't add precision without re-assessment |
| Higher tiers (cloning) would create a reusable synthetic voice | P1 if built | Not built; requires consent verification, retention/deletion design and approval (Research §5) |
