# ADR-0032 — Timeline normalization, causal validation and latency policy

Status: Accepted (Phase 7, pending the independent audit) · Date: 2026-11-24

## Decision

* **Clock domains are kept separate**:
  * the sample clock (`frame_index`) is authoritative per stream;
  * reported `time_range` values are CE monotonic time derived from a
    host anchor;
  * `host_time` and `device_time` are separate time bases;
  * wall time is never used for ordering or durations.
* **Normalization** produces media time `frames / rate`. It is exact,
  and it is placed *next to* the reported monotonic times, never instead
  of them.
  * The observed offset (`time_range.start − media(start)`) is tracked
    per stage. A change of more than one frame is a **re-anchor**: it
    opens a new timeline segment, is counted, and is reported, never
    smoothed.
  * When `time_range` is unanchored (zero while frames advance), timing
    is `unknown` and the span is marked unknown. Nothing is invented.
  * The uncertainty of the monotonic mapping is reported as `unknown`
    unless a bound is provided. CE does not claim device and host
    synchronization.
* **Causal validation annotates and counts; it does not drop.**
  * Outputs are classified against the reconciled source revision as
    current, superseded, unknown (a violation) or evicted (outside the
    bounded window).
  * Dropping a result within a generation would break Phase 5–6 consumer
    contracts (gapless translation sequences, observation versions).
    Only stale-generation signals are dropped (ADR-0031).
* **Ordering**:
  * Transcript revisions per utterance must increase: older is `stale`,
    equal is `duplicate`, and anything after a final or cancel is
    `after_final`.
  * A new utterance must not start before the previous one on the same
    stage; otherwise it is `out_of_order`.
  * A range with end before start, or a duration inconsistent with its
    frames, is a temporal-integrity violation.
* **Latency is measured on monotonic time only.**
  * Audio-to-output latency is `now − source time_range.end` for final
    transcripts, final semantic observations and final translations.
  * Queue wait and processing time come from Phase 6 timing.
  * Orchestration overhead is the time spent in the observer per event.
* **Deadlines** are optional per-output budgets. A miss is a typed event
  and a counter. A late result is still published (Phase 4–6 consumers
  rely on finals) and never replaces newer state. Deadlines never cancel
  work silently.

## Consequences

* Latency is only as accurate as the anchor: bounded by the capture poll
  interval plus one device buffer (Phase 2). It is reported as such.
* Observation does not prevent violations; it makes them visible. Within
  a generation, workers remain the authority on their own ordering.

## Audit amendment (AUD-08, AUD-09)

* Audio gaps are stream-level findings: `timeline.stage` is optional, and
  `null` for gaps.
* At most 32 findings per kind and stage are published per epoch. The
  rest are counted (`suppressed_findings`).

## Final amendment (verification audit VA-02)

The event limit applies to **publication only**. The engine-wide
metrics (`causal_violations`, `timeline_anomalies`, `deadline_misses`)
follow the observer's complete counters.
