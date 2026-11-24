# ADR-0021 — Streaming transcript and revision semantics

Status: Accepted (Phase 4) · Date: 2026-11-24

## Decision
* **Identity.** A transcript belongs to a stream and an *utterance*
  (`SpeechMeta::utterance`, 0-based per stream, never reused). Utterances
  never span a discontinuity (gap, restart, format change).
* **Revisions.** Each utterance emits revisions numbered 0, 1, 2, … without
  gaps; the last one has status `Final` or `Cancelled`; nothing follows it.
  Every revision carries the **full snapshot** (`committed` + `provisional`)
  and a **delta** from the previous one (`keep_committed_chars`,
  `append_committed`, `provisional`). Applying deltas in order from
  revision 0 reproduces every snapshot exactly; a consumer that missed a
  revision uses the snapshot. `TranscriptAssembler` is the reference
  consumer: it rejects stale and duplicate revisions (number ≤ last
  applied), anything after the end, and revisions of another utterance.
* **Stable / committed text** (model-independent `StabilityPolicy`): a word
  prefix is stable when unchanged across `stable_after_hypotheses`
  consecutive hypotheses (default 3) or for `stable_after_ms` of audio
  (default 800 ms); the stable prefix minus `holdback_words` (default 1)
  is committed. Committed text is append-only before the final. A later
  hypothesis that contradicts committed words is counted but does not
  rewrite them (the provisional tail continues after the same number of
  words). **Only the final may amend committed text, and it says so**
  (`commit_amended`). Corrections in the provisional tail are never
  suppressed; identical hypotheses are deduplicated.
* **Partials** are rate-limited by `partial_interval_ms` of audio
  (hypotheses in between are skipped) and suppressed in `Throttled` mode or
  when the adapter has no partial results.
* **Finalization.** `SegmentEnd` (Phase 3 segment closed on its silence
  policy; the utterance ends `trailing_ms` after the segment's last
  activity), `EndpointCandidate` (configured tier), `AdapterEndpoint`,
  `MaxDuration` (then a continuation opens if activity continues),
  `Discontinuity`, `StreamEnd`, `Stopped`. Cancellation (`Cancelled`):
  `Reset` (requested), `Failure` (adapter error), revocation (queued audio
  discarded). Cancelled revisions keep what was committed and drop the
  provisional tail.
* **Delimitation by hints, not rigid boundaries.** Phase 3 activity onsets
  queue a start at the backdated onset (served from a `preroll_ms` ring);
  onsets inside an open utterance (pauses shorter than the segment-close
  silence) are absorbed; ends of earlier stretches are discarded as stale;
  hints about audio not yet received wait for it, so they map onto the
  right timeline across gaps. The adapter's endpoint and the maximum
  duration still apply. After a recognizer-only gap during activity the
  utterance resumes at the first audio after the gap.
* **Timestamps.** Frames are on the capture stream's sample clock and
  times on the Phase 2 block timeline (the same as Phase 3 results).
  Adapter sample `a` maps to frame `anchor + ⌊a · in_rate / adapter_rate⌋`
  exactly (the resampler adds no group delay to the mapping); word timings
  appear only when the adapter declares them and they validate. Nothing is
  shifted or invented.
* **Scores** carry `ScoreSemantics`; adapters without declared semantics
  produce no scores. No score is a calibrated probability without
  calibration evidence (none exists in Phase 4).

## Consequences
Consumers can render live text with a stable prefix that never jumps back
before the final, and reconstruct state exactly from either deltas or
snapshots. The policy is evaluated for churn, commit delay and amendments
in the Phase 4 evaluation; it trades earlier commits against visible
corrections and never suppresses a correction in the final.
