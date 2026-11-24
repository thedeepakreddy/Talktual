# ADR-0025 — Semantic layer: contracts in ce-core, rules baseline in ce-semantic

Status: Accepted (Phase 5) · Date: 2026-11-24

## Context
Phase 5 adds semantic and contextual interpretation on top of Phase 4
transcript revisions. Partial revisions change, finals amend committed
text, and utterances can be cancelled. The ASR baseline emits upper-case
words without punctuation.

The alternatives considered are in `docs/phases/phase-5-research.md`:
* ML NER (GLiNER on ONNX Runtime) needs a model download and Rust ≥ 1.92;
* a hosted LLM is paid, needs credentials and is remote;
* no maintained Rust crate parses spoken numbers, times or dates.

## Decision
* **Contracts** live in `ce_core::semantic`, with no new dependency:
  * `SemanticInput`;
  * `SemanticObservation`, with an entity, relation, reference or act
    payload;
  * `Epistemic` (observation / hypothesis / resolved interpretation /
    user-confirmed fact);
  * `Uncertainty` (a rule tier, an optional score with declared
    `ScoreSemantics`, `calibrated: false`);
  * `Evidence` (explicit / inferred / context / profile) and `Provenance`
    (adapter, version, config fingerprint, profile, locality);
  * `ContextItem`, `DomainProfile`, `SemanticEvent` and `SemanticStatus`.

  These are carried by `EventPayload::Semantic` on `EventTopic::Semantic`.
* **Observations reference, never edit, the transcript.** Each has
  `SourceRef { stream, utterance, revision, revision_status, span,
  committed }`. Spans are character offsets into the revision's snapshot
  text: `committed` + space + `provisional`.
* **Observation ids are stable per utterance.** They are derived from
  kind, category and normalized surface form, plus an occurrence index.
  Later revisions therefore update, stabilize, finalize or retract the
  same id. Retracted observations stay identifiable.
* **Epistemic status is never upgraded implicitly.**
  * A resolved reference is a `ResolvedInterpretation`.
  * An act label inferred from cues is a `Hypothesis`.
  * `UserConfirmedFact` is set only by the host confirming an observation
    (`confirm_observation`), never by an adapter.
* **Context is ephemeral by default** (session scope, cleared at stop,
  reset, cancellation and shutdown). It is bounded by items, bytes, TTL
  and utterance distance, and admits only `Medium`-or-better observations.
  Longer scopes (user memory) are not supported.
* **Locality policy.** Remote and local-service adapters are refused
  unless the policy allows them. Locality is disclosed in capabilities and
  provenance.
* **Profiles are data** (JSON, `deny_unknown_fields`), validated, bounded
  and fingerprinted. They never set certainty.
* **New crate `ce-semantic`** holds the adapter trait, a scripted adapter,
  the deterministic English rule adapter `ce.semantic.rules`, the context
  manager, the per-stream processor, the supervised worker and replay
  evaluation.

## Consequences
* There are no model or network dependencies, and tests are deterministic.
* Rule coverage is limited:
  * English only;
  * the categories listed in `docs/architecture/semantic.md`;
  * no person, organization or location names. Recognizing names reliably
    on upper-case, unpunctuated ASR text needs an ML model, which is
    deferred pending approval.
* An ML or hosted adapter can be added outside the workspace (as with
  `ce-asr-sherpa`) behind the same trait and policy.
