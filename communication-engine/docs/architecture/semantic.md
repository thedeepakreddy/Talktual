# Semantic & contextual intelligence (Phase 5)

Phase 5 turns Phase 4 transcript revisions into typed, versioned,
evidence-linked **observations**:
* entities;
* relations;
* reference resolutions;
* optional communicative acts.

It keeps a scoped, bounded, expiring **context** per stream.

Decisions: ADR-0025 (layer and contracts) and ADR-0026 (rule baseline and
evaluation). Alternatives: `docs/phases/phase-5-research.md`. Results:
`docs/phases/phase-5-evaluation.md`.

```text
Phase 4 recognizer ──SpeechEvent::Transcript(rev)──▶ runtime core
                                                     │ forward_transcript (non-blocking push)
                                                     ▼
                                         ce-semantic worker (one thread per stream)
                                           bounded queue · coalescing · supervision
                                                     │
                                         SemanticProcessor
                                           stale/dup/after-final rejection
                                           input validation (size, shape)
                                           ContextManager::view (scoped, permitted)
                                           SemanticAdapter::analyze  ──▶ drafts
                                           contract enforcement on drafts
                                           stable ids · status from revision · diff
                                           context admission / invalidation
                                                     │
                                         SemanticSignal ──▶ core ──▶ EventPayload::Semantic
                                                                     (topic Semantic, session of the stream)
```

## Crates and ownership

| Piece | Location |
|---|---|
| Contracts: input, observation, epistemic status, uncertainty, evidence, provenance, context item, profile, event, status, config | `ce-core::semantic` (`config`, `model`, `profile`) |
| Adapter trait and factory, draft types | `ce-semantic::adapter` |
| Deterministic English rule adapter `ce.semantic.rules` | `ce-semantic::rules`, `ce-semantic::text` |
| Scripted adapter (tests) | `ce-semantic::scripted` |
| Context manager | `ce-semantic::context` |
| Per-stream processor | `ce-semantic::processor` |
| Supervised worker | `ce-semantic::worker` |
| Replay evaluation | `ce-semantic::eval`, `examples/evaluate_semantic.rs`, `fixtures/semantic/` |
| Runtime orchestration | `communication-engine/src/runtime/core/semantic.rs` |
| Facade | `Engine::{start_semantics, stop_semantics, reset_semantic_context, confirm_observation, semantic_status, semantic_adapters}`, `EngineBuilder::{semantic_adapter, without_default_semantic_adapter}` |
| Talktual bridge (optional) | `adapters/talktual-bridge`: `"semantics": true` |

The core depends on no model, provider or network SDK. `ce-semantic`
adds no third-party dependency beyond the workspace's existing ones
(serde, serde_json, tokio sync, uuid).

## Contracts

* **`SemanticInput`**:
  * one revision of one utterance: stream, utterance, revision, status;
  * snapshot text (`committed` + space + `provisional`) and how many
    characters of it are committed;
  * language, if Phase 4 decided one;
  * recognition score with its semantics, if any;
  * stream-timeline time range and audio time.

  `input_from_revision` builds it from a `TranscriptRevision` without
  changing the text.
* **`SemanticObservation`** carries:
  * a stable `id`, `schema_version` and a per-observation `version`;
  * a `payload` (tagged `type`): `entity`, `relation`, `reference` or
    `act`;
  * a `status`: `provisional`, `stable`, `final` or `retracted`;
  * `epistemic` (below), `uncertainty` and `evidence` (at least one item);
  * a `source`: stream, utterance, revision, revision status, character
    span and whether the span is committed;
  * `provenance`: adapter, version, config fingerprint, profile identity
    and locality;
  * `produced_at`, on the stream timeline.
* **Epistemic status** is never upgraded implicitly:

  | Status | Meaning |
  |---|---|
  | `observation` | stated in the text |
  | `hypothesis` | inferred: act labels, ambiguous profile terms, cue-based relations |
  | `resolved_interpretation` | linked through scoped context |
  | `user_confirmed_fact` | set only by `confirm_observation`; the processor rejects drafts that claim it |
* **Uncertainty**:
  * a rule **tier** (`low`, `medium`, `high`), which is not a probability;
  * an optional adapter score with declared `ScoreSemantics`;
  * `calibrated` (always false: no calibration evidence exists).

  Scores whose semantics differ from the adapter's declared semantics are
  rejected. Scores are never compared across adapters.
