//! Lifecycle state machines for modules and the runtime.
//!
//! Transitions are defined by explicit tables ([`ModuleState::can_transition_to`],
//! [`RuntimeState::can_transition_to`]). Anything not in a table is illegal and
//! is rejected with a `Lifecycle` error. See `docs/architecture/lifecycle.md`.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::CeError;

/// Lifecycle of one registered module.
///
/// ```text
/// Discovered ─► Validated ─► Initialized ─► Ready ─► Running
///     │             │             │           │         │
///     │             └───────┐     └─────┬─────┴────┬────┘
///     │                     ▼           ▼          ▼
///     │                  Stopped ◄── Stopping ◄────┘
///     └──────────► Failed ──► Stopping (cleanup)
/// ```
///
/// * **Discovered** — descriptor accepted by the registry; no instance exists.
/// * **Validated** — configuration and dependencies resolved.
/// * **Initialized** — instance created and `initialize` succeeded.
/// * **Ready** — every module in the graph initialized; eligible to start.
/// * **Running** — `start` succeeded; receiving events.
/// * **Stopping** — `stop` in progress.
/// * **Stopped** — terminal; resources released.
/// * **Failed** — an operation failed, timed out or panicked. A failed module
///   may still move to `Stopping` so the runtime can release its resources.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleState {
    /// Registered; no instance.
    Discovered,
    /// Configuration and dependencies validated.
    Validated,
    /// Instance initialized.
    Initialized,
    /// Ready to start.
    Ready,
    /// Running.
    Running,
    /// Stopping.
    Stopping,
    /// Stopped (terminal).
    Stopped,
    /// Failed.
    Failed,
}

impl ModuleState {
    /// All states, for exhaustive tests and tooling.
    pub const ALL: [ModuleState; 8] = [
        ModuleState::Discovered,
        ModuleState::Validated,
        ModuleState::Initialized,
        ModuleState::Ready,
        ModuleState::Running,
        ModuleState::Stopping,
        ModuleState::Stopped,
        ModuleState::Failed,
    ];

    /// The transition table.
    pub const fn can_transition_to(self, next: ModuleState) -> bool {
        use ModuleState::*;
        matches!(
            (self, next),
            (Discovered, Validated)
                | (Discovered, Failed)
                | (Discovered, Stopped)
                | (Validated, Initialized)
                | (Validated, Failed)
                | (Validated, Stopped)
                | (Initialized, Ready)
                | (Initialized, Stopping)
                | (Initialized, Failed)
                | (Ready, Running)
                | (Ready, Stopping)
                | (Ready, Failed)
                | (Running, Stopping)
                | (Running, Failed)
                | (Stopping, Stopped)
                | (Stopping, Failed)
                | (Failed, Stopping)
        )
    }

    /// Validate a transition.
    pub fn transition_to(self, next: ModuleState) -> Result<ModuleState, CeError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(CeError::lifecycle(
                "lifecycle.illegal_module_transition",
                "illegal module state transition",
            )
            .with_context("from", self.to_string())
            .with_context("to", next.to_string()))
        }
    }

    /// No further transitions are possible.
    pub const fn is_terminal(self) -> bool {
        matches!(self, ModuleState::Stopped)
    }

    /// An instance exists and may hold resources that must be released.
    pub const fn has_instance(self) -> bool {
        matches!(
            self,
            ModuleState::Initialized
                | ModuleState::Ready
                | ModuleState::Running
                | ModuleState::Stopping
        )
    }

    /// Stable lowercase name.
    pub const fn as_str(self) -> &'static str {
        match self {
            ModuleState::Discovered => "discovered",
            ModuleState::Validated => "validated",
            ModuleState::Initialized => "initialized",
            ModuleState::Ready => "ready",
            ModuleState::Running => "running",
            ModuleState::Stopping => "stopping",
            ModuleState::Stopped => "stopped",
            ModuleState::Failed => "failed",
        }
    }
}

impl fmt::Display for ModuleState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Lifecycle of the runtime itself.
///
/// ```text
/// Configuring ─► Starting ─► Running ─► Stopping ─► Stopped
///      │             │          │
///      │             ▼          │
///      │          Failed ───────┴──► Stopping
///      └─────────────────────────────► Stopping
/// ```
///
/// Modules can be registered only while `Configuring`. Sessions can be
/// created only while `Running`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeState {
    /// Accepting registrations.
    Configuring,
    /// Resolving, initializing and starting modules.
    Starting,
    /// Running.
    Running,
    /// Shutting down.
    Stopping,
    /// Shut down (terminal).
    Stopped,
    /// Start failed; only shutdown is possible.
    Failed,
}

