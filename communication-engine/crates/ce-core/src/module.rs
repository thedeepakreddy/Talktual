//! The module contract.
//!
//! Registration is split in two so that discovery never instantiates anything
//! expensive (models, devices, network clients):
//!
//! * [`ModuleFactory`] — registered with the runtime. Exposes a static
//!   [`ModuleDescriptor`] and validates configuration. Must be cheap.
//! * [`Module`] — the instance, created by [`ModuleFactory::create`] only when
//!   the runtime starts.
//!
//! ## Execution model (ADR-0003)
//!
//! Each module instance is owned by one runtime task (an actor). Its methods
//! are called **sequentially** with `&mut self`, so a module needs no internal
//! locking for its own state. Lifecycle operations take priority over event
//! delivery. Every operation is bounded by the configured module operation
//! timeout; a module that exceeds it is failed and its task aborted.
//!
//! Long-running work must go through [`ModuleContext::spawn`], which runs it
//! as a supervised task: its failure or panic fails the module, and it is
//! cancelled and joined when the module stops. Modules must not spawn
//! detached tasks of their own.
//!
//! Control-plane async code is **not** a real-time audio path. See
//! `docs/architecture/realtime.md`.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::BoxFuture;
use crate::capability::{CapabilityDescriptor, CapabilityRequirement, resolution_error};
use crate::config::ModuleConfig;
use crate::error::CeError;
use crate::event::{EventEnvelope, EventTopic, ModuleEventDraft};
use crate::health::HealthStatus;
use crate::ids::{EventId, ModuleId};
use crate::observability::Observation;
use crate::time::Clock;
use crate::version::{ContractVersion, Version};

/// Static description of a module. Must be producible without instantiating
/// the module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleDescriptor {
    /// Stable identity.
    pub id: ModuleId,
    /// Human-readable name.
    pub name: String,
    /// Module implementation version.
    pub version: Version,
    /// Module contract version the module was built against.
    pub contract_version: ContractVersion,
    /// Short description.
    pub description: String,
    /// Capabilities provided.
    pub provides: Vec<CapabilityDescriptor>,
    /// Capabilities required.
    pub requires: Vec<CapabilityRequirement>,
    /// Modules that must not run alongside this one.
    pub conflicts_with: Vec<ModuleId>,
    /// Event topics delivered to [`Module::handle_event`].
    pub subscriptions: Vec<EventTopic>,
}

/// Maximum descriptor name length.
pub const MAX_MODULE_NAME_LEN: usize = 128;
/// Maximum descriptor description length.
pub const MAX_MODULE_DESCRIPTION_LEN: usize = 1024;

impl ModuleDescriptor {
    /// A descriptor with no capabilities, requirements or subscriptions,
    /// built against [`ContractVersion::CURRENT`].
    pub fn new(id: ModuleId, name: impl Into<String>, version: Version) -> Self {
        Self {
            id,
            name: name.into(),
            version,
            contract_version: ContractVersion::CURRENT,
            description: String::new(),
            provides: Vec::new(),
            requires: Vec::new(),
            conflicts_with: Vec::new(),
            subscriptions: Vec::new(),
        }
    }

    /// Set the description.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Add a provided capability.
    pub fn provides(mut self, capability: CapabilityDescriptor) -> Self {
        self.provides.push(capability);
        self
    }

    /// Add a requirement.
    pub fn requires(mut self, requirement: CapabilityRequirement) -> Self {
        self.requires.push(requirement);
        self
    }

    /// Declare a conflicting module.
    pub fn conflicts_with(mut self, module: ModuleId) -> Self {
        self.conflicts_with.push(module);
        self
    }

    /// Subscribe to a topic.
    pub fn subscribe(mut self, topic: EventTopic) -> Self {
        if !self.subscriptions.contains(&topic) {
            self.subscriptions.push(topic);
        }
        self
    }

