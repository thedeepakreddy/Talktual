# Phase 1 — Reconnaissance, baseline and plan

Date: 2026-11-24 · Branch: `deepak/gallant-hamilton-wegi2i` (from `main` @ `ae1e283`)

## Repository reconnaissance

* **Talktual** is a peer-to-peer translating walkie-talkie: Vite 6 + React 19
  + TypeScript frontend (`src/`), Node/Express/ws signaling and provider proxy
  (`signaling-server/`), Playwright-driven E2E scripts (`e2e/`), evaluation
  harnesses (`eval/`), product phase docs (`docs/phase1` … `docs/phase8_5`).
  Deployed as a static site (Vercel) plus a Node service.
* **No Rust** anywhere (no `Cargo.toml`, no `.rs`). No `.github/` CI
  workflows; the documented CI gate is `npm run test:all`.
* **Existing engine-like code:** `src/engine/` (≈5.3k lines TS) —
  Talktual's React-free `CommunicationSession` with state machines, a
  versioned event bus, capability registry, privacy controller and redacted
  diagnostics. Talktual-specific (WebRTC, Web Speech, push-to-talk).
* **Naming collisions to avoid:** "Communication Engine" is already used for
  the TS layer in Talktual's Phase 2 docs; `docs/phaseN` directories are
  Talktual product phases.
* **Target platform signals:** Talktual's DR-8.5-1 weighs web-first vs. a
  thin native iOS shell (AVAudioSession, SFSpeechRecognizer); Android and
  desktop are plausible. CE must therefore stay portable to iOS, Android,
  desktop and (later) wasm.
* **Git:** working tree clean; `main` and the session branch at the same
  commit before work began.

## Baseline (before any change) — Linux x86_64, Node 22, Rust 1.97.0

| Command | Result |
| --- | --- |
| `npm run lint` (tsc) | pass |
| `npm test` (Vitest) | 30 files, 537 tests pass |
| `npm run test:server` | 60 tests pass |
| `npm run build` | pass (chunk-size warning only) |
| Rust toolchain | rustc/cargo 1.97.0 stable, clippy + rustfmt present; crates.io reachable |

## Integration decision

CE is a self-contained Cargo workspace at `communication-engine/` (see
[talktual-integration.md](../architecture/talktual-integration.md) and
ADR-0001). No existing Talktual file is modified except: a new path-filtered
workflow under `.github/workflows/`, and a short pointer section in the root
README.

## Plan (spec §15 sequence)

0. Reconnaissance and baseline (this document).
1. ADRs 0001–0009.
2. Workspace (`ce-core`, `communication-engine`, `ce-testkit`), lints,
   toolchain pin, CI workflow, verify script.
3. Contracts and lifecycle tables with exhaustive tests.
4. Registry, bounded command/event flow, single-owner core, actors,
   cancellation, shutdown.
5. Sessions and correlation.
6. Mock module/provider with failure scripting.
7. Facade and smoke example.
8. Test kit, fixtures, integration tests (lifecycle, capabilities, sessions,
   failures, backpressure, cancellation, observability).
9. Gates: fmt, clippy, tests, docs, smoke, cross-target, MSRV, cargo-deny;
   re-run Talktual baseline.
10. Architecture review and report.
