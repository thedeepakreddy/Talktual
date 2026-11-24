# Privacy, data lifecycle and voice governance (foundations)

Phase 1 **represents** privacy decisions and enforces them in observable
output. It persists no user data of any kind and processes no media.

## Data classes

`DataClass` (ordered): `public` < `operational` < `personal` < `content` <
`biometric` < `secret`.

* **operational** — ids, states, counters, durations, capability names.
* **personal** — data about a person that is not communication content (e.g.
  language preference, account or device identifiers).
* **content** — transcripts, translations, messages.
* **biometric** — raw audio, voice prints, speaker embeddings.
* **secret** — credentials. Never visible under any policy.

## Where classification is enforced

| Surface | Rule |
| --- | --- |
| Logs/metrics (`ObservabilitySink`) | the runtime redacts every field above `RuntimeConfig.redaction.max_visible` (default `operational`) before the sink sees it; module observations pass through the same redaction |
| `CeError` `Display`/`Debug` | context above `operational` is redacted; sources never rendered |
| `RuntimeError` events | carry `ErrorReport`, redacted with the runtime policy |
| Event envelopes | every envelope declares `privacy`; runtime events are `operational`; module events declare their own class; the runtime never logs module attributes |
| Session config | language metadata is never logged; no free-form metadata field exists |
| Configuration | secret-looking keys must use `SecretRef` references; literals are rejected at `build()` and `start()` |

Tests: `tests/observability.rs` drives a module that logs, emits and fails
with a transcript and a token, and asserts neither reaches logs or error
reports (and that a permissive policy still hides secrets).

## Retention and consent representations

* `RetentionPolicy`: `ephemeral` (default), `session`, `bounded{max_ms}`.
  There is deliberately no "forever".
* `ConsentPurpose`: `capture`, `remote_processing`, `retention`,
  `voice_profile`. `ConsentState`: `unknown` (= not granted), `granted{at}`,
  `denied`, `revoked{at}`.
* Sessions record consent per purpose; capabilities declare
  `PrivacyFlags { processes, requires_consent, retention, leaves_device }`
  where `None` means unknown, never "safe".
* `DataErasure` is the deletion interface any future component that retains
  personal, content or biometric data must implement.

**Phase 2 enforces a capture gate** (ADR-0013): audio capture requires the
session's `capture` consent (optionally not older than
`audio.consent_max_age_ms`) **and** platform permission. It is checked
before any device opens, at every start, and continuously; revocation stops
data flow synchronously and releases the device. Generic gating of other
capabilities on `requires_consent` remains future work (no other capability
processes data yet).

### Audio data (Phase 2)

* Captured samples are **biometric** data. CE keeps them only in bounded
  in-memory rings and queues, hands them to the host, and never logs,
  persists, uploads or puts them in events.
* Device names are **personal** data (they can name people): returned in the
  catalog to the host, never in events or logs; events use opaque device ids.
* On revocation, queued blocks are purged, not delivered.

## Audio analysis (Phase 3)

Analysis describes the signal, never the person (ADR-0017): no identity,
emotion, health, personality or other trait inference; a closed set of six
signal conditions; overlap detection not claimed. Analysis events are
`Operational` and contain no samples. Analysis is local and ephemeral and
runs only on authorized capture streams under `Capture` consent; revocation
ends it and discards queued audio.

## Speech and language (Phase 4)

Transcript events are classified `Content` and language observations
`Personal` on the event envelope (`SpeechEvent::data_class`); everything
else is `Operational` (ADR-0022). Raw audio and transcript text are never
logged: runtime observations carry stream ids, adapter ids, configuration
fingerprints and end reasons only. Nothing is persisted or uploaded.
Language observations describe the language of the text, never the
speaker — no nationality, ethnicity, identity, location, emotion or health
inference. Recognition runs only on authorized capture streams; revocation
cancels the open utterance and discards queued audio.

## Voice governance (no implementation)

CE implements no voice enrollment, cloning, conversion or TTS. Before any
voice-profile capability may be added, it must have: explicit, per-purpose
consent (`voice_profile`) from the voice's owner; authorization and access
control; revocation that triggers `DataErasure`; provenance for generated
audio; abuse prevention; and review. Nothing in Phase 1 certifies compliance
with any regulation.
