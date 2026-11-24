# Talktual integration strategy

## Where CE lives

`communication-engine/` at the root of the Talktual repository, as a
self-contained Cargo workspace (its own `Cargo.toml`, `Cargo.lock`,
toolchain pin, `target/`, docs and CI workflow). Rationale:

* Talktual had **no Rust code** and is a Vite/React frontend plus a Node
  signaling server; a root-level Cargo workspace would mix toolchains and
  surprise existing tooling (Vercel builds, `tsc`, Vitest).
* A self-contained directory can be extracted into its own repository with
  `git subtree split` and no rewrite.
* Its CI workflow is path-filtered to `communication-engine/**`, so Talktual's
  existing pipeline is unaffected.
* CE docs live under `communication-engine/docs/` to avoid colliding with
  Talktual's `docs/phase1…phase8_5` (Talktual's own product phases, which are
  unrelated to CE's phases).

Existing Talktual tooling was verified unaffected after the change: `npm run
lint`, `npm test` (537 tests), `npm run test:server` (60 tests) and `npm run
build` were re-run with the same results as the baseline.

## Relationship to Talktual's existing TypeScript "engine"

Talktual already has `src/engine/CommunicationSession.ts` (its Phase 2): a
React-free, in-browser orchestrator for one call — WebRTC transport, Web
Speech recognition, translation and TTS adapters, call-phase state machines,
privacy gates and a versioned event bus. That code is **Talktual-specific and
remains Talktual's**; Phase 1 does not modify, replace or depend on it.

Naming: in CE documents, "Communication Engine (CE)" means this Rust
workspace. Talktual's TypeScript layer is referred to as Talktual's
`CommunicationSession` engine.

The TypeScript engine is the best available statement of what Talktual will
need from CE, and its concepts map onto CE contracts:

| Talktual TS engine | CE contract |
| --- | --- |
| call phase / activity state machines | `SessionState` + future module state; product phases stay in the product |
| `EngineEvent` v1, peer wire messages | `EventEnvelope` (schema v1); wire protocols stay in transport adapters |
| `CapabilityRegistry` (browser/server capabilities, degradation reasons) | capability descriptors with unknown-stays-unknown fields + health |
| `PrivacyPolicyController` (modes, consent records, capture gate) | `ConsentPurpose`/`ConsentState` per session; gating to be enforced in Phase 2 |
| `Diagnostics` (redacted ring buffer) | `ObservabilitySink` + `RedactionPolicy` + metrics |
| provider adapters in `src/lib/` | `Provider` contract + future CE adapter crates |

## Integration paths (not implemented in Phase 1)

Phase 1 explicitly excludes Talktual feature integration. The candidate paths,
for decision after Phase 2's platform research and Talktual's open decision
record DR-8.5-1 (web-first vs. native shell):

1. **Native shell (iOS/Android)** embeds CE through an FFI layer (e.g.
   UniFFI), with commands/events crossing as serialized `Command` /
   `EventEnvelope`. CE core and runtime already check-compile for
   `aarch64-apple-ios` and `aarch64-linux-android`.
2. **Server side** (Talktual's Node signaling server) via a Node binding or a
   sidecar process speaking the same serialized protocol.
3. **Browser** via WebAssembly. `ce-core` compiles for `wasm32-unknown-unknown`;
   the runtime does not yet (UUID randomness needs a wasm source, and Tokio's
   timer is unavailable there). This needs a dedicated spike.

In every path the boundary is the facade plus serialized commands and events;
Talktual-specific concepts (rooms, peers, push-to-talk UI) stay in Talktual.
