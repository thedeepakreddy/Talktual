# ADR-0016 — Analysis results, metadata and units

Status: Accepted (Phase 3) · Date: 2026-11-24

## Decision
* **Every result carries `ResultMeta`**: stream, source block sequence
  range, sample-clock frame range `[start, end)`, monotonic time range
  (the Phase 2 block `timestamp` timeline), the discontinuity that shaped it
  (while the detector re-adapts after a gap) and the degraded mode in force.
  `Started` and `AnalysisStatus` carry the algorithm identity (detector id,
  version, `AnalysisConfig::fingerprint`).
* **Units**: times in CE monotonic nanoseconds; durations in ms; levels in
  dBFS where 0 dBFS is a full-scale square wave (a full-scale sine is
  −3.01 dBFS); ratios in [0, 1].
* **Activity** boundaries are backdated to the first qualifying hop;
  `decision_latency_ms` reports how much later the change was decided.
* **Segments are acoustic candidates**: they close on silence policy,
  maximum duration (with a continuation), stop, stream end, discontinuity,
  resource policy or failure — always with that reason. **Endpoint
  candidates** mean "activity has paused this long" (tiers, default 250 /
  500 ms), never "the speaker finished": turn-taking is a later phase.
* **Quality** is per window (default 1 s): level, peak, clipping ratio,
  noise floor (p10 of inactive hops over 3 s), SNR estimate, DC offset,
  channel imbalance, non-finite samples, lost frames, activity ratio,
  usability (Good/Fair/Poor/Unusable) with sorted reasons. Values that
  cannot be measured are `None`, never guessed. Changes are reported after
  two consistent windows.
* **Events are sparse**: transitions, segments, candidates and changes
  only — never per-hop data or audio. Measured on 10 minutes of synthetic
  speech/noise: < 400 events per minute (gate in the stress test).
* **Discontinuities are never bridged**: a gap closes the open segment
  (`Discontinuity`), ends activity, resets adaptive state and is reported
  once with its context (capture kind and lost frames, analysis drop with
  block count, or format change).

## Consequences
Downstream phases can align results with audio by frame index and time,
know exactly which algorithm and configuration produced them, and tell
degraded or gap-affected results apart.
