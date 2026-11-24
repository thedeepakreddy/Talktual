//! Bounded queues: saturation, explicit policies and visibility.

mod common;

use std::sync::Arc;
use std::time::Duration;

use ce_testkit::fixtures::{capacity, module, test_config};
use ce_testkit::{MockBehavior, MockModuleFactory, Stage};
use common::*;
use communication_engine::contracts::resource::{AdmissionPolicy, OverflowPolicy, QueueKind};
use communication_engine::prelude::*;

fn saturations(h: &Harness) -> Vec<(QueueKind, u64)> {
    h.events
        .events()
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::QueueSaturated {
                queue,
                dropped_total,
                ..
            } => Some((queue.clone(), *dropped_total)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn slow_module_mailbox_drops_newest_and_reports_coalesced_saturation() {
    let mut config = test_config();
    config.runtime.module_mailbox_capacity = capacity(2);
    let mut h = harness_with(config);
    let slow = MockModuleFactory::with_behavior(
        module("ce.test.slow").subscribe(EventTopic::Sessions),
        MockBehavior {
            event_delay: Some(Duration::from_millis(20)),
            ..Default::default()
        },
    );
    let probe = slow.probe();
    h.engine.register_module(slow).await.unwrap();
    h.engine.start().await.unwrap();
    for _ in 0..30 {
        h.engine
            .create_session(SessionConfig::default())
            .await
            .unwrap();
    }
    let dropped = h.engine.metrics().events_dropped_mailbox;
    assert!(dropped > 0, "mailbox must have overflowed");
    h.events
        .wait_for_kind(WAIT, "queue_saturated")
        .await
        .unwrap();
    let reports = saturations(&h);
    assert!(reports.iter().all(|(q, _)| matches!(q, QueueKind::ModuleMailbox { module } if module.as_str() == "ce.test.slow")));
    assert!(
        (reports.len() as u64) < dropped,
        "saturation events are coalesced"
    );
    // The dispatcher never blocked: every command completed and the module
    // still sees what it received in order.
    let handled = probe.handled();
    assert!(handled.windows(2).all(|w| w[0].0 < w[1].0));
    assert_eq!(
        h.engine.health().await.unwrap().modules[0].state,
        ModuleState::Running
    );
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn fail_module_overflow_policy_fails_the_slow_module() {
    let mut config = test_config();
    config.runtime.module_mailbox_capacity = capacity(1);
    config.runtime.module_mailbox_overflow = OverflowPolicy::FailModule;
    let mut h = harness_with(config);
    h.engine
        .register_module(MockModuleFactory::with_behavior(
            module("ce.test.slow").subscribe(EventTopic::Sessions),
            MockBehavior {
                event_delay: Some(Duration::from_millis(50)),
                ..Default::default()
            },
        ))
        .await
        .unwrap();
    h.engine.start().await.unwrap();
    for _ in 0..5 {
        h.engine
            .create_session(SessionConfig::default())
            .await
            .unwrap();
    }
    h.events
        .wait_for(WAIT, |e| matches!(&e.payload, EventPayload::RuntimeError { error, .. } if error.code == "module.mailbox_overflow"))
        .await
        .unwrap();
    assert_eq!(
        h.engine.health().await.unwrap().modules[0].state,
        ModuleState::Failed
    );
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn lagging_subscribers_are_told_what_they_missed() {
    let mut config = test_config();
    config.runtime.event_broadcast_capacity = capacity(4);
    let mut h = harness_with(config);
    let mut slow = h.engine.subscribe().unwrap();
    h.engine.start().await.unwrap();
    for _ in 0..10 {
        h.engine
            .create_session(SessionConfig::default())
            .await
            .unwrap();
    }
    let lag = slow.recv().await.unwrap_err();
    let SubscriptionError::Lagged { missed } = lag else {
        panic!("expected lag, got {lag:?}")
    };
    assert!(missed > 0);
    // The subscription continues with the oldest retained event.
    assert!(slow.recv().await.is_ok());
    assert!(h.engine.metrics().events_missed_by_subscribers >= missed);
    // The loss is published as an event; the harness recorder (which also
    // lags here) eventually sees it.
    let seen = h
        .events
        .wait_for(WAIT, |e| {
            matches!(
                &e.payload,
                EventPayload::QueueSaturated {
                    queue: QueueKind::Subscriber,
                    ..
                }
            )
        })
        .await;
    assert!(seen.is_some());
    h.engine.shutdown().await.unwrap();
}

async fn saturate_commands(admission: AdmissionPolicy) {
    let mut config = test_config();
    config.runtime.command_queue_capacity = capacity(1);
    config.runtime.command_admission = admission;
    let mut h = harness_with(config);
    h.engine
        .register_module(MockModuleFactory::with_behavior(
            module("ce.test.hang"),
            MockBehavior {
                hang_at: Some(Stage::Initialize),
                ..Default::default()
            },
        ))
        .await
        .unwrap();
    let engine = Arc::new(h.engine);
    // The core is busy for the whole module operation timeout.
    let starter = {
        let e = engine.clone();
        tokio::spawn(async move { e.start().await })
    };
    tokio::task::yield_now().await;
    // Concurrent submitters: one fills the single slot, the rest find it full.
    let submitters: Vec<_> = (0..5)
        .map(|_| {
            let e = engine.clone();
            tokio::spawn(async move { e.capabilities().await })
        })
        .collect();
    let mut results = Vec::new();
    for s in submitters {
        results.push(s.await.unwrap());
    }
    let rejected: Vec<_> = results.iter().filter_map(|r| r.as_ref().err()).collect();
    assert!(!rejected.is_empty(), "a full command queue must reject");
    for e in &rejected {
        assert_eq!(
            (e.category(), e.code(), e.is_retryable()),
            (ErrorCategory::QueueSaturated, "queue.saturated", true)
        );
    }
    assert!(engine.metrics().commands_rejected >= rejected.len() as u64);
    assert_eq!(
        starter.await.unwrap().unwrap_err().code(),
        "module.operation_timeout"
    );
    // The next accepted command publishes the coalesced saturation event.
    engine.health().await.unwrap();
    let event = h
        .events
        .wait_for(WAIT, |e| {
            matches!(
                &e.payload,
                EventPayload::QueueSaturated {
                    queue: QueueKind::Command,
                    ..
                }
            )
        })
        .await;
    assert!(event.is_some());
    let engine = Arc::into_inner(engine).unwrap();
    engine.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn full_command_queue_rejects_immediately_under_reject_policy() {
    saturate_commands(AdmissionPolicy::Reject).await;
}

#[tokio::test(start_paused = true)]
async fn full_command_queue_applies_bounded_backpressure_then_rejects() {
    saturate_commands(AdmissionPolicy::WaitUpTo { timeout_ms: 50 }).await;
}

#[tokio::test]
async fn module_emission_beyond_ingress_capacity_is_rejected_and_reported() {
    let mut config = test_config();
    config.runtime.event_ingress_capacity = capacity(4);
    let mut h = harness_with(config);
    let outcomes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = outcomes.clone();
    h.engine
        .register_module(
            ScriptFactory::new(module("ce.test.flood")).on_start(move |ctx| {
                for i in 0..50 {
                    let draft = ModuleEventDraft::new(
                        EventName::new("ce.test.flood").unwrap(),
                        DataClass::Operational,
                    )
                    .attribute("i", i64::from(i));
                    sink.lock().unwrap().push(ctx.emit(draft).map(|_| ()));
                }
                Ok(())
            }),
        )
        .await
        .unwrap();
    h.engine.start().await.unwrap();
    let outcomes = outcomes.lock().unwrap().clone();
    let rejected: Vec<_> = outcomes.iter().filter_map(|r| r.as_ref().err()).collect();
    assert!(!rejected.is_empty());
    assert!(
        rejected
            .iter()
            .all(|e| e.category() == ErrorCategory::QueueSaturated)
    );
    assert_eq!(
        h.engine.metrics().events_rejected_ingress,
        rejected.len() as u64
    );
    h.events
        .wait_for(WAIT, |e| {
            matches!(
                &e.payload,
                EventPayload::QueueSaturated {
                    queue: QueueKind::EventIngress,
                    ..
                }
            )
        })
        .await
        .unwrap();
    // Accepted emissions were all delivered, in order.
    let delivered: Vec<i64> = h
        .events
        .events()
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Module(m) => m.attributes["i"].as_i64(),
            _ => None,
        })
        .collect();
    assert!(delivered.windows(2).all(|w| w[0] < w[1]));
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn supervised_task_count_is_bounded() {
    let mut config = test_config();
    config.runtime.max_tasks_per_module = 2;
    let h = harness_with(config);
    let results = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = results.clone();
    h.engine
        .register_module(
            ScriptFactory::new(module("ce.test.tasks")).on_start(move |ctx| {
                for _ in 0..3 {
                    let cancelled = ctx.cancelled();
                    sink.lock()
                        .unwrap()
                        .push(ctx.spawn("ce.test.wait", async move {
                            cancelled.await;
                            Ok(())
                        }));
                }
                Ok(())
            }),
        )
        .await
        .unwrap();
    h.engine.start().await.unwrap();
    let results = results.lock().unwrap().clone();
    assert!(results[0].is_ok() && results[1].is_ok());
    assert_eq!(
        results[2].as_ref().unwrap_err().category(),
        ErrorCategory::QueueSaturated
    );
    assert!(
        h.engine.shutdown().await.unwrap().clean,
        "the two tasks end on cancellation"
    );
}
