//! Sessions, correlation, event delivery, ordering and serialization.

mod common;

use ce_testkit::fixtures::{module, test_config};
use ce_testkit::{MockBehavior, MockModuleFactory};
use common::*;
use communication_engine::contracts::capability::LanguageTag;
use communication_engine::contracts::event::EventSource;
use communication_engine::contracts::ids::CorrelationId;
use communication_engine::contracts::session::SessionLanguages;
use communication_engine::prelude::*;

fn acking_module() -> MockModuleFactory {
    MockModuleFactory::with_behavior(
        module("ce.test.ack").subscribe(EventTopic::Sessions),
        MockBehavior {
            ack_sessions: true,
            ..MockBehavior::default()
        },
    )
}

#[tokio::test]
async fn sessions_are_correlated_across_commands_runtime_events_and_module_events() {
    let mut h = harness();
    h.engine.register_module(acking_module()).await.unwrap();
    h.engine.start().await.unwrap();
    let correlation = CorrelationId::from_uuid(uuid::Uuid::from_u128(0xC0FFEE));
    let config = SessionConfig {
        languages: Some(SessionLanguages {
            input: Some(LanguageTag::new("hu").unwrap()),
            outputs: vec![LanguageTag::new("en").unwrap()],
        }),
        mode: Some(
            communication_engine::contracts::ids::ModeId::new("example.push_to_talk").unwrap(),
        ),
        ..SessionConfig::default()
    };
    let outcome = h
        .engine
        .execute_correlated(
            Command::CreateSession {
                config: config.clone(),
            },
            correlation,
        )
        .await
        .unwrap();
    let CommandOutcome::SessionCreated { session } = outcome else {
        panic!("unexpected outcome")
    };
    assert_eq!(session.correlation_id, correlation);
    assert_eq!(session.state, SessionState::Active);
    assert_eq!(session.config, config);

    let created = h
        .events
        .wait_for_kind(WAIT, "session_created")
        .await
        .unwrap();
    assert_eq!(
        (created.session_id, created.correlation_id),
        (Some(session.id), Some(correlation))
    );
    let active = h
        .events
        .wait_for_kind(WAIT, "session_state_changed")
        .await
        .unwrap();
    assert_eq!(active.correlation_id, Some(correlation));
    assert!(active.sequence > created.sequence);

    let ack = h
        .events
        .wait_for(WAIT, |e| matches!(e.payload, EventPayload::Module(_)))
        .await
        .unwrap();
    assert_eq!(
        ack.source,
        EventSource::Module(ModuleId::new("ce.test.ack").unwrap())
    );
    assert_eq!(ack.session_id, Some(session.id));
    assert_eq!(ack.correlation_id, Some(correlation));
    assert_eq!(ack.causation_id, Some(created.id));
    assert_eq!(ack.privacy, DataClass::Operational);

    let closed = h.engine.close_session(session.id).await.unwrap();
    assert_eq!(closed.state, SessionState::Closed);
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn session_limits_unknown_sessions_and_shutdown_closure() {
    let mut config = test_config();
    config.runtime.max_sessions = 2;
    let mut h = harness_with(config);
    h.engine.start().await.unwrap();
    let a = h
        .engine
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    h.engine
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let err = h
        .engine
        .create_session(SessionConfig::default())
        .await
        .unwrap_err();
    assert_eq!(
        (err.code(), err.is_retryable()),
        ("session.limit_reached", true)
    );
    h.engine.close_session(a.id).await.unwrap();
    let err = h.engine.close_session(a.id).await.unwrap_err();
    assert_eq!(
        (err.category(), err.code()),
        (ErrorCategory::InvalidInput, "session.not_found")
    );
    h.engine
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    assert_eq!(h.engine.health().await.unwrap().open_sessions, 2);
    let report = h.engine.shutdown().await.unwrap();
    assert_eq!(report.sessions_closed, 2);
    h.events.drain_until_closed(WAIT).await;
    let closed = h
        .events
        .events()
        .iter()
        .filter(|e| {
            matches!(
                e.payload,
                EventPayload::SessionStateChanged {
                    to: SessionState::Closed,
                    ..
                }
            )
        })
        .count();
    assert_eq!(closed, 3);
}

#[tokio::test]
async fn invalid_session_configuration_is_rejected() {
    let h = harness();
    h.engine.start().await.unwrap();
    let en = LanguageTag::new("en").unwrap();
    let config = SessionConfig {
        languages: Some(SessionLanguages {
            input: None,
            outputs: vec![en.clone(), en],
        }),
        ..SessionConfig::default()
    };
    assert_eq!(
        h.engine.create_session(config).await.unwrap_err().code(),
        "session.duplicate_language"
    );
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn modules_receive_only_subscribed_topics_in_sequence_order_and_never_their_own_events() {
    let h = harness();
    let sessions_only = acking_module();
    let everything = MockModuleFactory::new(
        module("ce.test.all")
            .subscribe(EventTopic::Sessions)
            .subscribe(EventTopic::ModuleEvents)
            .subscribe(EventTopic::Health)
            .subscribe(EventTopic::Diagnostics),
    );
    let (ack_probe, all_probe) = (sessions_only.probe(), everything.probe());
    h.engine.register_module(sessions_only).await.unwrap();
    h.engine.register_module(everything).await.unwrap();
    h.engine.start().await.unwrap();
    for _ in 0..5 {
        let s = h
            .engine
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        h.engine.close_session(s.id).await.unwrap();
    }
    h.engine.health().await.unwrap(); // round trip so deliveries settle
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let ack = ack_probe.handled();
    assert!(
        ack.iter()
            .all(|(_, k)| *k == "session_created" || *k == "session_state_changed"),
        "{ack:?}"
    );
    assert_eq!(
        ack.iter().filter(|(_, k)| *k == "session_created").count(),
        5
    );
    assert!(ack.windows(2).all(|w| w[0].0 < w[1].0), "in sequence order");
    let all = all_probe.handled();
    assert_eq!(
        all.iter().filter(|(_, k)| *k == "module").count(),
        5,
        "sees the other module's acks"
    );
    assert!(all.windows(2).all(|w| w[0].0 < w[1].0));
    // The acking module never received its own module events.
    assert!(!ack.iter().any(|(_, k)| *k == "module"));
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn every_observed_event_serializes_round_trips_and_runtime_events_are_operational() {
    let mut h = harness();
    h.engine.register_module(acking_module()).await.unwrap();
    h.engine.start().await.unwrap();
    let s = h
        .engine
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    h.events
        .wait_for(WAIT, |e| matches!(e.payload, EventPayload::Module(_)))
        .await
        .unwrap();
    h.engine.close_session(s.id).await.unwrap();
    h.engine.shutdown().await.unwrap();
    h.events.drain_until_closed(WAIT).await;
    assert!(h.events.events().len() > 20);
    for event in h.events.events() {
        let json = serde_json::to_string(event.as_ref()).unwrap();
        let back: EventEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, event.as_ref());
        back.check_schema().unwrap();
        if event.source == EventSource::Runtime {
            assert_eq!(event.privacy, DataClass::Operational, "{}", event.kind());
        }
    }
}
