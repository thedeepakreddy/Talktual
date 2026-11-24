# Communication Engine (CE)

A product-agnostic Rust foundation for a communication intelligence engine.
Talktual is its first consumer, not its boundary.

**Status: Phase 1 — foundation, contracts and runtime.** There is no audio,
speech recognition, translation, semantic processing, voice or TTS here, and
no real-time, latency or device claims. All APIs are provisional (facade API
`0.1`). See the [Phase 1 report](docs/phases/phase-1-report.md).

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
cargo test --workspace --all-targets                         # tests
scripts/verify.sh                                            # all gates
```

Requires Rust 1.85+ (the pinned toolchain, 1.97.0, is installed automatically
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
