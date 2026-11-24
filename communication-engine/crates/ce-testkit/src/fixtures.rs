//! Small, valid building blocks for tests.

use ce_core::capability::{CapabilityDescriptor, CapabilityRequirement};
use ce_core::config::EngineConfig;
use ce_core::ids::{CapabilityId, ModuleId};
use ce_core::module::ModuleDescriptor;
use ce_core::resource::QueueCapacity;
use ce_core::version::{Version, VersionReq};

/// A configuration with short timeouts suitable for tests.
pub fn test_config() -> EngineConfig {
    let mut config = EngineConfig::default();
    config.runtime.module_operation_timeout_ms = 1_000;
    config.runtime.shutdown_timeout_ms = 3_000;
    config
}

/// Capacity helper.
pub fn capacity(n: u32) -> QueueCapacity {
    QueueCapacity::new(n).expect("valid capacity")
}

/// Module descriptor `id` at version 1.0.0.
pub fn module(id: &str) -> ModuleDescriptor {
    ModuleDescriptor::new(
        ModuleId::new(id).expect("valid module id"),
        format!("Mock {id}"),
        Version::new(1, 0, 0),
    )
    .description("test module")
}

/// Capability `id` at `major.0.0`.
pub fn capability(id: &str, major: u64) -> CapabilityDescriptor {
    CapabilityDescriptor::new(
        CapabilityId::new(id).expect("valid capability id"),
        Version::new(major, 0, 0),
    )
}

/// Mandatory requirement on `id` matching `req`.
pub fn requires(id: &str, req: &str) -> CapabilityRequirement {
    CapabilityRequirement::required(
        CapabilityId::new(id).expect("valid capability id"),
        VersionReq::parse(req).expect("valid version requirement"),
    )
}
