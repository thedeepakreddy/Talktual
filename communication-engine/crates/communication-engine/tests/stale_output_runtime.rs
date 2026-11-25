//! AUD-03: end-to-end proof that an output of a replaced, reset-surviving
//! or abandoned translation worker is admitted or rejected at the runtime's
//! publication boundary. Requires the `test-hooks` feature (run by
//! `verify.sh` in the all-features test step):
//!
//! `cargo test -p communication-engine --features test-hooks --test stale_output_runtime`
//!
//! Mechanism: the core keeps a clone of each translation worker's **own
//! signal sender** (the sender the worker thread writes to). The test sends
//! a marked result through it. The signal travels the real path — the
//! worker's forwarder task tags it with that worker's generation, the
//! stage's core channel, generation admission in the translation handler,
//! publication — so the test observes exactly what a late signal of that
//! worker would produce. Synchronization is by events (the rejection
//! report or the published marker), not by timing.

#![cfg(feature = "test-hooks")]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ce_testkit::fake_audio::FakeDriver;
use common::*;
use communication_engine::contracts::config::EngineConfig;
use communication_engine::contracts::orchestration::StopOutcome;
use communication_engine::contracts::semantic::Locality;
use communication_engine::contracts::speech::{
    LanguageTag, RevisionStatus as Rs, SegmentationMode,
};
use communication_engine::contracts::translation::{
    PartialPolicy, SourceLanguagePolicy, SourceLanguageProvenance, SourceLanguageRef,
    TranslationProvenance, TranslationSource, TranslationTiming,
};
use communication_engine::prelude::*;
use communication_engine::speech::fake::{ScriptedFactory as ScriptedAsr, UtteranceScript};
use communication_engine::translation::{AdapterOutcome, ScriptedFactory, scripted_capabilities};

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

fn tag(s: &str) -> LanguageTag {
    LanguageTag::parse(s).unwrap()
}

fn spec(adapter: &str, stop: StopMode) -> PipelineSpec {
    let mut rec = RecognitionConfig {
        segmentation: SegmentationMode::Continuous,
        partial_interval_ms: 50,
        ..RecognitionConfig::default()
    };
    rec.budget.degrade = false;
    rec.budget.queue_blocks = 4_000;
    let tr = TranslationConfig {
        adapter: Some(adapter.into()),
        target: Some(tag("es")),
        source: SourceLanguagePolicy::Explicit {
            language: tag("en"),
        },
        partials: PartialPolicy::Full,
        ..TranslationConfig::default()
    };
    let mut s = PipelineSpec::new(vec![
        StageSpec::required(StageKind::Recognition)
            .with_config(StageConfig::Recognition(Box::new(rec))),
        StageSpec::required(StageKind::Translation)
            .with_config(StageConfig::Translation(Box::new(tr))),
    ]);
    s.stop = stop;
    s
}

/// A marked result as a translation worker would emit it.
fn marker(stream: AudioStreamId, text: &str) -> TranslationEvent {
    TranslationEvent::Result(Box::new(TranslationResult {
        sequence: 0,
        state: TranslationState::Final,
        source: TranslationSource {
            stream,
            utterance: 9_999,
            revision: 0,
            revision_status: Rs::Final,
            chars: 1,
            frame_range: (0, 0),
            time_range: Default::default(),
        },
        source_language: SourceLanguageRef {
            language: tag("en"),
            provenance: SourceLanguageProvenance::Explicit,
        },
        target: tag("es"),
        text: text.into(),
        flags: vec![],
        score: None,
        provenance: TranslationProvenance {
            adapter: "t.echo".into(),
            version: "1".into(),
            model_fingerprint: None,
            config_fingerprint: "f".into(),
            locality: Locality::InProcess,
            terminology: None,
            terms_applied: 0,
            context_hints: 0,
            protected_spans: 0,
            fallback_from: None,
        },
        timing: TranslationTiming {
            queued_ms: 0.0,
            adapter_ms: 0.0,
        },
    }))
}

fn published(h: &AudioHarness, text: &str) -> bool {
    h.events.events().iter().any(|e| {
        matches!(&e.payload, EventPayload::Translation(TranslationEvent::Result(r)) if r.text == text)
    })
}

