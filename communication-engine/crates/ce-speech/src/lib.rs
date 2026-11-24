//! Communication Engine speech recognition and language intelligence
//! (Phase 4). WORK IN PROGRESS — see docs/phases/phase-4-handoff.md.
//!
//! Done: [`adapter`] (ASR adapter boundary + output sanitization),
//! [`transcript`] (stable-prefix commits, snapshot/delta revisions,
//! consumer assembler), [`resample`] (downmix + streaming resampler),
//! [`metrics`] (bounded percentiles).
//! Pending: language identification, fake/reference adapters, the
//! per-stream recognizer, the supervised worker, evaluation/replay.

#![forbid(unsafe_code)]

pub mod adapter;
pub mod metrics;
pub mod resample;
pub mod transcript;
