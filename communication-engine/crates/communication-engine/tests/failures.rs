//! Failure propagation, supervision, timeouts, cleanup and health.

mod common;

use std::sync::Arc;
use std::time::Duration;

use ce_testkit::fixtures::{capability, module, requires};
use ce_testkit::{BackgroundTask, MockBehavior, MockModuleFactory, MockProvider, Stage};
use common::*;
use communication_engine::contracts::command::ModuleShutdownOutcome;
use communication_engine::contracts::provider::ProviderErrorKind;
use communication_engine::prelude::*;

fn behaving(b: MockBehavior) -> MockModuleFactory {
    MockModuleFactory::with_behavior(
        module("ce.test.bad").requires(requires("ce.cap.base", "*")),
        b,
    )
}

fn base() -> MockModuleFactory {
    MockModuleFactory::new(module("ce.test.base").provides(capability("ce.cap.base", 1)))
}

/// Start must fail with `code`; the already-initialized provider must be
/// stopped (cleanup) and shutdown must still succeed.
async fn assert_start_failure(b: MockBehavior, code: &str, category: ErrorCategory) {
    let mut h = harness();
    let base = base();
    let base_probe = base.probe();
    h.engine.register_module(base).await.unwrap();
    h.engine.register_module(behaving(b)).await.unwrap();
    let err = h.engine.start().await.unwrap_err();
    assert_eq!((err.category(), err.code()), (category, code));
    assert_eq!(
        base_probe.calls().last(),
        Some(&"stop"),
        "initialized provider cleaned up"
    );
    let health = h.engine.health().await.unwrap();
    assert_eq!(health.state, RuntimeState::Failed);
    let report = h.engine.shutdown().await.unwrap();
    assert!(!report.clean);
    h.events.drain_until_closed(WAIT).await;
    assert!(module_transitions(&h.events, "ce.test.bad").contains(&ModuleState::Failed));
    assert_eq!(
        module_transitions(&h.events, "ce.test.bad").last(),
        Some(&ModuleState::Stopped)
    );
    assert_eq!(
        module_transitions(&h.events, "ce.test.base").last(),
        Some(&ModuleState::Stopped)
    );
    assert!(error_codes(&h.events).iter().any(|c| c == code));
    assert_eq!(h.events.kinds().last(), Some(&"shutdown_completed"));
}

#[tokio::test]
async fn initialize_error_fails_start_and_cleans_up() {
    assert_start_failure(
        MockBehavior {
            fail_at: Some(Stage::Initialize),
            ..Default::default()
        },
        "mock.scripted_failure",
        ErrorCategory::Internal,
    )
    .await;
}

#[tokio::test]
async fn start_error_fails_start_and_cleans_up() {
    assert_start_failure(
        MockBehavior {
            fail_at: Some(Stage::Start),
            ..Default::default()
        },
        "mock.scripted_failure",
        ErrorCategory::Internal,
    )
    .await;
}

#[tokio::test]
async fn create_error_fails_start_and_cleans_up() {
    assert_start_failure(
        MockBehavior {
            fail_at: Some(Stage::Create),
            ..Default::default()
        },
        "mock.scripted_failure",
        ErrorCategory::Internal,
    )
    .await;
}

