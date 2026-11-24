//! # ce-testkit — deterministic test support for the Communication Engine
//!
//! * [`ManualClock`] and [`SequentialIds`] — deterministic time and identity.
//! * [`RecordingSink`] — captures (redacted) observations for assertions.
//! * [`MockProvider`] — a provider adapter with scriptable health/failures.
//! * [`MockModuleFactory`] / [`MockProbe`] — a configurable module that can
//!   fail, hang or panic at any lifecycle stage, run supervised background
//!   tasks, consume events slowly and acknowledge sessions with correlated
//!   events.
//! * [`EventRecorder`] — waits for and collects events with timeouts.
//! * [`fake_audio`] — a scriptable fake audio backend with fault injection.
//! * [`fixtures`] — small configurations and descriptors.
//!
//! This crate is for tests and examples only.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod clock;
pub mod fake_audio;
pub mod fixtures;
mod mock_module;
mod mock_provider;
mod recorder;

pub use clock::{ManualClock, RecordingSink, SequentialIds};
pub use mock_module::{
    BackgroundTask, MockBehavior, MockModuleFactory, MockProbe, ProbeState, Stage,
};
pub use mock_provider::MockProvider;
pub use recorder::EventRecorder;
