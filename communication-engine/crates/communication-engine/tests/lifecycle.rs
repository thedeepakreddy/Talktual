//! Runtime boot, lifecycle ordering, illegal operations and configuration.

mod common;

use ce_testkit::fixtures::{capability, module, requires, test_config};
use ce_testkit::{MockModuleFactory, SequentialIds};
use common::*;
use communication_engine::contracts::command::ModuleShutdownOutcome;
use communication_engine::contracts::config::ModuleConfig;
use communication_engine::contracts::event::StopReason;
use communication_engine::contracts::time::Clock;
use communication_engine::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn boot_start_shutdown_emits_a_complete_contiguous_ordered_event_stream() {
    let mut h = harness();
    let base =
        MockModuleFactory::new(module("ce.test.base").provides(capability("ce.cap.base", 1)));
    let app = MockModuleFactory::new(module("ce.test.app").requires(requires("ce.cap.base", "^1")));
    let (base_probe, app_probe) = (base.probe(), app.probe());
    // Register the dependent first: start order must still follow dependencies.
    h.engine.register_module(app).await.unwrap();
    h.engine.register_module(base).await.unwrap();
    assert_eq!(h.engine.start().await.unwrap(), 2);
    let report = h.engine.shutdown().await.unwrap();
    assert!(report.clean);
    assert_eq!(report.reason, StopReason::Requested);
    assert!(
        h.events.drain_until_closed(WAIT).await,
        "stream closes after shutdown"
    );

    let events = h.events.events();
    // Sequence numbers start at 1 and are contiguous: nothing was dropped.
    for (i, e) in events.iter().enumerate() {
        assert_eq!(e.sequence, i as u64 + 1);
        e.check_schema().unwrap();
    }
    assert_eq!(events.last().unwrap().kind(), "shutdown_completed");

    let kinds = h.events.kinds();
    let pos = |k: &str| {
        kinds
            .iter()
            .position(|x| *x == k)
            .unwrap_or_else(|| panic!("{k} missing"))
    };
    assert!(pos("module_registered") < pos("runtime_starting"));
    assert!(pos("runtime_starting") < pos("runtime_started"));
    assert!(pos("runtime_started") < pos("runtime_stopping"));
    assert!(pos("runtime_stopping") < pos("runtime_stopped"));
    assert_eq!(
        kinds
            .iter()
            .filter(|k| **k == "capability_registered")
            .count(),
        1
    );

    use ModuleState::*;
    let full = vec![Validated, Initialized, Ready, Running, Stopping, Stopped];
    assert_eq!(module_transitions(&h.events, "ce.test.base"), full);
    assert_eq!(module_transitions(&h.events, "ce.test.app"), full);

    // Providers start before dependents and stop after them.
    let initialized_seq = |m: &str| {
        events.iter().find_map(|e| match &e.payload {
            EventPayload::ModuleStateChanged {
                module,
                to: Initialized,
                ..
            } if module.as_str() == m => Some(e.sequence),
            _ => None,
        })
    };
    assert!(initialized_seq("ce.test.base") < initialized_seq("ce.test.app"));
    let stopped = report
        .modules
        .iter()
        .map(|m| m.module.as_str())
        .collect::<Vec<_>>();
    assert_eq!(stopped, ["ce.test.app", "ce.test.base"]);
    assert!(
        report
            .modules
            .iter()
            .all(|m| m.outcome == ModuleShutdownOutcome::Clean)
    );
    assert_eq!(base_probe.calls(), ["initialize", "start", "stop"]);
    assert_eq!(app_probe.calls(), ["initialize", "start", "stop"]);
}

#[tokio::test]
async fn injected_clock_and_ids_make_events_deterministic() {
    let mut h = harness();
    h.engine.start().await.unwrap();
    let started = h
        .events
        .wait_for_kind(WAIT, "runtime_started")
        .await
        .unwrap();
    assert_eq!(started.emitted_at, h.clock.wall());
    assert_eq!(started.monotonic.as_nanos(), 0);
    h.clock.advance(std::time::Duration::from_millis(5));
    let s = h
        .engine
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    assert_eq!(s.created_at, h.clock.wall());
    let created = h
        .events
        .wait_for_kind(WAIT, "session_created")
        .await
        .unwrap();
    assert_eq!(created.monotonic.as_nanos(), 5_000_000);
    // A wall-clock jump backwards does not affect ordering or monotonic time.
    h.clock.rewind_wall(std::time::Duration::from_secs(60));
    h.engine.close_session(s.id).await.unwrap();
    let closed = h
        .events
        .wait_for(WAIT, |e| {
            matches!(
                e.payload,
                EventPayload::SessionStateChanged {
                    to: SessionState::Closed,
                    ..
                }
            )
        })
        .await
        .unwrap();
    assert!(closed.sequence > created.sequence);
    assert!(closed.monotonic >= created.monotonic);
    assert!(closed.emitted_at < created.emitted_at);
    // Identifiers come from the injected sequential generator.
    assert!(created.id.as_uuid().as_u128() < 1_000);
}

