# ADR-0017 — Analysis privacy and inference boundaries

Status: Accepted (Phase 3) · Date: 2026-11-24

## Decision
* Analysis outputs describe **the signal, never the person**. CE does not
  infer or expose speaker identity, emotion, psychological state, health,
  personality, age, gender or any sensitive characteristic, and does not
  attempt speaker diarization, language detection or speech content.
* The condition set is **closed and documented**: sustained silence,
  probable activity, sustained noise, clipping, overload, dropout. Adding a
  condition is a contract change requiring review against this ADR.
* **Overlap (several simultaneous talkers) is not claimed**:
  `AnalysisCapabilities::overlap_detection` is `false`.
* **No audio leaves the analysis**: events contain numbers, enums and time
  ranges only; samples are never logged, put in events, persisted or sent.
  Analysis events are `Operational` privacy class.
* **Authorization**: analysis consumes audio of an authorized capture
  stream only, locally and ephemerally, so it is covered by the session's
  `Capture` consent; it needs no remote-processing or retention consent.
  Revocation ends the analysis immediately and discards queued audio.
* **Fixtures** are synthetic TTS (no real person's voice); see
  `fixtures/speech/PROVENANCE.md`.

## Consequences
Later phases that need content or speaker features (ASR, diarization,
voice profiles) must bring their own consent purposes and review; nothing
in Phase 3 pre-authorizes them.
