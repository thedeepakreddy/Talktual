# ADR-0031 — Lifecycle epochs and stale-output rejection

Status: Accepted (Phase 7, pending the independent audit) · Date: 2026-11-24

## Context

Each stage type has one signal channel into the core, shared by all its
workers, and signals identify only the stream. A signal queued by a
stopped or abandoned worker is processed as if it came from the current
worker on that stream:

* a stale `Ended` removes the new worker's entry;
* stale events are published after the core already reported that worker
  `Stopped`.

Spec §9 and §11 require restart to establish a new generation and reject
prior-generation results.

## Decision

* **Every stage worker start gets a generation**: a u64 that is
  monotonic across the engine.
* **Per-worker channel and forwarder**: the runtime gives the worker its
  own bounded channel (64). A forwarding task tags each signal and sends
  it into the stage's core channel.
  * Order within a worker is preserved, and backpressure is preserved
    (the forwarder awaits the core channel).
  * The task ends when the worker drops its sender (its thread exits) or
    the core's receiver is gone. A hung, abandoned worker keeps its
    forwarder until its thread exits. This is bounded by the abandoned
    workers and visible in the `worker_forwarders` gauge.
  * The worker crates (Phases 3–6) are **unchanged**.
* **Admission**: the core admits a tagged signal only if the stream's
  running entry has the same generation. Anything else is counted
  (`stale_outputs_rejected`), logged, reported as
  `orchestration.stale_output_rejected` (operational; ids only) and not
  published.
* **Pipelines** add `epoch` (start or restart) and `reset_epoch`. Both
  are reported in orchestration events and in the status.
* **Translation**: `Started` resets consumer state, and sequences are
  worker-local (Phase 6). The observer's translation view resets on
  `Started`, and generation admission prevents an older run's results
  from following a newer `Started`.

## Consequences

* **Behaviour change**: events from an abandoned worker that were still
  queued are no longer published after the core's synthesized `Stopped`.
  Before, they could be. This is intended; it fixes a lifecycle defect.
* **Cost**: one forwarding task and one 64-slot channel per running
  worker, plus one hop.
* **Evidence**: the race needs a signal in flight while a worker is
  abandoned. It is covered by unit tests of the admission logic and by
  runtime restart tests; it is not reproduced end to end
  deterministically (see the report).
