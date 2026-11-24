//! # ce-audio — Communication Engine audio foundation (Phase 2)
//!
//! * [`backend`] — the **backend adapter boundary**: the [`AudioBackend`]
//!   trait every platform/library adapter implements, in CE-owned types only.
//! * [`rt`] — the **real-time handoff**: [`CaptureEndpoint`] and
//!   [`PlaybackEndpoint`], the only objects a device callback touches. They
//!   use pre-allocated lock-free SPSC rings (`rtrb`) and atomics: no locks,
//!   no allocation, no logging, no I/O.
//! * [`convert`] — explicit, tested sample-format conversion and channel
//!   downmix.
//! * [`worker`] — one supervised worker thread per stream: drains the ring,
//!   assembles timestamped blocks, applies overflow policy, reports faults.
//! * [`tap`] — a bounded, non-blocking analysis tap on capture streams
//!   (copies for in-engine consumers; drops instead of blocking).
//! * [`io`] — the host-facing data plane: [`CaptureReader`] and
//!   [`PlaybackWriter`] over bounded queues.
//! * `cpal_backend` (feature `cpal`) — the cross-platform adapter selected
//!   in ADR-0010.
//!
//! The control plane (lifecycle, consent, events) lives in the
//! `communication-engine` runtime. See `docs/architecture/audio.md`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod backend;
pub mod convert;
pub mod io;
pub mod rt;
pub mod tap;
pub mod worker;

#[cfg(feature = "cpal")]
pub mod cpal_backend;

pub use backend::{
    AudioBackend, AudioSignal, BackendDevice, BackendEnumeration, BackendNotice, BackendNotifier,
    BackendStream, OpenSpec, PermissionOverride,
};
pub use io::{CaptureReader, PlaybackWriter, ReadError};
pub use rt::{CaptureEndpoint, FaultReporter, PlaybackEndpoint, StreamFault};
pub use tap::{TapEnd, TapReceiver, TapRecvError, TapSender, analysis_tap};
pub use worker::{
    StreamContext, StreamEndpoint, WorkerCommand, WorkerConfig, WorkerHandle, WorkerReport,
    spawn_stream,
};