async fn wait_published(h: &mut AudioHarness, text: &str) {
    if published(h, text) {
        return;
    }
    let t = text.to_owned();
    let found = h
        .events
        .wait_for(WAIT, move |e| {
            matches!(&e.payload, EventPayload::Translation(TranslationEvent::Result(r)) if r.text == t)
        })
        .await;
    assert!(found.is_some(), "marker {text} was not published");
}

async fn wait_rejected(h: &mut AudioHarness, generation: u64) -> Option<u64> {
    let pred = |e: &EventEnvelope| matches!(&e.payload, EventPayload::Orchestration(OrchestrationEvent::StaleOutputRejected { generation: g, .. }) if *g == generation);
    let e = match h.events.events().iter().find(|e| pred(e)) {
        Some(e) => e.clone(),
        None => h
            .events
            .wait_for(WAIT, pred)
            .await
            .expect("stale-output rejection reported"),
    };
    match &e.payload {
        EventPayload::Orchestration(OrchestrationEvent::StaleOutputRejected {
            current, ..
        }) => *current,
        _ => unreachable!(),
    }
}

fn translation_generation(st: &PipelineStatus) -> u64 {
    st.stages
        .iter()
        .find(|s| s.kind == StageKind::Translation)
        .and_then(|s| s.generation)
        .unwrap()
}