    /// Self-consistency checks performed at registration.
    pub fn validate(&self) -> Result<(), CeError> {
        let err = |code: &'static str, msg: &'static str| {
            resolution_error(code, msg).with_context("module", self.id.to_string())
        };
        if self.name.trim().is_empty() || self.name.len() > MAX_MODULE_NAME_LEN {
            return Err(err(
                "module.invalid_name",
                "module name must be 1..=128 characters",
            ));
        }
        if self.description.len() > MAX_MODULE_DESCRIPTION_LEN {
            return Err(err(
                "module.invalid_description",
                "module description is too long",
            ));
        }
        for (i, cap) in self.provides.iter().enumerate() {
            if self.provides[..i].iter().any(|c| c.id == cap.id) {
                return Err(
                    err("module.duplicate_capability", "capability provided twice")
                        .with_context("capability", cap.id.to_string()),
                );
            }
        }
        for req in &self.requires {
            if self.provides.iter().any(|c| c.id == req.capability) {
                return Err(err(
                    "module.self_dependency",
                    "module requires a capability it provides itself",
                )
                .with_context("capability", req.capability.to_string()));
            }
        }
        if self.conflicts_with.contains(&self.id) {
            return Err(err(
                "module.self_conflict",
                "module declares a conflict with itself",
            ));
        }
        Ok(())
    }
}

/// Registered with the runtime; creates module instances on demand.
pub trait ModuleFactory: Send + Sync + 'static {
    /// The module's static descriptor. Called during registration and
    /// discovery; must be cheap and side-effect free.
    fn descriptor(&self) -> ModuleDescriptor;

    /// Validate module configuration without instantiating anything.
    fn validate_config(&self, config: &ModuleConfig) -> Result<(), CeError> {
        let _ = config;
        Ok(())
    }

    /// Create an instance. Called once, when the runtime starts. Heavy
    /// resource acquisition belongs in [`Module::initialize`], not here.
    fn create(&self, config: &ModuleConfig) -> Result<Box<dyn Module>, CeError>;
}

/// A module instance. See the module-level docs for the execution model.
pub trait Module: Send + 'static {
    /// Acquire resources. Called once, before `start`.
    fn initialize<'a>(&'a mut self, ctx: &'a ModuleContext) -> BoxFuture<'a, Result<(), CeError>>;

    /// Begin operating. Called once, after every module has initialized.
    fn start<'a>(&'a mut self, ctx: &'a ModuleContext) -> BoxFuture<'a, Result<(), CeError>> {
        let _ = ctx;
        Box::pin(async { Ok(()) })
    }

    /// Stop operating and release resources. Called at most once. After it
    /// returns, the runtime cancels and joins the module's supervised tasks.
    fn stop<'a>(&'a mut self, ctx: &'a ModuleContext) -> BoxFuture<'a, Result<(), CeError>> {
        let _ = ctx;
        Box::pin(async { Ok(()) })
    }

    /// Current health. Must be cheap and non-blocking.
    fn health(&self) -> HealthStatus {
        HealthStatus::Healthy
    }

    /// Handle an event from a subscribed topic. Events arrive in sequence
    /// order; the module never receives events it emitted itself.
    ///
    /// A returned error is surfaced as a `RuntimeError` event. A `Fatal`
    /// error fails the module; any other error marks it degraded.
    fn handle_event<'a>(
        &'a mut self,
        event: &'a EventEnvelope,
        ctx: &'a ModuleContext,
    ) -> BoxFuture<'a, Result<(), CeError>> {
        let _ = (event, ctx);
        Box::pin(async { Ok(()) })
    }
}

/// Services the runtime provides to a module. Implemented by the runtime;
/// module authors use [`ModuleContext`].
pub trait ModuleHost: Send + Sync + fmt::Debug {
    /// Submit an event without blocking. Fails with `QueueSaturated` if the
    /// ingress queue is full, or `Shutdown` if the runtime is gone.
    fn emit(&self, draft: ModuleEventDraft) -> Result<EventId, CeError>;
    /// Run `task` as a supervised task of this module.
    fn spawn(
        &self,
        name: &'static str,
        task: BoxFuture<'static, Result<(), CeError>>,
    ) -> Result<(), CeError>;
    /// Whether the module has been asked to stop.
    fn is_cancelled(&self) -> bool;
    /// Resolves when the module is asked to stop.
    fn cancelled(&self) -> BoxFuture<'static, ()>;
    /// The runtime clock.
    fn clock(&self) -> Arc<dyn Clock>;
    /// Record an observation (redacted by the runtime's policy).
    fn observe(&self, observation: Observation);
}

