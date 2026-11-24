# Phase 1 report — Foundation, contracts and runtime

Date: 2026-11-24 · Branch: `deepak/gallant-hamilton-wegi2i` · Verified on:
Linux x86_64 (cloud container, 4 vCPU), Rust 1.97.0 (pinned) and 1.85.0
(MSRV), headless. **No device, audio or real-time verification was performed
or is claimed.**

## 1. Summary

Phase 1 is implemented: a three-crate Rust workspace in
`communication-engine/` providing typed contracts, a single-owner runtime with
per-module actors, registry and capability discovery, bounded event and
command flow with explicit overflow policies, explicit lifecycles, cooperative
cancellation and bounded shutdown, sessions with correlation, redacted
observability, a test kit with a scriptable mock module and provider, a public
facade, and a runnable smoke example. Every gate in spec §14 was run and
passed (§6). Recommendation: **Phase 1 gates are met; proceed to Phase 2
research** with the conditions in §10.

## 2. Baseline

See [phase-1-baseline-and-plan.md](phase-1-baseline-and-plan.md). Talktual had
no Rust code; its gates (tsc, 537 client tests, 60 server tests, build) passed
before and pass unchanged after this work.

## 3. Architecture decisions

ADR-0001 workspace at `communication-engine/`, three crates (types+contracts,
runtime+facade, testkit) · ADR-0002 Tokio on the host's runtime, single-owner
core, actor per module, `CancellationToken` tree · ADR-0003 factory/instance
split, sequential `&mut self` actor, boxed futures, capability-based
dependencies, contract version 1.0 · ADR-0004 typed versioned envelope, one
sequence, broadcast + bounded mailboxes · ADR-0005 explicit lifecycle tables ·
ADR-0006 `CeError` with categories, codes, recoverability, redaction ·
ADR-0007 privacy classes and redaction by default · ADR-0008 control plane vs
real-time plane · ADR-0009 dependencies, licensing, toolchain.

## 4. Workspace and file inventory

| Path | Contents | Lines (Rust) |
| --- | --- | --- |
| `crates/ce-core/src/` | `ids`, `version`, `time`, `value`, `privacy`, `error`, `resource`, `config`, `lifecycle`, `capability`, `module`, `event`, `command`, `session`, `health`, `provider`, `observability` (+ unit tests) | 4 378 |
| `crates/communication-engine/src/` | facade (`lib.rs`), `runtime/{core,actor,registry,metrics}.rs`, optional `tracing_sink.rs` | 2 467 |
| `crates/communication-engine/tests/` | `lifecycle`, `capabilities`, `sessions_events`, `failures`, `backpressure`, `cancellation`, `observability`, `common/` | 1 900 |
| `crates/communication-engine/examples/runtime_smoke.rs` | end-to-end smoke example (CI gate) | 151 |
| `crates/ce-testkit/src/` | mock module/provider, manual clock, sequential ids, recording sink, recorder, fixtures | 664 |
| root of workspace | `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `rustfmt.toml`, `deny.toml`, `.gitignore`, `README.md`, `ARCHITECTURE.md` | — |
| `docs/architecture/` | lifecycle, events, errors-and-health, privacy, realtime, talktual-integration, ADR 0001–0009 | — |
| `docs/phases/` | baseline-and-plan, this report | — |
| `scripts/verify.sh` | all gates | — |
| repo root | `.github/workflows/communication-engine.yml` (path-filtered); root `README.md` pointer section | — |

## 5. Contracts and runtime demonstration

All contracts in spec §5 are implemented and exercised by tests (none are
empty stubs): module identity/descriptor validation; lifecycle tables;
module/factory/host contract; capability registry with duplicate, conflict,
exclusive-capability, contract-version, missing-dependency,
incompatible-version and cycle detection; event envelope; typed commands with
validation; session scaffold; cancellation; error model; configuration;
injectable clock; observability with redaction; resource/queue limits;
provider contract with error→category/recoverability/health mapping and a
mock; privacy/consent/retention/erasure representations; facade.

`cargo run -p communication-engine --example runtime_smoke` (public APIs only;
exits non-zero on any failed expectation):

```
communication-engine facade 0.1
  capability example.cap.app v1.0.0 by example.app (discovered)
  capability example.cap.base v1.0.0 by example.base (discovered)
  ok   two capabilities discoverable before start
  ok   discovery instantiated nothing
  ok   both modules running
  ok   provider initialized and started
  ok   module event belongs to the session
  ok   module event carries the session correlation id
  ok   module event source is the module
  ok   runtime healthy
  ok   clean shutdown
  ok   event stream closed after shutdown
  ok   shutdown_completed is the last event
  ok   observed sequence numbers strictly increase
  ok   modules stopped
  34 events observed