#[tokio::test]
async fn initialize_panic_is_contained_and_reported() {
    assert_start_failure(
        MockBehavior {
            panic_at: Some(Stage::Initialize),
            ..Default::default()
        },
        "module.panicked",
        ErrorCategory::Internal,
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn initialize_hang_times_out_and_is_aborted() {
    assert_start_failure(
        MockBehavior {
            hang_at: Some(Stage::Initialize),
            ..Default::default()
        },
        "module.operation_timeout",
        ErrorCategory::Timeout,
    )
    .await;
}

#[tokio::test]
async fn provider_failures_are_mapped_through_the_adapter_boundary() {
    let provider = Arc::new(MockProvider::new("ce.mock.provider", vec![]));
    provider.fail_with(ProviderErrorKind::Unavailable);
    let mut h = harness();
    h.engine
        .register_module(MockModuleFactory::with_behavior(
            module("ce.test.p"),
            MockBehavior {
                provider: Some(provider),
                ..Default::default()
            },
        ))
        .await
        .unwrap();
    let err = h.engine.start().await.unwrap_err();
    assert_eq!(
        (err.category(), err.code(), err.is_retryable()),
        (ErrorCategory::Provider, "provider.unavailable", true)
    );
    h.engine.shutdown().await.unwrap();
    h.events.drain_until_closed(WAIT).await;
    let report = h.events.events().iter().find_map(|e| match &e.payload {
        EventPayload::RuntimeError { error, .. } if error.code == "provider.unavailable" => {
            Some(error.clone())
        }
        _ => None,
    });
    assert!(
        report
            .unwrap()
            .context
            .iter()
            .any(|f| f.key == "provider" && f.value == "ce.mock.provider")
    );
}

#[tokio::test]
async fn provider_health_changes_surface_as_module_and_runtime_health() {
    let provider = Arc::new(MockProvider::new("ce.mock.provider", vec![]));
    let mut h = harness();
    h.engine
        .register_module(MockModuleFactory::with_behavior(
            module("ce.test.p"),
            MockBehavior {
                provider: Some(provider.clone()),
                ..Default::default()
            },
        ))
        .await
        .unwrap();
    h.engine.start().await.unwrap();
    assert_eq!(
        h.engine.health().await.unwrap().overall,
        HealthStatus::Healthy
    );
    provider.fail_with(ProviderErrorKind::RateLimited);
    let health = h.engine.health().await.unwrap();
    assert_eq!(health.modules[0].health.as_str(), "degraded");
    assert_eq!(
        health.modules[0].state,
        ModuleState::Running,
        "health is orthogonal to lifecycle"
    );
    assert_eq!(health.overall.as_str(), "degraded");
    provider.set_health(HealthStatus::Healthy);
    assert_eq!(
        h.engine.health().await.unwrap().overall,
        HealthStatus::Healthy
    );
    h.engine.shutdown().await.unwrap();
    h.events.drain_until_closed(WAIT).await;
    let runtime_changes: Vec<_> = h
        .events
        .events()
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::HealthChanged {
                subject: communication_engine::contracts::health::HealthSubject::Runtime,
                to,
                ..
            } => Some(to.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        runtime_changes
            .windows(2)
            .any(|w| w == ["degraded", "healthy"]),
        "{runtime_changes:?}"
    );
}

async fn running_with(b: MockBehavior) -> (Harness, ce_testkit::MockProbe) {
    let h = harness();
    let f =
        MockModuleFactory::with_behavior(module("ce.test.bad").subscribe(EventTopic::Sessions), b);
    let probe = f.probe();
    h.engine.register_module(f).await.unwrap();
    h.engine.start().await.unwrap();
    (h, probe)
}

#[tokio::test]
async fn background_task_failure_fails_the_module_and_is_never_swallowed() {
    let (mut h, _) = running_with(MockBehavior {
        background: Some(BackgroundTask::FailAfter(Duration::from_millis(5))),
        ..Default::default()
    })
    .await;
    let failed = h
        .events
        .wait_for(WAIT, |e| {
            matches!(
                e.payload,
                EventPayload::ModuleStateChanged {
                    to: ModuleState::Failed,
                    ..
                }
            )
        })
        .await;
    assert!(failed.is_some());
    let error = h.events.wait_for_kind(WAIT, "runtime_error").await.unwrap();
    let EventPayload::RuntimeError { module, error } = &error.payload else {
        unreachable!()
    };
    assert_eq!(module.as_ref().unwrap().as_str(), "ce.test.bad");
    assert_eq!(error.code, "mock.task_failed");
    assert!(
        error
            .context
            .iter()
            .any(|f| f.key == "task" && f.value == "mock.background")
    );
    let health = h.engine.health().await.unwrap();
    assert_eq!(
        health.state,
        RuntimeState::Running,
        "no automatic failover or shutdown"
    );
    assert_eq!(health.overall.as_str(), "degraded");
    assert_eq!(health.metrics.module_failures, 1);
    // Failed modules no longer receive events.
    h.engine
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let report = h.engine.shutdown().await.unwrap();
    assert_eq!(
        report.modules[0].outcome,
        ModuleShutdownOutcome::AlreadyFailed
    );
    assert!(!report.clean);
}

#[tokio::test]
async fn background_task_panic_is_reported() {
    let (mut h, _) = running_with(MockBehavior {
        background: Some(BackgroundTask::PanicAfter(Duration::from_millis(5))),
        ..Default::default()
    })
    .await;
    h.events.wait_for(WAIT, |e| matches!(&e.payload, EventPayload::RuntimeError { error, .. } if error.code == "module.task_panicked")).await.unwrap();
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn recoverable_handler_errors_degrade_and_fatal_ones_fail() {
    let (mut h, probe) = running_with(MockBehavior {
        fail_at: Some(Stage::HandleEvent),
        ..Default::default()
    })
    .await;
    h.engine
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    h.events.wait_for(WAIT, |e| matches!(&e.payload, EventPayload::RuntimeError { error, .. } if error.code == "mock.scripted_failure")).await.unwrap();
    let health = h.engine.health().await.unwrap();
    assert_eq!(health.modules[0].state, ModuleState::Running);
    assert!(!probe.handled().is_empty());
    h.engine.shutdown().await.unwrap();

    let (mut h, _) = running_with(MockBehavior {
        fail_at: Some(Stage::HandleEvent),
        fail_recoverability: Some(Recoverability::Fatal),
        ..Default::default()
    })
    .await;
    h.engine
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    h.events
        .wait_for(WAIT, |e| {
            matches!(
                e.payload,
                EventPayload::ModuleStateChanged {
                    to: ModuleState::Failed,
                    ..
                }
            )
        })
        .await
        .unwrap();
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_panicking_event_handler_is_detected_and_the_module_failed() {
    let (mut h, _) = running_with(MockBehavior {
        panic_at: Some(Stage::HandleEvent),
        ..Default::default()
    })
    .await;
    h.engine
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    h.events.wait_for(WAIT, |e| matches!(&e.payload, EventPayload::RuntimeError { error, .. } if error.code == "module.panicked")).await.unwrap();
    let health = h.engine.health().await.unwrap();
    assert_eq!(health.modules[0].state, ModuleState::Failed);
    let report = h.engine.shutdown().await.unwrap();
    assert_eq!(
        report.modules[0].outcome,
        ModuleShutdownOutcome::AlreadyFailed
    );
}

#[tokio::test]
async fn stop_errors_are_reported_and_resources_still_released() {
    let (mut h, probe) = running_with(MockBehavior {
        fail_at: Some(Stage::Stop),
        ..Default::default()
    })
    .await;
    let report = h.engine.shutdown().await.unwrap();
    assert!(
        matches!(report.modules[0].outcome, ModuleShutdownOutcome::StopFailed(ref e) if e.code == "mock.scripted_failure")
    );
    assert!(!report.clean);
    assert_eq!(probe.calls(), ["initialize", "start", "stop"]);
    h.events.drain_until_closed(WAIT).await;
    assert_eq!(
        module_transitions(&h.events, "ce.test.bad").last(),
        Some(&ModuleState::Stopped)
    );
}

#[tokio::test(start_paused = true)]
async fn hanging_stop_is_forced_within_the_shutdown_deadline() {
    let (h, _) = running_with(MockBehavior {
        hang_at: Some(Stage::Stop),
        ..Default::default()
    })
    .await;
    let started = tokio::time::Instant::now();
    let report = h.engine.shutdown().await.unwrap();
    assert_eq!(report.modules[0].outcome, ModuleShutdownOutcome::Forced);
    assert!(
        started.elapsed() <= Duration::from_secs(3),
        "bounded by shutdown timeout"
    );
}

#[tokio::test(start_paused = true)]
async fn tasks_that_ignore_cancellation_are_aborted_not_leaked() {
    let (h, probe) = running_with(MockBehavior {
        background: Some(BackgroundTask::IgnoreCancellation),
        ..Default::default()
    })
    .await;
    let report = h.engine.shutdown().await.unwrap();
    assert_eq!(report.modules[0].outcome, ModuleShutdownOutcome::Forced);
    assert_eq!(probe.calls().last(), Some(&"stop"));
}
