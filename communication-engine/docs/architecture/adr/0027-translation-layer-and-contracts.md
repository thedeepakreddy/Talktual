# ADR-0027 — Translation layer and contracts

Status: Accepted (Phase 6) · Date: 2026-11-24

## Context
Phase 6 adds translation of Phase 4 transcripts, optionally informed by
Phase 5 observations, as an engine capability independent of Talktual.
Transcripts and observations must stay immutable; translations must be
derived, revision-linked, bounded and replaceable.

## Decision
* **Contracts in `ce-core::translation`** (no adapter types leak):
  `TranslationConfig` (target, `SourceLanguagePolicy`, `PartialPolicy`,
  terminology id, `ContextPolicy`, `TranslationPrivacy`, `TranslationBudget`,
  explicit `fallback`; validated, fingerprinted), `TranslationCapabilities`
  (directed pairs, locality, detection, partial input, incremental output,
  context, terminology, placeholders, limits, determinism, score
  semantics, model), `TranslationResult` (sequence, `TranslationState`,
  `TranslationSource` = stream, utterance, revision, revision status,
  characters, frame and time range; `SourceLanguageRef` with provenance;
  target; text; sorted `UncertaintyFlag`s; optional score with semantics;
  `TranslationProvenance`; timing), `TranslationEvent` (started, result,
  withdrawn, unavailable, input_rejected, degraded, failed, stopped),
  `TranslationStatus`, `TerminologyProfile`.
* **Logic in a new crate `ce-translate`** (no new third-party dependency):
  `adapter` (trait + factory), `memory`, `scripted`, `fidelity`,
  `processor`, `worker`, `eval`. One more crate is justified like
  `ce-semantic`: the runtime must not depend on adapter code paths, and
  the processor/worker are testable without the runtime.
* **Runtime** (`runtime/core/translation.rs`) owns workers; Phase 4
  transcripts and Phase 5 observation events are forwarded as read-only
  clones with a non-blocking push; results are published as
  `EventPayload::Translation` (topic `Translation`) in the stream's
  session; results are `Content`, everything else `Operational`.
* **Facade**: `start_translation`, `stop_translation`,
  `reset_translation`, `translation_status`, `translation_adapters`;
  builder `translation_adapter`, `terminology_profile`. Serializable
  commands mirror them. No default adapter is registered (a translation
  memory needs data; there is no bundled MT model).
* **Lifecycle**: discovery (`capabilities`, never loads) → create → `load`
  on the worker thread → translate* → reset/drop. Supervised worker:
  `catch_unwind`, heartbeat stall detection, bounded coalescing queue,
  stop/reset flags; `SourceEnded`/`Shutdown` drain, `Stopped`/`Failed`
  discard. Session close and shutdown stop translations after
  recognitions and semantics, before streams close.

## Consequences
A real MT adapter (Phase 6 stage F, blocked on approval) plugs in behind
`TranslationFactory` without changing consumers. Translation never runs on
audio callbacks or the control plane (tested).
