//! Health and status semantics.
//!
//! Health is orthogonal to lifecycle: a `Running` module can be `Degraded`.
//! Phase 1 reports health; it performs no automatic failover or restart.

use serde::{Deserialize, Serialize};

use crate::ids::ModuleId;
use crate::lifecycle::{ModuleState, RuntimeState};

/// Health of a module, provider or the runtime.
///
/// `reason` strings must contain operational text only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "status", content = "reason")]
pub enum HealthStatus {
    /// Not yet determined.
    #[default]
    Unknown,
    /// Fully functional.
    Healthy,
    /// Functional with reduced quality or capacity.
    Degraded(String),
    /// Not functional.
    Unhealthy(String),
}

impl HealthStatus {
    /// Severity rank: Healthy < Unknown < Degraded < Unhealthy.
    pub const fn severity(&self) -> u8 {
        match self {
            HealthStatus::Healthy => 0,
            HealthStatus::Unknown => 1,
            HealthStatus::Degraded(_) => 2,
            HealthStatus::Unhealthy(_) => 3,
        }
    }

    /// Stable lowercase name.
    pub const fn as_str(&self) -> &'static str {
        match self {
            HealthStatus::Unknown => "unknown",
            HealthStatus::Healthy => "healthy",
            HealthStatus::Degraded(_) => "degraded",
            HealthStatus::Unhealthy(_) => "unhealthy",
        }
    }
}

/// Whose health changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "module")]
pub enum HealthSubject {
    /// The runtime as a whole.
    Runtime,
    /// One module.
    Module(ModuleId),
}

/// Status of one module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleStatus {
    /// Module.
    pub module: ModuleId,
    /// Lifecycle state.
    pub state: ModuleState,
    /// Last known health.
    pub health: HealthStatus,
}

/// Point-in-time counters maintained by the runtime.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetricsSnapshot {
    /// Commands admitted to the command queue.
    pub commands_accepted: u64,
    /// Commands rejected because the queue was full.
    pub commands_rejected: u64,
    /// Events dispatched (published with a sequence number).
    pub events_dispatched: u64,
    /// Module-emitted events rejected because the ingress queue was full.
    pub events_rejected_ingress: u64,
    /// Events dropped because a module mailbox was full.
    pub events_dropped_mailbox: u64,
    /// Events external subscribers missed because they fell behind.
    pub events_missed_by_subscribers: u64,
    /// Module failures observed.
    pub module_failures: u64,
    /// Sessions created since start.
    pub sessions_created: u64,
}

/// Health and status of the runtime and every module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHealth {
    /// Runtime lifecycle state.
    pub state: RuntimeState,
    /// Aggregated health.
    pub overall: HealthStatus,
    /// Per-module status, in registration order.
    pub modules: Vec<ModuleStatus>,
    /// Open sessions.
    pub open_sessions: u32,
    /// Counters.
    pub metrics: MetricsSnapshot,
}

/// Aggregate runtime health from the runtime state and module statuses.
///
/// * Runtime `Failed` → Unhealthy. Not `Running` → Unknown.
/// * Any module `Failed` or `Unhealthy` → Degraded (the runtime keeps serving
///   the remaining modules; there is no automatic failover in Phase 1).
/// * Any module `Degraded` or `Unknown` → Degraded.
/// * Otherwise Healthy.
pub fn aggregate_health(state: RuntimeState, modules: &[ModuleStatus]) -> HealthStatus {
    match state {
        RuntimeState::Failed => return HealthStatus::Unhealthy("runtime start failed".into()),
        RuntimeState::Running => {}
        _ => return HealthStatus::Unknown,
    }
    let failed = modules
        .iter()
        .filter(|m| {
            m.state == ModuleState::Failed || matches!(m.health, HealthStatus::Unhealthy(_))
        })
        .count();
    if failed > 0 {
        return HealthStatus::Degraded(format!("{failed} module(s) failed or unhealthy"));
    }
    let degraded = modules.iter().filter(|m| m.health.severity() > 0).count();
    if degraded > 0 {
        return HealthStatus::Degraded(format!("{degraded} module(s) degraded or unknown"));
    }
    HealthStatus::Healthy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: ModuleState, health: HealthStatus) -> ModuleStatus {
        ModuleStatus {
            module: ModuleId::new("ce.test.m").unwrap(),
            state,
            health,
        }
    }

    #[test]
    fn aggregation_follows_documented_rules() {
        use HealthStatus::*;
        assert_eq!(aggregate_health(RuntimeState::Running, &[]), Healthy);
        assert_eq!(
            aggregate_health(RuntimeState::Configuring, &[]).as_str(),
            "unknown"
        );
        assert_eq!(
            aggregate_health(RuntimeState::Failed, &[]).as_str(),
            "unhealthy"
        );
        let ok = status(ModuleState::Running, Healthy);
        assert_eq!(
            aggregate_health(RuntimeState::Running, std::slice::from_ref(&ok)),
            Healthy
        );
        let degraded = status(ModuleState::Running, Degraded("slow".into()));
        assert_eq!(
            aggregate_health(RuntimeState::Running, &[ok.clone(), degraded]).as_str(),
            "degraded"
        );
        let failed = status(ModuleState::Failed, Unhealthy("x".into()));
        assert!(matches!(
            aggregate_health(RuntimeState::Running, &[ok, failed]),
            Degraded(r) if r.contains("1 module")
        ));
    }

    #[test]
    fn severity_orders_statuses() {
        assert!(HealthStatus::Healthy.severity() < HealthStatus::Unknown.severity());
        assert!(
            HealthStatus::Degraded(String::new()).severity()
                < HealthStatus::Unhealthy(String::new()).severity()
        );
        let json = serde_json::to_string(&HealthStatus::Degraded("slow".into())).unwrap();
        assert_eq!(json, r#"{"status":"degraded","reason":"slow"}"#);
    }
}