runtime smoke: PASS
```

## 6. Verification evidence (all executed; results observed)

| Gate (spec §14) | Command | Result |
| --- | --- | --- |
| Format | `cargo fmt --all -- --check` | pass |
| Lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass, no exclusions |
| Tests | `cargo test --workspace --all-targets` | **97 passed**, 0 failed |
| Tests, all features + doctests | `cargo test --workspace --all-features` | 98 passed + 1 doctest |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features` | pass |
| Smoke example | `cargo run -p communication-engine --example runtime_smoke` | PASS |
| Targets | `cargo check -p ce-core -p communication-engine --all-features --target …` | pass: `aarch64-apple-ios`, `aarch64-linux-android`, `x86_64-pc-windows-msvc`, `aarch64-apple-darwin` (check only, not linked or run) |
| wasm | `cargo check -p ce-core --target wasm32-unknown-unknown` | pass; runtime crate **not** supported on wasm (see §8) |
| MSRV | `cargo +1.85.0 check --workspace --all-targets --all-features` | pass |
| Dependencies | `cargo deny check` (advisories from RustSec, licenses, bans, sources) | advisories ok, bans ok, licenses ok, sources ok |
| Stability | full test suite run 25 consecutive times | 0 failing runs |
| Talktual regression | `npm run lint`, `npm test`, `npm run test:server`, `npm run build` | unchanged: pass, 537, 60, pass |

Test breakdown: `ce-core` 42 unit (id/name validation, serde, lifecycle
matrices, config defaults/overrides/rejections, secret detection, capacities,
error redaction and recoverability, privacy policy, event versioning/topics,
provider mapping, health aggregation); runtime 4 unit (registry) + 1 with
`tracing`; testkit 2; integration 49 — lifecycle 10 (ordered contiguous event
stream, dependency order, deterministic clock/ids incl. wall-clock rewind,
illegal operations, config failures, multi-thread concurrency), capabilities 6
(discovery without instantiation, version filtering, six invalid
registrations, missing/incompatible/cyclic dependencies), sessions/events 5
(correlation and causation chain, limits, topic filtering, own-event
exclusion, ordering, JSON round-trip of every observed event), failures 14
(error/panic/hang/create failure at start with cleanup, provider mapping and
health, task failure/panic, handler errors, handler panic, stop error, hung
stop, cancellation-ignoring task), backpressure 7 (mailbox drop-newest
coalescing, fail-module policy, subscriber lag, command reject and bounded
wait, ingress rejection, task limit), cancellation 5, observability 2.

CI: `.github/workflows/communication-engine.yml` runs `scripts/verify.sh`
(with cargo-deny) and the MSRV check. It has **not been executed on GitHub**
from this session; its steps were executed locally via the same script.

## 7. Dependency and license review

Direct: serde, semver, uuid, tokio (rt/sync/time/macros), tokio-util,
tracing (optional); dev: serde_json. Full transitive tree (all features, all
targets) is MIT, Apache-2.0 or Unicode-3.0 compatible; `deny.toml` enforces
that list. No RustSec advisories at time of check. No duplicate crate
versions. Rationale and rejected alternatives: ADR-0009. CE's own crates are
`UNLICENSED`/`publish = false` pending an owner decision.

## 8. Limitations

* Headless Linux only; other targets are compile-checked, not run. No audio,
  latency, battery or device claims.
* Runtime not available on `wasm32-unknown-unknown` (UUID v4 randomness and
  Tokio timers); `ce-core` is.
* Module event payloads are semi-structured (`ModuleEvent` attributes);
  typed per-module schemas are deferred.
* Commands are processed serially; a slow `start` delays other commands
  (bounded by timeouts; admission policy makes it explicit).