* **Entity**:
  * a category: a built-in, or `profile:<category>`;
  * the mention, exactly as in the transcript span;
  * a `normalized` value, only when the rule supports it. It is never
    guessed: an unstated am/pm stays `null`, and a weekday is never turned
    into a date;
  * attributes such as `meridiem`, `modifier`, `ambiguity`, `alternatives`
    or `resolved_date`.
* **Relation**: `relation` type (`quantity_of`, `on_day`, `refers_to`),
  `from` observation id and `to` observation or context item id.
* **ReferenceResolution**:
  * the mention and its kind (pronoun, demonstrative, locative, temporal,
    personal);
  * candidates (context items, most recent first);
  * an outcome: `resolved{target}`, `ambiguous`, or `unresolved{reason}`.
* **CommunicativeAct**: taxonomy id and version, a decision
  (`labeled{label}` or `abstained{reason}`) and the alternative labels
  whose cues were present.
* **ContextItem**:
  * scope (utterance or session) and category;
  * mention and normalized value;
  * source observation and span;
  * certainty, status and `DataClass::Content`;
  * created and expires timestamps, and size in bytes.
* **SemanticEvent** (`EventPayload::Semantic`, topic `Semantic`):
  * `started`, `observation{change}`, `utterance_finalized`,
    `utterance_cancelled`;
  * `context{change}`, `input_rejected{reason}`, `degraded{reason}`;
  * `failed`, `stopped{counters}`.

  The change values are `new`, `updated`, `stabilized`, `finalized`,
  `retracted` and `confirmed`. **Privacy**: observations are `Content`;
  everything else is `Operational` and carries ids, sizes and reasons
  only.

## Streaming and revision semantics

* Inputs of an utterance must increase in revision number:
  * an older revision is rejected as `stale`, an equal one as
    `duplicate`;
  * any input after the utterance finalized or was cancelled is rejected
    as `after_final`.

  Stale input never becomes current. Revision gaps are normal (the worker
  coalesces).
* Each processed revision is interpreted as a whole snapshot and diffed
  against the current set. Unchanged observations emit nothing.
* **Identity across revisions.** The adapter's key (kind, category, folded
  mention) plus an occurrence index decides identity. Failing that, an
  overlapping span of the same kind and category keeps the id, so
  "TWENTY" → "TWENTY FIVE" is an `updated`. A category change (number →
  duration) retracts the old observation and creates a new one.
* **Status comes from the revision, not the adapter**: final revision →
  `final`; span inside committed text → `stable`; otherwise
  `provisional`.
* A **final that amends committed text** retracts unsupported
  observations. It also invalidates their context items, and *cascades*:
  resolved references (and `refers_to` relations) that pointed to those
  items are retracted, including in recently finalized utterances.
* A **cancelled** utterance retracts all its observations and invalidates
  its context items.
* Partial interpretation can be turned off (`budget.partials = false`).
  The robustness replay shows finals are identical either way on the
  corpus.

## Bounded execution

| Bound | Default | Behaviour |
|---|---|---|
| `budget.queue` | 64 | A newer partial replaces a queued partial of the same utterance (coalesced). When full, the oldest queued partial is dropped. Finals and cancellations may use up to 2× before being dropped. Any drop raises `degraded(overload)`; the runtime counts handover drops in `semantic_inputs_dropped`. `push` never blocks. |
| `budget.max_input_chars` | 4096 | Longer input is rejected (`oversized`); a final still closes the utterance. |
| `budget.deadline_ms` | 250 | A late result is discarded (`degraded(deadline_exceeded)`); a late final closes the utterance with no observations. In-process adapters run to completion; a hung adapter is caught by the stall detector. |
| `budget.max_observations` | 64 | Excess drafts (and relations to them) are dropped (`degraded(output_limited)`). |
| `budget.max_consecutive_errors` | 5 | Adapter errors raise `degraded(adapter_error)`; this many in a row fail the worker. |
| `budget.stall_timeout_ms` | 5000 | The runtime abandons a worker with no heartbeat (`failed` + `stopped(failed)`); it sends nothing more. |
| `max_concurrent` (engine) | 4 | Further starts fail with `semantic.capacity_exhausted`. |

Panics are contained (`semantic.worker_panicked`). One stream's failure
does not affect others (tested).

**Stop reasons.**
* `source_ended` (the recognition stopped) and `shutdown` drain queued
  revisions.
* `stopped` (host) and `failed` discard them.

Session close and shutdown stop semantics after recognitions and before
streams close, so final transcripts are interpreted.

## Context

* **Scope**:
  * `off`: references stay `unresolved(context_disabled)`;
  * `utterance`: dropped at the utterance's end;
  * `session` (default): the stream's semantic session.

  Nothing is persisted. There is no cross-stream or cross-session context:
  each worker owns its context manager.
