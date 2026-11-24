# Communication Engine — Architecture

Status: Phase 1 foundation, Phase 2 audio foundation, Phase 3 audio
intelligence, Phase 4 speech recognition & language intelligence pipeline
and Phase 5 semantic & contextual intelligence, implemented and verified on
Linux x86_64 (see the
[Phase 1](docs/phases/phase-1-report.md),
[Phase 2](docs/phases/phase-2-report.md),
[Phase 3](docs/phases/phase-3-report.md),
[Phase 4](docs/phases/phase-4-report.md) and
[Phase 5](docs/phases/phase-5-report.md) reports). APIs are
**provisional**.

The Communication Engine (CE) is a product-agnostic Rust foundation for a
communication intelligence engine. Talktual is its first consumer, not its
boundary. Phase 1 provides contracts, a runtime, a registry, bounded event
flow, lifecycle, cancellation, sessions, a test kit, a public facade and a mock
module. Phase 2 adds the audio foundation — device discovery, format
negotiation, capture and playback streams, bounded real-time handoff,
timestamps, device-change handling and a capture authorization gate (see
[audio.md](docs/architecture/audio.md)). Phase 3 adds signal analysis in
`ce-audio-intel` — voice activity, candidate segments and endpoints,
silence windows, quality, a closed set of signal conditions, adaptive and
resource policy — fed by a bounded, non-blocking tap on capture streams (see
[audio-intelligence.md](docs/architecture/audio-intelligence.md)). Phase 4
adds `ce-speech` — the ASR adapter boundary, revisioned streaming
transcripts with stable/committed text and finalization, Phase 3 hint
integration, text language observations with explicit scope and
abstention, supervised bounded recognition workers, and replay/evaluation
(see [speech.md](docs/architecture/speech.md)). A real English recognizer
(sherpa-onnx streaming Zipformer INT8) is an optional adapter in
`adapters/ce-asr-sherpa`. It is built outside the workspace because it
links native libraries and needs Rust ≥ 1.89, and it has been evaluated on
synthetic speech only. Tests use a scripted adapter, and pipeline
evaluation uses an oracle adapter.
Phase 5 adds `ce-semantic`:
* typed, versioned observations (entities, relations, references,
  optional acts) linked to transcript revisions and spans, with epistemic
  status, uncertainty, evidence and provenance;
* stable identity across revisions; stale rejection; stabilization,
  finalization, retraction and correction cascades;
* scoped, bounded, expiring, ephemeral context; locality policy; domain
  profiles as validated data;
* a supervised coalescing worker and a deterministic English rule adapter.

See [semantic.md](docs/architecture/semantic.md). There is no
diarization, translation, voice or TTS functionality, nothing that infers
personal traits, and no latency or device claims (no audio hardware has
been tested).

## 1. Crates and dependency direction

```
            ┌────────────────────────────┐
 hosts ───► │ communication-engine       │  facade (public) + runtime (private)
 (apps,     │  Engine, EngineBuilder,    │  deps: ce-core, tokio, tokio-util, uuid
  SDKs,     │  EventSubscription         │  feature `tracing`: TracingSink
  tests)    └─────────────┬──────────────┘
                          │ re-exports as `contracts`
                          │ re-exports as `audio`, `audio_intel`, `speech`, `semantic`
            ┌─────────────▼──────────────┐
            │ ce-semantic (Phase 5)      │  semantic adapter boundary, rule/scripted
            │                            │  adapters, context, processor, worker, eval
            │                            │  (deps: ce-core only)
            └────────────────────────────┘
            ┌─────────────▼──────────────┐
            │ ce-speech (Phase 4)        │  ASR adapter boundary, scripted/oracle
            │                            │  adapters, transcript revisions, LID
            │                            │  (whichlang), recognizer, workers, eval
            └─────────────┬──────────────┘
            ┌─────────────▼──────────────┐
            │ ce-audio-intel (Phase 3)   │  framer/features, VAD adapters, hysteresis,
            │                            │  segments, quality, conditions, policy,
            │                            │  supervised analysis workers, eval/replay
            └─────────────┬──────────────┘
            ┌─────────────▼──────────────┐
 adapters ► │ ce-audio (Phase 2)         │  AudioBackend trait, rt endpoints (rtrb),
            │                            │  conversion, stream workers; feature
            │                            │  `cpal`: cpal adapter
            └─────────────┬──────────────┘
            ┌─────────────▼──────────────┐
 modules ─► │ ce-core                    │  types + contracts, runtime-agnostic
 adapters   │  ids, errors, config,      │  deps: serde, semver, uuid (no rng)
            │  lifecycle, module, event… │  no async runtime, no I/O
            └─────────────▲──────────────┘
            ┌─────────────┴──────────────┐
            │ ce-testkit (tests only)    │  mock module/provider, manual clock,
            └────────────────────────────┘  sequential ids, recorder, fixtures
```

* **Modules and adapters depend on `ce-core` only.** They never link the
  runtime or an executor, so a module can be tested with any harness and the
  runtime can change without touching modules.
