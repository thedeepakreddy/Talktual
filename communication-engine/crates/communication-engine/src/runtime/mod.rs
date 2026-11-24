//! Runtime internals. Nothing here is public; the facade in `lib.rs` is the
//! only supported entry point.

pub(crate) mod actor;
pub(crate) mod core;
pub(crate) mod metrics;
pub(crate) mod registry;
