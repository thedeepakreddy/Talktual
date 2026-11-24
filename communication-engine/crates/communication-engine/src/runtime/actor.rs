//! The per-module actor task and the `ModuleHost` implementation it exposes.
//!
//! One actor task owns one module instance. It serializes the module's
//! lifecycle operations, health probes and event handling, and supervises the
//! module's background tasks in a `JoinSet` (so dropping the actor aborts them:
//! nothing is ever detached).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ce_core::BoxFuture;
use ce_core::error::{CeError, ErrorCategory};
use ce_core::event::{EventEnvelope, ModuleEventDraft};
use ce_core::health::HealthStatus;
use ce_core::ids::{EventId, IdGenerator, ModuleId};
use ce_core::module::{Module, ModuleContext, ModuleHost};
use ce_core::observability::{ObservabilitySink, Observation};
use ce_core::privacy::RedactionPolicy;
use ce_core::resource::QueueKind;
use ce_core::time::Clock;
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

use super::metrics::Metrics;

/// Messages from module actors and subscribers into the core.
#[derive(Debug)]
pub(crate) enum Ingress {
    ModuleEvent {
        index: usize,
        id: EventId,
        draft: ModuleEventDraft,
    },
    Report {
        index: usize,
        report: ActorReport,
    },
    SubscriberLagged {
        missed: u64,
    },
}

#[derive(Debug)]
pub(crate) enum ActorReport {
    /// A supervised task returned an error or panicked.
    TaskFailed { task: &'static str, error: CeError },
    /// `handle_event` returned an error.
    HandlerError(CeError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LifecycleOp {
    Initialize,
    Start,
    Stop,
}

pub(crate) enum Ctrl {
    Op(LifecycleOp, oneshot::Sender<Result<(), CeError>>),
    Health(oneshot::Sender<HealthStatus>),
}

pub(crate) struct SpawnRequest {
    name: &'static str,
    task: BoxFuture<'static, Result<(), CeError>>,
}

/// The core's handle to a running actor.
#[derive(Debug)]
pub(crate) struct ActorHandle {
    pub(crate) ctrl: mpsc::Sender<Ctrl>,
    pub(crate) events: mpsc::Sender<Arc<EventEnvelope>>,
    pub(crate) join: JoinHandle<()>,
    pub(crate) token: CancellationToken,
}

impl std::fmt::Debug for Ctrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ctrl::Op(op, _) => write!(f, "Op({op:?})"),
            Ctrl::Health(_) => f.write_str("Health"),
        }
    }
}

/// Everything needed to spawn an actor.
pub(crate) struct ActorSpec {
    pub(crate) index: usize,
    pub(crate) module_id: ModuleId,
    pub(crate) module: Box<dyn Module>,
    pub(crate) token: CancellationToken,
    pub(crate) ingress: mpsc::Sender<Ingress>,
    pub(crate) mailbox_capacity: usize,
    pub(crate) task_queue_capacity: usize,
    pub(crate) max_tasks: usize,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) ids: Arc<dyn IdGenerator>,
    pub(crate) sink: Arc<dyn ObservabilitySink>,
    pub(crate) policy: RedactionPolicy,
    pub(crate) metrics: Arc<Metrics>,
    pub(crate) exit_notify: Arc<Notify>,
}

/// Capacity of the control channel. The core sends at most one operation at
/// a time per module, plus occasional health probes.
const CTRL_CAPACITY: usize = 4;

pub(crate) fn spawn_actor(spec: ActorSpec) -> ActorHandle {
    let (ctrl_tx, ctrl_rx) = mpsc::channel(CTRL_CAPACITY);
    let (event_tx, event_rx) = mpsc::channel(spec.mailbox_capacity);
    let (spawn_tx, spawn_rx) = mpsc::channel(spec.task_queue_capacity);
    let active_tasks = Arc::new(AtomicUsize::new(0));
    let host = Arc::new(Host {
        module_id: spec.module_id.clone(),
        index: spec.index,
        ingress: spec.ingress.clone(),
        spawn_tx,
        token: spec.token.clone(),
        clock: spec.clock,
        ids: spec.ids,
        sink: spec.sink,
        policy: spec.policy,
        metrics: spec.metrics,
        active_tasks: active_tasks.clone(),
        max_tasks: spec.max_tasks,
    });
    let ctx = ModuleContext::new(spec.module_id, host);
    let actor = Actor {
        index: spec.index,
        module: spec.module,
        ctx,
        ctrl_rx,
        event_rx,
        spawn_rx,
        token: spec.token.clone(),
        ingress: spec.ingress,
        active_tasks,
        _exit: ExitGuard(spec.exit_notify),
    };
    let join = tokio::spawn(actor.run());
    ActorHandle {
        ctrl: ctrl_tx,
        events: event_tx,
        join,
        token: spec.token,
    }
}

