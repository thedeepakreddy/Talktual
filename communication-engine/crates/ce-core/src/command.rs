//! Typed runtime commands.
//!
//! Commands are plain, serializable data so that a future transport (FFI,
//! IPC, SDK) can carry them unchanged. The runtime pairs each command with a
//! reply channel internally. Module registration is not a command because it
//! carries code (a factory), not data.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::capability::CapabilityInfo;
use crate::error::{CeError, ErrorReport};
use crate::event::StopReason;
use crate::health::RuntimeHealth;
use crate::ids::{CapabilityId, CommandId, CorrelationId, ModuleId, SessionId};
use crate::lifecycle::ModuleState;
use crate::session::{SessionConfig, SessionInfo};
use crate::version::VersionReq;

/// A runtime command.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "command")]
#[non_exhaustive]
pub enum Command {
    /// Resolve the module graph, then initialize and start every module.
    Start,
    /// Open a session.
    CreateSession {
        /// Configuration.
        config: SessionConfig,
    },
    /// Close a session.
    CloseSession {
        /// The session.
        session_id: SessionId,
    },
    /// List registered capabilities, optionally filtered.
    ListCapabilities {
        /// Only this capability.
        capability: Option<CapabilityId>,
        /// Only versions matching this requirement.
        version: Option<VersionReq>,
    },
    /// Report runtime and module health.
    QueryHealth,
    /// Stop everything and release resources.
    Shutdown,
}

impl Command {
    /// Stable command name, safe to log.
    pub fn name(&self) -> &'static str {
        match self {
            Command::Start => "start",
            Command::CreateSession { .. } => "create_session",
            Command::CloseSession { .. } => "close_session",
            Command::ListCapabilities { .. } => "list_capabilities",
            Command::QueryHealth => "query_health",
            Command::Shutdown => "shutdown",
        }
    }

    /// Validate the command's arguments (state checks happen in the runtime).
    pub fn validate(&self) -> Result<(), CeError> {
        match self {
            Command::CreateSession { config } => config.validate(),
            Command::ListCapabilities {
                capability: None,
                version: Some(_),
            } => Err(CeError::invalid_input(
                "command.version_without_capability",
                "a version filter requires a capability filter",
            )),
            _ => Ok(()),
        }
    }
}

/// A command plus its identity and correlation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandEnvelope {
    /// Command id.
    pub id: CommandId,
    /// Correlation id carried into resulting events.
    pub correlation_id: CorrelationId,
    /// The command.
    pub command: Command,
}

/// Successful command results.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
#[non_exhaustive]
pub enum CommandOutcome {
    /// Runtime started.
    Started {
        /// Running modules.
        modules: u32,
    },
    /// Session created.
    SessionCreated {
        /// The session.
        session: SessionInfo,
    },
    /// Session closed.
    SessionClosed {
        /// The session, in its final state.
        session: SessionInfo,
    },
    /// Capability listing.
    Capabilities {
        /// Matching capabilities.
        capabilities: Vec<CapabilityInfo>,
    },
    /// Health report.
    Health {
        /// Report.
        health: RuntimeHealth,
    },
    /// Shutdown completed.
    ShutdownCompleted {
        /// Report.
        report: ShutdownReport,
    },
}

/// How a module ended during shutdown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome", content = "error")]
pub enum ModuleShutdownOutcome {
    /// Never instantiated; nothing to release.
    NotStarted,
    /// `stop` completed and its tasks ended.
    Clean,
    /// `stop` returned an error; resources were still released.
    StopFailed(ErrorReport),
    /// `stop` or task cleanup exceeded its deadline; the module's task was
    /// aborted.
    Forced,
    /// The module had failed earlier; any remaining instance was terminated.
    AlreadyFailed,
    /// The module had already been stopped cleanly before shutdown began
    /// (e.g. during cleanup of a failed start).
    AlreadyStopped,
}

/// Per-module shutdown result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleShutdown {
    /// Module.
    pub module: ModuleId,
    /// State before shutdown began.
    pub state_before: ModuleState,
    /// Outcome.
    pub outcome: ModuleShutdownOutcome,
}

/// Summary of a completed shutdown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShutdownReport {
    /// Why the runtime stopped.
    pub reason: StopReason,
    /// Wall time the shutdown took.
    pub duration: Duration,
    /// Sessions closed during shutdown.
    pub sessions_closed: u32,
    /// Per-module results, in stop order.
    pub modules: Vec<ModuleShutdown>,
    /// Whether no module failed: every module stopped cleanly, was already
    /// stopped cleanly, or never started.
    pub clean: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_serializable_and_validated() {
        let cmd = Command::ListCapabilities {
            capability: Some(CapabilityId::new("ce.test.cap").unwrap()),
            version: Some(VersionReq::parse("^1").unwrap()),
        };
        cmd.validate().unwrap();
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"command\":\"list_capabilities\""));
        assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), cmd);
        let bad = Command::ListCapabilities {
            capability: None,
            version: Some(VersionReq::STAR),
        };
        assert_eq!(
            bad.validate().unwrap_err().code(),
            "command.version_without_capability"
        );
        assert_eq!(Command::Shutdown.name(), "shutdown");
    }
}
