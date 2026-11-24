//! The runtime core: one control-plane task that exclusively owns the
//! registry, module table, sessions and event sequencing.
//!
//! Single ownership means no locks around runtime state and a single,
//! well-defined order for every event (see ADR-0002 and events.md).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use ce_core::command::{
    Command, CommandEnvelope, CommandOutcome, ModuleShutdown, ModuleShutdownOutcome, ShutdownReport,
};
use ce_core::config::EngineConfig;
use ce_core::error::{CeError, ErrorCategory};
use ce_core::event::{
    EVENT_SCHEMA_VERSION, EventEnvelope, EventPayload, EventSource, EventTopic, StopReason,
};
use ce_core::health::{
    HealthStatus, HealthSubject, ModuleStatus, RuntimeHealth, aggregate_runtime_health,
};
use ce_core::ids::{CorrelationId, EventId, IdGenerator, ModuleId, SessionId};
use ce_core::lifecycle::{ModuleState, RuntimeState};
use ce_core::module::{ModuleDescriptor, ModuleFactory};
use ce_core::observability::{Level, ObservabilitySink, Observation};
use ce_core::privacy::DataClass;
use ce_core::resource::{OverflowPolicy, QueueKind};
use ce_core::session::{SessionConfig, SessionInfo, SessionState};
use ce_core::time::Clock;
use ce_core::version::ContractVersion;
use ce_core::{CeResult, ErrorCategory as Cat};
use tokio::sync::{Notify, broadcast, mpsc, oneshot};
use tokio::time::{Instant, sleep_until, timeout_at};
use tokio_util::sync::CancellationToken;

mod audio;
mod intel;
mod semantic;
mod speech;

pub(crate) use audio::OpenedAudio;
use audio::{AUDIO_SIGNAL_CAPACITY, AudioState};
use ce_audio::{AudioBackend, AudioSignal, BackendNotifier};
use ce_audio_intel::VoiceActivityDetector;
use ce_audio_intel::worker::IntelSignal;
use ce_core::audio::StreamRequest;
use ce_semantic::SemanticSignal;
use ce_speech::worker::SpeechSignal;
use intel::{INTEL_SIGNAL_CAPACITY, IntelState};
pub(crate) use semantic::SemanticProviders;
use semantic::{SEMANTIC_SIGNAL_CAPACITY, SemanticState};
pub(crate) use speech::SpeechProviders;
use speech::{SPEECH_SIGNAL_CAPACITY, SpeechState};

use super::actor::{ActorHandle, ActorReport, ActorSpec, Ctrl, Ingress, LifecycleOp, spawn_actor};
use super::metrics::Metrics;
use super::registry::Registry;

/// The contract version this runtime implements.
pub(crate) const RUNTIME_CONTRACT: ContractVersion = ContractVersion::CURRENT;

/// Grace period for an aborted actor task to unwind.
const ABORT_GRACE: Duration = Duration::from_millis(100);

pub(crate) enum CoreMsg {
    Command {
        envelope: CommandEnvelope,
        reply: oneshot::Sender<CeResult<CommandOutcome>>,
    },
    Register {
        factory: Arc<dyn ModuleFactory>,
        reply: oneshot::Sender<CeResult<ModuleDescriptor>>,
    },
    /// Open an audio stream (not a serializable command: it returns the
    /// in-process data-plane endpoint).
    OpenAudio {
        session_id: SessionId,
        request: StreamRequest,
        reply: oneshot::Sender<CeResult<OpenedAudio>>,
    },
    /// Start analysis with a caller-supplied detector (not serializable).
    StartAnalysisWith {
        stream_id: ce_core::ids::AudioStreamId,
        config: Option<ce_core::intel::AnalysisConfig>,
        detector: Box<dyn VoiceActivityDetector>,
        reply: oneshot::Sender<CeResult<CommandOutcome>>,
    },
}

impl std::fmt::Debug for CoreMsg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoreMsg::Command { envelope, .. } => write!(f, "Command({})", envelope.command.name()),
            CoreMsg::Register { .. } => f.write_str("Register"),
            CoreMsg::OpenAudio { .. } => f.write_str("OpenAudio"),
            CoreMsg::StartAnalysisWith { .. } => f.write_str("StartAnalysisWith"),
        }
    }
}

/// Shared services handed to the core at construction.
pub(crate) struct CoreDeps {
    pub(crate) config: Arc<EngineConfig>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) ids: Arc<dyn IdGenerator>,
    pub(crate) sink: Arc<dyn ObservabilitySink>,
    pub(crate) metrics: Arc<Metrics>,
    pub(crate) root: CancellationToken,
    pub(crate) bus: broadcast::Sender<Arc<EventEnvelope>>,
    pub(crate) cmd_rx: mpsc::Receiver<CoreMsg>,
    pub(crate) ingress_tx: mpsc::Sender<Ingress>,
    pub(crate) ingress_rx: mpsc::Receiver<Ingress>,
    pub(crate) audio_backend: Option<Arc<dyn AudioBackend>>,
    pub(crate) speech: SpeechProviders,
    pub(crate) semantic: SemanticProviders,
}

#[derive(Debug)]
struct Slot {
    id: ModuleId,
    subscriptions: Vec<EventTopic>,
    state: ModuleState,
    health: HealthStatus,
    actor: Option<ActorHandle>,
    mailbox_saturated: bool,
    mailbox_dropped: u64,
    /// Whether the module has ever failed (survives the move to `Stopped`).
    has_failed: bool,
}

/// Why waiting on an actor ended without a reply.
enum WaitError {
    TimedOut,
    ActorGone,
}

pub(crate) struct Core {
    config: Arc<EngineConfig>,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGenerator>,
    sink: Arc<dyn ObservabilitySink>,
    metrics: Arc<Metrics>,
    root: CancellationToken,
    bus: broadcast::Sender<Arc<EventEnvelope>>,
    cmd_rx: mpsc::Receiver<CoreMsg>,
    ingress_tx: mpsc::Sender<Ingress>,
    ingress_rx: mpsc::Receiver<Ingress>,
    exit_notify: Arc<Notify>,