* **Admission.** Only entity observations whose span is committed
  (`stable`/`final`) and whose certainty is at least `min_certainty`
  (default `medium`) become context items. Provisional text and weak
  hypotheses never do.
* **Bounds**:
  * `max_items` (32), evicting the oldest;
  * `max_bytes` (8 KiB);
  * `max_tokens` (512 whitespace words of mentions);
  * `ttl_ms` (120 s on the stream timeline);
  * `max_utterance_distance` (2).
* **Teardown.** The context is cleared on `reset_semantic_context`,
  stop, cancellation, session close and shutdown.
* Context events carry ids and sizes, never mentions. Context items are
  `Content` and never leave the worker except as candidates inside
  `Content` reference observations.

## Locality policy and disclosure

`policy.allow_local_service` and `policy.allow_remote` are false by
default. An adapter whose `SemanticCapabilities::locality` is not
permitted is refused (`semantic.locality_not_permitted`). The locality is
in `SemanticStatus`, in each observation's provenance and in the
`semantic.started` log. Configured features the adapter lacks are refused
(`semantic.feature_unsupported`).

## `ce.semantic.rules` 1.0.0 (English)

**Tokens.**
* Split on whitespace and hyphens.
* Punctuation is trimmed at token edges.
* `$`, `%`, `:` and `.` inside digits are kept.
* Matching is case-insensitive.

**Entities** (passes in order; earlier passes claim tokens):

1. **Profile terms.** The longest folded word sequence wins, with a
   regular plural of the last word.
   * Canonical form: `high`.
   * Alias or plural: `medium`.
   * A form shared by several terms: a `low` hypothesis, `profile:ambiguous`
     when categories differ, with `alternatives` and no normalized value.
2. **`digit_sequence`.** Four or more single digits (`oh` inside the run
   counts as zero). Kept as a digit string, never interpreted as a phone
   number or code.
3. **`time_of_day`.**
   * Strong forms (`high`):
     * `noon`, `midnight`;
     * `half past H`, `quarter past/to H`;
     * `h:mm`;
     * `H o'clock`;
     * `H [M] am/pm/a m/p m/in the morning/afternoon/evening/at night`.
   * Weaker forms (`medium`):
     * a cue word (`at`, `by`, `until`, `till`, `around`, `before`,
       `after`, `from`) before `H [M]`;
     * an hour word (1–12) followed by a spoken minute (`EIGHT THIRTY`).
   * am/pm stays unknown unless said (`meridiem: unspecified`).
   * Not a time when followed by a unit or counted noun.
4. **`date`.**
   * Forms: `month [day] [year]` and `ordinal of month [year]`.
   * A month alone needs a preposition (`in may`, so `i may go` is not a
     date).
   * Days are validated against the month.
   * Spoken years: `twenty twenty six`, `nineteen ninety`, `twenty oh five`,
     `two thousand …`.
5. **`relative_day`.**
   * Words: `today`, `tonight`, `tomorrow`, `yesterday`,
     `day after tomorrow`, `day before yesterday`.
   * Normalized as an offset, plus `resolved_date` only when the host
     configures `reference_date`. The system clock is never read.
6. **`weekday`.** Optional `next`, `last`, `this` or `coming` modifier,
   and a plural for recurring. Never resolved to a date: that is
   ambiguous.
7. **`duration`.**
   * `N unit` (seconds to weeks), `a/an unit` (`medium`), `half an hour`,
     `… and a half`.
   * `a second` and `a day` are skipped: ordinal or frequency readings.
8. **`money`, `percentage`, `quantity`.**
   * Currency words map to ISO 4217 codes: dollars → USD, euros → EUR,
     rupees → INR, yen → JPY, bucks → USD at `medium`. `and N cents` is
     included; `$N` is supported.
   * Percentages: `percent`, `per cent`, `N%`.
   * Measures (km, kg, liters, degrees …) are normalized to a singular
     unit.
   * **`pound(s)`** is a `low` hypothesis quantity with
     `ambiguity: currency_or_weight`.
   * `cent(s)` is a quantity: the currency is unknown.
   * A counted plural noun after an integer other than 1 is `medium`
     (`two boxes`, `four people`).
9. **`ordinal`.** Except `a/one/per/each second`.
10. **`number`.**
    * Digits or number words, decimals with `point`.
    * Runs of bare digits are not merged.
    * A lone `one` is a `low` hypothesis (`number_or_pronoun`), and is
      skipped after determiners and ordinals or before `of`.