/// Wakes the core whenever an actor task ends, including by panic or abort.
struct ExitGuard(Arc<Notify>);

impl Drop for ExitGuard {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}

struct Actor {
    index: usize,
    module: Box<dyn Module>,
    ctx: ModuleContext,
    ctrl_rx: mpsc::Receiver<Ctrl>,
    event_rx: mpsc::Receiver<Arc<EventEnvelope>>,
    spawn_rx: mpsc::Receiver<SpawnRequest>,
    token: CancellationToken,
    ingress: mpsc::Sender<Ingress>,
    active_tasks: Arc<AtomicUsize>,
    _exit: ExitGuard,
}

/// Send a report to the core. Backpressure is acceptable here: the core
/// drains ingress even while it waits on module operations, so this cannot
/// deadlock. (A free function so the actor future stays `Send` without
/// requiring `Module: Sync`.)
async fn report(ingress: &mpsc::Sender<Ingress>, index: usize, report: ActorReport) {
    let _ = ingress.send(Ingress::Report { index, report }).await;
}

impl Actor {
    async fn run(mut self) {
        let mut tasks: JoinSet<Result<(), CeError>> = JoinSet::new();
        let mut names: HashMap<tokio::task::Id, &'static str> = HashMap::new();
        loop {
            tokio::select! {
                biased;
                ctrl = self.ctrl_rx.recv() => match ctrl {
                    None => break,
                    Some(Ctrl::Health(reply)) => {
                        let _ = reply.send(self.module.health());
                    }
                    Some(Ctrl::Op(op, reply)) => {
                        let mut result = match op {
                            LifecycleOp::Initialize => self.module.initialize(&self.ctx).await,
                            LifecycleOp::Start => self.module.start(&self.ctx).await,
                            LifecycleOp::Stop => self.module.stop(&self.ctx).await,
                        };
                        if op == LifecycleOp::Stop {
                            // Cooperative cancellation of supervised tasks, then
                            // join them. The core bounds this with its deadline
                            // and aborts this task (and so every child) if it
                            // is exceeded. A task that fails while draining is
                            // reported through the stop result, not dropped.
                            self.token.cancel();
                            while let Some(joined) = tasks.join_next_with_id().await {
                                self.active_tasks.fetch_sub(1, Ordering::Relaxed);
                                let failed = match joined {
                                    Ok((id, r)) => { names.remove(&id); r.is_err() }
                                    Err(e) => { names.remove(&e.id()); e.is_panic() }
                                };
                                if failed && result.is_ok() {
                                    result = Err(CeError::internal(
                                        "module.task_failed_during_stop",
                                        "a supervised task failed while the module was stopping",
                                    ));
                                }
                            }
                            let _ = reply.send(result);
                            break;
                        }
                        let _ = reply.send(result);
                    }
                },
                Some(request) = self.spawn_rx.recv() => {
                    let handle = tasks.spawn(request.task);
                    names.insert(handle.id(), request.name);
                },
                Some(joined) = tasks.join_next_with_id(), if !tasks.is_empty() => {
                    self.active_tasks.fetch_sub(1, Ordering::Relaxed);
                    let failure = match joined {
                        Ok((id, Ok(()))) => { names.remove(&id); None }
                        Ok((id, Err(error))) => Some((names.remove(&id), error)),
                        Err(join_error) => {
                            let name = names.remove(&join_error.id());
                            if join_error.is_panic() {
                                Some((name, CeError::internal("module.task_panicked", "supervised task panicked")))
                            } else {
                                None
                            }
                        }
                    };
                    if let Some((name, error)) = failure {
                        report(&self.ingress, self.index, ActorReport::TaskFailed { task: name.unwrap_or("unnamed"), error }).await;
                    }
                },
                Some(event) = self.event_rx.recv() => {
                    if let Err(error) = self.module.handle_event(&event, &self.ctx).await {
                        report(&self.ingress, self.index, ActorReport::HandlerError(error)).await;
                    }
                },
            }
        }
        // Dropping `tasks` aborts anything still running.
    }
}

