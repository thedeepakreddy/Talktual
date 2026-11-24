//! Runtime smoke example: exercises the public facade end to end.
//!
//! create runtime → register mock modules → query capabilities → start →
//! create session → observe correlated events → health → shutdown.
//!
//! Exits non-zero if any expectation fails, so CI can run it as a gate.
//! Run with: `cargo run -p communication-engine --example runtime_smoke`

#![allow(clippy::print_stdout)]

use std::process::ExitCode;
use std::time::Duration;

use ce_testkit::fixtures::{capability, module, requires};
use ce_testkit::{EventRecorder, MockBehavior, MockModuleFactory};
use communication_engine::contracts::event::EventSource;
use communication_engine::prelude::*;

fn check(ok: bool, what: &str) -> Result<(), String> {
    if ok {
        println!("  ok   {what}");
        Ok(())
    } else {
        Err(format!("FAILED: {what}"))
    }
}

async fn run() -> Result<(), String> {
    let err = |e: CeError| e.to_string();
    let engine = Engine::builder().build().map_err(err)?;
    let mut events = EventRecorder::new(engine.subscribe().map_err(err)?);
    println!(
        "communication-engine facade {}",
        communication_engine::FACADE_API_VERSION
    );

    // A provider module and a dependent that acknowledges sessions.
    let base =
        MockModuleFactory::new(module("example.base").provides(capability("example.cap.base", 1)));
    let app = MockModuleFactory::with_behavior(
        module("example.app")
            .provides(capability("example.cap.app", 1))
            .requires(requires("example.cap.base", "^1"))
            .subscribe(EventTopic::Sessions),
        MockBehavior {
            ack_sessions: true,
            ..MockBehavior::default()
        },
    );
    let (base_probe, app_probe) = (base.probe(), app.probe());
    engine.register_module(app).await.map_err(err)?;
    engine.register_module(base).await.map_err(err)?;

    let caps = engine.capabilities().await.map_err(err)?;
    for cap in &caps {
        println!(
            "  capability {} v{} by {} ({})",
            cap.descriptor.id, cap.descriptor.version, cap.provider, cap.provider_state
        );
    }
    check(
        caps.len() == 2,
        "two capabilities discoverable before start",
    )?;
    check(
        base_probe.created() + app_probe.created() == 0,
        "discovery instantiated nothing",
    )?;

    let running = engine.start().await.map_err(err)?;
    check(running == 2, "both modules running")?;
    check(
        base_probe.calls() == ["initialize", "start"],
        "provider initialized and started",
    )?;

    let session = engine
        .create_session(SessionConfig::default())
        .await
        .map_err(err)?;
    println!(
        "  session {} correlation {}",
        session.id, session.correlation_id
    );
    let ack = events
        .wait_for(Duration::from_secs(5), |e| matches!(&e.payload, EventPayload::Module(m) if m.name.as_str() == "ce.mock.session_ack"))
        .await
        .ok_or("no session acknowledgement")?;
    check(
        ack.session_id == Some(session.id),
        "module event belongs to the session",
    )?;
    check(
        ack.correlation_id == Some(session.correlation_id),
        "module event carries the session correlation id",
    )?;
    check(
        matches!(&ack.source, EventSource::Module(m) if m.as_str() == "example.app"),
        "module event source is the module",
    )?;

    let health = engine.health().await.map_err(err)?;
    println!(
        "  health {:?} ({} modules, {} open sessions)",
        health.overall.as_str(),
        health.modules.len(),
        health.open_sessions
    );
    check(health.overall == HealthStatus::Healthy, "runtime healthy")?;

    engine.close_session(session.id).await.map_err(err)?;
    let report = engine.shutdown().await.map_err(err)?;
    check(report.clean, "clean shutdown")?;
    check(
        events.drain_until_closed(Duration::from_secs(5)).await,
        "event stream closed after shutdown",
    )?;
    check(
        events.kinds().last() == Some(&"shutdown_completed"),
        "shutdown_completed is the last event",
    )?;
    let seqs: Vec<u64> = events.events().iter().map(|e| e.sequence).collect();
    check(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "observed sequence numbers strictly increase",
    )?;
    check(
        base_probe.calls().last() == Some(&"stop"),
        "modules stopped",
    )?;
    println!(
        "  {} events observed; shutdown took {:?}",
        seqs.len(),
        report.duration
    );
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => {
            println!("runtime smoke: PASS");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("runtime smoke: {e}");
            ExitCode::FAILURE
        }
    }
}
