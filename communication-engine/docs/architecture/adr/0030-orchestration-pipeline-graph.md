# ADR-0030 — Orchestration pipelines and graph validation

Status: Accepted (Phase 7, pending the independent audit) · Date: 2026-11-24

## Context

Stages were started one by one. Their coupling was implicit: the core
forwarded data to whatever happened to be running. Nothing validated that
a stage's inputs existed, and nothing defined what an optional stage's
failure means.

## Decision

* A new pure crate, **`ce-orchestrate`**, holds the graph, epoch,
  reconciliation, causal, timeline and latency logic. It has no tokio
  dependency and checks for wasm32. The runtime keeps only the effectful
  parts.
* A **pipeline** is one capture stream plus stages from a fixed set of
  kinds: analysis, recognition, semantics and translation. Each stage is
  `required` or `optional` and has an optional configuration.
* **Dependencies come from the stage contracts** (see
  `architecture/orchestration.md` §2): hard dependencies, and soft inputs
  that become notes when absent.
* **Rules**:
  * a pipeline is never empty and has no duplicate kind;
  * every hard dependency is present;
  * a required stage never depends on an optional one;
  * per-stage configuration validates;
  * the generic DAG check rejects cycles, and the order is Kahn's with
    declaration-order tie-breaking (deterministic).
* **Runtime pre-flight** runs before anything starts: the stream is live
  and capture, adapters resolve, and capacity is available
  (`orchestration.max_pipelines`, default 8).
* **Execution**: start in topological order; a required failure rolls
  back in reverse order. Stop is `drain` (upstream first) or `cancel`
  (downstream first). Reset runs in dependency order; restart is stop
  then start, with a new epoch.
* **Fingerprint**: FNV over a versioned rendering of the validated graph,
  using configuration fingerprints, so no secrets are included.
* **No hot reconfiguration.** A changed spec applies at restart.
* The per-stage commands from Phases 3–6 remain available. A stage of a
  pipeline can still be stopped directly; the pipeline observes the end
  and degrades or fails accordingly.

## Consequences

* The changes are additive: new commands, events, status and
  configuration. Existing behaviour is unchanged for engines that do not
  use pipelines, apart from ADR-0031 and ADR-0033.
* The stage kinds are fixed. A new kind needs a new rule entry (a code
  change, deliberately).
