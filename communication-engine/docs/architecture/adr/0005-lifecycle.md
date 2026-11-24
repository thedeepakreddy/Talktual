# ADR-0005 — Explicit lifecycle tables

Status: Accepted (Phase 1) · Date: 2026-11-24

## Decision
Module (`Discovered → Validated → Initialized → Ready → Running → Stopping →
Stopped`, plus `Failed`), runtime and session lifecycles are `const fn`
transition tables in `ce-core`, applied only through `transition_to` /
`can_transition_to`; illegal transitions are typed `Lifecycle` errors. The
runtime applies module transitions only through the table, so a late reply
cannot resurrect a failed module.

* `Validated` = dependencies resolved and configuration accepted for the whole
  graph before any instance exists.
* `Ready` = every module in the graph initialized.
* `Failed → Stopping` exists so failed instances still release resources.
* A failed start stops all instantiated modules in reverse order and puts the
  runtime in `Failed`, from which only shutdown is allowed.

See lifecycle.md for the full table and stop semantics.