/// The runtime's implementation of [`ModuleHost`].
#[derive(Debug)]
struct Host {
    module_id: ModuleId,
    index: usize,
    ingress: mpsc::Sender<Ingress>,
    spawn_tx: mpsc::Sender<SpawnRequest>,
    token: CancellationToken,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGenerator>,
    sink: Arc<dyn ObservabilitySink>,
    policy: RedactionPolicy,
    metrics: Arc<Metrics>,
    active_tasks: Arc<AtomicUsize>,
    max_tasks: usize,
}

impl std::fmt::Debug for SpawnRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpawnRequest")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Host {
    fn saturated(&self, queue: QueueKind, capacity: usize) -> CeError {
        CeError::new(
            ErrorCategory::QueueSaturated,
            "queue.saturated",
            "queue is full",
        )
        .with_context("queue", queue.to_string())
        .with_context("capacity", capacity.to_string())
    }

    fn stopped(&self) -> CeError {
        CeError::new(
            ErrorCategory::Shutdown,
            "runtime.stopped",
            "runtime is no longer accepting work",
        )
        .with_context("module", self.module_id.to_string())
    }
}

impl ModuleHost for Host {
    fn emit(&self, draft: ModuleEventDraft) -> Result<EventId, CeError> {
        let id = EventId::generate(self.ids.as_ref());
        match self.ingress.try_send(Ingress::ModuleEvent {
            index: self.index,
            id,
            draft,
        }) {
            Ok(()) => Ok(id),
            Err(mpsc::error::TrySendError::Full(_)) => {
                Metrics::inc(&self.metrics.events_rejected_ingress, 1);
                self.metrics
                    .ingress_saturated
                    .store(true, Ordering::Release);
                Err(self.saturated(QueueKind::EventIngress, self.ingress.max_capacity()))
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(self.stopped()),
        }
    }

    fn spawn(
        &self,
        name: &'static str,
        task: BoxFuture<'static, Result<(), CeError>>,
    ) -> Result<(), CeError> {
        if self.token.is_cancelled() {
            return Err(CeError::new(
                ErrorCategory::Cancelled,
                "module.cancelled",
                "module is stopping; no new tasks",
            ));
        }
        let previous = self.active_tasks.fetch_add(1, Ordering::Relaxed);
        if previous >= self.max_tasks {
            self.active_tasks.fetch_sub(1, Ordering::Relaxed);
            return Err(self
                .saturated(
                    QueueKind::ModuleTasks {
                        module: self.module_id.clone(),
                    },
                    self.max_tasks,
                )
                .with_context("limit", "max_tasks_per_module"));
        }
        match self.spawn_tx.try_send(SpawnRequest { name, task }) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.active_tasks.fetch_sub(1, Ordering::Relaxed);
                Err(match e {
                    mpsc::error::TrySendError::Full(_) => self.saturated(
                        QueueKind::ModuleTasks {
                            module: self.module_id.clone(),
                        },
                        self.spawn_tx.max_capacity(),
                    ),
                    mpsc::error::TrySendError::Closed(_) => self.stopped(),
                })
            }
        }
    }

    fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    fn cancelled(&self) -> BoxFuture<'static, ()> {
        Box::pin(self.token.clone().cancelled_owned())
    }

    fn clock(&self) -> Arc<dyn Clock> {
        self.clock.clone()
    }

    fn observe(&self, observation: Observation) {
        let observation = match observation {
            Observation::Log {
                level,
                name,
                mut fields,
            } => {
                fields.insert(
                    0,
                    ce_core::observability::Field {
                        key: "module".into(),
                        value: self.module_id.to_string(),
                        class: ce_core::privacy::DataClass::Operational,
                    },
                );
                Observation::Log {
                    level,
                    name,
                    fields,
                }
            }
            other => other,
        };
        self.sink.record(&observation.redact(&self.policy));
    }
}
