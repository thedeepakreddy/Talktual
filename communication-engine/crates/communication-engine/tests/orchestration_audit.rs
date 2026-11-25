//! Phase 7 audit reproductions. `#[ignore]`d so they do not change the
//! gates; run with
//! `cargo test -p communication-engine --test orchestration_audit -- --ignored --nocapture`.
//! They record observed behaviour (fake audio, scripted adapters, manual
//! test clock); see `docs/phases/phase-7-audit.md`. After the audit
//! corrections they assert the **corrected** behaviour (R1, R2, R5); the
//! original observations are kept in the audit report.

#![allow(clippy::print_stderr)]

mod common;

use std::time::{Duration, Instant};

use ce_testkit::fake_audio::FakeDriver;
use common::*;
use communication_engine::contracts::config::EngineConfig;
use communication_engine::contracts::orchestration::StageState;
use communication_engine::contracts::speech::{LanguageTag, SegmentationMode};
use communication_engine::contracts::translation::{PartialPolicy, SourceLanguagePolicy};
use communication_engine::prelude::*;
use communication_engine::speech::fake::{ScriptedFactory as ScriptedAsr, UtteranceScript};
use communication_engine::translation::{Faults, ScriptedFactory, scripted_capabilities};

fn capture() -> StreamRequest {
    StreamRequest {
        format: FormatPreference {
            sample_rate: Some(16_000),
            channels: Some(1),
            sample_format: None,
            buffer_frames: None,
        },
        ..StreamRequest::capture()
    }
}

fn continuous() -> RecognitionConfig {
    let mut c = RecognitionConfig {
        segmentation: SegmentationMode::Continuous,
        partial_interval_ms: 50,
        ..RecognitionConfig::default()
    };
    c.budget.degrade = false;
    c.budget.queue_blocks = 4_000;
    c
}

fn en_es(adapter: &str) -> TranslationConfig {
    TranslationConfig {
        adapter: Some(adapter.into()),
        target: Some(LanguageTag::parse("es").unwrap()),
        source: SourceLanguagePolicy::Explicit {
            language: LanguageTag::parse("en").unwrap(),
        },
        partials: PartialPolicy::Full,
        ..TranslationConfig::default()
    }
}

fn spec(adapter: &str, translation: Requirement, stop: StopMode) -> PipelineSpec {
    let mut s = PipelineSpec::new(vec![
        StageSpec::required(StageKind::Recognition)
            .with_config(StageConfig::Recognition(Box::new(continuous()))),
        StageSpec {
            kind: StageKind::Translation,
            requirement: translation,
            config: Some(StageConfig::Translation(Box::new(en_es(adapter)))),
        },
    ]);
    s.stop = stop;
    s
}

fn asr(n: usize) -> ScriptedAsr {
    ScriptedAsr::new(
        (0..n)
            .map(|_| UtteranceScript::progressive("one two three four", 40).endpoint_at(200))
            .collect(),
    )
}

async fn harness(config: EngineConfig, n: usize, adapters: Vec<ScriptedFactory>) -> AudioHarness {
    audio_harness_custom(config, FakeDriver::Manual, move |mut b| {
        b = b.asr_adapter(asr(n));
        for a in adapters {
            b = b.translation_adapter(a);
        }
        b
    })
    .await
}

async fn open(h: &AudioHarness) -> (CaptureReader, AudioStreamId) {
    let reader = h
        .engine
        .open_capture(h.session.id, capture())
        .await
        .unwrap();
    let id = reader.info().id;
    h.engine.start_audio(id).await.unwrap();
    (reader, id)
}

async fn feed(h: &AudioHarness, r: &mut CaptureReader, blocks: usize) {
    for _ in 0..blocks {
        h.fake.tick(160);
        let _ = recv_block(r).await;
    }
}

