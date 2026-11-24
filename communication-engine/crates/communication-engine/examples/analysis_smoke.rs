//! Analysis smoke example (Phase 3): audio intelligence through the public
//! facade, on the test kit's fake backend with a real-time driver thread.
//!
//! consent → open + start capture → start analysis (default spectral
//! detector) → read capture while analysis runs → query status → start a
//! second analysis with a scripted detector → stop → shutdown. Exits
//! non-zero on any failed expectation (CI gate).
//! Run: `cargo run -p communication-engine --example analysis_smoke`

#![allow(clippy::print_stdout)]

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use ce_testkit::EventRecorder;
use ce_testkit::fake_audio::{FakeAudioBackend, FakeDriver};
use communication_engine::audio_intel::ScriptedVad;
use communication_engine::contracts::intel::AnalysisEndReason;
use communication_engine::contracts::time::{Clock, SystemClock};
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
    let fake = FakeAudioBackend::new(FakeDriver::Realtime {
        period: Duration::from_millis(5),
    });
    let engine = Engine::builder()
        .audio_backend(Arc::new(fake.clone()))
        .build()
        .map_err(err)?;
    let mut events = EventRecorder::new(engine.subscribe().map_err(err)?);
    engine.start().await.map_err(err)?;
    let session = engine
        .create_session(SessionConfig::default())
        .await
        .map_err(err)?;
    engine
        .update_consent(
            session.id,
            ConsentPurpose::Capture,
            ConsentState::Granted {
                at: SystemClock::new().wall(),
            },
        )
        .await
        .map_err(err)?;
    let mut readers = Vec::new();
    for _ in 0..2 {
        let r = engine
            .open_capture(session.id, StreamRequest::capture())
            .await
            .map_err(err)?;
        engine.start_audio(r.info().id).await.map_err(err)?;
        readers.push(r);
    }
    let (a, b) = (readers[0].info().id, readers[1].info().id);
    let st = engine.start_analysis(a, None).await.map_err(err)?;
    println!(
        "  analysis {} vad {} v{} ({:?}) config {}",
        a,
        st.algorithm.vad,
        st.algorithm.version,
        st.capabilities.vad.score_semantics,
        st.algorithm.config_fingerprint
    );
    check(
        st.running && !st.capabilities.overlap_detection,
        "default analysis running",
    )?;
    engine
        .start_analysis_with(
            b,
            None,
            Box::new(ScriptedVad::new(|i| if i % 120 < 60 { 0.95 } else { 0.0 })),
        )
        .await
        .map_err(err)?;

    let mut delivered = [0usize; 2];
    let until = tokio::time::Instant::now() + Duration::from_millis(2_500);
    while tokio::time::Instant::now() < until {
        for (k, r) in readers.iter_mut().enumerate() {
            while let Ok(Some(_)) = r.try_recv() {
                delivered[k] += 1;
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    println!("  capture blocks delivered while analysing: {delivered:?}");
    check(
        delivered.iter().all(|n| *n > 100),
        "capture unaffected by analysis",
    )?;

    let sa = engine.analysis_status(a).await.map_err(err)?;
    println!(
        "  stream a: {} blocks, {} hops, mode {:?}, quality {:?}",
        sa.counters.blocks,
        sa.counters.hops,
        sa.mode,
        sa.quality
            .as_ref()
            .map(|q| (q.usability, q.level_dbfs.round()))
    );
    check(sa.counters.hops > 100, "default analysis processed audio")?;
    let fb = engine.stop_analysis(b).await.map_err(err)?;
    println!("  stream b: {} segments", fb.counters.segments);
    check(
        fb.counters.segments >= 1,
        "scripted analysis produced segments",
    )?;
    check(
        fb.ended == Some(AnalysisEndReason::Stopped),
        "stop reports its reason",
    )?;
    let report = engine.shutdown().await.map_err(err)?;
    check(report.clean, "clean shutdown")?;
    events.drain_until_closed(Duration::from_secs(5)).await;
    let kinds: std::collections::BTreeMap<&str, usize> = events
        .events()
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Analysis(x) => Some(x.kind()),
            _ => None,
        })
        .fold(Default::default(), |mut m, k| {
            *m.entry(k).or_default() += 1;
            m
        });
    println!("  analysis events: {kinds:?}");
    check(
        kinds.get("stopped") == Some(&2),
        "both analyses reported their end",
    )?;
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => {
            println!("analysis smoke: PASS");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("analysis smoke: {e}");
            ExitCode::FAILURE
        }
    }
}