    state: RuntimeState,
    registry: Registry,
    slots: Vec<Slot>,
    order: Vec<usize>,
    sessions: BTreeMap<SessionId, SessionInfo>,
    sequence: u64,
    runtime_health: HealthStatus,
    current_correlation: Option<CorrelationId>,
    audio: Option<AudioState>,
    audio_rx: Option<mpsc::Receiver<AudioSignal>>,
    audio_tick: Option<tokio::time::Interval>,
    intel: Option<IntelState>,
    intel_rx: Option<mpsc::Receiver<IntelSignal>>,
    speech: Option<SpeechState>,
    speech_rx: Option<mpsc::Receiver<SpeechSignal>>,
    providers: SpeechProviders,
    semantic: Option<SemanticState>,
    semantic_rx: Option<mpsc::Receiver<SemanticSignal>>,
    semantic_providers: SemanticProviders,
}

pub(super) async fn recv_opt<T>(rx: &mut Option<mpsc::Receiver<T>>) -> Option<T> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

async fn tick_opt(tick: &mut Option<tokio::time::Interval>) {
    match tick {
        Some(t) => {
            t.tick().await;
        }
        None => std::future::pending().await,
    }
}

impl Core {
    pub(crate) fn new(deps: CoreDeps) -> Self {
        let (audio, audio_rx, audio_tick) = match deps.audio_backend {
            Some(backend) => {
                let (tx, rx) = mpsc::channel(AUDIO_SIGNAL_CAPACITY);
                backend.set_notifier(BackendNotifier::new(tx.clone()));
                let period = Duration::from_millis(deps.config.audio.housekeeping_interval_ms);
                let mut tick = tokio::time::interval_at(Instant::now() + period, period);
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                (Some(AudioState::new(backend, tx)), Some(rx), Some(tick))
            }
            None => (None, None, None),
        };
        let (intel, intel_rx) = if audio.is_some() {
            let (tx, rx) = mpsc::channel(INTEL_SIGNAL_CAPACITY);
            (Some(IntelState::new(tx)), Some(rx))
        } else {
            (None, None)
        };
        let (speech, speech_rx) = if audio.is_some() {
            let (tx, rx) = mpsc::channel(SPEECH_SIGNAL_CAPACITY);
            (Some(SpeechState::new(tx)), Some(rx))
        } else {
            (None, None)
        };
        let (semantic, semantic_rx) = if audio.is_some() {
            let (tx, rx) = mpsc::channel(SEMANTIC_SIGNAL_CAPACITY);
            (Some(SemanticState::new(tx)), Some(rx))
        } else {
            (None, None)
        };
        Self {
            intel,
            intel_rx,
            speech,
            speech_rx,
            providers: deps.speech,
            semantic,
            semantic_rx,
            semantic_providers: deps.semantic,
            audio,
            audio_rx,
            audio_tick,
            config: deps.config,
            clock: deps.clock,
            ids: deps.ids,
            sink: deps.sink,
            metrics: deps.metrics,
            root: deps.root,
            bus: deps.bus,
            cmd_rx: deps.cmd_rx,
            ingress_tx: deps.ingress_tx,
            ingress_rx: deps.ingress_rx,
            exit_notify: Arc::new(Notify::new()),
            state: RuntimeState::Configuring,
            registry: Registry::default(),
            slots: Vec::new(),
            order: Vec::new(),
            sessions: BTreeMap::new(),
            sequence: 0,
            runtime_health: HealthStatus::Unknown,
            current_correlation: None,
        }
    }

    /// The control loop. Returns after shutdown completes.
    pub(crate) async fn run(mut self) {
        loop {
            tokio::select! {
                biased;
                () = self.root.cancelled() => {
                    self.shutdown(StopReason::HandleDropped).await;
                    break;
                }
                msg = self.cmd_rx.recv() => match msg {
                    Some(msg) => {
                        if self.handle_msg(msg).await {
                            break;
                        }
                    }
                    None => {
                        self.shutdown(StopReason::HandleDropped).await;
                        break;
                    }
                },
                Some(ingress) = self.ingress_rx.recv() => self.handle_ingress(ingress),
                () = self.exit_notify.notified() => self.reap_actors().await,
                Some(signal) = recv_opt(&mut self.audio_rx) => {
                    self.handle_audio_signal(signal);
                    self.process_deferred_audio().await;
                }
                Some(signal) = recv_opt(&mut self.intel_rx) => self.handle_intel_signal(signal),
                Some(signal) = recv_opt(&mut self.speech_rx) => self.handle_speech_signal(signal),
                Some(signal) = recv_opt(&mut self.semantic_rx) => self.handle_semantic_signal(signal),
                () = tick_opt(&mut self.audio_tick) => {
                    self.speech_housekeeping();
                    self.semantic_housekeeping();
                    self.intel_housekeeping();
                    self.audio_housekeeping().await;
                }
            }
        }
        self.log(Observation::log(Level::Debug, "runtime.core_exited"));
    }

    // ---------------------------------------------------------------- utilities

    fn log(&self, observation: Observation) {
        self.sink
            .record(&observation.redact(&self.config.runtime.redaction));
    }

    fn op_timeout(&self) -> Duration {
        self.config.runtime.module_operation_timeout()
    }

    /// Publish a runtime-sourced event.
    fn emit(&mut self, payload: EventPayload, session_id: Option<SessionId>) -> EventId {
        self.emit_classified(payload, DataClass::Operational, session_id)
    }

