# Phase 6 research — translation approaches and candidates

Date: 2026-11-24. Nothing was downloaded except crates.io metadata and,
for metric cross-checking only, the sacreBLEU 2.4.3 Python package (into a
scratch virtual environment outside the repository).

**Evidence levels.** *Verified here*: checked in this session (crates.io
API). *Prior knowledge*: from published documentation known before this
session; **not verified here** because the network policy blocks Hugging
Face and GitHub metadata. Every prior-knowledge item must be re-checked
from the model card / licence file before any acquisition.

## Rust runtimes (verified here, crates.io, 2026-11-24)

| Crate | Version | Updated | Licence | Role |
|---|---|---|---|---|
| `ct2rs` | 0.11.3 | 2026-11-23 | MIT | CTranslate2 bindings (C++ native library) — runs Marian/OPUS-MT, M2M-100, NLLB converted models, int8 CPU |
| `candle-transformers` | 0.11.1 | 2026-10-31 | MIT OR Apache-2.0 | pure-Rust inference incl. a Marian model; large dependency tree |
| `rust-bert` | 0.23.0 | 2024-09-29 | Apache-2.0 | libtorch (very large native dependency); stale |
| `ort` | 2.0.0-rc.15 | 2026-10-19 | MIT OR Apache-2.0 | ONNX Runtime; MSRV 1.92 (> CE 1.85); pre-release |
| `tokenizers` | 0.23.2 | 2026-11-20 | Apache-2.0 | tokenization for Hugging Face models |
| `sentencepiece` | 0.14.0 | 2026-07-26 | MIT OR Apache-2.0 | SentencePiece bindings (C++) |
| Bergamot / Firefox Translations | — | — | — | no crate on crates.io (C++ `bergamot-translator`) |

## Models and services (prior knowledge — verify before acquisition)

| Candidate | Licence (to verify) | Pairs | Size class | Offline | Notes |
|---|---|---|---|---|---|
| OPUS-MT (Helsinki-NLP, Marian) | CC-BY 4.0 models | per directed pair (en↔es, en↔fr, en↔de exist) | ~300 MB fp32 per pair; much smaller int8 via CTranslate2 | yes | mature; quality evidence on public benchmarks; sentence-level, not streaming |
| Bergamot / Firefox Translations models | MPL-2.0 (models) | selected pairs, en-pivot | tens of MB per direction (quantized) | yes | designed for CPU, fast; C++ only |
| M2M-100 418M | MIT | many-to-many | > 1 GB | yes | large |
| MADLAD-400 | Apache-2.0 | many | multi-GB | yes | very large |
| NLLB-200 (incl. distilled 600M) | CC-BY-NC 4.0 | 200 languages | > 1 GB | yes | **non-commercial licence → rejected** |
| Argos Translate | MIT code; model licences vary | per pair | ~100 MB per pair | yes | Python; not embeddable in-process |
| LibreTranslate | AGPL-3.0 server | — | — | self-hosted | service + licence obligations |
| Hosted APIs (Google, DeepL, Azure, …) | commercial | many | — | no | paid, credentials, transcript transfer — out of scope without approval |

## Properties that matter for CE (from the specification)

* **Streaming**: none of the local candidates translates incrementally in a
  way that keeps earlier output stable; CE therefore treats adapters as
  per-request (batch) and builds streaming behaviour from source revisions
  (`partials = committed|full`) with explicit supersession and withdrawal.
* **Terminology**: Marian/CTranslate2 support no glossary constraints
  natively (prefix/forced decoding exists in CTranslate2 but is not
  terminology); CE protects `do_not_translate` terms with placeholders and
  verifies target terms (`term_missing`) independently of the adapter.
* **Context**: sentence-level MT models do not take structured context;
  CE passes bounded hints only to adapters that declare `context_hints`.
* **Uncertainty**: MT models expose log-probabilities, not calibrated
  confidence; any score would be `heuristic_confidence`.

## Decision
See ADR-0028: ship the deterministic translation-memory baseline and the
scripted adapter; real MT is **blocked pending approval**; the recommended
first real adapter is OPUS-MT via CTranslate2 (`ct2rs`) as a separately
built adapter, en↔es first.

## Metrics tooling
CE implements BLEU (sacreBLEU defaults: 13a, exp smoothing) and chrF2 in
`ce-translate::eval`. Semantic metrics (COMET, BLEURT, BERTScore) need
neural models (downloads) and GPUs for practical speed; not used.
Cross-check against sacreBLEU 2.4.3 (Apache-2.0): see the evaluation report.
