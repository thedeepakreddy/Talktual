//! Communication Engine speech recognition and language intelligence
//! (Phase 4).
//!
//! * [`adapter`] — the ASR adapter boundary and output sanitization;
//! * [`fake`] — the deterministic scripted adapter and the reference
//!   (oracle) adapter used for replay and pipeline evaluation;
//! * [`transcript`] — stable-prefix commits, snapshot/delta revisions and
//!   the consumer-side assembler;
//! * [`language`] — text language identification with scope, uncertainty
//!   and provenance;
//! * [`resample`] — downmix and streaming resampling to the adapter rate;
//! * [`recognizer`] — the deterministic per-stream recognizer (Phase 3
//!   hints, gaps, finalization, timestamps, governor);
//! * [`worker`] — the supervised, bounded recognition thread;
//! * [`eval`] — replay, manifests and metrics;
//! * [`metrics`] — bounded percentiles.
//!
//! Nothing here translates, interprets meaning, identifies speakers or
//! infers personal traits. Transcripts are `Content`.

#![forbid(unsafe_code)]

pub mod adapter;
pub mod eval;
pub mod fake;
pub mod language;
pub mod metrics;
pub mod recognizer;
pub mod resample;
pub mod transcript;
pub mod worker;

pub use adapter::{AdapterHypothesis, AdapterWord, AsrAdapter, AsrFactory, LoadOutcome};
pub use language::{LanguageIdentifier, WhichlangIdentifier};
pub use recognizer::StreamRecognizer;

/// FNV-1a 64-bit, 16 hex digits (stable fingerprints).
pub(crate) fn fnv(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}
