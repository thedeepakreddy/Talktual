# ADR-0033 — Control-plane scheduling: command priority, fair data arms

Status: Accepted (Phase 7, pending the independent audit) · Date: 2026-11-24

## Context

The core's single `select!` was `biased` across every arm. A data channel
polled early (audio, analysis, speech) that never empties would starve
the arms polled after it (semantics, translation, housekeeping). Spec §10
requires the absence of starvation or a justified priority.

## Decision

* An **outer `biased` select** keeps runtime cancellation and **commands**
  first. Commands arrive at user rate, and control must not wait behind
  data.
* An **inner, unbiased select** (random start) covers the data arms:
  * ingress;
  * module exits;
  * the audio, analysis, speech, semantic and translation signals;
  * housekeeping.

  Every ready data arm is chosen with non-zero probability on each
  iteration, so none can be starved by another.
* **Module exit**: module events emitted before the exit are processed
  first. The exit arm drains the ingress queued at that moment (bounded,
  audit AUD-14) before reaping the module.
  This guarantee used to come from the arm order, and it is now explicit.
* Per-stream FIFO order within a channel is unchanged. Cross-channel
  order was never guaranteed: consumers order by the envelope `sequence`
  (events.md).

## Consequences

* Command priority can delay data under a command flood. Commands are
  bounded by the command channel and handled quickly, so this is
  accepted.
* Behaviour is otherwise unchanged, and the event sequence remains the
  authoritative order.
