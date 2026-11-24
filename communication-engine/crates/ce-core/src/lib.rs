//! # ce-core — Communication Engine foundation and contracts
//!
//! This crate holds everything a Communication Engine (CE) module, adapter or
//! host needs to agree on, and nothing that requires an async runtime:
//!
//! * **Foundation types** — typed identifiers ([`ids`]), versions ([`version`]),
//!   time ([`time`]), structured values ([`value`]), privacy classes
//!   ([`privacy`]), typed errors ([`error`]) and resource limits ([`resource`]).
//! * **Contracts** — the module lifecycle state machine ([`lifecycle`]), the
//!   module/factory/host contract ([`module`]), capability descriptors
//!   ([`capability`]), the event envelope ([`event`]), commands ([`command`]),
//!   sessions ([`session`]), health ([`health`]), providers ([`provider`]) and
//!   observability ([`observability`]).
//!
//! The runtime that executes these contracts lives in the `communication-engine`
//! crate. Module authors depend on `ce-core` only, so a module never links the
//! runtime and never depends on a particular executor.
//!
//! ## Stability
//!
//! **Provisional (Phase 1).** Every public item here may change before CE
//! publishes a stable SDK. Versioned contracts ([`version::ContractVersion`],
//! [`event::EVENT_SCHEMA_VERSION`], [`config::CONFIG_VERSION`]) exist so that
//! changes can be detected rather than silently misinterpreted.
//!
//! ## Product neutrality
//!
//! Nothing in this crate knows about any product (including Talktual), UI,
//! transport, database, cloud vendor or speech provider.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod audio;
pub mod capability;
pub mod command;
pub mod config;
pub mod error;
pub mod event;
pub mod health;
pub mod ids;
pub mod intel;
pub mod lifecycle;
pub mod module;
pub mod observability;
pub mod privacy;
pub mod provider;
pub mod resource;
pub mod semantic;
pub mod session;
pub mod speech;
pub mod time;
pub mod value;
pub mod version;

pub use error::{CeError, ErrorCategory, Recoverability};

/// Result alias used across CE contracts.
pub type CeResult<T> = Result<T, CeError>;

/// A boxed, `Send` future — the async return type of every object-safe CE
/// contract method.
///
/// CE uses explicit boxed futures rather than `async fn` in traits because
/// modules are stored as trait objects (`Box<dyn Module>`), and `async fn` in
/// traits is not object-safe on the supported toolchains. See ADR-0003.
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;
