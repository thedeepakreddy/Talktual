# ADR-0001 — Workspace location and crate split

Status: Accepted (Phase 1) · Date: 2026-11-24

## Context
The spec suggests `communication-engine/` with `ce-types`, `ce-contracts`,
`ce-runtime`, `ce-facade`, `ce-testkit` and an optional `ce-cli`, while
warning against needless crates. Talktual has no Rust code; it is a Vite/React
app plus a Node server deployed on Vercel/Render.

## Decision
* A **self-contained Cargo workspace at `communication-engine/`** (not a
  repository-root workspace), with its own lockfile, toolchain pin, docs and a
  path-filtered CI workflow.
* **Three crates:**
  * `ce-core` = foundation types **and** contracts (spec's ce-types +
    ce-contracts);
  * `communication-engine` = runtime (private module) **and** facade (public
    API) (spec's ce-runtime + ce-facade);
  * `ce-testkit`.
* No `ce-cli` (optional in the spec; the smoke example covers the need).

## Rationale
* Types and contracts have the same consumers (module and adapter authors) and
  the same stability horizon; contracts are mostly type definitions. Splitting
  them adds a crate boundary without an ownership or reuse boundary.
* Making the runtime a private module behind the facade gives a **compiler
  enforced** boundary: hosts cannot reach runtime internals. Separate crates
  would require the runtime's internals to be `pub` for the facade, which is
  a weaker boundary.
* The split that matters — modules must not depend on the runtime or an
  executor — is preserved: `ce-core` has no async runtime dependency.
* A sub-directory workspace keeps Talktual's toolchains untouched and makes
  CE extractable with `git subtree split`.

## Consequences
* If a second runtime implementation or independent facade governance ever
  appears, `communication-engine` can be split without changing `ce-core`.
* Crate names are provisional; nothing is published (`publish = false`).
