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

Phase 1 does not yet *gate* capability use on session consent: no capability
processes data yet. Phase 2 must enforce `requires_consent` against the
session's consent before any capture or remote processing starts.

## Voice governance (no implementation)

CE implements no voice enrollment, cloning, conversion or TTS. Before any
voice-profile capability may be added, it must have: explicit, per-purpose
consent (`voice_profile`) from the voice's owner; authorization and access
control; revocation that triggers `DataErasure`; provenance for generated
audio; abuse prevention; and review. Nothing in Phase 1 certifies compliance
with any regulation.