/// A module's handle to the runtime.
#[derive(Clone, Debug)]
pub struct ModuleContext {
    module_id: ModuleId,
    host: Arc<dyn ModuleHost>,
}

impl ModuleContext {
    /// Construct a context. Called by runtimes and test harnesses.
    pub fn new(module_id: ModuleId, host: Arc<dyn ModuleHost>) -> Self {
        Self { module_id, host }
    }

    /// The module's id.
    pub fn module_id(&self) -> &ModuleId {
        &self.module_id
    }

    /// Emit an event (non-blocking).
    pub fn emit(&self, draft: ModuleEventDraft) -> Result<EventId, CeError> {
        self.host.emit(draft)
    }

    /// Run a supervised background task. It receives no special
    /// cancellation: use [`ModuleContext::cancelled`] inside it.
    pub fn spawn<F>(&self, name: &'static str, task: F) -> Result<(), CeError>
    where
        F: std::future::Future<Output = Result<(), CeError>> + Send + 'static,
    {
        self.host.spawn(name, Box::pin(task))
    }

    /// Whether the module has been asked to stop.
    pub fn is_cancelled(&self) -> bool {
        self.host.is_cancelled()
    }

    /// Resolves when the module is asked to stop.
    pub fn cancelled(&self) -> BoxFuture<'static, ()> {
        self.host.cancelled()
    }

    /// The runtime clock.
    pub fn clock(&self) -> Arc<dyn Clock> {
        self.host.clock()
    }

    /// Record an observation.
    pub fn observe(&self, observation: Observation) {
        self.host.observe(observation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilityDescriptor;
    use crate::ids::CapabilityId;
    use crate::version::VersionReq;

    fn cap(id: &str) -> CapabilityDescriptor {
        CapabilityDescriptor::new(CapabilityId::new(id).unwrap(), Version::new(1, 0, 0))
    }

    fn desc() -> ModuleDescriptor {
        ModuleDescriptor::new(
            ModuleId::new("ce.test.m").unwrap(),
            "Test",
            Version::new(0, 1, 0),
        )
    }

    #[test]
    fn valid_descriptors_pass() {
        desc()
            .provides(cap("ce.test.a"))
            .requires(CapabilityRequirement::required(
                CapabilityId::new("ce.test.b").unwrap(),
                VersionReq::STAR,
            ))
            .subscribe(EventTopic::Sessions)
            .subscribe(EventTopic::Sessions)
            .validate()
            .unwrap();
        assert_eq!(
            desc()
                .subscribe(EventTopic::Sessions)
                .subscribe(EventTopic::Sessions)
                .subscriptions
                .len(),
            1
        );
    }

    #[test]
    fn inconsistent_descriptors_are_rejected() {
        let cases = [
            (
                ModuleDescriptor {
                    name: " ".into(),
                    ..desc()
                },
                "module.invalid_name",
            ),
            (
                desc().description("x".repeat(2000)),
                "module.invalid_description",
            ),
            (
                desc().provides(cap("ce.test.a")).provides(cap("ce.test.a")),
                "module.duplicate_capability",
            ),
            (
                desc()
                    .provides(cap("ce.test.a"))
                    .requires(CapabilityRequirement::required(
                        CapabilityId::new("ce.test.a").unwrap(),
                        VersionReq::STAR,
                    )),
                "module.self_dependency",
            ),
            (
                desc().conflicts_with(ModuleId::new("ce.test.m").unwrap()),
                "module.self_conflict",
            ),
        ];
        for (d, code) in cases {
            assert_eq!(d.validate().unwrap_err().code(), code);
        }
    }
}
