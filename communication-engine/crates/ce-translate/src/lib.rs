//! Communication Engine translation intelligence (Phase 6).
//!
//! Consumes Phase 4 transcript revisions (and, optionally, final Phase 5
//! entity observations as bounded hints) and produces target-language
//! translation results linked to exact source revisions (contracts in
//! [`ce_core::translation`]).
//!
//! * [`adapter`] — the provider-neutral adapter boundary;
//! * [`memory`] — `ce.translate.memory`, a deterministic offline
//!   translation-memory baseline (exact segments only, never invents);
//! * [`scripted`] — a scripted adapter for lifecycle and failure tests;
//! * [`fidelity`] — terminology matching, protected spans, fidelity flags;
//! * [`processor`] — revision handling, language policy, selection and
//!   fallback, validation, lifecycle;
//! * [`worker`] — the supervised, bounded, coalescing worker thread;
//! * [`eval`] — corpus, metrics (BLEU, chrF, fidelity checks) and replay.
//!
//! No model, network or credential is needed by anything in this crate.

pub mod adapter;
pub mod eval;
pub mod fidelity;
pub mod memory;
pub mod metrics;
pub mod processor;
pub mod scripted;
pub mod worker;

pub use adapter::{
    AdapterOutcome, ContextHint, TranslationAdapter, TranslationFactory, TranslationRequest,
};
pub use memory::{MemoryFactory, TranslationMemory};
pub use processor::{AdapterSlot, ProcessorSetup, TranslationProcessor, check_setup};
pub use scripted::{Faults, ScriptedFactory, scripted_capabilities};
pub use worker::{PushOutcome, TranslationHandle, TranslationSignal, spawn_translation};