async fn harness(config: EngineConfig, adapters: Vec<ScriptedFactory>) -> AudioHarness {
    let h = audio_harness_custom(config, FakeDriver::Manual, move |mut b| {
        b = b.asr_adapter(ScriptedAsr::new(
            (0..400)
                .map(|_| UtteranceScript::progressive("one two three four", 40).endpoint_at(200))
                .collect(),
        ));
        for a in adapters {
            b = b.translation_adapter(a);
        }
        b
    })
    .await;
    h.engine.test_retain_translation_senders().await.unwrap();
    h
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

fn echo() -> ScriptedFactory {
    ScriptedFactory::echo(scripted_capabilities("t.echo", &[("en", "es")]))
}

#[tokio::test]
async fn outputs_of_replaced_workers_are_rejected_and_current_ones_published() {
    let mut h = harness(audio_config(), vec![echo()]).await;
    h.grant_capture().await;
    let (mut r, id) = open(&h).await;
    let first = h
        .engine
        .start_pipeline(id, spec("t.echo", StopMode::Drain))
        .await
        .unwrap();
    let g1 = translation_generation(&first);
    feed(&h, &mut r, 30).await;

    // Positive control: the current worker's signal is published, so the
    // rejections below are not an artefact of the injection path.
    h.engine
        .test_inject_translation(id, g1, marker(id, "MARK-CURRENT-G1"))
        .await
        .unwrap();
    wait_published(&mut h, "MARK-CURRENT-G1").await;

    // Reset keeps the worker (same generation): its outputs stay current.
    let reset = h.engine.reset_pipeline(id).await.unwrap();
    assert_eq!(translation_generation(&reset), g1);
    h.engine
        .test_inject_translation(id, g1, marker(id, "MARK-AFTER-RESET"))
        .await
        .unwrap();
    wait_published(&mut h, "MARK-AFTER-RESET").await;

    // Restart replaces the worker: a late signal of the old one is rejected
    // and reported, never published, while the new worker runs.
    let second = h.engine.restart_pipeline(id, None).await.unwrap();
    let g2 = translation_generation(&second);
    assert!(g2 > g1);
    let before = h.engine.metrics().stale_outputs_rejected;
    h.engine
        .test_inject_translation(id, g1, marker(id, "MARK-STALE-G1"))
        .await
        .unwrap();
    assert_eq!(wait_rejected(&mut h, g1).await, Some(g2));
    assert!(!published(&h, "MARK-STALE-G1"));

    // Publication race: old-generation signals interleaved with the new
    // worker's own outputs while audio flows. Each is rejected; the new
    // worker's results are published.
    for i in 0..20 {
        h.engine
            .test_inject_translation(id, g1, marker(id, &format!("MARK-RACE-{i}")))
            .await
            .unwrap();
        feed(&h, &mut r, 3).await;
    }
    let target = before + 21;
    let deadline = Instant::now() + WAIT;
    while h.engine.metrics().stale_outputs_rejected < target {
        assert!(
            Instant::now() < deadline,
            "rejections were not all processed"
        );
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let new_results = h
        .events
        .wait_for(WAIT, |e| {
            matches!(&e.payload, EventPayload::Translation(TranslationEvent::Result(r))
                if r.source.utterance != 9_999)
        })
        .await;
    assert!(new_results.is_some(), "the new worker publishes");
    assert!((0..20).all(|i| !published(&h, &format!("MARK-RACE-{i}"))));
    // The new worker's own retained sender is current: published.
    h.engine
        .test_inject_translation(id, g2, marker(id, "MARK-CURRENT-G2"))
        .await
        .unwrap();
    wait_published(&mut h, "MARK-CURRENT-G2").await;
    assert!(!published(&h, "MARK-STALE-G1"));
    let st = h.engine.pipeline_status(id).await.unwrap();
    assert!(st.counters.stale_outputs_rejected >= 21);
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn outputs_of_abandoned_workers_are_rejected() {
    let entered = Arc::new(AtomicUsize::new(0));
    let e2 = entered.clone();
    let hang = ScriptedFactory::new(scripted_capabilities("t.hang", &[("en", "es")]), move |r| {
        e2.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(1_500));
        AdapterOutcome::Translated {
            text: r.text.to_string(),
            score: None,
            detected_source: None,
        }
    });
    let mut h = harness(audio_config(), vec![hang]).await;
    h.grant_capture().await;
    let (mut r, id) = open(&h).await;
    let st = h
        .engine
        .start_pipeline(id, spec("t.hang", StopMode::Cancel))
        .await
        .unwrap();
    let g = translation_generation(&st);
    while entered.load(Ordering::SeqCst) == 0 {
        feed(&h, &mut r, 5).await;
    }
    let st = h.engine.stop_pipeline(id).await.unwrap();
    let tr = st
        .stages
        .iter()
        .find(|s| s.kind == StageKind::Translation)
        .unwrap();
    assert_eq!(tr.stop, Some(StopOutcome::TimedOut), "abandoned");
    // A late signal of the abandoned worker: rejected, no current worker.
    h.engine
        .test_inject_translation(id, g, marker(id, "MARK-ABANDONED"))
        .await
        .unwrap();
    assert_eq!(wait_rejected(&mut h, g).await, None);
    assert!(!published(&h, "MARK-ABANDONED"));
    h.engine.shutdown().await.unwrap();
}

/// VA-02: causal violations beyond the per-kind event limit. Each injected
/// result (current generation, so it is published) refers to a revision
/// that was never published: one `unknown_source` violation each, plus
/// `sequence_regression` for the repeated sequence. At most 32 events of a
/// kind are published; the pipeline counter and the engine metric count
/// every violation.
#[tokio::test]
async fn causal_violation_metrics_are_complete_beyond_the_event_limit() {
    let mut h = harness(audio_config(), vec![echo()]).await;
    h.grant_capture().await;
    let (mut r, id) = open(&h).await;
    let st = h
        .engine
        .start_pipeline(id, spec("t.echo", StopMode::Drain))
        .await
        .unwrap();
    let g = translation_generation(&st);
    feed(&h, &mut r, 5).await;
    for i in 0..40 {
        h.engine
            .test_inject_translation(id, g, marker(id, &format!("MARK-UNKNOWN-{i}")))
            .await
            .unwrap();
    }
    wait_published(&mut h, "MARK-UNKNOWN-39").await;
    let fin = h.engine.stop_pipeline(id).await.unwrap();
    let c = &fin.counters;
    assert!(c.causal_violations >= 40 + 39, "{c:?}");
    assert_eq!(
        h.engine.metrics().causal_violations,
        c.causal_violations,
        "engine metric = complete count"
    );
    let _ = h
        .events
        .wait_for(Duration::from_millis(200), |_| false)
        .await;
    let unknown_events = h
        .events
        .events()
        .iter()
        .filter(|e| {
            matches!(
                &e.payload,
                EventPayload::Orchestration(OrchestrationEvent::CausalViolation {
                    kind: communication_engine::contracts::orchestration::CausalKind::UnknownSource,
                    ..
                })
            )
        })
        .count();
    assert_eq!(unknown_events, 32, "publication limited per kind");
    assert!(c.suppressed_findings >= 40 - 32);
    h.engine.shutdown().await.unwrap();
}
