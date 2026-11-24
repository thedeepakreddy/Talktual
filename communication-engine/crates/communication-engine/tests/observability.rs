//! Privacy-aware observability: redaction in logs, errors and events.

mod common;

use ce_testkit::fixtures::{module, test_config};
use ce_testkit::{MockBehavior, MockModuleFactory, RecordingSink};
use common::*;
use communication_engine::contracts::capability::LanguageTag;
use communication_engine::contracts::observability::{Level, Observation};
use communication_engine::contracts::privacy::RedactionPolicy;
use communication_engine::contracts::session::SessionLanguages;
use communication_engine::prelude::*;
use std::sync::Arc;

const TRANSCRIPT: &str = "the confidential transcript";

fn content_module() -> ScriptFactory {
    ScriptFactory::new(module("ce.test.content")).on_start(|ctx| {
        ctx.observe(
            Observation::log(Level::Info, "test.recognized")
                .field("segment", 1)
                .classified("text", TRANSCRIPT, DataClass::Content)
                .classified("token", "sk-secret", DataClass::Secret),
        );
        ctx.emit(
            ModuleEventDraft::new(
                EventName::new("ce.test.recognized").unwrap(),
                DataClass::Content,
            )
            .attribute("text", TRANSCRIPT),
        )?;
        Err(CeError::new(
            ErrorCategory::Provider,
            "test.provider_failed",
            "provider failed",
        )
        .with_classified_context("utterance", TRANSCRIPT, DataClass::Content)
        .with_recoverability(Recoverability::Fatal))
    })
}

#[tokio::test]
async fn sensitive_values_never_reach_logs_or_error_reports_by_default() {
    let mut h = harness();
    h.engine.register_module(content_module()).await.unwrap();
    h.engine
        .register_module(MockModuleFactory::with_behavior(
            module("ce.test.ack"),
            MockBehavior::default(),
        ))
        .await
        .unwrap();
    let err = h.engine.start().await.unwrap_err();
    assert_eq!(err.code(), "test.provider_failed");
    assert!(!err.to_string().contains(TRANSCRIPT) && !format!("{err:?}").contains(TRANSCRIPT));
    let config = SessionConfig {
        languages: Some(SessionLanguages {
            input: Some(LanguageTag::new("hu").unwrap()),
            outputs: vec![],
        }),
        ..Default::default()
    };
    let _ = h.engine.create_session(config).await;
    h.engine.shutdown().await.unwrap();
    h.events.drain_until_closed(WAIT).await;

    let logs = h.sink.rendered();
    assert!(!logs.is_empty());
    assert!(!logs.contains(TRANSCRIPT), "transcript leaked into logs");
    assert!(!logs.contains("sk-secret"), "secret leaked into logs");
    assert!(logs.contains("[redacted:content]") && logs.contains("[redacted:secret]"));
    assert!(
        logs.contains("ce.test.content"),
        "operational fields remain visible"
    );

    // Error reports in events are redacted; the module's own event keeps its
    // declared classification so consumers can decide what to do with it.
    for e in h.events.events() {
        match &e.payload {
            EventPayload::RuntimeError { error, .. } => {
                let json = serde_json::to_string(error).unwrap();
                assert!(!json.contains(TRANSCRIPT), "{json}");
            }
            EventPayload::Module(m) if m.name.as_str() == "ce.test.recognized" => {
                assert_eq!(e.privacy, DataClass::Content);
            }
            _ => assert!(e.privacy <= DataClass::Operational),
        }
    }
}

#[tokio::test]
async fn a_more_permissive_policy_is_honoured_but_secrets_stay_hidden() {
    let mut config = test_config();
    config.runtime.redaction = RedactionPolicy {
        max_visible: DataClass::Secret,
    };
    let sink = Arc::new(RecordingSink::default());
    let engine = Engine::builder()
        .config(config)
        .observability(sink.clone())
        .build()
        .unwrap();
    engine.register_module(content_module()).await.unwrap();
    let _ = engine.start().await;
    engine.shutdown().await.unwrap();
    let logs = sink.rendered();
    assert!(logs.contains(TRANSCRIPT));
    assert!(!logs.contains("sk-secret"));
}
