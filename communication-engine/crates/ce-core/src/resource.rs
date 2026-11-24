//! Resource limits and queue policies.
//!
//! Phase 1 provides *declarations and limits* only; adaptive scheduling and
//! resource-aware routing are deferred. Every queue in CE is bounded by a
//! [`QueueCapacity`], which cannot express "unbounded".

use std::fmt;
use std::num::NonZeroU32;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::CeError;
use crate::ids::ModuleId;

/// Capacity of a bounded queue: `1..=QueueCapacity::MAX`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct QueueCapacity(NonZeroU32);

impl QueueCapacity {
    /// Hard upper bound for any CE queue.
    pub const MAX: u32 = 65_536;

    /// Validate a capacity.
    pub fn new(capacity: u32) -> Result<Self, CeError> {
        match NonZeroU32::new(capacity) {
            Some(nz) if capacity <= Self::MAX => Ok(Self(nz)),
            _ => Err(CeError::invalid_config(
                "config.queue_capacity",
                "queue capacity must be between 1 and 65536",
            )
            .with_context("capacity", capacity.to_string())),
        }
    }

    /// The capacity as `usize`.
    pub fn get(self) -> usize {
        self.0.get() as usize
    }
}

impl TryFrom<u32> for QueueCapacity {
    type Error = CeError;
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<QueueCapacity> for u32 {
    fn from(value: QueueCapacity) -> u32 {
        value.0.get()
    }
}

/// What a producer experiences when a bounded command queue is full.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "policy")]
pub enum AdmissionPolicy {
    /// Fail immediately with a `QueueSaturated` error.
    Reject,
    /// Wait for space (backpressure) for at most `timeout_ms`, then fail with
    /// a `QueueSaturated` error.
    WaitUpTo {
        /// Maximum wait in milliseconds.
        timeout_ms: u64,
    },
}

impl AdmissionPolicy {
    /// The wait bound, if any.
    pub fn wait(&self) -> Option<Duration> {
        match self {
            AdmissionPolicy::Reject => None,
            AdmissionPolicy::WaitUpTo { timeout_ms } => Some(Duration::from_millis(*timeout_ms)),
        }
    }
}

/// What the dispatcher does when a module's event mailbox is full.
///
/// The dispatcher never blocks on one module: a slow module must not stall
/// event delivery to everyone else.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverflowPolicy {
    /// Drop the new event for that module, count it and publish a (coalesced)
    /// `QueueSaturated` event.
    DropNewest,
    /// Treat saturation as a module failure: the module transitions to
    /// `Failed` and is cancelled.
    FailModule,
}

/// Identifies a bounded queue in saturation reports and metrics.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "queue")]
pub enum QueueKind {
    /// The runtime's command queue.
    Command,
    /// The queue carrying module-emitted events into the dispatcher.
    EventIngress,
    /// An external event subscription that fell behind.
    Subscriber,
    /// A module's event mailbox.
    ModuleMailbox {
        /// The module.
        module: ModuleId,
    },
    /// A module's queue of supervised task spawn requests.
    ModuleTasks {
        /// The module.
        module: ModuleId,
    },
}

impl fmt::Display for QueueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueueKind::Command => f.write_str("command"),
            QueueKind::EventIngress => f.write_str("event_ingress"),
            QueueKind::Subscriber => f.write_str("subscriber"),
            QueueKind::ModuleMailbox { module } => write!(f, "module_mailbox:{module}"),
            QueueKind::ModuleTasks { module } => write!(f, "module_tasks:{module}"),
        }
    }
}

/// Coarse compute class, for future scheduling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeClass {
    /// Negligible CPU.
    Light,
    /// Sustained but modest CPU.
    Moderate,
    /// Heavy CPU or accelerator use (e.g. on-device models).
    Heavy,
}

/// Resource hints a capability may declare. Every field is optional and
/// `None` means unknown — CE never infers a value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ResourceHints {
    /// Approximate resident memory, in bytes.
    pub memory_bytes: Option<u64>,
    /// Compute class.
    pub compute: Option<ComputeClass>,
    /// Whether a GPU/NPU accelerator is used.
    pub accelerator: Option<bool>,
    /// Whether network access is needed.
    pub network: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCategory;

    #[test]
    fn queue_capacity_is_always_bounded_and_non_zero() {
        assert!(QueueCapacity::new(0).is_err());
        assert!(QueueCapacity::new(QueueCapacity::MAX + 1).is_err());
        assert_eq!(QueueCapacity::new(1).unwrap().get(), 1);
        assert_eq!(
            QueueCapacity::new(QueueCapacity::MAX).unwrap().get(),
            65_536
        );
        let err = serde_json::from_str::<QueueCapacity>("0").unwrap_err();
        assert!(err.to_string().contains("between 1 and 65536"));
        assert_eq!(
            QueueCapacity::new(0).unwrap_err().category(),
            ErrorCategory::InvalidConfig
        );
    }

    #[test]
    fn policies_serialize_explicitly() {
        let p: AdmissionPolicy =
            serde_json::from_str(r#"{"policy":"wait_up_to","timeout_ms":250}"#).unwrap();
        assert_eq!(p.wait(), Some(Duration::from_millis(250)));
        assert_eq!(AdmissionPolicy::Reject.wait(), None);
        assert_eq!(
            serde_json::to_string(&OverflowPolicy::DropNewest).unwrap(),
            "\"drop_newest\""
        );
        let q = QueueKind::ModuleMailbox {
            module: ModuleId::new("ce.mock").unwrap(),
        };
        assert_eq!(q.to_string(), "module_mailbox:ce.mock");
    }
}
