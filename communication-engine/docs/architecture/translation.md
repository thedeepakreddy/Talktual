# Translation intelligence (Phase 6)

Phase 6 turns Phase 4 transcript revisions into target-language
**translation results** — derived objects linked to the exact source
revision — using replaceable adapters, bounded execution and explicit
uncertainty. Transcripts, language observations and Phase 5 observations
are never modified. A translation is not an instruction to act on.

Decisions: ADR-0027 (layer, contracts), ADR-0028 (adapters, selection,
fallback), ADR-0029 (revisions, context precedence, terminology, privacy).
Research: `docs/phases/phase-6-research.md`. Results:
`docs/phases/phase-6-evaluation.md`.

```text
Phase 4 SpeechEvent::Transcript ──┐            Phase 5 SemanticEvent::Observation (final entities)
                                  ▼                         │ read-only clone, bounded
                           runtime core ──push (non-blocking, clone)──▶ ce-translate worker (per stream)
                                                                         bounded queue · coalescing · supervision
                                                                         TranslationProcessor
                                                                           revision rules (stale/duplicate/after-final)
                                                                           partial policy · language policy
                                                                           selection + explicit fallback · privacy
                                                                           hints (bounded) · terminology · protection
                                                                           TranslationAdapter::translate
                                                                           output validation · fidelity flags
                                                                           sequence · supersede · withdraw
                                                                         │
                             EventPayload::Translation (topic Translation, stream's session) ◀── core
```

## Pieces

| Piece | Location |
|---|---|
| Contracts: config, capabilities, result, source linkage, language provenance, states, flags, provenance, events, status, terminology | `ce-core::translation` |
| Adapter trait and factory, request, hints, outcomes | `ce-translate::adapter` |
| Translation memory `ce.translate.memory` 1.0.0 | `ce-translate::memory` |
| Scripted adapter with fault injection | `ce-translate::scripted` |
| Terminology matching, placeholders, fidelity checks | `ce-translate::fidelity` |
| Per-stream processor | `ce-translate::processor` |
| Supervised worker | `ce-translate::worker` |
| Corpus, BLEU/chrF, screens, replay | `ce-translate::eval`, `examples/evaluate_translation.rs`, `fixtures/translation/` |
| Runtime orchestration | `communication-engine/src/runtime/core/translation.rs` |
| Facade | `Engine::{start_translation, stop_translation, reset_translation, translation_status, translation_adapters}`, `EngineBuilder::{translation_adapter, terminology_profile}` |

No new third-party dependency.

## Using it

```rust,ignore
let engine = Engine::builder()
    .audio_backend(backend)
    .translation_adapter(MemoryFactory::new(TranslationMemory::from_json(&json)?))
    .terminology_profile(profile)
    .build()?;
engine.start_recognition(stream, None).await?;
engine.start_semantics(stream, None).await?;            // optional: hints
let cfg = TranslationConfig {
    target: Some(LanguageTag::parse("es")?),
    source: SourceLanguagePolicy::Explicit { language: LanguageTag::parse("en")? },
    partials: PartialPolicy::Off,                      // finals only (default)
    terminology: Some("hospitality-en-es".into()),
    ..TranslationConfig::default()
};
engine.start_translation(stream, Some(cfg)).await?;
// EventPayload::Translation(TranslationEvent::Result(r)): r.source.{utterance, revision}
// TranslationEvent::Withdrawn { utterance, through_sequence, .. }: drop those results
```

A consumer keeps, per utterance, the result with the highest `sequence`
and discards everything up to `through_sequence` on `withdrawn`.

## Result states

| State | Meaning | Later changes |
|---|---|---|
| `provisional` | whole partial snapshot translated (`partials = full`) | superseded by any later result |
| `stable` | committed text of a partial translated (`partials = committed`) | superseded by a longer stable result or the final (which may differ) |
| `final` | the final source revision translated | none (withdrawn only if the source utterance is cancelled — not possible after its final) |
| withdrawn (event) | earlier results invalid: cancel, reset, stop before final, untranslatable final | — |
| unavailable (event) | no translation, typed reason (unsupported pair, uncertain source, same language, no coverage, input/output too long, malformed, adapter error, timeout, empty) | — |

## Bounds and failure policy

| Bound | Default | Behaviour |
|---|---|---|
| `budget.queue` | 32 | newer partial replaces a queued partial of the utterance; oldest partial dropped when full; finals up to 2×; drops → `degraded(overload)`; `push` never blocks |
| `budget.max_input_chars` / `max_output_chars` | 2000 / 4000 | `unavailable(input_too_long / output_too_long)` |
| `budget.deadline_ms` | 2000 | a later result is discarded: `unavailable(timeout)` + `degraded(deadline_exceeded)` |
| `budget.max_consecutive_errors` | 5 | errors → `unavailable(adapter_error)` + `degraded(adapter_error)`; this many in a row fail the worker |
| `budget.max_retries` | 0 | no automatic retry; ≤ 2 on the same adapter if configured |
| `budget.stall_timeout_ms` | 5000 | runtime abandons a worker without heartbeat (`failed(translation.stalled)`) |
| `context.max_hints` / `max_hint_chars` / `max_utterance_distance` | 16 / 512 / 1 | hint selection bounds |
| `max_concurrent` (engine) | 4 | `translation.capacity_exhausted` |

Start errors: `translation.no_target`, `translation.no_adapter`,
`translation.remote_not_allowed`, `translation.partials_unsupported`,
`translation.unknown_terminology`, `translation.terminology_mismatch`,
`translation.already_running`, `translation.capacity_exhausted`,
`translation.not_capture`, `translation.stream_ended`,
`config.translation_invalid`.

## Not supported (explicit)

Incremental (token-streaming) output, code-switched input, dialect and
transliteration handling, inflection-aware terminology, real MT (blocked
on model approval), hosted providers, persistent translation storage.
