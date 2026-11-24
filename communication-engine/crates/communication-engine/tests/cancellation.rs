//! Cooperative cancellation, shutdown and cleanup.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ce_testkit::fixtures::module;
use ce_testkit::{BackgroundTask, MockBehavior, MockModuleFactory};
use common::*;
use communication_engine::contracts::command::ModuleShutdownOutcome;
use communication_engine::contracts::event::StopReason;
use communication_engine::prelude::*;

#[tokio::test]
async fn supervised_tasks_observe_cancellation_and_are_joined_at_shutdown() {
    let h = harness();
    let f = MockModuleFactory::with_behavior(
        module("ce.test.m"),
        MockBehavior {
            background: Some(BackgroundTask::RunUntilCancelled),
            ..Default::default()
        },
    );
    let probe = f.probe();
    h.engine.register_module(f).await.unwrap();
    h.engine.start().await.unwrap();
    assert!(!probe.task_saw_cancellation());
    let report = h.engine.shutdown().await.unwrap();
    assert_eq!(report.modules[0].outcome, ModuleShutdownOutcome::Clean);
    assert!(
        probe.task_saw_cancellation(),
        "task ended cooperatively before stop completed"
    );
}

#[tokio::test]
async fn dropping_the_engine_performs_a_bounded_shutdown_in_the_background() {
    let mut h = harness();
    let f = MockModuleFactory::with_behavior(
        module("ce.test.m").subscribe(EventTopic::Sessions),
        MockBehavior {
            background: Some(BackgroundTask::RunUntilCancelled),
            ..Default::default()
        },
    );
    let probe = f.probe();
    h.engine.register_module(f).await.unwrap();
    h.engine.start().await.unwrap();
    h.engine
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    drop(h.engine);
    assert!(
        h.events.drain_until_closed(WAIT).await,
        "core exits and closes the stream"
    );
    let stopping = h.events.events().iter().find_map(|e| match e.payload {
        EventPayload::RuntimeStopping { reason } => Some(reason),
        _ => None,
    });
    assert_eq!(stopping, Some(StopReason::HandleDropped));
    assert_eq!(h.events.kinds().last(), Some(&"shutdown_completed"));
    assert!(h.events.events().iter().any(|e| matches!(
        e.payload,
        EventPayload::SessionStateChanged {
            to: SessionState::Closed,
            ..
        }
    )));
    assert_eq!(probe.calls().last(), Some(&"stop"));
    assert!(probe.task_saw_cancellation());
}

#[tokio::test]
async fn shutdown_before_start_releases_nothing_and_reports_not_started() {
    let mut h = harness();
    let f = MockModuleFactory::new(module("ce.test.m"));
    let probe = f.probe();
    h.engine.register_module(f).await.unwrap();
    let report = h.engine.shutdown().await.unwrap();
    assert!(report.clean);
    assert_eq!(report.modules[0].outcome, ModuleShutdownOutcome::NotStarted);
    assert_eq!(probe.created(), 0);
    h.events.drain_until_closed(WAIT).await;
    assert_eq!(
        module_transitions(&h.events, "ce.test.m"),
        [ModuleState::Stopped]
    );
}

#[tokio::test]
async fn context_reports_cancellation_and_refuses_new_tasks_while_stopping() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let sink = observed.clone();
    let h = harness();
    h.engine
        .register_module(ScriptFactory::new(module("ce.test.m")).on_stop(move |ctx| {
            let before = ctx.is_cancelled();
            let spawn = ctx.spawn("ce.test.late", async { Ok(()) });
            sink.lock().unwrap().push((before, spawn.is_ok()));
            Ok(())
        }))
        .await
        .unwrap();
    h.engine.start().await.unwrap();
    h.engine.shutdown().await.unwrap();
    // Cancellation is signalled after `stop` returns, so `stop` itself may
    // still schedule cleanup work, which is then cancelled and joined.
    assert_eq!(observed.lock().unwrap().as_slice(), [(false, true)]);
}

#[tokio::test]
async fn cancelled_future_resolves_for_every_module_on_shutdown() {
    let fired = Arc::new(Mutex::new(0u32));
    let h = harness();
    for id in ["ce.test.a", "ce.test.b"] {
        let fired = fired.clone();
        h.engine
            .register_module(ScriptFactory::new(module(id)).on_start(move |ctx| {
                let cancelled = ctx.cancelled();
                let fired = fired.clone();
                ctx.spawn("ce.test.watch", async move {
                    cancelled.await;
                    *fired.lock().unwrap() += 1;
                    Ok(())
                })
            }))
            .await
            .unwrap();
    }
    h.engine.start().await.unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(*fired.lock().unwrap(), 0);
    assert!(h.engine.shutdown().await.unwrap().clean);
    assert_eq!(*fired.lock().unwrap(), 2);
}
