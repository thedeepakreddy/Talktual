# ADR-0018 — Analysis resource policy and degraded modes

Status: Accepted (Phase 3) · Date: 2026-11-24

## Decision
* **Capture first**: analysis can only ever lose its *own copy* of audio
  (tap drops). Nothing in analysis can block or slow the capture worker,
  the host queue, other analyses or the control plane.
* **Load** = processing time of a block ÷ the block's audio duration,
  measured by the worker; **backlog** = blocks waiting in the tap.
* **Degraded levels** (`DegradedMode`), stepped one at a time:
  * `Full` — everything.
  * `Reduced` — spectral features off; the energy detector scores (its floor
    is kept warm in `Full`, so the switch is seamless); conditions cleared
    and suspended; quality continues.
  * `Shedding` — every other block is skipped (counted as lost frames in
    quality); activity and segmentation are suspended (an open segment
    closes with `ResourcePolicy`); quality continues on analysed blocks.
* **Hysteresis**: step down after `degrade_after` (5) consecutive blocks
  over budget (load > `max_load` 0.5 or backlog > half the tap); step up
  after `recover_after` (100) consecutive blocks comfortably within budget
  (load < `max_load`/2, backlog ≤ 1). Every change is a
  `DegradedModeChanged` event with the load and backlog that caused it;
  results carry the mode in `ResultMeta`.
* `budget.degrade = false` disables mode changes (overload then only shows
  as tap drops) — for hosts that prefer it and for functional tests on
  loaded machines.
* **Stall**: no heartbeat for `stall_timeout_ms` (2 s) → the core reports
  `Failed(analysis.stalled)` + `Stopped(Failed)`, stops and abandons the
  thread (reaped when it exits; shutdown waits for it, bounded).
* **Adaptive (input-driven) policy** is separate and also reversible:
  conservative segmentation on low SNR / sustained noise, suppressed level
  estimates on clipping/overload, low-confidence activity on unusable input.

## Consequences
Under overload CE degrades predictably and visibly instead of falling
behind silently. Load is wall-clock, so live mode changes are not
deterministic; replay feeds recorded (or zero) load to stay deterministic.
