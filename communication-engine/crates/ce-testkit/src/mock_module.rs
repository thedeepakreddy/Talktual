use std::sync::{Arc, Mutex};
use std::time::Duration;

use ce_core::BoxFuture;
use ce_core::config::ModuleConfig;
use ce_core::error::{CeError, ErrorCategory, Recoverability};
use ce_core::event::{EventEnvelope, EventPayload, ModuleEventDraft};
use ce_core::health::HealthStatus;
use ce_core::ids::EventName;
use ce_core::module::{Module, ModuleContext, ModuleDescriptor, ModuleFactory};
use ce_core::privacy::DataClass;

use crate::mock_provider::MockProvider;

/// A point in a module's life where scripted behaviour can trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// `ModuleFactory::validate_config`.
    ValidateConfig,
    /// `ModuleFactory::create`.
    Create,
    /// `Module::initialize`.
    Initialize,
    /// `Module::start`.
    Start,
    /// `Module::stop`.
    Stop,
    /// `Module::handle_event`.
    HandleEvent,
}

/// A supervised background task the module spawns when it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundTask {
    /// Runs until cancelled, then records that it observed cancellation.
    RunUntilCancelled,
    /// Returns an error after the delay.
    FailAfter(Duration),
    /// Panics after the delay.
    PanicAfter(Duration),
    /// Ignores cancellation and never ends (must be aborted).
    IgnoreCancellation,
}

/// Scripted behaviour for a mock module.
#[derive(Clone, Debug, Default)]
pub struct MockBehavior {
    /// Return an error at this stage.
    pub fail_at: Option<Stage>,
    /// Recoverability of the scripted error (default: Fatal for lifecycle
    /// stages, Recoverable for `HandleEvent`).
    pub fail_recoverability: Option<Recoverability>,
    /// Never complete at this stage.
    pub hang_at: Option<Stage>,
    /// Panic at this stage.
    pub panic_at: Option<Stage>,
    /// Sleep this long in every `handle_event` (a slow consumer).
    pub event_delay: Option<Duration>,
    /// On `SessionCreated`, emit a correlated `ce.mock.session_ack` event.
    pub ack_sessions: bool,
    /// Spawn this supervised task on start.
    pub background: Option<BackgroundTask>,
    /// Back health with this provider; `initialize` fails if it is failing.
    pub provider: Option<Arc<MockProvider>>,
}

/// What a mock module observed. Shared between the test and the module.
#[derive(Debug, Default)]
pub struct ProbeState {
    /// Number of instances created.
    pub created: u32,
    /// Lifecycle calls, in order (`"initialize"`, `"start"`, `"stop"`).
    pub calls: Vec<&'static str>,
    /// Sequence numbers and kinds of handled events.
    pub handled: Vec<(u64, &'static str)>,
    /// Whether a background task observed cancellation.
    pub task_saw_cancellation: bool,
    /// Results of attempts to emit events.
    pub emit_errors: Vec<CeError>,
}

/// Test-side view of a mock module.
#[derive(Clone, Debug, Default)]
pub struct MockProbe(Arc<Mutex<ProbeState>>);

impl MockProbe {
    fn with<R>(&self, f: impl FnOnce(&mut ProbeState) -> R) -> R {
        f(&mut self.0.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// Instances created so far.
    pub fn created(&self) -> u32 {
        self.with(|s| s.created)
    }

    /// Lifecycle calls so far.
    pub fn calls(&self) -> Vec<&'static str> {
        self.with(|s| s.calls.clone())
    }

    /// Events handled so far.
    pub fn handled(&self) -> Vec<(u64, &'static str)> {
        self.with(|s| s.handled.clone())
    }

    /// Whether a background task saw cancellation.
    pub fn task_saw_cancellation(&self) -> bool {
        self.with(|s| s.task_saw_cancellation)
    }

    /// Errors returned by `emit`.
    pub fn emit_errors(&self) -> Vec<CeError> {
        self.with(|s| s.emit_errors.clone())
    }
}

/// Factory for a scripted mock module.
#[derive(Debug)]
pub struct MockModuleFactory {
    descriptor: ModuleDescriptor,
    behavior: MockBehavior,
    probe: MockProbe,
}

impl MockModuleFactory {
    /// A factory with default (well-behaved) behaviour.
    pub fn new(descriptor: ModuleDescriptor) -> Self {
        Self::with_behavior(descriptor, MockBehavior::default())
    }

    /// A factory with scripted behaviour.
    pub fn with_behavior(descriptor: ModuleDescriptor, behavior: MockBehavior) -> Self {
        Self {
            descriptor,
            behavior,
            probe: MockProbe::default(),
        }
    }

    /// The probe shared with every instance.
    pub fn probe(&self) -> MockProbe {
        self.probe.clone()
    }
}

fn scripted_error(stage: Stage, recoverability: Option<Recoverability>) -> CeError {
    let default = if stage == Stage::HandleEvent {
        Recoverability::Recoverable
    } else {
        Recoverability::Fatal
    };
    CeError::new(
        ErrorCategory::Internal,
        "mock.scripted_failure",
        "scripted mock failure",
    )
    .with_context("stage", format!("{stage:?}"))
    .with_recoverability(recoverability.unwrap_or(default))
}

impl ModuleFactory for MockModuleFactory {
    fn descriptor(&self) -> ModuleDescriptor {
        self.descriptor.clone()
    }