* Health is probed on demand only; no periodic probing, no failover/restart.
* Consent is represented but not yet enforced against capability
  requirements (nothing processes data yet).
* Pending mailbox events are discarded when a module stops.
* Static registration only (by design: no dynamic plugins).
* A module whose code blocks the OS thread (non-yielding) cannot be aborted by
  Tokio; documented in realtime.md and the module contract.
* The Tokio `JoinSet` abort on timeout releases module tasks, but a module's
  external resources (devices, sockets) are only released if its `stop` runs.

## 9. Risk register update (spec §18)

| Risk | Status |
| --- | --- |
| Overengineering | 3 crates, not 6; each justified (ADR-0001) |
| Async mistaken for realtime | Separate domains documented (ADR-0008, realtime.md); no audio code |
| Unbounded queues | Impossible by type (`QueueCapacity` ≤ 65 536); every queue's policy tested |
| Premature plugin ecosystem | Internal static registry only |
| Provider lock-in | `Provider` contract; vendor types cannot appear in `ce-core` |
| API churn | Versioned facade/contract/event/config; marked provisional |
| Privacy leakage | Classification + redaction + leak tests; secrets never visible |
| False verification | This report lists exactly what ran where |
| **New:** browser embedding | wasm runtime unsupported — spike required before choosing a browser path |
| **New:** CI unexecuted on GitHub | First push will run it; local equivalent passed |

## 10. Deviations from the spec (with justification)

1. **Crates:** `ce-types`+`ce-contracts` → `ce-core`; `ce-runtime`+`ce-facade`
   → `communication-engine`; no `ce-cli` (optional). Fewer crates, a stronger
   (compiler-enforced) facade boundary, the module/runtime split preserved
   (ADR-0001).
2. **Docs layout:** `docs/architecture/` merges principles/boundaries into
   `ARCHITECTURE.md` and errors with health; `talktual-integration.md` added.
3. **Integration tests** live in `crates/communication-engine/tests/` (Cargo
   convention; they use only the public facade) rather than a top-level
   `tests/integration/`.
4. **Examples** live in the facade crate (`examples/runtime_smoke.rs`) so they
   can only use the public API.
5. **`InvalidInput`** error category added (argument validation is not
   configuration).
6. **CI workflow** lives at the repository root `.github/workflows/` (GitHub
   only reads workflows there), path-filtered to CE.

## 11. Phase 1 acceptance checklist (spec §16)

- [x] Reconnaissance and baseline documented
- [x] Workspace builds on declared host/target (Linux x86_64; MSRV 1.85; four other targets check)
- [x] Contracts exercised, not stubs
- [x] Lifecycle works; invalid transitions rejected (exhaustive matrices)
- [x] Registry and capability discovery through public API
- [x] Bounded event flow and saturation behaviour tested
- [x] Cancellation and shutdown tested
- [x] Mock module and example demonstrate the architecture
- [x] Integration tests pass
- [x] fmt/clippy/check/test gates pass (no exclusions)
- [x] No unsafe code (`forbid(unsafe_code)` in every crate)
- [x] No sensitive data or credentials in logs/repo (tested; config rejects inline secrets)
- [x] CE core has no Talktual dependency
- [x] Dependency/license decisions documented (ADR-0009, `deny.toml`)
- [x] Report includes files, tests, limits, risks, deviations, Phase 2 readiness
- [x] No claims of full engine capability or device/audio verification

## 12. Phase 2 readiness

**Ready to begin Phase 2 research**, which should, using CE contracts:

1. Decide target platforms with Talktual's DR-8.5-1 evidence; benchmark audio
   capture/playback on real devices. CPAL is a candidate only — validate
   backends (CoreAudio/AAudio/WASAPI), licenses and latency first.
2. Implement the real-time plane per realtime.md (SPSC buffers, no
   allocation/locking/logging in callbacks) behind an adapter module.
3. Enforce session consent against `requires_consent` before any capture or
   remote processing; implement `DataErasure` for anything retained.
4. Decide the embedding path (FFI for native, Node binding/sidecar, or a wasm
   spike) before Talktual integration.
5. Run the CI workflow on GitHub and add real-device test plans; headless
   results must stay labelled as such.
