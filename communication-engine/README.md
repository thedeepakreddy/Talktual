# Communication Engine (CE)

A product-agnostic Rust foundation for a communication intelligence engine.
Talktual is its first consumer, not its boundary.

**Status: Phase 4 — speech recognition & language intelligence pipeline**
on top of Phase 3 audio intelligence, the Phase 2 audio foundation and the
Phase 1 foundation. Capture and playback infrastructure
exists (device discovery, negotiation, bounded real-time handoff,
timestamps, device changes, capture consent gate) with a `cpal` backend,
and signal analysis runs on capture streams (voice activity, candidate
segments and endpoints, silence, quality, signal conditions, adaptive and
resource policy). Phase 4 adds the streaming speech-recognition pipeline (adapter boundary,
revisioned transcripts, finalization, timestamps, Phase 3 hints, bounded
supervised workers) and text language observations (whichlang, with
abstention). Real English recognition is available through the optional
sherpa-onnx streaming Zipformer adapter (`adapters/ce-asr-sherpa`, built
separately; evaluated on synthetic speech only); tests use a scripted
adapter and pipeline evaluation an oracle adapter. There is no diarization, translation, voice or TTS,
and **no audio hardware has been tested** — no latency or device claims.
All APIs are provisional (facade API `0.1`). See the
[Phase 1](docs/phases/phase-1-report.md),
[Phase 2](docs/phases/phase-2-report.md),
[Phase 3](docs/phases/phase-3-report.md) and
[Phase 4](docs/phases/phase-4-report.md) reports.

## What Phase 1 provides

* **Contracts** (`ce-core`): typed ids, versions, time and an injectable
  clock, typed errors with redaction, privacy classes, consent/retention
  representations, validated configuration, module lifecycle tables,
  module/factory/host contracts, capability descriptors, event envelope,
  commands, sessions, health, provider and observability contracts.
* **Runtime + facade** (`communication-engine`): registration and capability
  discovery without instantiation, dependency/version/conflict resolution,
  ordered start and reverse shutdown, per-module actors with supervised tasks,
  cooperative cancellation, bounded queues with explicit overflow policies,
  sessions with correlation, health aggregation and redacted observability.
* **Test kit** (`ce-testkit`): scriptable mock module (fail/hang/panic at any
  stage, background tasks, slow consumers, correlated acks), mock provider,
  manual clock, sequential ids, recording sink, event recorder, fixtures.

## Layout

```
communication-engine/
  Cargo.toml  Cargo.lock  rust-toolchain.toml  deny.toml  rustfmt.toml
  ARCHITECTURE.md
  crates/ce-core/                      foundation types + contracts (no async runtime)
  crates/ce-audio/                     audio backend boundary, rt handoff, workers, analysis tap, cpal adapter
  crates/ce-audio-intel/               audio intelligence: features, VAD, segmentation, quality, policy, eval
  crates/ce-speech/                    speech: ASR adapter boundary, transcripts, language ID, recognizer, eval
  fixtures/speech/                     synthetic TTS fixtures, transcript manifest + provenance
  fixtures/language/                   authored language-ID corpus + provenance (Phase 4)
  adapters/ce-asr-sherpa/              optional real ASR adapter (sherpa-onnx; outside the workspace)
  crates/communication-engine/         runtime (private) + public facade
    examples/runtime_smoke.rs          end-to-end example, used as a CI gate
    tests/                             integration tests (public API only)
  crates/ce-testkit/                   mocks, fakes, fixtures
  docs/architecture/                   lifecycle, events, errors, privacy, realtime, integration, ADRs
  docs/phases/                         Phase 1 baseline/plan and report
  scripts/verify.sh                    every verification gate
```

## Use

```bash
cd communication-engine
cargo run -p communication-engine --example runtime_smoke   # end-to-end demo
cargo run -p communication-engine --example audio_smoke     # audio API on the fake backend
cargo run -p communication-engine --example analysis_smoke  # audio intelligence on the fake backend
cargo run --release -p ce-audio-intel --example evaluate -- test   # fixture evaluation tables
cargo run -p communication-engine --example speech_smoke    # speech recognition on the fake backend (scripted adapter)
cargo run --release -p ce-speech --example evaluate_speech -- all  # Phase 4 evaluation tables
cargo run --release -p communication-engine --features cpal --example audio_probe -- --seconds 5   # real device
cargo test --workspace --all-targets                         # tests
scripts/verify.sh                                            # all gates
```

The `cpal` feature on Linux needs the ALSA development package
(`libasound2-dev`). Requires Rust 1.85+ (the pinned toolchain, 1.97.0, is installed automatically
by rustup). `scripts/verify.sh` runs fmt, clippy (`-D warnings`), tests
(default and all features), docs, the smoke example, cross-target checks for
installed targets and `cargo deny` (set `CE_SKIP_DENY=1` if not installed:
`cargo install cargo-deny --locked`).

CI: `.github/workflows/communication-engine.yml` at the repository root runs
the same gates plus an MSRV check, only when this directory changes.

## Writing a module

Depend on `ce-core` only. Implement `ModuleFactory` (cheap descriptor, config
validation, `create`) and `Module` (`initialize`, `start`, `stop`, `health`,
`handle_event`). Use `ModuleContext` to emit events (non-blocking), spawn
supervised tasks and observe cancellation. See `ce-testkit`'s mock module and
[ADR-0003](docs/architecture/adr/0003-module-contract.md).