    fn validate_config(&self, config: &ModuleConfig) -> Result<(), CeError> {
        if self.behavior.fail_at == Some(Stage::ValidateConfig)
            || config.get("reject").and_then(|v| v.as_bool()) == Some(true)
        {
            return Err(CeError::invalid_config(
                "mock.invalid_config",
                "mock module rejected its configuration",
            ));
        }
        Ok(())
    }

    fn create(&self, _config: &ModuleConfig) -> Result<Box<dyn Module>, CeError> {
        if self.behavior.fail_at == Some(Stage::Create) {
            return Err(scripted_error(
                Stage::Create,
                self.behavior.fail_recoverability,
            ));
        }
        self.probe.with(|s| s.created += 1);
        Ok(Box::new(MockModule {
            behavior: self.behavior.clone(),
            probe: self.probe.clone(),
        }))
    }
}

struct MockModule {
    behavior: MockBehavior,
    probe: MockProbe,
}

impl MockModule {
    /// Apply scripted failure/hang/panic for `stage`.
    async fn script(&self, stage: Stage) -> Result<(), CeError> {
        if self.behavior.panic_at == Some(stage) {
            panic!("scripted mock panic at {stage:?}");
        }
        if self.behavior.hang_at == Some(stage) {
            std::future::pending::<()>().await;
        }
        if self.behavior.fail_at == Some(stage) {
            return Err(scripted_error(stage, self.behavior.fail_recoverability));
        }
        Ok(())
    }
}

impl Module for MockModule {
    fn initialize<'a>(&'a mut self, _ctx: &'a ModuleContext) -> BoxFuture<'a, Result<(), CeError>> {
        Box::pin(async move {
            self.probe.with(|s| s.calls.push("initialize"));
            if let Some(provider) = &self.behavior.provider {
                // Adapter boundary: vendor failures become CE errors.
                provider.current().map_err(CeError::from)?;
            }
            self.script(Stage::Initialize).await
        })
    }

    fn start<'a>(&'a mut self, ctx: &'a ModuleContext) -> BoxFuture<'a, Result<(), CeError>> {
        Box::pin(async move {
            self.probe.with(|s| s.calls.push("start"));
            self.script(Stage::Start).await?;
            if let Some(task) = self.behavior.background {
                let cancelled = ctx.cancelled();
                let probe = self.probe.clone();
                ctx.spawn("mock.background", async move {
                    match task {
                        BackgroundTask::RunUntilCancelled => {
                            cancelled.await;
                            probe.with(|s| s.task_saw_cancellation = true);
                            Ok(())
                        }
                        BackgroundTask::FailAfter(d) => {
                            tokio::time::sleep(d).await;
                            Err(CeError::new(
                                ErrorCategory::Provider,
                                "mock.task_failed",
                                "scripted background task failure",
                            ))
                        }
                        BackgroundTask::PanicAfter(d) => {
                            tokio::time::sleep(d).await;
                            panic!("scripted background task panic");
                        }
                        BackgroundTask::IgnoreCancellation => {
                            std::future::pending::<()>().await;
                            Ok(())
                        }
                    }
                })?;
            }
            Ok(())
        })
    }

    fn stop<'a>(&'a mut self, _ctx: &'a ModuleContext) -> BoxFuture<'a, Result<(), CeError>> {
        Box::pin(async move {
            self.probe.with(|s| s.calls.push("stop"));
            self.script(Stage::Stop).await
        })
    }

    fn health(&self) -> HealthStatus {
        match &self.behavior.provider {
            Some(provider) => match provider.current() {
                Ok(health) => health,
                Err(e) => e.implied_health(),
            },
            None => HealthStatus::Healthy,
        }
    }

    fn handle_event<'a>(
        &'a mut self,
        event: &'a EventEnvelope,
        ctx: &'a ModuleContext,
    ) -> BoxFuture<'a, Result<(), CeError>> {
        Box::pin(async move {
            if let Some(delay) = self.behavior.event_delay {
                tokio::time::sleep(delay).await;
            }
            self.probe
                .with(|s| s.handled.push((event.sequence, event.kind())));
            self.script(Stage::HandleEvent).await?;
            if self.behavior.ack_sessions && matches!(event.payload, EventPayload::SessionCreated) {
                let draft = ModuleEventDraft::new(
                    EventName::new("ce.mock.session_ack").expect("valid name"),
                    DataClass::Operational,
                )
                .attribute(
                    "acknowledged_sequence",
                    i64::try_from(event.sequence).unwrap_or(i64::MAX),
                )
                .caused_by(event);
                if let Err(e) = ctx.emit(draft) {
                    self.probe.with(|s| s.emit_errors.push(e));
                }
            }
            Ok(())
        })
    }
}