impl RuntimeState {
    /// All states.
    pub const ALL: [RuntimeState; 6] = [
        RuntimeState::Configuring,
        RuntimeState::Starting,
        RuntimeState::Running,
        RuntimeState::Stopping,
        RuntimeState::Stopped,
        RuntimeState::Failed,
    ];

    /// The transition table.
    pub const fn can_transition_to(self, next: RuntimeState) -> bool {
        use RuntimeState::*;
        matches!(
            (self, next),
            (Configuring, Starting)
                | (Configuring, Stopping)
                | (Starting, Running)
                | (Starting, Failed)
                | (Running, Stopping)
                | (Failed, Stopping)
                | (Stopping, Stopped)
        )
    }

    /// Validate a transition.
    pub fn transition_to(self, next: RuntimeState) -> Result<RuntimeState, CeError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(CeError::lifecycle(
                "lifecycle.illegal_runtime_transition",
                "illegal runtime state transition",
            )
            .with_context("from", self.to_string())
            .with_context("to", next.to_string()))
        }
    }

    /// Stable lowercase name.
    pub const fn as_str(self) -> &'static str {
        match self {
            RuntimeState::Configuring => "configuring",
            RuntimeState::Starting => "starting",
            RuntimeState::Running => "running",
            RuntimeState::Stopping => "stopping",
            RuntimeState::Stopped => "stopped",
            RuntimeState::Failed => "failed",
        }
    }
}

impl fmt::Display for RuntimeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCategory;

    /// The full legal-transition matrix, written independently of the
    /// implementation so the two must agree.
    const MODULE_LEGAL: &[(ModuleState, ModuleState)] = {
        use ModuleState::*;
        &[
            (Discovered, Validated),
            (Discovered, Failed),
            (Discovered, Stopped),
            (Validated, Initialized),
            (Validated, Failed),
            (Validated, Stopped),
            (Initialized, Ready),
            (Initialized, Stopping),
            (Initialized, Failed),
            (Ready, Running),
            (Ready, Stopping),
            (Ready, Failed),
            (Running, Stopping),
            (Running, Failed),
            (Stopping, Stopped),
            (Stopping, Failed),
            (Failed, Stopping),
        ]
    };

    #[test]
    fn module_transition_matrix_is_exactly_the_documented_one() {
        let mut legal = 0;
        for from in ModuleState::ALL {
            for to in ModuleState::ALL {
                let expected = MODULE_LEGAL.contains(&(from, to));
                assert_eq!(from.can_transition_to(to), expected, "{from} -> {to}");
                match from.transition_to(to) {
                    Ok(s) => {
                        assert!(expected);
                        assert_eq!(s, to);
                        legal += 1;
                    }
                    Err(e) => {
                        assert!(!expected);
                        assert_eq!(e.category(), ErrorCategory::Lifecycle);
                        assert_eq!(e.code(), "lifecycle.illegal_module_transition");
                    }
                }
            }
        }
        assert_eq!(legal, MODULE_LEGAL.len());
    }

    #[test]
    fn module_states_have_expected_properties() {
        assert!(ModuleState::Stopped.is_terminal());
        assert!(ModuleState::ALL.iter().all(|s| !s.can_transition_to(*s)));
        assert!(
            ModuleState::ALL
                .iter()
                .all(|s| !ModuleState::Stopped.can_transition_to(*s))
        );
        assert!(ModuleState::Running.has_instance());
        assert!(!ModuleState::Validated.has_instance());
        // The happy path is legal end to end.
        let path = [
            ModuleState::Discovered,
            ModuleState::Validated,
            ModuleState::Initialized,
            ModuleState::Ready,
            ModuleState::Running,
            ModuleState::Stopping,
            ModuleState::Stopped,
        ];
        assert!(path.windows(2).all(|w| w[0].can_transition_to(w[1])));
    }

    #[test]
    fn runtime_transition_matrix_is_exactly_the_documented_one() {
        use RuntimeState::*;
        let legal = [
            (Configuring, Starting),
            (Configuring, Stopping),
            (Starting, Running),
            (Starting, Failed),
            (Running, Stopping),
            (Failed, Stopping),
            (Stopping, Stopped),
        ];
        for from in RuntimeState::ALL {
            for to in RuntimeState::ALL {
                let expected = legal.contains(&(from, to));
                assert_eq!(from.can_transition_to(to), expected, "{from} -> {to}");
                assert_eq!(from.transition_to(to).is_ok(), expected);
            }
        }
        let err = Running.transition_to(Starting).unwrap_err();
        assert!(err.to_string().contains("from=running"));
    }
}