**References.**
* **Forms**:
  * `it`, `it's`;
  * `they`, `them`, `they're`;
  * `this`, `that`, `that's`, `these`, `those` (demonstratives only before
    a short set of verbs and function words, or at the end; `that` after
    verbs of saying or thinking is a complementizer);
  * `there`, `here` (not existential, not after a greeting);
  * `then` (not after `and`, `so` … or at the start);
  * `he`, `him`, `she`, `her`.
* **Propositional demonstratives** (`that is right`, `is that correct`)
  are always `unresolved(propositional)`.
* **Compatibility**:

  | Referring expression | Compatible categories |
  |---|---|
  | singular | money, digit_sequence, percentage |
  | plural | quantity |
  | `then` | time_of_day, relative_day, weekday, date |
  | any | profile categories whose `hints` list the expression |

* Candidates must precede the reference.
  * One candidate → `resolved` (`medium`, `resolved_interpretation`, plus
    a `refers_to` relation).
  * Several → `ambiguous` (`low` hypothesis, no choice).
  * None → `unresolved`.

**Relations** (`hypothesis`, at most `medium`):
* `quantity_of`: a number immediately followed by a profile term;
* `on_day`: a time of day within two tokens of a day expression.

**Acts** (`ce.acts` 1.0.0: question, request, confirmation, refusal,
acknowledgement, correction, statement). Cue-based hypotheses.
* Leading discourse markers (`and`, `so`, `also`, `but`, `well`, `now`)
  are skipped.
* Strong correction cues (`i mean`, `i meant`, `i said`, `correction`,
  `not … but`) decide.
* `can/could/would/will you …` is a request, but `would you like` and
  `do you want` are offers (question).
* `yes please` is a confirmation; `no thanks` is a refusal.
* Weak correction cues (`actually`, `instead`, `rather`) count only alone.
* Several remaining cues → `abstained(ambiguous)` with alternatives.
* Greetings → `abstained(out_of_taxonomy)`.
* A subject-initial utterance of three or more words → `statement`.
* Otherwise → `abstained(no_cue)`.

**Not supported.**
* Person, organization and location names. Those need an ML model,
  deferred pending approval.
* Other languages: input whose language is not English yields nothing.
* Negative numbers (`minus five`) and `dozen`.
* Context-aware readings of `pounds` (currency versus weight).
* Inputs with punctuation-based sentence structure.

## Domain profiles

A profile is JSON data (`deny_unknown_fields`, so it cannot carry logic).
It has:
* schema version, id, `MAJOR.MINOR.PATCH` version and language;
* categories, plus terms (canonical form and aliases);
* context `hints` (which referring expressions may refer to a category);
* an optional evaluation-set id.

**Validation.**
* Ids, version and language format.
* At most 2000 terms, 16 aliases per term, 64 characters and 6 words per
  form, 64 categories and 256 hints.
* No duplicate surface form within a category. Across categories a shared
  form is allowed and reported as ambiguity.

The profile is fingerprinted and is part of the config fingerprint, and
its identity is in every observation's provenance. A profile never sets
certainty and never overrides other evidence. Example:
`fixtures/semantic/profiles/home.json`.

## Observability

* **Logs** (stream ids, adapter, locality, config fingerprint and reasons
  only): `semantic.started`, `semantic.ended`, `semantic.failed`,
  `semantic.stalled`, `semantic.stop_timed_out`.
* **Metrics**: `semantics_started`, `semantic_failures`,
  `semantic_inputs_dropped`.
* **`SemanticStatus` counters**:
  * inputs, processed, coalesced;
  * stale rejected, invalid rejected;
  * new, updated, stabilized, finalized, retracted, confirmed;
  * context evicted and expired;
  * context size in items, bytes and tokens;
  * adapter errors and invalid drafts, below-threshold drafts, deadlines
    exceeded, cancellations, truncations;
  * processing, queue-delay and first-observation latency percentiles;
  * context size.

  `RuntimeHealth::semantics` lists running and recent workers.
* A runtime test asserts that no transcript or observation text reaches
  logs, health or metrics.

## Adding an ML or hosted adapter (deferred, approval needed)

Implement `SemanticFactory`/`SemanticAdapter` in a crate outside the
workspace (like `adapters/ce-asr-sherpa`). Then:
1. Declare locality, capabilities, languages and score semantics.
2. Return drafts with spans and evidence.
3. Register with `EngineBuilder::semantic_adapter`.

A `remote` adapter runs only with `policy.allow_remote`. Model downloads,
paid APIs and credentials need the user's approval before any such adapter
is built (`docs/phases/phase-5-research.md`).