fn threads() -> usize {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Threads:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

/// AUD-R1: a stage whose adapter never returns is abandoned at every stop;
/// each abandoned worker keeps its OS thread and (Phase 7) its forwarder
/// task. Repeating the lifecycle grows both without bound.
#[tokio::test]
#[ignore]
async fn r1_hung_workers_accumulate_threads_and_forwarders() {
    let hung = ScriptedFactory::echo(scripted_capabilities("t.hung", &[("en", "es")])).with_faults(
        Faults {
            hang_at: Some((0, Duration::from_secs(120))),
            ..Faults::default()
        },
    );
    let h = harness(audio_config(), 200, vec![hung]).await;
    h.grant_capture().await;
    let (mut r, id) = open(&h).await;
    let t0 = threads();
    let mut rows = Vec::new();
    let cap = audio_config().orchestration.max_abandoned_workers as usize;
    let mut refused = 0;
    for round in 1..=(cap + 4) {
        if let Err(e) = h
            .engine
            .start_pipeline(id, spec("t.hung", Requirement::Required, StopMode::Cancel))
            .await
        {
            assert_eq!(e.code(), "orchestration.abandoned_workers_exhausted");
            refused += 1;
            rows.push((
                round,
                None,
                h.engine.metrics().worker_forwarders,
                threads() as i64 - t0 as i64,
            ));
            continue;
        }
        feed(&h, &mut r, 30).await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        let st = h.engine.stop_pipeline(id).await.unwrap();
        let tr = st
            .stages
            .iter()
            .find(|s| s.kind == StageKind::Translation)
            .unwrap()
            .stop;
        rows.push((
            round,
            tr,
            h.engine.metrics().worker_forwarders,
            threads() as i64 - t0 as i64,
        ));
    }
    eprintln!("round, translation stop outcome, worker_forwarders, threads delta");
    for row in &rows {
        eprintln!("{row:?}");
    }
    // Corrected (AUD-01): growth stops at the cap; further starts are refused.
    let last = rows.last().unwrap();
    assert_eq!(last.2 as usize, cap, "{rows:?}");
    assert_eq!(refused, 4, "{rows:?}");
    assert_eq!(h.engine.metrics().abandoned_workers as usize, cap);
}

/// AUD-R2: an optional stage stopped directly (per-stage command) leaves
/// the pipeline reported as `running` with no degradation event.
#[tokio::test]
#[ignore]
async fn r2_optional_stage_stopped_directly_is_not_reported_as_degraded() {
    let echo = ScriptedFactory::echo(scripted_capabilities("t.echo", &[("en", "es")]));
    let mut h = harness(audio_config(), 4, vec![echo]).await;
    h.grant_capture().await;
    let (mut r, id) = open(&h).await;
    h.engine
        .start_pipeline(id, spec("t.echo", Requirement::Optional, StopMode::Drain))
        .await
        .unwrap();
    feed(&h, &mut r, 20).await;
    h.engine.stop_translation(id).await.unwrap();
    let _ = h
        .events
        .wait_for(Duration::from_millis(300), |_| false)
        .await;
    let st = h.engine.pipeline_status(id).await.unwrap();
    let tr = st
        .stages
        .iter()
        .find(|s| s.kind == StageKind::Translation)
        .unwrap();
    let degraded_events = h
        .events
        .events()
        .iter()
        .filter(|e| {
            matches!(
                &e.payload,
                EventPayload::Orchestration(OrchestrationEvent::StageDegraded { .. })
            )
        })
        .count();
    eprintln!(
        "pipeline state {:?}; translation stage state {:?}; stage_degraded events {degraded_events}",
        st.state, tr.state
    );
    // Corrected (AUD-05).
    assert_eq!(st.state, PipelineState::Degraded);
    assert_eq!(tr.state, StageState::Ended);
    assert_eq!(degraded_events, 1);
}

/// AUD-R3: attempt to reproduce a stale output end to end — restart the
/// pipeline repeatedly while the translation worker is busy (full partials,
/// 2 ms adapter delay) and count rejected stale outputs.
#[tokio::test]
#[ignore]
async fn r3_attempt_stale_output_race_by_rapid_restart() {
    let slow = ScriptedFactory::echo(scripted_capabilities("t.slow", &[("en", "es")])).with_faults(
        Faults {
            delay: Duration::from_millis(2),
            ..Faults::default()
        },
    );
    let h = harness(audio_config(), 400, vec![slow]).await;
    h.grant_capture().await;
    let (mut r, id) = open(&h).await;
    h.engine
        .start_pipeline(id, spec("t.slow", Requirement::Required, StopMode::Cancel))
        .await
        .unwrap();
    for _ in 0..60 {
        feed(&h, &mut r, 12).await;
        h.engine.restart_pipeline(id, None).await.unwrap();
    }
    let m = h.engine.metrics();
    eprintln!(
        "restarts 60; stale_outputs_rejected {}; pipelines_started {}",
        m.stale_outputs_rejected, m.pipelines_started
    );
}

/// AUD-R4: command round-trip latency while two streams' pipelines process
/// a sustained burst (wall clock; fake audio; scripted adapters).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn r4_command_latency_under_load() {
    let echo = ScriptedFactory::echo(scripted_capabilities("t.echo", &[("en", "es")]));
    let h = std::sync::Arc::new(harness(audio_config(), 400, vec![echo]).await);
    h.grant_capture().await;
    let (mut ra, a) = open(&h).await;
    let (mut rb, b) = open(&h).await;
    for id in [a, b] {
        h.engine
            .start_pipeline(id, spec("t.echo", Requirement::Required, StopMode::Drain))
            .await
            .unwrap();
    }
    let h2 = h.clone();
    let feeder = tokio::spawn(async move {
        for _ in 0..1_500 {
            h2.fake.tick(160);
            let _ = recv_block(&mut ra).await;
            let _ = recv_block(&mut rb).await;
        }
    });
    let mut lat = Vec::new();
    while !feeder.is_finished() {
        let t = Instant::now();
        h.engine.pipeline_status(a).await.unwrap();
        lat.push(t.elapsed().as_secs_f64() * 1_000.0);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    feeder.await.unwrap();
    lat.sort_by(f64::total_cmp);
    let p = |q: f64| lat[((q * lat.len() as f64).ceil() as usize).clamp(1, lat.len()) - 1];
    eprintln!(
        "status round trips {}: p50 {:.3} ms, p95 {:.3} ms, max {:.3} ms",
        lat.len(),
        p(0.5),
        p(0.95),
        lat[lat.len() - 1]
    );
    let sa = h.engine.pipeline_status(a).await.unwrap();
    let sb = h.engine.pipeline_status(b).await.unwrap();
    eprintln!(
        "stream a: revisions {} derived {} violations {} anomalies {}; stream b: revisions {} derived {} violations {} anomalies {}",
        sa.counters.transcript_revisions,
        sa.counters.derived_outputs,
        sa.counters.causal_violations,
        sa.counters.timeline_anomalies,
        sb.counters.transcript_revisions,
        sb.counters.derived_outputs,
        sb.counters.causal_violations,
        sb.counters.timeline_anomalies
    );
    for l in &sa.latency {
        eprintln!(
            "stream a latency {:?}: count {} p50 {:.4} ms p95 {:.4} ms max {:.4} ms",
            l.metric, l.count, l.p50_ms, l.p95_ms, l.max_ms
        );
    }
    let m = h.engine.metrics();
    eprintln!(
        "translation_inputs_dropped {}; semantic_inputs_dropped {}; stale {}",
        m.translation_inputs_dropped, m.semantic_inputs_dropped, m.stale_outputs_rejected
    );
}

/// AUD-R5: a restart with a specification whose graph is invalid tears the
/// running pipeline down first and only then fails validation, leaving the
/// stream without a pipeline.
#[tokio::test]
#[ignore]
async fn r5_restart_with_an_invalid_graph_stops_the_running_pipeline_first() {
    let echo = ScriptedFactory::echo(scripted_capabilities("t.echo", &[("en", "es")]));
    let h = harness(audio_config(), 4, vec![echo]).await;
    h.grant_capture().await;
    let (mut r, id) = open(&h).await;
    h.engine
        .start_pipeline(id, spec("t.echo", Requirement::Required, StopMode::Drain))
        .await
        .unwrap();
    feed(&h, &mut r, 10).await;
    let only_translation = PipelineSpec::new(vec![StageSpec {
        kind: StageKind::Translation,
        requirement: Requirement::Required,
        config: Some(StageConfig::Translation(Box::new(en_es("t.echo")))),
    }]);
    let err = h
        .engine
        .restart_pipeline(id, Some(only_translation))
        .await
        .unwrap_err();
    let st = h.engine.pipeline_status(id).await.unwrap();
    eprintln!(
        "restart error {}; pipeline state afterwards {:?} (ended {:?}); recognition running {}",
        err.code(),
        st.state,
        st.ended,
        h.engine.recognition_status(id).await.unwrap().running
    );
    // Corrected (AUD-02): the running pipeline is kept.
    assert_eq!(err.code(), "orchestration.missing_dependency");
    assert_eq!(st.state, PipelineState::Running);
}

/// VA-R6 (verification audit): engine-wide finding metrics vs the
/// pipeline's own counters when more than 32 findings of one kind occur.
#[tokio::test]
#[ignore]
async fn r6_engine_metrics_vs_pipeline_counters_beyond_the_findings_limit() {
    let echo = ScriptedFactory::echo(scripted_capabilities("t.echo", &[("en", "es")]));
    let h = harness(audio_config(), 400, vec![echo]).await;
    h.grant_capture().await;
    let (mut r, id) = open(&h).await;
    let mut s = spec("t.echo", Requirement::Required, StopMode::Drain);
    s.deadlines.transcript_final_ms = Some(1);
    h.engine.start_pipeline(id, s).await.unwrap();
    feed(&h, &mut r, 5).await;
    // Every later final is published long "after" its audio on the test clock.
    h.clock.advance(Duration::from_secs(600));
    feed(&h, &mut r, 2_500).await;
    let st = h.engine.pipeline_status(id).await.unwrap();
    let m = h.engine.metrics();
    eprintln!(
        "pipeline counters: deadline_misses {}, suppressed_findings {}; engine metrics: deadline_misses {}",
        st.counters.deadline_misses, st.counters.suppressed_findings, m.deadline_misses
    );
    assert!(st.counters.deadline_misses > 32, "{:?}", st.counters);
}

/// VA-R7 (verification audit): analysis workers abandoned at a stop
/// timeout versus the abandoned-worker cap.
#[tokio::test]
#[ignore]
async fn r7_abandoned_analysis_workers_versus_the_cap() {
    use communication_engine::audio_intel::ScriptedVad;
    let h = harness(audio_config(), 4, vec![]).await;
    h.grant_capture().await;
    let (mut r, id) = open(&h).await;
    let t0 = threads();
    let mut refused = 0;
    let rounds = 20;
    for _ in 0..rounds {
        let mut cfg = AnalysisConfig::default();
        cfg.budget.degrade = false;
        cfg.budget.queue_blocks = 4_000;
        match h
            .engine
            .start_analysis_with(
                id,
                Some(cfg),
                Box::new(ScriptedVad::new(|_| 0.5).with_delay(Duration::from_secs(3))),
            )
            .await
        {
            Ok(_) => {}
            Err(e) => {
                eprintln!("refused: {}", e.code());
                refused += 1;
                continue;
            }
        }
        feed(&h, &mut r, 3).await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        h.engine.stop_analysis(id).await.unwrap();
    }
    let m = h.engine.metrics();
    eprintln!(
        "rounds {rounds}: refused {refused}; abandoned_workers gauge {}; worker_forwarders {}; analysis_failures {}; threads delta {}",
        m.abandoned_workers,
        m.worker_forwarders,
        m.analysis_failures,
        threads() as i64 - t0 as i64
    );
}