* **Hosts depend on `communication-engine` only.** The runtime is `pub(crate)`
  behind the facade; Rust visibility, not convention, enforces the boundary.
* Nothing in CE depends on Talktual or any product, UI, transport, database,
  cloud vendor or speech provider. See
  [Talktual integration](docs/architecture/talktual-integration.md).

The spec's logical layers map onto crates as follows (rationale: ADR-0001):

| Logical layer (spec §3) | Where |
| --- | --- |
| Foundation/Types | `ce-core`: `ids`, `version`, `time`, `value`, `privacy`, `error`, `resource`, `config` |
| Contracts | `ce-core`: `module`, `capability`, `event`, `command`, `session`, `health`, `provider`, `observability`, `lifecycle` |
| Runtime | `communication-engine/src/runtime/` (private) |
| Facade | `communication-engine/src/lib.rs` |
| Modules | future crates depending on `ce-core`; Phase 1 has only the mock |
| Adapters | `ce-core::provider` contract; mock in `ce-testkit` |
| Assurance | `ce-testkit`, `tests/`, `examples/`, `scripts/verify.sh`, CI workflow, docs |

## 2. Runtime model

One **core task** owns all runtime state (registry, module table, sessions,
event sequence counter). Hosts talk to it through a bounded command queue;
modules talk to it through a bounded ingress queue. Each started module runs in
its own **actor task**, which owns the module instance and its supervised
background tasks.

```
 Engine ──commands (bounded, admission policy)──► Core task ──► broadcast (bounded) ──► EventSubscription(s)
                                                    │  ▲
                        ctrl (bounded, priority)    │  │ ingress (bounded): module events,
                        events (bounded mailbox)    ▼  │ task/handler failure reports
                                               Module actor ──JoinSet──► supervised tasks
```

* **One async runtime: Tokio**, the caller's. CE never creates a runtime
  (ADR-0002). It works on current-thread and multi-thread runtimes (both
  tested).
* **Control plane only.** This async runtime is not, and must never become, a
  real-time audio path ([realtime.md](docs/architecture/realtime.md)).
* **No detached tasks.** The core's handle is held by `Engine`; actor handles
  by the core; module tasks by the actor's `JoinSet`. Shutdown joins
  everything or aborts it after its deadline.
* **No locks around runtime state.** Single ownership by the core.

Details: [lifecycle](docs/architecture/lifecycle.md),
[events and queues](docs/architecture/events.md),
[errors and health](docs/architecture/errors-and-health.md),
[privacy](docs/architecture/privacy.md).

## 3. Public facade

```rust
let engine = Engine::builder().config(cfg).build()?;   // validate config, spawn core
let mut events = engine.subscribe()?;                   // bounded, lag-reporting stream
engine.register_module(factory).await?;                 // descriptor only, no instance
engine.capabilities().await?;                           // discovery (+ find_capability)
engine.start().await?;                                  // resolve → validate → init → start
let s = engine.create_session(SessionConfig::default()).await?;
engine.health().await?;                                 // probes modules
engine.start_analysis(stream, None).await?;             // Phase 3
engine.start_recognition(stream, None).await?;          // Phase 4 (adapters via builder.asr_adapter)
engine.start_semantics(stream, None).await?;            // Phase 5 (rule adapter by default; builder.semantic_adapter)
engine.execute(Command::…).await?;                      // typed, serializable commands
engine.shutdown().await?;                               // bounded, reports per module
```

The facade version is `FACADE_API_VERSION = "0.1"` (provisional). Commands,
outcomes, events and configuration are `serde`-serializable so a later FFI,
IPC or SDK layer can carry them unchanged.

## 4. Configuration

`EngineConfig { config_version, runtime: RuntimeConfig, modules }` — typed,
validated at `build()`, partial documents override defaults, unknown fields are
rejected, unsupported versions are rejected, and secret-looking keys must use
`{"secret_ref": "NAME"}` references instead of literal values. CE reads no
files or environment variables; the host chooses the source. Defaults are
documented on `RuntimeConfig`.

## 5. Decisions

ADRs in [docs/architecture/adr](docs/architecture/adr/):
0001 workspace and crates · 0002 async runtime and concurrency · 0003 module
contract · 0004 event model · 0005 lifecycle · 0006 errors · 0007 privacy ·
0008 real-time boundary · 0009 dependencies and licensing · 0010 audio
backend selection · 0011 audio stream architecture · 0012 audio
normalization contract · 0013 capture authorization gate · 0014 audio
intelligence architecture · 0015 VAD selection · 0016 analysis results and
units · 0017 analysis privacy · 0018 analysis resource policy · 0019
speech architecture · 0020 ASR and language-ID selection · 0021 transcript
revision semantics · 0022 language observations · 0023 real-ASR baseline
(sherpa-onnx) · 0024 Talktual ↔ CE bridge · 0025 semantic layer and
contracts · 0026 rule baseline and semantic evaluation.