    /// Publish a runtime-sourced event with an explicit privacy class.
    fn emit_classified(
        &mut self,
        payload: EventPayload,
        privacy: DataClass,
        session_id: Option<SessionId>,
    ) -> EventId {
        let correlation = session_id
            .and_then(|s| self.sessions.get(&s).map(|i| i.correlation_id))
            .or(self.current_correlation);
        self.dispatch(
            EventId::generate(self.ids.as_ref()),
            EventSource::Runtime,
            payload,
            privacy,
            session_id,
            correlation,
            None,
        )
    }

    /// Assign a sequence number, publish to subscribers and deliver to
    /// subscribed running modules. Never blocks.
    #[allow(clippy::too_many_arguments)]
    fn dispatch(
        &mut self,
        id: EventId,
        source: EventSource,
        payload: EventPayload,
        privacy: DataClass,
        session_id: Option<SessionId>,
        correlation_id: Option<CorrelationId>,
        causation_id: Option<EventId>,
    ) -> EventId {
        self.sequence += 1;
        let topic = payload.topic();
        let envelope = Arc::new(EventEnvelope {
            schema_version: EVENT_SCHEMA_VERSION,
            id,
            sequence: self.sequence,
            emitted_at: self.clock.wall(),
            monotonic: self.clock.monotonic(),
            source,
            session_id,
            correlation_id,
            causation_id,
            privacy,
            payload,
        });
        Metrics::inc(&self.metrics.events_dispatched, 1);
        self.log(
            Observation::log(Level::Debug, "event.dispatched")
                .field("kind", envelope.kind())
                .field("sequence", envelope.sequence)
                .field(
                    "source",
                    match &envelope.source {
                        EventSource::Runtime => "runtime".to_owned(),
                        EventSource::Module(m) => m.to_string(),
                    },
                )
                .field("privacy", envelope.privacy.as_str()),
        );
        // No receivers is not an error.
        let _ = self.bus.send(envelope.clone());

        let mut newly_saturated = Vec::new();
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.state != ModuleState::Running || !slot.subscriptions.contains(&topic) {
                continue;
            }
            if matches!(&envelope.source, EventSource::Module(m) if m == &slot.id) {
                continue;
            }
            let Some(actor) = &slot.actor else { continue };
            match actor.events.try_send(envelope.clone()) {
                Ok(()) => slot.mailbox_saturated = false,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    slot.mailbox_dropped += 1;
                    Metrics::inc(&self.metrics.events_dropped_mailbox, 1);
                    if !slot.mailbox_saturated {
                        slot.mailbox_saturated = true;
                        newly_saturated.push(i);
                    }
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {}
            }
        }
        for i in newly_saturated {
            let module = self.slots[i].id.clone();
            match self.config.runtime.module_mailbox_overflow {
                OverflowPolicy::DropNewest => {
                    let dropped_total = self.slots[i].mailbox_dropped;
                    self.emit(
                        EventPayload::QueueSaturated {
                            queue: QueueKind::ModuleMailbox { module },
                            capacity: self.config.runtime.module_mailbox_capacity.into(),
                            dropped_total,
                        },
                        None,
                    );
                }
                OverflowPolicy::FailModule => {
                    let err = CeError::new(
                        Cat::QueueSaturated,
                        "module.mailbox_overflow",
                        "module event mailbox overflowed under the fail-module policy",
                    )
                    .with_context("module", module.to_string())
                    .with_recoverability(ce_core::Recoverability::Fatal);
                    self.fail_module(i, err);
                }
            }
        }
        id
    }

    fn set_runtime_state(&mut self, to: RuntimeState) -> CeResult<()> {
        let from = self.state;
        self.state = from.transition_to(to)?;
        self.log(
            Observation::log(Level::Info, "runtime.state_changed")
                .field("from", from)
                .field("to", to),
        );
        self.emit(EventPayload::RuntimeStateChanged { from, to }, None);
        self.refresh_runtime_health();
        Ok(())
    }

    /// Apply a module transition if legal. Returns whether it was applied.
    fn set_module_state(&mut self, i: usize, to: ModuleState) -> bool {
        let from = self.slots[i].state;
        if !from.can_transition_to(to) {
            return false;
        }
        self.slots[i].state = to;
        let module = self.slots[i].id.clone();
        self.log(
            Observation::log(Level::Info, "module.state_changed")
                .field("module", &module)
                .field("from", from)
                .field("to", to),
        );
        self.emit(EventPayload::ModuleStateChanged { module, from, to }, None);
        true
    }

    /// Drive a module to `Stopped` along legal transitions.
    fn finish_stopped(&mut self, i: usize) {
        if self.slots[i].state == ModuleState::Failed {
            self.set_module_state(i, ModuleState::Stopping);
        }
        self.set_module_state(i, ModuleState::Stopped);
    }

    fn set_module_health(&mut self, i: usize, to: HealthStatus) {
        if self.slots[i].health == to {
            return;
        }
        let from = std::mem::replace(&mut self.slots[i].health, to.clone());
        let subject = HealthSubject::Module(self.slots[i].id.clone());
        self.emit(EventPayload::HealthChanged { subject, from, to }, None);
    }

    fn statuses(&self) -> Vec<ModuleStatus> {
        self.slots
            .iter()
            .map(|s| ModuleStatus {
                module: s.id.clone(),
                state: s.state,
                health: s.health.clone(),
            })
            .collect()
    }

    fn refresh_runtime_health(&mut self) {
        let to = aggregate_runtime_health(self.state, &self.statuses(), &self.audio_statuses());
        if to != self.runtime_health {
            let from = std::mem::replace(&mut self.runtime_health, to.clone());
            self.emit(
                EventPayload::HealthChanged {
                    subject: HealthSubject::Runtime,
                    from,
                    to,
                },
                None,
            );
        }
    }

    fn report_error(&mut self, module: Option<ModuleId>, error: &CeError) {
        self.log(
            Observation::log(Level::Warn, "runtime.error")
                .field(
                    "module",
                    module.as_ref().map(ToString::to_string).unwrap_or_default(),
                )
                .field("error", error),
        );
        let report = error.report(&self.config.runtime.redaction);
        self.emit(
            EventPayload::RuntimeError {
                module,
                error: report,
            },
            None,
        );
    }

    /// Mark a module failed, surface the error and cancel the module's tasks.
    fn fail_module(&mut self, i: usize, error: CeError) {
        if !self.set_module_state(i, ModuleState::Failed) {
            return;
        }
        Metrics::inc(&self.metrics.module_failures, 1);
        self.slots[i].has_failed = true;
        if let Some(actor) = &self.slots[i].actor {
            actor.token.cancel();
        }
        let module = self.slots[i].id.clone();
        self.report_error(Some(module), &error);
        self.set_module_health(i, HealthStatus::Unhealthy(error.code().to_owned()));
        self.refresh_runtime_health();
    }

    fn abort_actor(&mut self, i: usize) {
        if let Some(actor) = self.slots[i].actor.take() {
            actor.token.cancel();
            actor.join.abort();
        }
    }

    // ---------------------------------------------------------------- ingress

    fn handle_ingress(&mut self, ingress: Ingress) {
        match ingress {
            Ingress::ModuleEvent { index, id, draft } => {
                let accepting = matches!(
                    self.slots[index].state,
                    ModuleState::Initialized
                        | ModuleState::Ready
                        | ModuleState::Running
                        | ModuleState::Stopping
                );
                let module = self.slots[index].id.clone();
                if !accepting {
                    // A failed or stopped module's late emissions are dropped,
                    // and that fact is recorded.
                    self.log(
                        Observation::log(Level::Debug, "event.dropped_from_inactive_module")
                            .field("module", &module),
                    );
                } else if draft
                    .session_id
                    .is_some_and(|s| !self.sessions.contains_key(&s))
                {
                    let err = CeError::invalid_input(
                        "event.unknown_session",
                        "module emitted an event for a session that is not open",
                    )
                    .with_context("module", module.to_string());
                    self.report_error(Some(module), &err);
                } else {
                    self.dispatch(
                        id,
                        EventSource::Module(module),
                        EventPayload::Module(draft.event),
                        draft.privacy,
                        draft.session_id,
                        draft.correlation_id,
                        draft.causation_id,
                    );
                }
            }
            Ingress::Report { index, report } => match report {
                ActorReport::TaskFailed { task, error } => {
                    let error = error.with_context("task", task);
                    self.fail_module(index, error);
                }
                ActorReport::HandlerError(error) => {
                    if error.recoverability() == ce_core::Recoverability::Fatal {
                        self.fail_module(index, error);
                    } else {
                        let module = self.slots[index].id.clone();
                        self.report_error(Some(module), &error);
                        if self.slots[index].state == ModuleState::Running {
                            self.set_module_health(
                                index,
                                HealthStatus::Degraded(error.code().to_owned()),
                            );
                            self.refresh_runtime_health();
                        }
                    }
                }
            },
            Ingress::SubscriberLagged { missed } => {
                let total = self
                    .metrics
                    .events_missed_by_subscribers
                    .load(Ordering::Relaxed);
                let _ = missed;
                self.emit(
                    EventPayload::QueueSaturated {
                        queue: QueueKind::Subscriber,
                        capacity: self.config.runtime.event_broadcast_capacity.into(),
                        dropped_total: total,
                    },
                    None,
                );
            }
        }
        if self.metrics.ingress_saturated.swap(false, Ordering::AcqRel) {
            let dropped_total = self.metrics.events_rejected_ingress.load(Ordering::Relaxed);
            self.emit(
                EventPayload::QueueSaturated {
                    queue: QueueKind::EventIngress,
                    capacity: self.config.runtime.event_ingress_capacity.into(),
                    dropped_total,
                },
                None,
            );
        }
    }

    /// Wait for an actor reply while continuing to drain ingress, so actors
    /// blocked on reporting can always make progress.
    async fn await_reply<T>(
        &mut self,
        mut rx: oneshot::Receiver<T>,
        deadline: Instant,
    ) -> Result<T, WaitError> {
        loop {
            tokio::select! {
                biased;
                reply = &mut rx => return reply.map_err(|_| WaitError::ActorGone),
                () = sleep_until(deadline) => return Err(WaitError::TimedOut),
                Some(ingress) = self.ingress_rx.recv() => self.handle_ingress(ingress),
                Some(signal) = recv_opt(&mut self.audio_rx) => self.handle_audio_signal(signal),
                Some(signal) = recv_opt(&mut self.intel_rx) => self.handle_intel_signal(signal),
                Some(signal) = recv_opt(&mut self.speech_rx) => self.handle_speech_signal(signal),
                Some(signal) = recv_opt(&mut self.semantic_rx) => self.handle_semantic_signal(signal),
            }
        }
    }

    async fn module_op(&mut self, i: usize, op: LifecycleOp, budget: Duration) -> CeResult<()> {
        let module = self.slots[i].id.to_string();
        let Some(ctrl) = self.slots[i].actor.as_ref().map(|a| a.ctrl.clone()) else {
            return Err(
                CeError::internal("module.no_instance", "module has no running instance")
                    .with_context("module", module),
            );
        };
        let deadline = Instant::now() + budget;
        let (tx, rx) = oneshot::channel();
        let gone = || {
            CeError::internal(
                "module.panicked",
                "module task terminated during an operation",
            )
            .with_context("module", module.clone())
            .with_context("operation", format!("{op:?}"))
        };
        match timeout_at(deadline, ctrl.send(Ctrl::Op(op, tx))).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => return Err(gone()),
            Err(_) => return Err(self.op_timeout_error(&module, op, budget)),
        }
        match self.await_reply(rx, deadline).await {
            Ok(result) => result.map_err(|e| e.with_context("operation", format!("{op:?}"))),
            Err(WaitError::ActorGone) => Err(gone()),
            Err(WaitError::TimedOut) => Err(self.op_timeout_error(&module, op, budget)),
        }
    }

    fn op_timeout_error(&self, module: &str, op: LifecycleOp, budget: Duration) -> CeError {
        CeError::new(
            ErrorCategory::Timeout,
            "module.operation_timeout",
            "module lifecycle operation exceeded its timeout",
        )
        .with_context("module", module.to_owned())
        .with_context("operation", format!("{op:?}"))
        .with_context("timeout_ms", budget.as_millis().to_string())
        .with_recoverability(ce_core::Recoverability::Fatal)
    }

    /// Handle actors whose task ended outside a lifecycle operation
    /// (panic in an event handler, abort).
    async fn reap_actors(&mut self) {
        for i in 0..self.slots.len() {
            let finished = self.slots[i]
                .actor
                .as_ref()
                .is_some_and(|a| a.join.is_finished());
            if !finished {
                continue;
            }
            let Some(actor) = self.slots[i].actor.take() else {
                continue;
            };
            let panicked = matches!(actor.join.await, Err(e) if e.is_panic());
            let error = if panicked {
                CeError::internal("module.panicked", "module task panicked")
            } else {
                CeError::internal("module.actor_exited", "module task exited unexpectedly")
            };
            self.fail_module(
                i,
                error.with_context("module", self.slots[i].id.to_string()),
            );
        }
    }

    // ---------------------------------------------------------------- commands

    /// Returns true when the core should exit.
    async fn handle_msg(&mut self, msg: CoreMsg) -> bool {
        if self.metrics.command_saturated.swap(false, Ordering::AcqRel) {
            let dropped_total = self.metrics.commands_rejected.load(Ordering::Relaxed);
            self.emit(
                EventPayload::QueueSaturated {
                    queue: QueueKind::Command,
                    capacity: self.config.runtime.command_queue_capacity.into(),
                    dropped_total,
                },
                None,
            );
        }
        match msg {
            CoreMsg::Register { factory, reply } => {
                let result = self.register(factory);
                let _ = reply.send(result);
                false
            }
            CoreMsg::OpenAudio {
                session_id,
                request,
                reply,
            } => {
                let result = self.open_audio(session_id, request).await;
                let _ = reply.send(result);
                false
            }
            CoreMsg::StartAnalysisWith {
                stream_id,
                config,
                detector,
                reply,
            } => {
                let result = match config
                    .as_ref()
                    .map(ce_core::intel::AnalysisConfig::validate)
                {
                    Some(Err(e)) => Err(e),
                    _ => self.start_analysis(stream_id, config, Some(detector)),
                };
                let _ = reply.send(result);
                false
            }
            CoreMsg::Command { envelope, reply } => {
                self.current_correlation = Some(envelope.correlation_id);
                self.log(
                    Observation::log(Level::Debug, "command.received")
                        .field("command", envelope.command.name())
                        .field("command_id", envelope.id),
                );
                let is_shutdown = matches!(envelope.command, Command::Shutdown);
                let result = match envelope.command.validate() {
                    Err(e) => Err(e),
                    Ok(()) => match envelope.command {
                        Command::Start => self.start().await,
                        Command::CreateSession { config } => self.create_session(config),
                        Command::CloseSession { session_id } => {
                            self.close_session(session_id).await
                        }
                        Command::ListCapabilities {
                            capability,
                            version,
                        } => Ok(CommandOutcome::Capabilities {
                            capabilities: self.registry.capabilities(
                                capability.as_ref(),
                                version.as_ref(),
                                |i| self.slots[i].state,
                            ),
                        }),
                        Command::QueryHealth => Ok(CommandOutcome::Health {
                            health: self.query_health().await,
                        }),
                        Command::Shutdown => Ok(CommandOutcome::ShutdownCompleted {
                            report: self.shutdown(StopReason::Requested).await,
                        }),
                        Command::UpdateConsent {
                            session_id,
                            purpose,
                            state,
                        } => self.update_consent(session_id, purpose, state).await,
                        Command::ListAudioDevices => self.list_audio_devices().await,
                        Command::StartAudioStream { stream_id } => {
                            self.start_audio(stream_id).await
                        }
                        Command::StopAudioStream { stream_id } => self.stop_audio(stream_id).await,
                        Command::CloseAudioStream { stream_id } => {
                            self.close_audio(stream_id).await
                        }
                        Command::ListAudioStreams => self.list_audio_streams(),
                        Command::StartAnalysis { stream_id, config } => {
                            self.start_analysis(stream_id, config, None)
                        }
                        Command::StopAnalysis { stream_id } => self.stop_analysis(stream_id).await,
                        Command::QueryAnalysis { stream_id } => self.query_analysis(stream_id),
                        Command::StartRecognition { stream_id, config } => {
                            self.start_recognition(stream_id, config)
                        }
                        Command::StopRecognition { stream_id } => {
                            self.stop_recognition(stream_id).await
                        }
                        Command::ResetRecognition { stream_id } => {
                            self.reset_recognition(stream_id)
                        }
                        Command::QueryRecognition { stream_id } => {
                            self.query_recognition(stream_id)
                        }
                        Command::ListRecognizers => Ok(self.list_recognizers()),
                        Command::StartSemantics { stream_id, config } => {
                            self.start_semantics(stream_id, config)
                        }
                        Command::StopSemantics { stream_id } => {
                            self.stop_semantics(stream_id).await
                        }
                        Command::ResetSemanticContext { stream_id } => {
                            self.reset_semantic_context(stream_id)
                        }
                        Command::ConfirmObservation {
                            stream_id,
                            observation,
                        } => self.confirm_observation(stream_id, observation),
                        Command::QuerySemantics { stream_id } => self.query_semantics(stream_id),
                        Command::ListSemanticAdapters => Ok(self.list_semantic_adapters()),
                        _ => Err(CeError::invalid_input(
                            "command.unsupported",
                            "command not supported by this runtime",
                        )),
                    },
                };
                self.current_correlation = None;
                let _ = reply.send(result);
                is_shutdown
            }
        }
    }

    fn register(&mut self, factory: Arc<dyn ModuleFactory>) -> CeResult<ModuleDescriptor> {
        if self.state != RuntimeState::Configuring {
            return Err(CeError::lifecycle(
                "runtime.registration_closed",
                "modules can only be registered before the runtime starts",
            )
            .with_context("state", self.state.to_string()));
        }
        let index = self.registry.register(factory, RUNTIME_CONTRACT)?;
        let descriptor = self.registry.get(index).descriptor.clone();
        self.slots.push(Slot {
            id: descriptor.id.clone(),
            subscriptions: descriptor.subscriptions.clone(),
            state: ModuleState::Discovered,
            health: HealthStatus::Unknown,
            actor: None,
            mailbox_saturated: false,
            mailbox_dropped: 0,
            has_failed: false,
        });
        self.emit(
            EventPayload::ModuleRegistered {
                module: descriptor.id.clone(),
                version: descriptor.version.clone(),
            },
            None,
        );
        for cap in &descriptor.provides {
            self.emit(
                EventPayload::CapabilityRegistered {
                    capability: cap.id.clone(),
                    version: cap.version.clone(),
                    provider: descriptor.id.clone(),
                },
                None,
            );
        }
        Ok(descriptor)
    }

    async fn start(&mut self) -> CeResult<CommandOutcome> {
        if self.state != RuntimeState::Configuring {
            return Err(CeError::lifecycle(
                "runtime.not_configuring",
                "the runtime can only be started once, from the configuring state",
            )
            .with_context("state", self.state.to_string()));
        }
        self.set_runtime_state(RuntimeState::Starting)?;
        self.emit(EventPayload::RuntimeStarting, None);

        // Configuration for modules that are not registered is almost always
        // a typo; refuse rather than silently ignore it.
        if let Some(unknown) = self
            .config
            .modules
            .keys()
            .find(|id| !self.registry.contains(id))
        {
            let err = CeError::invalid_config(
                "config.unknown_module",
                "configuration names a module that is not registered",
            )
            .with_context("module", unknown.to_string());
            return self.fail_start(err).await;
        }

        let order = match self.registry.resolve() {
            Ok(order) => order,
            Err(failures) => {
                let first = failures[0].1.clone();
                for (i, err) in failures {
                    self.fail_module(i, err);
                }
                return self.fail_start(first).await;
            }
        };
        self.order = order.clone();

        // Validate every configuration before instantiating anything.
        for &i in &order {
            let registration = self.registry.get(i);
            let config = self.config.module(&registration.descriptor.id);
            let checked = config
                .check_no_inline_secrets()
                .and_then(|()| registration.factory.validate_config(&config));
            if let Err(err) = checked {
                let err = err.with_context("module", self.slots[i].id.to_string());
                self.fail_module(i, err.clone());
                return self.fail_start(err).await;
            }
            self.set_module_state(i, ModuleState::Validated);
        }

        // Instantiate and initialize in dependency order.
        for &i in &order {
            if let Err(err) = self.instantiate(i) {
                self.fail_module(i, err.clone());
                return self.fail_start(err).await;
            }
            match self
                .module_op(i, LifecycleOp::Initialize, self.op_timeout())
                .await
            {
                Ok(()) if self.set_module_state(i, ModuleState::Initialized) => {}
                outcome => {
                    let err = outcome.err().unwrap_or_else(|| self.failed_meanwhile(i));
                    if err.category() == ErrorCategory::Timeout {
                        self.abort_actor(i);
                    }
                    self.fail_module(i, err.clone());
                    return self.fail_start(err).await;
                }
            }
        }
        for &i in &order {
            self.set_module_state(i, ModuleState::Ready);
        }
        for &i in &order {
            match self
                .module_op(i, LifecycleOp::Start, self.op_timeout())
                .await
            {
                Ok(()) if self.set_module_state(i, ModuleState::Running) => {
                    let health = self.probe_health(i).await;
                    self.set_module_health(i, health);
                }
                outcome => {
                    let err = outcome.err().unwrap_or_else(|| self.failed_meanwhile(i));
                    if err.category() == ErrorCategory::Timeout {
                        self.abort_actor(i);
                    }
                    self.fail_module(i, err.clone());
                    return self.fail_start(err).await;
                }
            }
        }
        self.set_runtime_state(RuntimeState::Running)?;
        let modules = u32::try_from(order.len()).unwrap_or(u32::MAX);
        self.emit(EventPayload::RuntimeStarted { modules }, None);
        Ok(CommandOutcome::Started { modules })
    }

    fn failed_meanwhile(&self, i: usize) -> CeError {
        CeError::internal(
            "module.failed_during_operation",
            "module failed while a lifecycle operation was in progress",
        )
        .with_context("module", self.slots[i].id.to_string())
    }

    fn instantiate(&mut self, i: usize) -> CeResult<()> {
        let registration = self.registry.get(i);
        let config = self.config.module(&registration.descriptor.id);
        let module = registration.factory.create(&config)?;
        let rt = &self.config.runtime;
        let handle = spawn_actor(ActorSpec {
            index: i,
            module_id: self.slots[i].id.clone(),
            module,
            token: self.root.child_token(),
            ingress: self.ingress_tx.clone(),
            mailbox_capacity: rt.module_mailbox_capacity.get(),
            task_queue_capacity: rt.module_task_queue_capacity.get(),
            max_tasks: rt.max_tasks_per_module as usize,
            clock: self.clock.clone(),
            ids: self.ids.clone(),
            sink: self.sink.clone(),
            policy: rt.redaction,
            metrics: self.metrics.clone(),
            exit_notify: self.exit_notify.clone(),
        });
        self.slots[i].actor = Some(handle);
        Ok(())
    }

    /// Clean up after a failed start: stop every instantiated module in
    /// reverse order, then enter `Failed`. Only shutdown is possible after.
    async fn fail_start(&mut self, err: CeError) -> CeResult<CommandOutcome> {
        let deadline = Instant::now() + self.config.runtime.shutdown_timeout();
        let order: Vec<usize> = if self.order.is_empty() {
            (0..self.slots.len()).collect()
        } else {
            self.order.clone()
        };
        for &i in order.iter().rev() {
            if self.slots[i].state.has_instance() || self.slots[i].actor.is_some() {
                self.stop_module(i, deadline).await;
            }
        }
        self.report_error(None, &err);
        self.set_runtime_state(RuntimeState::Failed)?;
        Err(err)
    }

    async fn probe_health(&mut self, i: usize) -> HealthStatus {
        let Some(ctrl) = self.slots[i].actor.as_ref().map(|a| a.ctrl.clone()) else {
            return self.slots[i].health.clone();
        };
        let deadline = Instant::now() + self.op_timeout();
        let (tx, rx) = oneshot::channel();
        if !matches!(
            timeout_at(deadline, ctrl.send(Ctrl::Health(tx))).await,
            Ok(Ok(()))
        ) {
            return HealthStatus::Unhealthy("health probe could not be delivered".into());
        }
        match self.await_reply(rx, deadline).await {
            Ok(health) => health,
            Err(WaitError::TimedOut) => HealthStatus::Degraded("health probe timed out".into()),
            Err(WaitError::ActorGone) => HealthStatus::Unhealthy("module task terminated".into()),
        }
    }

    async fn query_health(&mut self) -> RuntimeHealth {
        for i in 0..self.slots.len() {
            if matches!(
                self.slots[i].state,
                ModuleState::Initialized | ModuleState::Ready | ModuleState::Running
            ) {
                let health = self.probe_health(i).await;
                self.set_module_health(i, health);
            }
        }
        self.refresh_runtime_health();
        RuntimeHealth {
            state: self.state,
            overall: self.runtime_health.clone(),
            modules: self.statuses(),
            open_sessions: u32::try_from(self.sessions.len()).unwrap_or(u32::MAX),
            metrics: self.metrics.snapshot(),
            audio_streams: self.audio_statuses(),
            analyses: self.analysis_statuses(),
            recognitions: self.recognition_statuses(),
            semantics: self.semantic_statuses(),
        }
    }

    // ---------------------------------------------------------------- sessions

    fn create_session(&mut self, config: SessionConfig) -> CeResult<CommandOutcome> {
        if self.state != RuntimeState::Running {
            return Err(CeError::lifecycle(
                "runtime.not_running",
                "sessions can only be created while the runtime is running",
            )
            .with_context("state", self.state.to_string()));
        }
        let max = self.config.runtime.max_sessions as usize;
        if self.sessions.len() >= max {
            return Err(CeError::new(
                ErrorCategory::QueueSaturated,
                "session.limit_reached",
                "the maximum number of open sessions has been reached",
            )
            .with_context("max_sessions", max.to_string()));
        }
        let id = SessionId::generate(self.ids.as_ref());
        let correlation_id = self
            .current_correlation
            .unwrap_or_else(|| CorrelationId::generate(self.ids.as_ref()));
        let info = SessionInfo {
            id,
            correlation_id,
            state: SessionState::Created,
            config,
            created_at: self.clock.wall(),
        };
        self.sessions.insert(id, info);
        Metrics::inc(&self.metrics.sessions_created, 1);
        self.emit(EventPayload::SessionCreated, Some(id));
        self.set_session_state(id, SessionState::Active)?;
        Ok(CommandOutcome::SessionCreated {
            session: self.sessions[&id].clone(),
        })
    }

    fn set_session_state(&mut self, id: SessionId, to: SessionState) -> CeResult<()> {
        let info = self.sessions.get_mut(&id).ok_or_else(|| {
            CeError::invalid_input("session.not_found", "no open session with this id")
                .with_context("session", id.to_string())
        })?;
        let from = info.state;
        info.state = from.transition_to(to)?;
        self.emit(EventPayload::SessionStateChanged { from, to }, Some(id));
        Ok(())
    }

    async fn close_session(&mut self, id: SessionId) -> CeResult<CommandOutcome> {
        self.set_session_state(id, SessionState::Closing)?;
        // End the session's recognitions (final transcripts), then its
        // analyses (their final events belong to the session), then release
        // its audio devices.
        let deadline = Instant::now() + self.config.audio.operation_timeout();
        self.stop_recognitions(
            Some(id),
            ce_core::speech::RecognitionEndReason::StreamClosed,
            deadline,
        )
        .await;
        // Semantics drain after recognitions so final transcripts are
        // interpreted.
        self.stop_all_semantics(
            Some(id),
            ce_core::semantic::SemanticEndReason::SourceEnded,
            deadline,
        )
        .await;
        self.stop_analyses(
            Some(id),
            ce_core::intel::AnalysisEndReason::StreamClosed,
            deadline,
        )
        .await;
        self.close_session_streams(id).await;
        self.set_session_state(id, SessionState::Closed)?;
        let session = self.sessions.remove(&id).ok_or_else(|| {
            CeError::internal("session.vanished", "session disappeared while closing")
        })?;
        Ok(CommandOutcome::SessionClosed { session })
    }

    // ---------------------------------------------------------------- shutdown

    /// Stop one module within `deadline`.
    async fn stop_module(&mut self, i: usize, deadline: Instant) -> ModuleShutdownOutcome {
        let before = self.slots[i].state;
        match before {
            ModuleState::Stopped if self.slots[i].has_failed => {
                ModuleShutdownOutcome::AlreadyFailed
            }
            ModuleState::Stopped => ModuleShutdownOutcome::AlreadyStopped,
            ModuleState::Discovered | ModuleState::Validated => {
                self.set_module_state(i, ModuleState::Stopped);
                ModuleShutdownOutcome::NotStarted
            }
            ModuleState::Failed => {
                // Give a failed instance a bounded chance to release
                // resources, then terminate it.
                if self.slots[i]
                    .actor
                    .as_ref()
                    .is_some_and(|a| !a.join.is_finished())
                {
                    let budget = self.budget(deadline);
                    let _ = self.module_op(i, LifecycleOp::Stop, budget).await;
                }
                self.join_or_abort(i, deadline).await;
                self.finish_stopped(i);
                ModuleShutdownOutcome::AlreadyFailed
            }
            ModuleState::Initialized
            | ModuleState::Ready
            | ModuleState::Running
            | ModuleState::Stopping => {
                self.set_module_state(i, ModuleState::Stopping);
                let budget = self.budget(deadline);
                let result = self.module_op(i, LifecycleOp::Stop, budget).await;
                let outcome = match result {
                    Ok(()) => {
                        if self.join_or_abort(i, deadline).await {
                            ModuleShutdownOutcome::Clean
                        } else {
                            ModuleShutdownOutcome::Forced
                        }
                    }
                    Err(err) if err.category() == ErrorCategory::Timeout => {
                        self.abort_actor(i);
                        let module = self.slots[i].id.clone();
                        self.report_error(Some(module), &err);
                        ModuleShutdownOutcome::Forced
                    }
                    Err(err) => {
                        self.join_or_abort(i, deadline).await;
                        let module = self.slots[i].id.clone();
                        self.report_error(Some(module), &err);
                        ModuleShutdownOutcome::StopFailed(
                            err.report(&self.config.runtime.redaction),
                        )
                    }
                };
                self.finish_stopped(i);
                outcome
            }
        }
    }

    fn budget(&self, deadline: Instant) -> Duration {
        self.op_timeout()
            .min(deadline.saturating_duration_since(Instant::now()))
            .max(Duration::from_millis(1))
    }

    /// Join the module's actor task. Returns false if it had to be aborted.
    async fn join_or_abort(&mut self, i: usize, deadline: Instant) -> bool {
        let Some(mut actor) = self.slots[i].actor.take() else {
            return true;
        };
        actor.token.cancel();
        let grace = deadline.max(Instant::now() + ABORT_GRACE);
        match timeout_at(grace, &mut actor.join).await {
            Ok(_) => true,
            Err(_) => {
                actor.join.abort();
                let _ = timeout_at(Instant::now() + ABORT_GRACE, &mut actor.join).await;
                false
            }
        }
    }

    async fn shutdown(&mut self, reason: StopReason) -> ShutdownReport {
        let started = Instant::now();
        let deadline = started + self.config.runtime.shutdown_timeout();
        let reason = if self.state == RuntimeState::Failed && reason == StopReason::Requested {
            StopReason::StartFailed
        } else {
            reason
        };
        // Configuring, Running and Failed may all move to Stopping.
        let _ = self.set_runtime_state(RuntimeState::Stopping);
        self.emit(EventPayload::RuntimeStopping { reason }, None);

        // Recognitions end first (final transcripts), then analyses, with an
        // explicit Shutdown reason.
        self.stop_recognitions(
            None,
            ce_core::speech::RecognitionEndReason::Shutdown,
            deadline,
        )
        .await;
        self.stop_all_semantics(
            None,
            ce_core::semantic::SemanticEndReason::Shutdown,
            deadline,
        )
        .await;
        self.stop_analyses(None, ce_core::intel::AnalysisEndReason::Shutdown, deadline)
            .await;
        let open: Vec<SessionId> = self.sessions.keys().copied().collect();
        let streams_before = self.audio_statuses().len();
        let mut sessions_closed = 0u32;
        for id in open {
            if self.close_session(id).await.is_ok() {
                sessions_closed += 1;
            }
        }
        // Streams can only exist inside sessions, so all were closed above;
        // wait (bounded) for any worker thread that missed its deadline.
        let audio_streams_closed =
            u32::try_from(streams_before.saturating_sub(self.audio_statuses().len()))
                .unwrap_or(u32::MAX);
        let stuck = self.drain_zombies(deadline).await
            + self.drain_intel(deadline).await
            + self.drain_speech(deadline).await
            + self.drain_semantic(deadline).await;
        if stuck > 0 {
            self.log(
                Observation::log(Level::Warn, "audio.workers_did_not_exit").field("count", stuck),
            );
        }

        let order: Vec<usize> = if self.order.len() == self.slots.len() {
            self.order.clone()
        } else {
            (0..self.slots.len()).collect()
        };
        let mut modules = Vec::with_capacity(order.len());
        for &i in order.iter().rev() {
            let state_before = self.slots[i].state;
            let outcome = self.stop_module(i, deadline).await;
            modules.push(ModuleShutdown {
                module: self.slots[i].id.clone(),
                state_before,
                outcome,
            });
        }
        // Cancel anything that still holds a token derived from the root.
        self.root.cancel();
        self.emit(EventPayload::RuntimeStopped, None);
        let _ = self.set_runtime_state(RuntimeState::Stopped);

        let clean = modules.iter().all(|m| {
            matches!(
                m.outcome,
                ModuleShutdownOutcome::Clean
                    | ModuleShutdownOutcome::NotStarted
                    | ModuleShutdownOutcome::AlreadyStopped
            )
        });
        let report = ShutdownReport {
            reason,
            duration: started.elapsed(),
            sessions_closed,
            audio_streams_closed,
            modules,
            clean: clean && stuck == 0,
        };
        self.log(
            Observation::log(Level::Info, "runtime.shutdown_completed")
                .field("clean", clean)
                .field("sessions_closed", sessions_closed)
                .field("duration_ms", report.duration.as_millis()),
        );
        self.emit(
            EventPayload::ShutdownCompleted {
                report: report.clone(),
            },
            None,
        );
        report
    }
}
