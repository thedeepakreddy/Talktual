# Phase 5 research — methods, providers and decision

Date: 2026-11-24. Facts are from crates.io metadata fetched on that date.
Nothing was downloaded beyond crate metadata, and no service was called.

## Baseline (reproduced, not assumed)

* Branch `deepak/gallant-hamilton-wegi2i` at `31c3173`, clean, 9 commits
  ahead of `main` (`9e6d4fe`). PR #10 (Phase 4) is open and mergeable, with
  CI `gates` and `msrv` green.
* `cargo test --workspace --all-targets`: **271 passed, 0 failed**.
* Talktual: `npx tsc --noEmit` passes; `npm test` **547 passed**;
  `npm run test:server` **60 passed**.
* Constraints:
  * MSRV 1.85; `forbid(unsafe_code)`;
  * event topics and `DataClass` privacy classes on every envelope;
  * workers are supervised `std` threads with bounded channels (Phase 3/4
    pattern);
  * deterministic replay tests.

## What Phase 5 consumes

Phase 4 `SpeechEvent::Transcript(TranscriptRevision)` per stream:
* utterance id and revision number (gapless);
* status `Partial` / `Final` / `Cancelled`;
* snapshot text (`committed` + `provisional`) and delta;
* frame/time ranges and `input_end_frame`;
* utterance language observation (finals);
* recognizer identity and config fingerprints.

Typical ASR output (sherpa-onnx) is **upper case without punctuation**.
Methods that rely on capitalization or punctuation (most NER) are unreliable
on it.

## Candidates

| Approach | Example | License | Locality / privacy | Cost | Footprint, maintenance | Fit |
|---|---|---|---|---|---|---|
| Deterministic rules + lexicons (CE-owned) | — | repository | on-device; nothing leaves the process | none | no dependency; fully testable | **chosen** for the baseline: numbers, quantities, times, dates, ordinals, contact patterns, profile terms, references, acts |
| Date/time parsing crates | chrono-english 0.2.1 (MIT), interim 0.2.3 (MIT, MSRV 1.81), two_timer 3.0.0 (MIT, MSRV 1.85), human-date-parser 0.3.1 (MIT) | MIT | local | none | small; expect written dates, not spoken forms ("nine thirty tomorrow") | not adopted: spoken-number handling is the hard part and is needed anyway |
| Spoken-number / duckling-style | rustling-ontology (Snips) | — | local | none | no longer on crates.io; upstream unmaintained since 2019 | rejected |
| Rule-based grammar | nlprule 0.6.4 | MIT/Apache | local | none | unmaintained since 2021; large data files | rejected |
| Transformer NER (GLiNER, ONNX) | gline-rs 1.1.0 (Apache-2.0) on `ort` 2.0.0-rc.15 | Apache-2.0 / MIT | local | none | model download of hundreds of MB; `ort` MSRV 1.92 > 1.85; native ONNX Runtime | **deferred**: needs approval for the model download; would be an optional adapter crate outside the workspace (like `ce-asr-sherpa`) |
| Transformer NLP (libtorch) | rust-bert 0.23.0 (Apache-2.0) | Apache-2.0 | local | none | libtorch (GBs); last release 2024-09 | rejected for footprint |
| Hosted LLM (structured output) | e.g. Anthropic Claude, OpenAI | proprietary service | **remote**: transcript text leaves the device | paid per token; needs credentials | network latency (hundreds of ms+), nondeterministic | **deferred**: paid service, credentials and data processing need approval; the contract supports it (`Locality::Remote`, policy-gated) |
| Fuzzy matching | fuzzy-matcher 0.3.7 (MIT), aho-corasick 1.1.5 | MIT/Unlicense | local | none | small | not needed: profile terms use an exact token-sequence match (and ASR-normalized aliases) |

## Evaluation methodology options

* No public dataset was acquired. Approval is needed, and none matches
  spoken, upper-case ASR output with CE's categories.
* An **authored replay corpus**: synthetic utterances written for CE with
  gold spans, references and acts, split dev / held-out test. Plus the
  real Phase 4 transcripts of the TTS fixtures (synthetic speech, real ASR
  errors).
* Small and synthetic: results describe this corpus only; no generalized
  quality claims (EVA-11).

## Decision

1. **Contracts in `ce-core::semantic`** (provider-agnostic, no
   dependency):
   * inputs, observations (entity, relation, reference, act);
   * epistemic status (observation / hypothesis / resolved
     interpretation / user-confirmed fact);
   * evidence spans, provenance, uncertainty with declared score semantics;
   * context items, domain profiles, events, status.
2. **New crate `ce-semantic`**:
   * adapter traits;
   * a **scripted** adapter (lifecycle, failure and revision tests);
   * the **deterministic rule adapter** `ce.semantic.rules` (English);
   * the context manager;
   * the per-stream processor (revision handling, stale rejection,
     update/stabilize/retract, finalization);
   * the supervised bounded worker;
   * replay evaluation.

   No new third-party dependency.
3. **Runtime integration**: the core forwards Phase 4 transcript revisions
   of a stream to its semantic worker (bounded, coalescing) and publishes
   `EventPayload::Semantic`. Transcripts are `Content`, and so are
   semantic observations that carry mentions.
4. **Deferred, approval required**: ML NER (GLiNER/ONNX; model download)
   and hosted LLM adapters (paid, credentials, remote processing). The
   adapter contract, locality/policy gating and capability flags are
   built for them now.