#[tokio::test]
async fn illegal_operations_are_rejected_with_lifecycle_errors() {
    let mut h = harness();
    let err = h
        .engine
        .create_session(SessionConfig::default())
        .await
        .unwrap_err();
    assert_eq!(
        (err.category(), err.code()),
        (ErrorCategory::Lifecycle, "runtime.not_running")
    );
    h.engine.start().await.unwrap();
    let err = h.engine.start().await.unwrap_err();
    assert_eq!(err.code(), "runtime.not_configuring");
    let err = h
        .engine
        .register_module(MockModuleFactory::new(module("ce.test.late")))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "runtime.registration_closed");

    // Shutdown through the command API; the handle stays but is inert.
    h.engine.execute(Command::Shutdown).await.unwrap();
    assert!(h.events.drain_until_closed(WAIT).await);
    let err = h.engine.health().await.unwrap_err();
    assert_eq!(err.category(), ErrorCategory::Shutdown);
    assert_eq!(
        h.engine.subscribe().unwrap_err().category(),
        ErrorCategory::Shutdown
    );
}

#[test]
fn building_outside_an_async_runtime_is_a_typed_error() {
    let err = Engine::builder().build().unwrap_err();
    assert_eq!(err.code(), "runtime.no_async_runtime");
}

#[tokio::test]
async fn invalid_configuration_is_rejected_before_anything_runs() {
    let mut config = test_config();
    config.runtime.max_sessions = 0;
    assert_eq!(
        Engine::builder().config(config).build().unwrap_err().code(),
        "config.out_of_range"
    );
    let mut config = test_config();
    config.config_version = 99;
    assert_eq!(
        Engine::builder()
            .config(config)
            .build()
            .unwrap_err()
            .category(),
        ErrorCategory::InvalidConfig
    );
}

#[tokio::test]
async fn configuration_for_an_unregistered_module_fails_start_and_cleans_up() {
    let mut config = test_config();
    config.modules.insert(
        ModuleId::new("ce.test.typo").unwrap(),
        ModuleConfig::default(),
    );
    let mut h = harness_with(config);
    let real = MockModuleFactory::new(module("ce.test.real"));
    let probe = real.probe();
    h.engine.register_module(real).await.unwrap();
    let err = h.engine.start().await.unwrap_err();
    assert_eq!(err.code(), "config.unknown_module");
    assert_eq!(probe.created(), 0);
    let health = h.engine.health().await.unwrap();
    assert_eq!(health.state, RuntimeState::Failed);
    assert_eq!(health.overall.as_str(), "unhealthy");
    let report = h.engine.shutdown().await.unwrap();
    assert_eq!(report.reason, StopReason::StartFailed);
    assert_eq!(report.modules[0].outcome, ModuleShutdownOutcome::NotStarted);
    h.events.drain_until_closed(WAIT).await;
    assert!(error_codes(&h.events).contains(&"config.unknown_module".to_owned()));
}

#[tokio::test]
async fn module_config_is_validated_before_any_instance_is_created() {
    let mut config = test_config();
    config.modules.insert(
        ModuleId::new("ce.test.picky").unwrap(),
        ModuleConfig::default().with("reject", true),
    );
    let h = harness_with(config);
    let ok = MockModuleFactory::new(module("ce.test.ok"));
    let picky = MockModuleFactory::new(module("ce.test.picky"));
    let (ok_probe, picky_probe) = (ok.probe(), picky.probe());
    h.engine.register_module(ok).await.unwrap();
    h.engine.register_module(picky).await.unwrap();
    let err = h.engine.start().await.unwrap_err();
    assert_eq!(
        (err.category(), err.code()),
        (ErrorCategory::InvalidConfig, "mock.invalid_config")
    );
    assert_eq!(ok_probe.created() + picky_probe.created(), 0);
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn inline_secrets_in_module_config_are_rejected_without_leaking() {
    let mut config = test_config();
    config.modules.insert(
        ModuleId::new("ce.test.m").unwrap(),
        ModuleConfig::default().with("api_key", "sk-live-should-not-leak"),
    );
    let mut h = harness();
    // Validation at build already rejects it.
    assert_eq!(
        Engine::builder().config(config).build().unwrap_err().code(),
        "config.inline_secret"
    );
    h.engine.shutdown().await.unwrap();
    h.events.drain_until_closed(WAIT).await;
    assert!(!h.sink.rendered().contains("sk-live"));
}

#[tokio::test]
async fn an_empty_runtime_starts_healthy_and_stops_cleanly() {
    let h = harness();
    assert_eq!(h.engine.start().await.unwrap(), 0);
    let health = h.engine.health().await.unwrap();
    assert_eq!(health.state, RuntimeState::Running);
    assert_eq!(health.overall, HealthStatus::Healthy);
    assert!(h.engine.shutdown().await.unwrap().clean);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_runtime_works_on_a_multi_thread_runtime_with_concurrent_callers() {
    let engine = Arc::new(
        Engine::builder()
            .config(test_config())
            .id_generator(Arc::new(SequentialIds::default()))
            .build()
            .unwrap(),
    );
    engine
        .register_module(MockModuleFactory::new(
            module("ce.test.m").subscribe(EventTopic::Sessions),
        ))
        .await
        .unwrap();
    engine.start().await.unwrap();
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let e = engine.clone();
        tasks.push(tokio::spawn(async move {
            let s = e.create_session(SessionConfig::default()).await.unwrap();
            e.close_session(s.id).await.unwrap();
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    assert_eq!(engine.metrics().sessions_created, 16);
    let engine = Arc::into_inner(engine).unwrap();
    assert!(engine.shutdown().await.unwrap().clean);
}
